//! Local traffic observations, surface conflict holds and optional simulated pilot reports.
pub mod advisories;
mod conversation;
#[cfg(test)]
mod conversation_tests;
mod policy;
mod wake;
use crate::{EngineState, Shared};
use axum::{
    Json,
    extract::State as AxumState,
    response::{IntoResponse, Response},
};
use openatc_core::{
    apply::add_transmission,
    dialogue::say,
    state::{PhaseCode, SpeechTag, TrafficTarget},
    stations::{Station, nearby, tuned},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Landing {
    vacated: bool,
    airport: String,
    runway: String,
    weather: Option<Value>,
    heading: f64,
    at: Instant,
}
struct Pilot {
    target: TrafficTarget,
    seen: Instant,
    landing: Option<Landing>,
    voice: String,
    climb_fpm: Option<f64>,
    conversation: std::collections::VecDeque<(String, String)>,
}
#[derive(Default)]
pub struct Traffic {
    wake: wake::Monitor,
    advisories: BTreeMap<u32, advisories::Call>,
    last_advisory: Option<Instant>,
    last_urgent: Option<Instant>,
    pilots: BTreeMap<u32, Pilot>,
    observed: Option<Instant>,
    report: Option<(u32, String)>,
    last_radio: Option<Instant>,
    clear_since: Option<Instant>,
    speed: Option<(f64, Option<f64>)>,
    speed_declined: bool,
    speed_sequence: u32,
    chat_pending: bool,
    last_chat: Option<Instant>,
    last_chat_pilot: u32,
    policy: Option<policy::Chat>,
}

pub(crate) fn departure_wake_delay(
    state: &EngineState,
    id: u32,
    airport: &openatc_core::airport::Airport,
    runway: &str,
    aircraft: &str,
    latitude: f64,
    longitude: f64,
) -> f64 {
    state
        .traffic
        .wake
        .departure_delay(id, airport, runway, aircraft, latitude, longitude)
}

pub fn observe(state: &mut EngineState) {
    wake::observe(state);
    crate::generated::refresh_departure_wake(state);
    if state.session.telemetry.paused
        || state
            .traffic
            .observed
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
    {
        return;
    }
    state.traffic.observed = Some(Instant::now());
    state
        .traffic
        .pilots
        .retain(|_, p| p.seen.elapsed() < Duration::from_secs(120));
    let Some(targets) = &state.session.telemetry.traffic else {
        state.generated.observed_climbs(BTreeMap::new());
        return;
    };
    for target in targets
        .iter()
        .take(126)
        .filter(|t| t.id != 0 && !t.callsign.is_empty())
    {
        if !openatc_core::traffic::advisory::valid(target) {
            continue;
        }
        let climb_fpm = state.traffic.pilots.get(&target.id).and_then(|old| {
            let elapsed = old.seen.elapsed().as_secs_f64();
            if old.target.callsign != target.callsign || !(0.5..5.0).contains(&elapsed) {
                return None;
            }
            let speed = (target.altitude_feet - old.target.altitude_feet) * 60.0 / elapsed;
            (speed.abs() <= 15000.0).then_some(speed)
        });
        let landed = state.traffic.pilots.get(&target.id).is_some_and(|p| {
            p.target.callsign == target.callsign
                && !p.target.on_ground
                && target.on_ground
                && p.seen.elapsed() < Duration::from_secs(5)
        });
        let landing = if landed {
            state
                .arrival_airport
                .iter()
                .chain(state.airport.iter())
                .find_map(|airport| {
                    airport.runways.iter().find_map(|runway| {
                        let p = openatc_core::traffic::runway_position(
                            airport,
                            &runway.first_name,
                            target.latitude,
                            target.longitude,
                        )?;
                        if !(0.0..=p.2).contains(&p.0) || p.1.abs() > runway.width / 2.0 + 10.0 {
                            return None;
                        }
                        let first =
                            openatc_core::navigation::turn(target.track_degrees, p.3).abs() < 90.0;
                        Some(Landing {
                            vacated: false,
                            airport: airport.icao.clone(),
                            runway: if first {
                                runway.first_name.clone()
                            } else {
                                runway.second_name.clone()
                            },
                            heading: if first { p.3 } else { (p.3 + 180.0) % 360.0 },
                            weather: state
                                .sim_weather
                                .get(&airport.icao)
                                .filter(|(_, at)| at.elapsed().as_secs() < 45)
                                .map(|(value, _)| value.clone()),
                            at: Instant::now(),
                        })
                    })
                })
        } else {
            None
        };
        let voice = state
            .traffic
            .pilots
            .get(&target.id)
            .filter(|p| p.target.callsign == target.callsign)
            .map(|p| p.voice.clone())
            .unwrap_or_else(|| {
                let available: Vec<_> = state
                    .voices
                    .iter()
                    .filter(|v| !state.settings.english_voices_only || v.english)
                    .collect();
                available
                    .iter()
                    .min_by_key(|v| {
                        let reserved = state
                            .controllers
                            .assignments
                            .values()
                            .any(|c| c.voice == v.id)
                            || [
                                &state.settings.pilot_voice,
                                &state.settings.copilot_voice,
                                &state.settings.attendant_voice,
                                &state.settings.ground_voice,
                            ]
                            .contains(&&v.id);
                        (
                            reserved,
                            state
                                .traffic
                                .pilots
                                .values()
                                .filter(|p| p.voice == v.id)
                                .count(),
                        )
                    })
                    .map_or_else(|| state.settings.pilot_voice.clone(), |v| v.id.clone())
            });
        let old = state
            .traffic
            .pilots
            .remove(&target.id)
            .filter(|p| p.target.callsign == target.callsign);
        let conversation = old
            .as_ref()
            .map(|p| p.conversation.clone())
            .unwrap_or_default();
        let mut landing = landing.or_else(|| old.and_then(|p| p.landing));
        if let Some(l) = &mut landing
            && !l.vacated
            && target.on_ground
            && l.at.elapsed().as_secs() < 300
            && let Some(airport) = state
                .arrival_airport
                .iter()
                .chain(state.airport.iter())
                .find(|a| a.icao == l.airport)
        {
            let observed = openatc_core::state::Telemetry {
                aircraft_icao: target.aircraft.clone(),
                latitude: target.latitude,
                longitude: target.longitude,
                heading_degrees: target.track_degrees,
                wingspan_meters: target.wingspan_meters,
                ..Default::default()
            };
            if !openatc_core::airport::aircraft_on_runway(airport, &observed) {
                l.vacated = true;
                state.traffic.report = Some((target.id, l.airport.clone()));
            }
        }
        if state.traffic.pilots.len() < 126 {
            state.traffic.pilots.insert(
                target.id,
                Pilot {
                    climb_fpm,
                    conversation,
                    target: target.clone(),
                    seen: Instant::now(),
                    landing,
                    voice,
                },
            );
        }
    }
    state.generated.observed_climbs(
        state
            .traffic
            .pilots
            .iter()
            .filter_map(|(&id, pilot)| {
                pilot
                    .climb_fpm
                    .filter(|_| pilot.seen.elapsed() < Duration::from_secs(5))
                    .map(|rate| (id, rate))
            })
            .collect(),
    );
}

pub(crate) fn pilot_voice(state: &EngineState, id: u32) -> String {
    state.traffic.pilots.get(&id).map_or_else(
        || state.settings.pilot_voice.clone(),
        |pilot| pilot.voice.clone(),
    )
}

pub(crate) fn ready(state: &EngineState) -> bool {
    !state.session.telemetry.paused
        && state.session.telemetry.radio_power
        && !state.session.telemetry.radio_busy
        && state.pending_copilot_reply.is_none()
        && !state
            .session
            .telemetry
            .radio_sequence_seen
            .is_some_and(|seen| {
                state
                    .session
                    .transcript
                    .iter()
                    .any(|t| t.sequence > seen && !t.background)
            })
        && !state
            .session
            .transcript
            .iter()
            .rev()
            .find(|t| t.is_controller_instruction())
            .is_some_and(|t| !t.pilot_reply.is_empty())
}
fn atc(
    state: &mut EngineState,
    station: &Station,
    key: &str,
    slots: &[(&str, String)],
    pilot_key: &str,
) {
    let voice = crate::flight::tag(state, station);
    add_transmission(&mut state.session, "ATC", &say(key, slots), &voice);
    if !pilot_key.is_empty()
        && let Some(last) = state.session.transcript.last_mut()
    {
        last.pilot_reply = say(pilot_key, slots);
    }
}

/// Called after higher-priority flight instructions, never from the render callback.
pub fn tick(state: &mut EngineState) {
    if !ready(state) || !state.session.telemetry.position_valid {
        return;
    }
    let stations = nearby(&state.stations, &state.session.telemetry);
    let Some(station) = state
        .session
        .control_frequency()
        .and_then(|frequency| tuned(&stations, frequency))
        .cloned()
    else {
        return;
    };
    if advisories::tick(state, &station) || surface(state, &station) || spacing(state, &station) {
        return;
    }
    if conversation::initiate(state) {
        return;
    }
    if !state.settings.hear_traffic_radio
        || state
            .traffic
            .last_radio
            .is_some_and(|at| at.elapsed().as_secs() < 45)
    {
        return;
    }
    let Some((id, airport)) = state.traffic.report.take() else {
        return;
    };
    if station.airport != airport || !["Tower", "Ground"].contains(&station.service.as_str()) {
        return;
    }
    let Some(pilot) = state
        .traffic
        .pilots
        .get(&id)
        .filter(|p| p.seen.elapsed().as_secs() < 5)
    else {
        return;
    };
    let Some(landing) = pilot
        .landing
        .as_ref()
        .filter(|l| l.vacated && l.at.elapsed().as_secs() < 300)
    else {
        return;
    };
    let text = say(
        "traffic_vacated_report",
        &[
            ("traffic", pilot.target.callsign.clone()),
            ("runway", landing.runway.clone()),
        ],
    );
    let callsign = pilot.target.callsign.clone();
    let voice = pilot.voice.clone();
    transmit(state, &callsign, &voice, &text);
    state.traffic.last_radio = Some(Instant::now());
}

fn surface(state: &mut EngineState, station: &Station) -> bool {
    let route = &state.session.taxi_clearance;
    if !matches!(state.session.phase, PhaseCode::Taxi | PhaseCode::TaxiIn)
        || !route.approved
        || route.pending_readback
        || !route.crossing_runway.is_empty()
        || !["Ground", "Tower"].contains(&station.service.as_str())
        || station.airport != route.airport
    {
        return false;
    }
    let Some(airport) = &state.airport else {
        return false;
    };
    if !route.traffic_hold
        && !route.hold_short_runway.is_empty()
        && openatc_core::airport::taxi_hold_reached(airport, route, &state.session.telemetry)
    {
        return false;
    }
    let conflict = openatc_core::traffic::taxi_conflict(airport, &state.session.telemetry);
    if let Some(target) = conflict {
        state.traffic.clear_since = None;
        if route.traffic_hold {
            return true;
        }
        let traffic = if target.callsign.is_empty() {
            say("traffic_unidentified", &[])
        } else {
            target.callsign.clone()
        };
        state.session.taxi_clearance.traffic_hold = true;
        atc(
            state,
            station,
            "traffic_taxi_hold",
            &[
                ("callsign", state.session.plan.callsign.clone()),
                ("traffic", traffic),
            ],
            "pilot_traffic_hold",
        );
        return true;
    }
    if route.traffic_hold {
        if state.session.telemetry.traffic.is_none() {
            return true;
        }
        let since = state.traffic.clear_since.get_or_insert_with(Instant::now);
        if since.elapsed().as_secs() < 3 {
            return true;
        }
        let instructions = route.instructions.clone();
        state.session.taxi_clearance.traffic_hold = false;
        state.traffic.clear_since = None;
        atc(
            state,
            station,
            "traffic_taxi_resume",
            &[
                ("callsign", state.session.plan.callsign.clone()),
                ("instructions", instructions),
            ],
            "pilot_traffic_resume",
        );
        return true;
    }
    false
}

/// Approach clearances here do not restate a prior assigned speed.
pub(crate) fn clear_approach_speed(state: &mut EngineState) -> Option<(String, String)> {
    let assigned = state.flow.monitor.speed.take().is_some();
    state.traffic.speed = None;
    state.traffic.speed_sequence = 0;
    if !assigned {
        return None;
    }
    let published = state
        .session
        .plan
        .fixes
        .iter()
        .skip(state.flow.monitor.next_fix)
        .any(|fix| fix.maximum_speed_knots.is_some());
    Some((
        say(
            if published {
                "arrival_resume_published_speed"
            } else {
                "arrival_resume_normal_speed"
            },
            &[],
        ),
        say(
            if published {
                "pilot_resume_published_speed_instruction"
            } else {
                "pilot_resume_normal_speed_instruction"
            },
            &[],
        ),
    ))
}

fn spacing(state: &mut EngineState, station: &Station) -> bool {
    let Some(profiles) = state.traffic_profiles.current() else {
        return false;
    };
    let policy = &profiles.spacing;
    if state.session.emergency_declared
        || !["Approach", "Tower"].contains(&station.service.as_str())
        || state.session.telemetry.on_ground
        || !state.flow.approach_cleared
    {
        return false;
    }
    let Some(airport) = state.arrival_airport.as_ref() else {
        return false;
    };
    let t = &state.session.telemetry;
    let Some(position) = openatc_core::traffic::runway_position(
        airport,
        &state.session.plan.arrival_runway,
        t.latitude,
        t.longitude,
    ) else {
        return false;
    };
    let distance = -position.0 / 1852.0;
    if position.1.abs() > policy.lateral_meters
        || openatc_core::navigation::turn(
            t.ground_track_degrees.unwrap_or(t.heading_degrees),
            position.3,
        )
        .abs()
            > policy.alignment_degrees
    {
        return false;
    }
    if state.traffic.speed.is_some() && distance <= policy.release_nm {
        let (issued, prior) = state.traffic.speed.take().unwrap();
        if state.flow.monitor.speed != Some(issued) {
            return false;
        }
        state.flow.monitor.speed = prior;
        let slots = [("callsign", state.session.plan.callsign.clone())];
        let published = state
            .session
            .plan
            .fixes
            .iter()
            .skip(state.flow.monitor.next_fix)
            .any(|fix| fix.maximum_speed_knots.is_some());
        atc(
            state,
            station,
            if published {
                "traffic_resume_published_speed"
            } else {
                "traffic_resume_normal_speed"
            },
            &slots,
            if published {
                "pilot_resume_published_speed"
            } else {
                "pilot_resume_normal_speed"
            },
        );
        return true;
    }
    if state.traffic.speed_declined
        || state.traffic.speed.is_some()
        || !(policy.start_nm..=policy.end_nm).contains(&distance)
        || state.flow.monitor.speed.is_some()
        || t.indicated_altitude_feet
            .unwrap_or(t.altitude_feet)
            .max(t.altitude_feet)
            >= policy.maximum_altitude_feet
    {
        return false;
    }
    let aircraft = if t.aircraft_icao.is_empty() {
        &state.session.plan.aircraft
    } else {
        &t.aircraft_icao
    };
    let Some(envelope) = profiles
        .aircraft_for(aircraft)
        .and_then(|aircraft| aircraft.spacing_speed.as_ref())
    else {
        return false;
    };
    let Some(ias) = t
        .indicated_speed_knots
        .filter(|s| (envelope.minimum_knots..=envelope.maximum_knots).contains(s))
    else {
        return false;
    };
    let Some(targets) = &t.traffic else {
        return false;
    };
    let mut ahead = false;
    let mut reduction = None;
    let mut increase_candidate = None;
    for p in targets.iter().take(126).filter(|p| !p.on_ground) {
        let candidate = (|| {
            let other = openatc_core::traffic::runway_position(
                airport,
                &state.session.plan.arrival_runway,
                p.latitude,
                p.longitude,
            )?;
            if other.1.abs() > policy.lateral_meters
                || (p.altitude_feet - t.altitude_feet).abs() > policy.vertical_feet
                || openatc_core::navigation::turn(p.track_degrees, position.3).abs()
                    > policy.alignment_degrees
            {
                return None;
            }
            let gap = (other.0 - position.0) / 1852.0;
            if gap > 0.0 && gap <= policy.gap_max_nm {
                ahead = true;
            }
            let speed = p.ground_speed_knots?;
            if (policy.gap_min_nm..policy.gap_max_nm).contains(&gap)
                && t.ground_speed_knots > speed + policy.closing_knots
            {
                Some((
                    envelope.minimum_knots.max(ias - policy.speed_step_knots),
                    false,
                ))
            } else if (-policy.gap_max_nm..-policy.gap_min_nm).contains(&gap)
                && speed > t.ground_speed_knots + policy.closing_knots
            {
                Some((
                    envelope.maximum_knots.min(ias + policy.speed_step_knots),
                    true,
                ))
            } else {
                None
            }
        })();
        if let Some((speed, increase)) = candidate {
            if increase {
                increase_candidate = Some((speed, true));
            } else {
                reduction = Some((speed, false));
            }
        }
    }
    let candidate = reduction.or_else(|| if ahead { None } else { increase_candidate });
    let Some((speed, increase)) = candidate else {
        return false;
    };
    let minimum = (envelope.minimum_knots / 5.0).ceil() * 5.0;
    let maximum = (envelope.maximum_knots / 5.0).floor() * 5.0;
    if minimum > maximum {
        return false;
    }
    let speed = ((speed / policy.speed_step_knots).round() * policy.speed_step_knots)
        .clamp(minimum, maximum);
    if (speed - ias).abs() < policy.minimum_change_knots
        || (increase && speed <= ias)
        || (!increase && speed >= ias)
        || state
            .session
            .plan
            .fixes
            .iter()
            .skip(state.flow.monitor.next_fix)
            .take(2)
            .any(|f| f.maximum_speed_knots.is_some_and(|v| speed > v))
    {
        return false;
    }
    state.traffic.speed = Some((speed, state.flow.monitor.speed));
    state.flow.monitor.speed = Some(speed);
    let slots = [
        ("callsign", state.session.plan.callsign.clone()),
        ("speed", format!("{speed:.0}")),
    ];
    atc(
        state,
        station,
        if increase {
            "traffic_increase_speed"
        } else {
            "traffic_reduce_speed"
        },
        &slots,
        "pilot_speed_readback",
    );
    state.traffic.speed_sequence = state.session.transcript.last().map_or(0, |t| t.sequence);
    true
}

pub fn unable(
    state: &mut EngineState,
    station: &Station,
    request: &openatc_core::state::Request,
) -> bool {
    if state
        .session
        .transcript
        .iter()
        .rev()
        .find(|t| t.is_controller_instruction())
        .is_none_or(|t| t.sequence != state.traffic.speed_sequence)
    {
        return false;
    }
    let Some((issued, prior)) = state.traffic.speed.take() else {
        return false;
    };
    state.traffic.speed_declined = true;
    if state.flow.monitor.speed == Some(issued) {
        state.flow.monitor.speed = prior;
    }
    if let Some(last) = state
        .session
        .transcript
        .iter_mut()
        .rev()
        .find(|t| t.is_controller_instruction())
    {
        last.pilot_reply.clear();
    }
    if !request.controller_initiated {
        let callsign = state.session.plan.callsign.clone();
        add_transmission(
            &mut state.session,
            &callsign,
            &request.text,
            &SpeechTag {
                radio: Some(request.radio),
                ..Default::default()
            },
        );
    }
    atc(
        state,
        station,
        "traffic_unable_speed",
        &[("callsign", state.session.plan.callsign.clone())],
        "",
    );
    if let Some(reply) = state.session.transcript.last_mut() {
        reply.radio = Some(request.radio);
    }
    true
}

fn transmit(state: &mut EngineState, callsign: &str, voice: &str, text: &str) {
    let spoken_text = state.generated.spoken_text(text);
    let spoken_callsign = state.generated.spoken_text(callsign);
    add_transmission(
        &mut state.session,
        &spoken_callsign,
        &spoken_text,
        &SpeechTag {
            position: "Traffic".into(),
            voice: voice.into(),
            ..Default::default()
        },
    );
    if let Some(last) = state.session.transcript.last_mut() {
        last.background = true;
    }
}

pub async fn talk(AxumState(shared): AxumState<Shared>, Json(body): Json<Value>) -> Response {
    if body["message"]
        .as_str()
        .is_some_and(|text| !text.trim().is_empty())
    {
        return conversation::request(shared, body).await;
    }
    let mut state = shared.write().await;
    if !state.settings.talk_to_traffic {
        return crate::routes::bad_request(
            "Enable conversations with simulated traffic pilots first.",
        );
    }
    if !ready(&state) {
        return crate::routes::bad_request("Wait for the current radio exchange to finish.");
    }
    let id = body["id"].as_u64().unwrap_or(0) as u32;
    let topic = body["topic"].as_str().unwrap_or("");
    if !["weather", "landing", "crosswind"].contains(&topic) {
        return crate::routes::bad_request("Ask about weather, landing or crosswind.");
    }
    let Some(pilot) = state
        .traffic
        .pilots
        .get(&id)
        .filter(|p| p.seen.elapsed().as_secs() < 5)
    else {
        return crate::routes::bad_request(
            "That aircraft is no longer in the current traffic scan.",
        );
    };
    if openatc_core::ops::distance_nm(
        state.session.telemetry.latitude,
        state.session.telemetry.longitude,
        pilot.target.latitude,
        pilot.target.longitude,
    ) > 20.0
    {
        return crate::routes::bad_request(
            "That aircraft is outside local traffic conversation range.",
        );
    }
    let callsign = pilot.target.callsign.clone();
    let voice = pilot.voice.clone();
    let landing = pilot
        .landing
        .as_ref()
        .filter(|l| l.at.elapsed().as_secs() < 300);
    let text = if topic == "landing" {
        if let Some(l) = landing {
            say("traffic_landing_observed", &[("runway", l.runway.clone())])
        } else {
            say("traffic_landing_unknown", &[])
        }
    } else if topic == "crosswind" {
        if let Some(l) = landing
            && let Some(weather) = &l.weather
            && let Some((component, right)) = weather["windDegrees"]
                .as_f64()
                .zip(weather["windKnots"].as_f64())
                .and_then(|(d, s)| openatc_core::traffic::crosswind(d, s, l.heading))
        {
            say(
                "traffic_crosswind_observed",
                &[
                    ("runway", l.runway.clone()),
                    ("speed", format!("{component:.0}")),
                    (
                        "side",
                        say(
                            if component < 1.0 {
                                "traffic_negligible_crosswind"
                            } else if right {
                                "traffic_from_right"
                            } else {
                                "traffic_from_left"
                            },
                            &[],
                        ),
                    ),
                ],
            )
        } else {
            say("traffic_crosswind_unknown", &[])
        }
    } else {
        let nearby = state
            .airport
            .iter()
            .chain(state.arrival_airport.iter())
            .filter(|a| {
                openatc_core::ops::distance_nm(
                    pilot.target.latitude,
                    pilot.target.longitude,
                    a.reference_latitude,
                    a.reference_longitude,
                ) < 5.0
            })
            .find_map(|a| {
                state
                    .sim_weather
                    .get(&a.icao)
                    .filter(|(_, at)| at.elapsed().as_secs() < 45)
                    .map(|(w, _)| (a, w))
            });
        if let Some((a, w)) = nearby
            && let Some((direction, speed)) = w["windDegrees"].as_f64().zip(w["windKnots"].as_f64())
            && let Some(visibility) = w["visibilityMeters"].as_f64()
        {
            say(
                "traffic_weather_observed",
                &[
                    ("airport", a.name.clone()),
                    ("direction", format!("{direction:03.0}")),
                    ("speed", format!("{speed:.0}")),
                    ("visibility", format!("{:.0}", visibility / 1000.0)),
                ],
            )
        } else {
            say("traffic_weather_unknown", &[])
        }
    };
    let call = state.session.plan.callsign.clone();
    let tag = SpeechTag {
        voice: state.settings.pilot_voice.clone(),
        ..Default::default()
    };
    let request = say(
        &format!("pilot_traffic_{topic}"),
        &[("callsign", call.clone()), ("traffic", callsign.clone())],
    );
    let phase = state.session.phase;
    let record = state.request_memory.begin("traffic", phase, &request);
    add_transmission(&mut state.session, &call, &request, &tag);
    transmit(&mut state, &callsign, &voice, &text);
    state.request_memory.finish(
        record,
        &json!({"result":{"accepted":true,"message":text}}),
        &[],
    );
    Json(json!({"accepted":true})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use openatc_core::{
        airport::{Airport, Runway},
        state::{Point, TaxiClearance, Telemetry},
    };
    fn airport() -> Airport {
        Airport {
            icao: "TEST".into(),
            runways: vec![Runway {
                first_name: "36".into(),
                second_name: "18".into(),
                second: Point {
                    north: 2000.0,
                    ..Default::default()
                },
                width: 45.0,
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    #[test]
    fn taxi_hold_releases_only_after_clear_scan_and_preserves_runway_limit() {
        let mut s = crate::test_state();
        s.airport = Some(Airport {
            icao: "TEST".into(),
            ..Default::default()
        });
        s.session.phase = PhaseCode::Taxi;
        s.session.taxi_clearance = TaxiClearance {
            airport: "TEST".into(),
            approved: true,
            instructions: "Taxi via A, hold short of runway 36.".into(),
            hold_short_runway: "36".into(),
            ..Default::default()
        };
        s.session.telemetry = Telemetry {
            on_ground: true,
            heading_degrees: 0.0,
            ground_speed_knots: 10.0,
            traffic: Some(vec![TrafficTarget {
                id: 4,
                callsign: "QFA1453".into(),
                latitude: 0.0005,
                longitude: 0.0006,
                on_ground: true,
                track_degrees: 270.0,
                ground_speed_knots: Some(10.0),
                ..Default::default()
            }]),
            ..Default::default()
        };
        let station = Station {
            airport: "TEST".into(),
            service: "Ground".into(),
            ..Default::default()
        };
        assert!(surface(&mut s, &station));
        assert!(s.session.taxi_clearance.traffic_hold);
        assert!(
            s.session
                .transcript
                .last()
                .unwrap()
                .text
                .contains("QFA1453")
        );
        let count = s.session.transcript.len();
        assert!(surface(&mut s, &station));
        assert_eq!(s.session.transcript.len(), count);
        s.session.telemetry.traffic = None;
        assert!(surface(&mut s, &station));
        assert!(s.session.taxi_clearance.traffic_hold);
        s.session.telemetry.traffic = Some(vec![]);
        assert!(surface(&mut s, &station));
        assert!(s.session.taxi_clearance.traffic_hold);
        s.traffic.clear_since = Some(Instant::now() - Duration::from_secs(4));
        assert!(surface(&mut s, &station));
        assert!(!s.session.taxi_clearance.traffic_hold);
        assert_eq!(s.session.taxi_clearance.hold_short_runway, "36");
        assert!(s.session.taxi_clearance.crossing_runway.is_empty());
    }
    #[test]
    fn mixed_spacing_prioritises_the_leader_and_respects_each_aircraft_envelope() {
        for (code, ias, minimum) in [
            ("C172", 90.0, 80.0),
            ("DH8D", 160.0, 135.0),
            ("A320", 180.0, 170.0),
        ] {
            let make = |airspeed| {
                let mut state = crate::test_state();
                state.arrival_airport = Some(airport());
                state.flow.approach_cleared = true;
                state.session.plan.arrival_runway = "36".into();
                state.session.plan.aircraft = code.into();
                state.session.telemetry = Telemetry {
                    latitude: -0.18,
                    on_ground: false,
                    ground_speed_knots: airspeed,
                    indicated_speed_knots: Some(airspeed),
                    traffic: Some(vec![
                        TrafficTarget {
                            id: 1,
                            latitude: -0.25,
                            aircraft: "A320".into(),
                            on_ground: false,
                            ground_speed_knots: Some(230.0),
                            ..Default::default()
                        },
                        TrafficTarget {
                            id: 2,
                            latitude: -0.11,
                            aircraft: "PA28".into(),
                            on_ground: false,
                            ground_speed_knots: Some(airspeed - 30.0),
                            ..Default::default()
                        },
                    ]),
                    ..Default::default()
                };
                state
            };
            let station = Station {
                service: "Approach".into(),
                ..Default::default()
            };
            let mut state = make(ias);
            assert!(spacing(&mut state, &station), "{code}");
            assert_eq!(state.flow.monitor.speed, Some(ias - 10.0));
            let mut below_floor = make(minimum - 10.0);
            assert!(
                !spacing(&mut below_floor, &station),
                "{code}: must not issue reduce-to-higher-speed"
            );
            assert!(below_floor.flow.monitor.speed.is_none());
            let mut clear_ahead = make(ias);
            clear_ahead
                .session
                .telemetry
                .traffic
                .as_mut()
                .unwrap()
                .pop();
            assert!(
                spacing(&mut clear_ahead, &station),
                "{code}: feasible increase"
            );
            assert_eq!(clear_ahead.flow.monitor.speed, Some(ias + 10.0));
            for published in [false, true] {
                let mut release = make(ias);
                release.session.telemetry.latitude = -0.08;
                release.flow.monitor.speed = Some(ias + 10.0);
                release.traffic.speed = Some((ias + 10.0, None));
                if published {
                    release
                        .session
                        .plan
                        .fixes
                        .push(openatc_core::state::RouteFix {
                            maximum_speed_knots: Some(ias),
                            ..Default::default()
                        });
                }
                assert!(spacing(&mut release, &station));
                assert!(release.flow.monitor.speed.is_none());
                let text = &release.session.transcript.last().unwrap().text;
                assert!(text.contains(if published {
                    "published"
                } else {
                    "normal speed"
                }));
            }
        }
    }
    #[test]
    fn spacing_obeys_published_speed_short_final_and_unable() {
        let mut s = crate::test_state();
        s.arrival_airport = Some(airport());
        s.flow.approach_cleared = true;
        s.session.plan.arrival_runway = "36".into();
        s.session.plan.aircraft = "A320".into();
        s.session.telemetry = Telemetry {
            latitude: -0.18,
            on_ground: false,
            ground_speed_knots: 170.0,
            indicated_speed_knots: Some(170.0),
            traffic: Some(vec![TrafficTarget {
                latitude: -0.25,
                on_ground: false,
                ground_speed_knots: Some(210.0),
                ..Default::default()
            }]),
            ..Default::default()
        };
        let station = Station {
            service: "Approach".into(),
            ..Default::default()
        };
        s.session.plan.fixes = vec![openatc_core::state::RouteFix {
            maximum_speed_knots: Some(170.0),
            ..Default::default()
        }];
        assert!(!spacing(&mut s, &station));
        s.session.plan.fixes.clear();
        assert!(spacing(&mut s, &station));
        assert_eq!(s.flow.monitor.speed, Some(180.0));
        assert!(unable(
            &mut s,
            &station,
            &openatc_core::state::Request {
                text: "Unable that speed".into(),
                ..Default::default()
            }
        ));
        assert!(s.flow.monitor.speed.is_none());
        assert!(!spacing(&mut s, &station));
        assert_eq!(
            s.session
                .transcript
                .iter()
                .filter(|t| t.text == "Unable that speed")
                .count(),
            1
        );
        // A pilot's own assigned speed is not replaced by automatic spacing.
        s.flow.monitor.speed = Some(160.0);
        assert!(!spacing(&mut s, &station));
        s.flow.monitor.speed = None;
        s.session.telemetry.latitude = -0.03;
        assert!(!spacing(&mut s, &station));
    }
    #[tokio::test]
    async fn traffic_talk_requires_current_target_and_remembers_observed_landing_weather() {
        let mut s = crate::test_state();
        s.airport = Some(airport());
        s.settings.talk_to_traffic = true;
        s.session.telemetry = Telemetry {
            radio_power: true,
            latitude: 0.001,
            traffic: Some(vec![TrafficTarget {
                id: 42,
                callsign: "QFA42".into(),
                latitude: -0.001,
                track_degrees: 0.0,
                ..Default::default()
            }]),
            ..Default::default()
        };
        s.sim_weather.insert(
            "TEST".into(),
            (
                json!({"windDegrees":90.0,"windKnots":20.0,"visibilityMeters":10000.0}),
                Instant::now(),
            ),
        );
        observe(&mut s);
        s.traffic.observed = None;
        {
            let target = &mut s.session.telemetry.traffic.as_mut().unwrap()[0];
            target.on_ground = true;
            target.latitude = 0.001;
        }
        observe(&mut s);
        assert!(s.traffic.pilots[&42].landing.is_some());
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(s));
        let response = talk(
            AxumState(shared.clone()),
            Json(json!({"id":42,"topic":"crosswind"})),
        )
        .await;
        assert!(response.status().is_success());
        {
            let s = shared.read().await;
            assert!(s.session.transcript.last().unwrap().text.contains("20"));
            assert!(s.session.transcript.last().unwrap().background);
        }
        shared.write().await.settings.talk_to_traffic = false;
        assert!(
            !talk(
                AxumState(shared.clone()),
                Json(json!({"id":42,"topic":"landing"}))
            )
            .await
            .status()
            .is_success()
        );
        shared.write().await.settings.talk_to_traffic = true;
        shared
            .write()
            .await
            .traffic
            .pilots
            .get_mut(&42)
            .unwrap()
            .seen = Instant::now() - Duration::from_secs(10);
        assert!(
            !talk(AxumState(shared), Json(json!({"id":42,"topic":"landing"})))
                .await
                .status()
                .is_success()
        );
    }
}
