//! Published-station handoffs and explicit arrival permissions.
use crate::EngineState;
use openatc_core::{
    apply::add_transmission,
    dialogue::say,
    ops::distance_nm,
    state::{PhaseCode, Request, SpeechTag},
    stations::{Station, nearby, tuned},
};
use serde_json::{Value, json};

#[derive(Default)]
pub struct FlightFlow {
    pub discipline: crate::discipline::Monitor,
    pub monitor: crate::instructions::Monitor,
    arrival_requested: bool,
    checked_in_station: String,
    landing_hold: bool,
    landing_retry: Option<std::time::Instant>,
    candidate: String,
    evidence: u8,
    pending_station: String,
    pub(crate) approach_cleared: bool,
    pub(crate) landing_cleared: bool,
    pub(crate) takeoff_cleared: bool,
    pub(crate) missed_approach: bool,
    pub(crate) vacate_reported: bool,
    warning: openatc_core::compliance::WarningCount,
    clearance_sequence: u32,
    last_correction: Option<std::time::Instant>,
}

pub fn station_key(station: &Station) -> String {
    format!(
        "{}:{}:{}:{}",
        station.service, station.name, station.khz, station.airport
    )
}

pub(crate) fn tag(state: &mut EngineState, station: &Station) -> SpeechTag {
    let settings = state.settings.clone();
    let mut tag = crate::routes::assign_airspace_controller(
        state,
        &settings,
        &station_key(station),
        "standard",
    );
    tag.position.clone_from(&station.name);
    tag.radio = Some(state.session.control_radio);
    tag
}

fn airport_distance(state: &EngineState, id: &str) -> Option<f64> {
    let station = state.stations.iter().find(|s| s.airport == id)?;
    Some(distance_nm(
        state.session.telemetry.latitude,
        state.session.telemetry.longitude,
        station.latitude,
        station.longitude,
    ))
}

fn next_station(state: &EngineState, stations: &[Station], current: &Station) -> Option<Station> {
    let t = &state.session.telemetry;
    let arrival_distance =
        airport_distance(state, &state.session.plan.destination).unwrap_or(f64::INFINITY);
    let arrival = matches!(
        state.session.phase,
        PhaseCode::Arrival | PhaseCode::Approach | PhaseCode::Landed | PhaseCode::TaxiIn
    ) || (state.session.has_departed
        && arrival_distance < 45.0
        && state.session.phase != PhaseCode::Departure);
    let choose = |roles: &[&str], airport: Option<&str>| {
        roles.iter().find_map(|role| {
            stations
                .iter()
                .filter(|s| {
                    s.receivable && s.service == *role && airport.is_none_or(|id| s.airport == id)
                })
                .filter(|s| s.coverage.is_empty() || s.coverage.iter().any(|c| c.contains(t)))
                .min_by(|a, b| a.distance_nm.total_cmp(&b.distance_nm))
                .cloned()
        })
    };
    if t.on_ground {
        if matches!(state.session.phase, PhaseCode::Landed | PhaseCode::TaxiIn) {
            let airport = state
                .airport
                .as_ref()
                .filter(|a| a.icao == state.session.plan.destination)?;
            if openatc_core::airport::aircraft_on_runway(airport, t) {
                return None;
            }
            return choose(&["Ground", "Tower"], Some(&state.session.plan.destination));
        }
        return None;
    }
    if state.session.phase == PhaseCode::Departure && t.height_agl_feet < 500.0 {
        return None;
    }
    if arrival && state.flow.approach_cleared
        && final_position(state).is_some_and(|distance| distance < 12.0)
    {
        // Transfer final-approach communications before entering Tower's surface volume.
        return stations.iter()
            .filter(|station| station.receivable && station.service == "Tower"
                && station.airport == state.session.plan.destination)
            .min_by(|a, b| a.distance_nm.total_cmp(&b.distance_nm)).cloned();
    }
    let mut ahead = t.clone();
    let position = openatc_core::arrival::destination(
        [t.latitude, t.longitude],
        t.ground_track_degrees.unwrap_or(t.heading_degrees),
        (t.ground_speed_knots / 120.0).clamp(0.0, 5.0),
    );
    ahead.latitude = position[0];
    ahead.longitude = position[1];
    ahead.altitude_feet += t.vertical_speed_fpm * 0.5;
    let roles = if arrival {
        ["Approach", "Departure", "Center"]
    } else {
        ["Departure", "Approach", "Center"]
    };
    let owns = |s: &Station, at: &openatc_core::state::Telemetry| {
        s.coverage.iter().any(|v| v.contains(at))
    };
    let regional = |at: &openatc_core::state::Telemetry| {
        roles.iter().find_map(|role| {
            stations
                .iter()
                .filter(|s| s.receivable && s.service == *role && owns(s, at))
                .min_by(|a, b| a.distance_nm.total_cmp(&b.distance_nm))
                .cloned()
        })
    };
    let preferred = regional(&ahead).or_else(|| regional(t));
    if ["Approach", "Departure"].contains(&current.service.as_str())
        && state.session.clearance.as_ref().is_some_and(|c| {
            c.acknowledged && (t.altitude_feet - f64::from(c.altitude_feet)).abs() < 300.0
        })
        && let Some(ceiling) = current
            .coverage
            .iter()
            .filter(|v| v.contains(t))
            .map(|v| v.ceiling)
            .reduce(f64::max)
        && f64::from(state.session.plan.cruise_feet) > ceiling
        && t.altitude_feet >= ceiling - 1500.0
        && !arrival
        && let Some(center) = stations
            .iter()
            .filter(|s| {
                s.receivable
                    && s.service == "Center"
                    && owns(s, t)
                    && s.coverage
                        .iter()
                        .any(|v| v.contains(t) && v.ceiling > ceiling)
            })
            .min_by(|a, b| a.distance_nm.total_cmp(&b.distance_nm))
    {
        return Some(center.clone());
    }
    if owns(current, t) && owns(current, &ahead) {
        if arrival
            && current.service == "Center"
            && preferred.as_ref().is_some_and(|s| s.service == "Approach")
        {
            return preferred;
        }
        return Some(current.clone());
    }
    if let Some(next) = preferred {
        return Some(next);
    }
    // Airports without published sector polygons retain the reception-based fallback.
    if !current.coverage.is_empty() {
        return None;
    }
    if arrival {
        return choose(
            &["Approach", "Tower"],
            Some(&state.session.plan.destination),
        );
    }
    if state.session.phase == PhaseCode::Departure
        && airport_distance(state, &state.session.plan.departure).is_some_and(|d| d < 30.0)
    {
        return choose(
            &["Departure", "Approach", "Center"],
            Some(&state.session.plan.departure),
        )
        .or_else(|| choose(&["Center"], None));
    }
    choose(&["Center"], None)
}

/// Process handoffs only on a powered, receiving radio with unpaused valid telemetry.
/// Runway protection can interrupt a normal readback on short final.
pub fn urgent(state: &mut EngineState) -> bool {
    let t = &state.session.telemetry;
    if state.session.demo || t.paused || !t.position_valid || !t.radio_power || t.on_ground {
        return false;
    }
    let stations = nearby(&state.stations, t);
    let Some(current) = state
        .session
        .control_frequency()
        .and_then(|frequency| tuned(&stations, frequency))
        .cloned()
    else {
        return false;
    };
    protect_final(state, &current)
}

pub fn tick(state: &mut EngineState) {
    if matches!(state.session.phase, PhaseCode::Landed | PhaseCode::TaxiIn)
        && let Some(airport) = &state.arrival_airport
        && state
            .airport
            .as_ref()
            .is_none_or(|a| a.icao != airport.icao)
    {
        state.airport = Some(airport.clone());
    }
    let t = &state.session.telemetry;
    if state.session.demo
        || t.paused
        || !t.position_valid
        || !t.radio_power
        || !state.session.has_departed
    {
        return;
    }
    let stations = nearby(&state.stations, t);
    let Some(current) = state
        .session
        .control_frequency()
        .and_then(|frequency| tuned(&stations, frequency))
        .cloned()
        .filter(|s| !["ATIS", "Unicom"].contains(&s.service.as_str()))
    else {
        return;
    };
    if !state.session.telemetry.on_ground
        && !state.flow.missed_approach
        && matches!(
            state.session.phase,
            PhaseCode::Departure | PhaseCode::Cruise
        )
        && let (Some(origin), Some(destination)) = (
            airport_distance(state, &state.session.plan.departure),
            airport_distance(state, &state.session.plan.destination),
        )
        && origin > 10.0
        && destination < 45.0
        && destination < origin
    {
        state.session.phase = PhaseCode::Arrival;
    }
    if protect_final(state, &current) {
        return;
    }
    if crate::instructions::awaiting_reply(state, &current) {
        return;
    }
    if arrival_tick(state, &current) {
        return;
    }
    let before = state.session.next_sequence;
    compliance(state, &current);
    if state.session.next_sequence != before {
        return;
    }
    if state.session.clearance_cancelled {
        return;
    }
    if state.session.phase == PhaseCode::Landed
        && state.session.telemetry.ground_speed_knots < 40.0
        && !state.flow.vacate_reported
    {
        state.flow.vacate_reported = true;
        let text = say(
            "arrival_vacate",
            &[
                ("callsign", state.session.plan.callsign.clone()),
                ("runway", state.session.plan.arrival_runway.clone()),
            ],
        );
        let voice = tag(state, &current);
        add_transmission(&mut state.session, "ATC", &text, &voice);
        return;
    }
    let Some(next) = next_station(state, &stations, &current) else {
        crate::instructions::tick(state, &current);
        return;
    };
    let key = station_key(&next);
    if key == station_key(&current) || key == state.flow.pending_station {
        state.flow.evidence = 0;
        if state.flow.pending_station.is_empty() {
            crate::instructions::tick(state, &current);
        }
        return;
    }
    if state.flow.candidate != key {
        state.flow.candidate.clone_from(&key);
        state.flow.evidence = 0;
    }
    state.flow.evidence = state.flow.evidence.saturating_add(1);
    if state.flow.evidence < 4 {
        return;
    }
    state.flow.pending_station = key;
    state.session.recommended_frequency_khz = next.khz;
    state.session.frequency_sequence = state.session.next_sequence;
    let text = say(
        "flight_handoff",
        &[
            ("callsign", state.session.plan.callsign.clone()),
            ("station", next.name.clone()),
            ("frequency", format!("{:.3}", f64::from(next.khz) / 1000.0)),
        ],
    );
    let tag = tag(state, &current);
    add_transmission(&mut state.session, "ATC", &text, &tag);
    if let Some(entry) = state.session.transcript.last_mut() {
        entry.pilot_reply = say(
            "pilot_frequency_readback",
            &[
                ("station", next.name.clone()),
                ("frequency", format!("{:.3}", f64::from(next.khz) / 1000.0)),
                ("callsign", state.session.plan.callsign.clone()),
            ],
        );
    }
}

fn respond(
    state: &mut EngineState,
    request: &Request,
    tag: &SpeechTag,
    accepted: bool,
    text: String,
) -> Value {
    let callsign = state.session.plan.callsign.clone();
    if !request.controller_initiated {
        add_transmission(
            &mut state.session,
            &callsign,
            &request.text,
            &SpeechTag::default(),
        );
    }
    add_transmission(&mut state.session, "ATC", &text, tag);
    json!({"accepted":accepted,"message":text})
}

/// Handle check-in, frequency requests and approach/landing permissions on their actual stations.
pub fn request(
    state: &mut EngineState,
    station: &Station,
    request: &Request,
    tag: &SpeechTag,
) -> Option<Value> {
    if ["departure_runway", "arrival_runway", "change_approach"].contains(&request.intent.as_str())
    {
        return Some(amend_airport(state, station, request, tag));
    }
    if !["checkin", "frequency", "approach", "landing"].contains(&request.intent.as_str()) {
        return None;
    }
    let callsign = state.session.plan.callsign.clone();
    if request.intent == "frequency" {
        let stations = nearby(&state.stations, &state.session.telemetry);
        let next = next_station(state, &stations, station);
        let (accepted, text) =
            if let Some(next) = next.filter(|s| station_key(s) != station_key(station)) {
                state.session.recommended_frequency_khz = next.khz;
                state.session.frequency_sequence = state.session.next_sequence;
                state.flow.pending_station = station_key(&next);
                (
                    true,
                    say(
                        "flight_handoff",
                        &[
                            ("callsign", callsign),
                            ("station", next.name),
                            ("frequency", format!("{:.3}", f64::from(next.khz) / 1000.0)),
                        ],
                    ),
                )
            } else {
                (
                    true,
                    say(
                        "flight_remain_frequency",
                        &[("callsign", callsign), ("station", station.name.clone())],
                    ),
                )
            };
        return Some(respond(state, request, tag, accepted, text));
    }
    if request.intent == "checkin" {
        state.session.control_radio = request.radio;
        state.flow.checked_in_station = station_key(station);
        state.flow.pending_station.clear();
        state.flow.evidence = 0;
        state.session.recommended_frequency_khz = station.khz;
        state.session.frequency_sequence = 0;
        let altitude = openatc_core::altitude_text(
            state.session.telemetry.reported_altitude_feet(),
            openatc_core::UnitSystem::Imperial,
            true,
        );
        let assigned = state.session.clearance.as_ref().filter(|c| c.acknowledged);
        let (phrase, altitude) =
            if !state.session.telemetry.on_ground && !state.flow.approach_cleared {
                assigned.map_or(("flight_checkin", altitude), |c| {
                    let transition = openatc_core::regions::region_for(
                        &state.regions,
                        &state.session.plan.departure,
                    )
                    .transition_feet;
                    let observed = state
                        .session
                        .telemetry
                        .clearance_altitude_feet(c.altitude_feet, transition);
                    let phrase = if observed < f64::from(c.altitude_feet) - 200.0 {
                        "flight_checkin_climb"
                    } else if observed > f64::from(c.altitude_feet) + 200.0 {
                        "flight_checkin_descent"
                    } else {
                        "flight_checkin_level"
                    };
                    (
                        phrase,
                        openatc_core::altitude_text(
                            f64::from(c.altitude_feet),
                            openatc_core::UnitSystem::Imperial,
                            true,
                        ),
                    )
                })
            } else {
                ("flight_checkin", altitude)
            };
        let text = say(
            phrase,
            &[
                ("callsign", callsign),
                ("station", station.name.clone()),
                ("altitude", altitude.clone()),
            ],
        );
        let reply = match phrase {
            "flight_checkin_climb" => Some(say(
                "pilot_continue_climb",
                &[
                    ("callsign", state.session.plan.callsign.clone()),
                    ("altitude", altitude),
                ],
            )),
            "flight_checkin_descent" => Some(say(
                "pilot_continue_descent",
                &[
                    ("callsign", state.session.plan.callsign.clone()),
                    ("altitude", altitude),
                ],
            )),
            "flight_checkin_level" => Some(say(
                "pilot_maintain_altitude",
                &[
                    ("callsign", state.session.plan.callsign.clone()),
                    ("altitude", altitude),
                ],
            )),
            _ => None,
        };
        let result = respond(state, request, tag, true, text);
        if let Some(reply) = reply
            && let Some(last) = state.session.transcript.last_mut()
        {
            last.pilot_reply = reply;
        }
        return Some(result);
    }
    let airport = state
        .arrival_airport
        .as_ref()
        .or(state.airport.as_ref())
        .filter(|a| a.icao == state.session.plan.destination)
        .cloned();
    let Some(airport) = airport else {
        return Some(respond(
            state,
            request,
            tag,
            false,
            say("arrival_airport_unavailable", &[]),
        ));
    };
    let runway = state.session.plan.arrival_runway.clone();
    if runway.is_empty()
        || !airport
            .runways
            .iter()
            .any(|r| r.first_name == runway || r.second_name == runway)
    {
        return Some(respond(
            state,
            request,
            tag,
            false,
            say("arrival_runway_required", &[]),
        ));
    }
    if state.session.telemetry.on_ground {
        return Some(respond(
            state,
            request,
            tag,
            false,
            say("arrival_airborne_required", &[]),
        ));
    }
    if request.intent == "approach" {
        if let Some(r) = state.session.plan.approach.strip_prefix("ILS") {
            state.session.plan.approach = format!("I{r}");
        }
        if !["Approach", "Tower"].contains(&station.service.as_str()) {
            return Some(respond(
                state,
                request,
                tag,
                false,
                say("arrival_contact_approach", &[]),
            ));
        }
        state.flow.arrival_requested = true;
        state.flow.missed_approach = false;
        state.flow.landing_cleared = false;
        state.flow.landing_hold = false;
        state.flow.landing_retry = None;
        state.session.phase = PhaseCode::Arrival;
        state.flow.approach_cleared = false;
        let text = say(
            "arrival_expect_runway",
            &[
                ("callsign", callsign),
                ("runway", runway),
                ("airport", airport.name),
            ],
        );
        return Some(respond(state, request, tag, true, text));
    }
    if station.service != "Tower" {
        return Some(respond(
            state,
            request,
            tag,
            false,
            say("arrival_contact_tower", &[]),
        ));
    }
    if !state.flow.approach_cleared || final_position(state).is_none_or(|d| d > 12.0) {
        return Some(respond(
            state,
            request,
            tag,
            false,
            say(
                "arrival_report_final",
                &[("callsign", callsign), ("runway", runway)],
            ),
        ));
    }
    if let Some(blocked) = landing_conflict(state, &airport, &runway) {
        return Some(respond(
            state,
            request,
            tag,
            false,
            say(blocked, &[("runway", runway)]),
        ));
    }
    state.flow.landing_cleared = true;
    let text = say(
        "arrival_landing_clearance",
        &[
            ("callsign", callsign),
            ("runway", runway),
            ("weather", weather(state, station)),
        ],
    );
    let result = respond(state, request, tag, true, text);
    let reply = say(
        "pilot_landing_readback",
        &[
            ("callsign", state.session.plan.callsign.clone()),
            ("runway", state.session.plan.arrival_runway.clone()),
        ],
    );
    if let Some(last) = state.session.transcript.last_mut() {
        last.pilot_reply = reply;
    }
    Some(result)
}

fn weather(state: &EngineState, station: &Station) -> String {
    let Some((w, _)) = state
        .sim_weather
        .get(&station.airport)
        .filter(|(_, t)| t.elapsed().as_secs() < 45)
    else {
        return String::new();
    };
    let pressure = if station.airport.starts_with('K') {
        say(
            "flight_altimeter",
            &[(
                "pressure",
                format!(
                    "{:.2}",
                    w["pressureHpa"].as_f64().unwrap_or(0.0) / 33.863_886_666_7
                ),
            )],
        )
    } else {
        let pressure = w["pressureHpa"].as_f64().unwrap_or(0.0).round();
        say(
            if pressure < 1000.0 {
                "flight_qnh_units"
            } else {
                "flight_qnh"
            },
            &[("pressure", format!("{pressure:.0}"))],
        )
    };
    say(
        "flight_surface_weather",
        &[
            (
                "direction",
                format!("{:03.0}", w["windDegrees"].as_f64().unwrap_or(0.0)),
            ),
            (
                "speed",
                format!("{:.0}", w["windKnots"].as_f64().unwrap_or(0.0)),
            ),
            ("pressure", pressure),
        ],
    )
}

pub fn takeoff_wording(
    state: &mut EngineState,
    station: &Station,
    result: &mut openatc_core::apply::RequestOutcome,
) {
    state.flow.takeoff_cleared = true;
    let plan = &state.session.plan;
    let sid = state.airport.as_ref().is_some_and(|a| {
        a.procedures
            .iter()
            .any(|p| p.kind.eq_ignore_ascii_case("SID") && p.name == plan.sid)
    });
    let altitude = state
        .session
        .clearance
        .as_ref()
        .map_or(plan.initial_altitude_feet, |c| c.altitude_feet);
    let climb = if sid {
        say(
            "flight_sid_climb",
            &[
                ("sid", plan.sid.clone()),
                (
                    "altitude",
                    openatc_core::altitude_text(
                        f64::from(altitude),
                        openatc_core::UnitSystem::Imperial,
                        true,
                    ),
                ),
            ],
        )
    } else {
        say(
            "flight_initial_climb",
            &[(
                "altitude",
                openatc_core::altitude_text(
                    f64::from(altitude),
                    openatc_core::UnitSystem::Imperial,
                    true,
                ),
            )],
        )
    };
    let expected = if plan.cruise_feet > altitude {
        say(
            "flight_expect_level",
            &[(
                "altitude",
                openatc_core::altitude_text(
                    f64::from(plan.cruise_feet),
                    openatc_core::UnitSystem::Imperial,
                    true,
                ),
            )],
        )
    } else {
        String::new()
    };
    result.message = say(
        "flight_takeoff",
        &[
            ("callsign", plan.callsign.clone()),
            ("runway", plan.runway.clone()),
            ("weather", weather(state, station)),
            ("climb", climb),
            ("expected", expected),
        ],
    );
    if let Some(entry) = state
        .session
        .transcript
        .iter_mut()
        .rev()
        .find(|e| e.speaker == "ATC")
    {
        entry.text.clone_from(&result.message);
        entry.pilot_reply = say(
            "pilot_takeoff_readback",
            &[
                ("runway", state.session.plan.runway.clone()),
                ("callsign", state.session.plan.callsign.clone()),
            ],
        );
    }
}

fn compliance(state: &mut EngineState, station: &Station) {
    if !state.settings.enforce_clearance_constraints || state.session.emergency_declared {
        state.flow.warning.resolved();
        state.flow.last_correction = None;
        state.flow.clearance_sequence = 0;
        return;
    }
    if state.session.clearance_cancelled || state.session.telemetry.on_ground {
        return;
    }
    let Some(clearance) = state.session.clearance.as_ref().filter(|c| c.acknowledged) else {
        return;
    };
    // Once cleared and established on final, the approach path replaces the entry level.
    if state.flow.approach_cleared && !state.flow.missed_approach
        && let Some(distance) = final_position(state).filter(|distance| *distance <= 15.0)
        && let Some(airport) = state.arrival_airport.as_ref()
    {
        let glide = state.arrival_profile.as_ref()
            .and_then(|profile| openatc_core::arrival::target(profile, &state.session.plan.arrival_runway, &state.session.plan.approach))
            .and_then(|target| target.glide_degrees).unwrap_or(3.0);
        let path = airport.elevation_feet + openatc_core::arrival::gradient(glide) * distance;
        if (state.session.telemetry.altitude_feet - path).abs() <= 500.0 {
            state.flow.warning.resolved();
            state.flow.last_correction = None;
            return;
        }
    }
    let sequence = clearance.sequence;
    let target = f64::from(clearance.altitude_feet);
    let t = &state.session.telemetry;
    let transition =
        openatc_core::regions::region_for(&state.regions, &state.session.plan.departure)
            .transition_feet;
    let error = t.clearance_altitude_feet(clearance.altitude_feet, transition) - target;
    // Climbing or descending toward an assigned level is compliance, not a level bust.
    let converging = (error < -300.0 && t.vertical_speed_fpm > 200.0)
        || (error > 300.0 && t.vertical_speed_fpm < -200.0);
    if error.abs() <= 300.0 || converging {
        state.flow.warning.resolved();
        state.flow.last_correction = None;
        return;
    }
    let now = std::time::Instant::now();
    if state.flow.clearance_sequence != sequence {
        state.flow.clearance_sequence = sequence;
        state.flow.warning.resolved();
        state.flow.last_correction = Some(now);
        return;
    }
    let Some(last) = state.flow.last_correction else {
        state.flow.last_correction = Some(now);
        return;
    };
    if now.duration_since(last).as_secs() < 30 {
        return;
    }
    state.flow.last_correction = Some(now);
    let repeated = state.flow.warning.continuing();
    let altitude = openatc_core::altitude_text(target, openatc_core::UnitSystem::Imperial, true);
    let mut text = say(
        "compliance_altitude_correction",
        &[
            ("callsign", state.session.plan.callsign.clone()),
            ("altitude", altitude),
        ],
    );
    if repeated && state.settings.terminate_after_warnings {
        text.push(' ');
        text.push_str(&say("compliance_assistance", &[]));
    }
    let voice = tag(state, station);
    add_transmission(&mut state.session, "ATC", &text, &voice);
}

fn amend_airport(
    state: &mut EngineState,
    station: &Station,
    request: &Request,
    tag: &SpeechTag,
) -> Value {
    if !openatc_core::ops::request_available(&state.session, &request.intent) {
        return respond(
            state,
            request,
            tag,
            false,
            say("amendment_unavailable", &[]),
        );
    }
    let departure = request.intent == "departure_runway";
    let id = if departure {
        &state.session.plan.departure
    } else {
        &state.session.plan.destination
    };
    let Some(airport) = state
        .arrival_airport
        .as_ref()
        .filter(|a| a.icao == *id)
        .or_else(|| state.airport.as_ref().filter(|a| a.icao == *id))
    else {
        return respond(
            state,
            request,
            tag,
            false,
            say("arrival_airport_unavailable", &[]),
        );
    };
    if departure && station.airport != *id {
        return respond(
            state,
            request,
            tag,
            false,
            say("amendment_wrong_airport", &[]),
        );
    }
    let value = request.waypoint.trim().to_uppercase();
    let callsign = state.session.plan.callsign.clone();
    if request.intent == "change_approach" {
        let canonical = value
            .strip_prefix("ILS")
            .map_or_else(|| value.clone(), |r| format!("I{r}"));
        if !airport
            .procedures
            .iter()
            .any(|p| p.kind == "APPCH" && p.name == canonical)
        {
            return respond(
                state,
                request,
                tag,
                false,
                say("amendment_approach_unavailable", &[]),
            );
        }
        if let Some(runway) = approach_runway(&canonical) {
            if !airport
                .runways
                .iter()
                .any(|r| r.first_name == runway || r.second_name == runway)
            {
                return respond(
                    state,
                    request,
                    tag,
                    false,
                    say("amendment_runway_unavailable", &[]),
                );
            }
            state.session.plan.arrival_runway = runway.into();
        }
        state.session.plan.approach = canonical.clone();
        openatc_core::navigation::enrich(&mut state.session.plan, &state.settings.simulator_root);
        state.flow.arrival_requested = true;
        state.flow.approach_cleared = false;
        state.flow.landing_cleared = false;
        if state.session.phase == PhaseCode::Approach {
            state.session.phase = PhaseCode::Arrival;
        }
        let text = say(
            "amendment_expect_approach",
            &[
                ("callsign", callsign.clone()),
                ("approach", approach_label(&canonical)),
            ],
        );
        let result = respond(state, request, tag, true, text);
        if let Some(last) = state.session.transcript.last_mut() {
            last.pilot_reply = say(
                "pilot_approach_amendment",
                &[
                    ("callsign", callsign),
                    ("approach", approach_label(&canonical)),
                ],
            );
        }
        return result;
    }
    if !airport
        .runways
        .iter()
        .any(|r| r.first_name == value || r.second_name == value)
    {
        return respond(
            state,
            request,
            tag,
            false,
            say("amendment_runway_unavailable", &[]),
        );
    }
    if departure {
        state.flow.takeoff_cleared = false;
        let sid_entries: Vec<_> = airport
            .procedures
            .iter()
            .filter(|p| p.kind == "SID" && p.name == state.session.plan.sid)
            .collect();
        let has_runway_transitions = sid_entries.iter().any(|p| p.transition.starts_with("RW"));
        if !state.session.plan.sid.is_empty()
            && (sid_entries.is_empty()
                || (has_runway_transitions
                    && !sid_entries.iter().any(|p| {
                        p.transition == format!("RW{value}")
                            || p.transition
                                == format!("RW{}B", value.trim_end_matches(['L', 'R', 'C']))
                    })))
        {
            return respond(
                state,
                request,
                tag,
                false,
                say("amendment_departure_incompatible", &[]),
            );
        }
        state.session.plan.runway = value.clone();
        state.session.taxi_clearance = Default::default();
        state.session.phase = PhaseCode::Clearance;
        if let Some(c) = state.session.clearance.as_mut() {
            c.runway = value.clone();
            c.sequence = state.session.next_sequence;
            c.readback_scope = "runway".into();
            c.acknowledged = false;
        }
    } else {
        state.session.plan.arrival_runway = value.clone();
        state.session.plan.approach.clear();
        state.flow.approach_cleared = false;
        state.flow.landing_cleared = false;
        if state.session.phase == PhaseCode::Approach {
            state.session.phase = PhaseCode::Arrival;
        }
    }
    openatc_core::navigation::enrich(&mut state.session.plan, &state.settings.simulator_root);
    let text = say(
        "amendment_runway",
        &[("callsign", callsign.clone()), ("runway", value.clone())],
    );
    let result = respond(state, request, tag, true, text);
    if !departure && let Some(last) = state.session.transcript.last_mut() {
        last.pilot_reply = say(
            "pilot_runway_amendment",
            &[("callsign", callsign), ("runway", value)],
        );
    }
    result
}

fn final_position(state: &EngineState) -> Option<f64> {
    let airport = state
        .arrival_airport
        .as_ref()
        .or(state.airport.as_ref())
        .filter(|a| a.icao == state.session.plan.destination)?;
    let name = &state.session.plan.arrival_runway;
    let runway = airport
        .runways
        .iter()
        .find(|r| r.first_name == *name || r.second_name == *name)?;
    let start = runway.landing_threshold(name)?;
    let end = if runway.first_name == *name {
        runway.second
    } else {
        runway.first
    };
    let t = &state.session.telemetry;
    let p = openatc_core::airport::airport_point(airport, t.latitude, t.longitude);
    let dx = end.east - start.east;
    let dn = end.north - start.north;
    let length = dx.hypot(dn);
    if length < 100.0 {
        return None;
    }
    let along = ((p.east - start.east) * dx + (p.north - start.north) * dn) / length;
    let cross = ((p.east - start.east) * dn - (p.north - start.north) * dx).abs() / length;
    let track = t.ground_track_degrees.unwrap_or(t.heading_degrees);
    let course = dx.atan2(dn).to_degrees().rem_euclid(360.0);
    if along > 0.0
        || cross > 0.3 * 1852.0
        || openatc_core::navigation::turn(track, course).abs() > 20.0
    {
        return None;
    }
    Some(-along / 1852.0)
}
pub(crate) fn go_around(state: &mut EngineState) {
    state.flow.approach_cleared = false;
    state.flow.landing_cleared = false;
    state.flow.arrival_requested = false;
    state.flow.missed_approach = true;
    state.flow.landing_hold = false;
    state.flow.landing_retry = None;
    state.session.phase = PhaseCode::Departure;
    state.session.phase_evidence = 0;
    state.pending_copilot_reply = None;
}

fn landing_conflict(
    state: &EngineState,
    airport: &openatc_core::airport::Airport,
    runway: &str,
) -> Option<&'static str> {
    if state.session.arrival_wake_conflict {
        Some("holding_wake")
    } else {
        openatc_core::traffic::landing_blocked(airport, runway, &state.session.telemetry)
    }
}

// Recheck the runway even while a landing readback is outstanding.
fn protect_final(state: &mut EngineState, current: &Station) -> bool {
    if current.service != "Tower"
        || state.session.telemetry.on_ground
        || state.flow.missed_approach
        || !state.flow.approach_cleared
    {
        return false;
    }
    let Some(distance) = final_position(state).filter(|d| *d <= 15.0) else {
        return false;
    };
    let Some(airport) = state.arrival_airport.as_ref().or(state.airport.as_ref()) else {
        return false;
    };
    let blocked = landing_conflict(state, airport, &state.session.plan.arrival_runway);
    let confirmed_conflict = blocked.is_some_and(|reason| reason != "holding_standby");
    if !(state.session.arrival_wake_conflict
        || confirmed_conflict && state.flow.landing_cleared && distance < 2.5
        || blocked.is_some() && distance < 1.0)
    {
        return false;
    }
    go_around(state);
    let mut voice = tag(state, current);
    voice.urgent = true;
    voice.delivery = "urgent".into();
    let slots = [("callsign", state.session.plan.callsign.clone())];
    add_transmission(
        &mut state.session,
        "ATC",
        &say(
            match blocked {
                Some("holding_traffic_final") => "arrival_go_around_traffic",
                Some("holding_runway_occupied") => "arrival_go_around_blocked",
                Some("holding_wake") => "arrival_go_around_wake",
                _ => "arrival_go_around_unconfirmed",
            },
            &slots,
        ),
        &voice,
    );
    if let Some(last) = state.session.transcript.last_mut() {
        last.pilot_reply = say("pilot_go_around_blocked", &slots);
    }
    true
}

fn arrival_tick(state: &mut EngineState, current: &Station) -> bool {
    if state.flow.missed_approach || state.session.clearance_cancelled {
        return false;
    }
    if matches!(
        state.session.phase,
        PhaseCode::Arrival | PhaseCode::Approach
    ) && !state.session.plan.approach.is_empty()
    {
        state.flow.arrival_requested = true;
    }
    if state.session.telemetry.on_ground || !state.flow.arrival_requested {
        return false;
    }
    let Some(distance) = final_position(state).filter(|d| *d <= 15.0) else {
        return false;
    };
    if !state.flow.approach_cleared && ["Approach", "Center"].contains(&current.service.as_str()) {
        let Some(airport) = state.arrival_airport.as_ref().or(state.airport.as_ref()) else {
            return false;
        };
        let approach = &state.session.plan.approach;
        if approach.is_empty()
            || !airport
                .procedures
                .iter()
                .any(|p| p.kind == "APPCH" && p.name == *approach)
        {
            return false;
        }
        // Issue the published entry-level amendment before clearing the approach.
        if let Some(entry) = state.arrival_profile.as_ref()
            .and_then(|profile| openatc_core::arrival::target(profile, &state.session.plan.arrival_runway, approach))
            .and_then(|target| target.intercept_feet)
            && state.session.clearance.as_ref().is_some_and(|clearance| f64::from(clearance.altitude_feet) > entry)
            && state.session.telemetry.indicated_altitude_feet.unwrap_or(state.session.telemetry.altitude_feet) > entry + 300.0
        {
            return false;
        }
        let agl = state.session.telemetry.altitude_feet - airport.elevation_feet;
        if agl < 200.0 || agl > openatc_core::arrival::gradient(3.0) * distance + 500.0 {
            return false;
        }
        let slots = [
            ("callsign", state.session.plan.callsign.clone()),
            ("approach", approach_label(approach)),
            ("runway", state.session.plan.arrival_runway.clone()),
        ];
        state.flow.approach_cleared = true;
        state.session.phase = PhaseCode::Approach;
        let speed_release = crate::traffic::clear_approach_speed(state);
        let mut text = say("arrival_approach_clearance", &slots);
        if let Some((controller, _)) = &speed_release {
            text.push(' ');
            text.push_str(controller);
        }
        let voice = tag(state, current);
        add_transmission(&mut state.session, "ATC", &text, &voice);
        if let Some(last) = state.session.transcript.last_mut() {
            last.pilot_reply = say("pilot_approach_clearance", &slots);
            if let Some((_, pilot)) = speed_release {
                last.pilot_reply.push(' ');
                last.pilot_reply.push_str(&pilot);
            }
        }
        return true;
    }
    if current.service != "Tower"
        || !state.flow.approach_cleared
        || state.flow.landing_cleared
        || state.flow.checked_in_station != station_key(current)
    {
        return false;
    }
    if state
        .session
        .transcript
        .iter()
        .rev()
        .find(|e| e.is_controller_instruction())
        .is_some_and(|e| !e.pilot_reply.is_empty())
    {
        return false;
    }
    if state
        .flow
        .landing_retry
        .is_some_and(|last| last.elapsed().as_secs() < 15)
    {
        return false;
    }
    let Some(airport) = state.arrival_airport.as_ref().or(state.airport.as_ref()) else {
        return false;
    };
    let blocked = landing_conflict(state, airport, &state.session.plan.arrival_runway);
    if blocked.is_some() {
        state.flow.landing_retry = Some(std::time::Instant::now());
        if !state.flow.landing_hold {
            state.flow.landing_hold = true;
            let voice = tag(state, current);
            let text = say(
                match blocked {
                    Some("holding_traffic_final") => "arrival_continue_traffic",
                    Some("holding_runway_occupied") => "arrival_continue_no_landing",
                    Some("holding_wake") => "arrival_continue_wake",
                    _ => "arrival_continue_unconfirmed",
                },
                &[("callsign", state.session.plan.callsign.clone())],
            );
            add_transmission(&mut state.session, "ATC", &text, &voice);
            return true;
        }
        return false;
    }
    state.flow.landing_cleared = true;
    state.flow.landing_hold = false;
    let slots = [
        ("callsign", state.session.plan.callsign.clone()),
        ("runway", state.session.plan.arrival_runway.clone()),
        ("weather", weather(state, current)),
    ];
    let voice = tag(state, current);
    add_transmission(
        &mut state.session,
        "ATC",
        &say("arrival_landing_clearance", &slots),
        &voice,
    );
    if let Some(last) = state.session.transcript.last_mut() {
        last.pilot_reply = say("pilot_landing_readback", &slots);
    }
    true
}

fn approach_runway(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    if bytes.len() < 3
        || !name.is_ascii()
        || !b"IRLVN".contains(&bytes[0])
        || !bytes[1..3].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let end = if bytes.get(3).is_some_and(|c| b"LRC".contains(c)) {
        4
    } else {
        3
    };
    let suffix = &name[end..];
    (suffix.is_empty() || matches!(suffix, "X" | "Y" | "Z")).then_some(&name[1..end])
}
/// Turn a published procedure identifier into a readable approach name.
fn approach_label(name: &str) -> String {
    let (prefix, suffix) = name.split_at(name.char_indices().nth(1).map_or(name.len(), |(i, _)| i));
    if suffix.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        let kind = match prefix {
            "I" => "ILS",
            "R" => "RNAV",
            "L" => "Localizer",
            "V" => "VOR",
            "N" => "NDB",
            _ => return name.into(),
        };
        format!("{kind} runway {suffix}")
    } else {
        name.into()
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    use openatc_core::stations::Coverage;
    #[test]
    fn routine_qnh_omits_units_but_low_pressure_keeps_them() {
        let mut state = crate::test_state();
        let station = Station {
            airport: "LFMN".into(),
            ..Default::default()
        };
        for (pressure, units) in [(1022.0, false), (993.0, true)] {
            state.sim_weather.insert(
                "LFMN".into(),
                (
                    json!({"pressureHpa":pressure,"windDegrees":95,"windKnots":7}),
                    std::time::Instant::now(),
                ),
            );
            let report = weather(&state, &station);
            assert!(report.contains(&format!("QNH {pressure:.0}")), "{report}");
            assert_eq!(report.contains("hectopascals"), units, "{report}");
        }
    }
    #[test]
    fn standard_pressure_level_does_not_bust_geometric_altitude() {
        let mut state = crate::test_state();
        state.settings.enforce_clearance_constraints = true;
        state.session.plan.departure = "LFMN".into();
        state.session.telemetry.on_ground = false;
        state.session.telemetry.altitude_feet = 18311.7;
        state.session.telemetry.indicated_altitude_feet = Some(18000.0);
        state.session.telemetry.pressure_altitude_feet = Some(18000.0);
        state.session.clearance = Some(openatc_core::state::Clearance {
            altitude_feet: 18000,
            acknowledged: true,
            sequence: 1,
            ..Default::default()
        });
        let station = Station {
            service: "Center".into(),
            ..Default::default()
        };
        for altitude in [18000.0, 17950.0, 18070.0, 18299.0, 18000.0] {
            state.session.telemetry.pressure_altitude_feet = Some(altitude);
            state.flow.last_correction =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(31));
            compliance(&mut state, &station);
            assert!(state.session.transcript.is_empty());
        }
        state.session.telemetry.pressure_altitude_feet = Some(18500.0);
        compliance(&mut state, &station);
        assert!(
            state.session.transcript.is_empty(),
            "A sustained deviation needs grace"
        );
        state.flow.last_correction =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(31));
        compliance(&mut state, &station);
        assert_eq!(state.session.transcript.len(), 1);
        state.session.telemetry.vertical_speed_fpm = -500.0;
        state.flow.last_correction =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(31));
        compliance(&mut state, &station);
        assert_eq!(
            state.session.transcript.len(),
            1,
            "A correcting descent is compliant"
        );
    }
    #[test]
    fn altitude_corrections_continue_without_cancelling_ifr() {
        for assistance in [false, true] {
            let mut state = crate::test_state();
            state.settings.enforce_clearance_constraints = true;
            state.settings.terminate_after_warnings = assistance;
            state.session.telemetry.on_ground = false;
            state.session.telemetry.altitude_feet = 6000.0;
            state.session.clearance = Some(openatc_core::state::Clearance {
                sequence: 1,
                altitude_feet: 5000,
                acknowledged: true,
                ..Default::default()
            });
            let station = Station {
                service: "Center".into(),
                ..Default::default()
            };
            compliance(&mut state, &station);
            for count in 1..=5 {
                state.flow.last_correction =
                    Some(std::time::Instant::now() - std::time::Duration::from_secs(31));
                compliance(&mut state, &station);
                assert_eq!(state.session.transcript.len(), count);
                assert!(!state.session.clearance_cancelled);
                assert_eq!(
                    state.session.clearance.as_ref().unwrap().altitude_feet,
                    5000
                );
            }
            state.session.telemetry.altitude_feet = 5000.0;
            compliance(&mut state, &station);
            assert!(state.flow.last_correction.is_none());
        }
    }
    fn station(name: &str, left: f64, right: f64, distance: f64) -> Station {
        Station {
            name: name.into(),
            service: "Approach".into(),
            receivable: true,
            distance_nm: distance,
            coverage: vec![Coverage {
                floor: 0.0,
                ceiling: 10000.0,
                points: vec![[-1.0, left], [1.0, left], [1.0, right], [-1.0, right]],
            }],
            ..Default::default()
        }
    }
    #[test]
    fn published_owner_wins_over_a_nearer_unrelated_airport() {
        let mut state = crate::test_state();
        state.session.phase = PhaseCode::Departure;
        state.session.has_departed = true;
        state.session.telemetry = openatc_core::state::Telemetry {
            on_ground: false,
            altitude_feet: 5000.0,
            height_agl_feet: 5000.0,
            longitude: 0.5,
            ground_speed_knots: 120.0,
            ground_track_degrees: Some(90.0),
            ..Default::default()
        };
        let old = station("Old", -1.0, 0.0, 1.0);
        let wrong = station("Wrong", 2.0, 3.0, 0.1);
        let owner = station("Owner", 0.0, 1.0, 20.0);
        assert_eq!(
            next_station(&state, &[wrong, owner.clone()], &old)
                .unwrap()
                .name,
            "Owner"
        );
        assert_eq!(
            next_station(&state, std::slice::from_ref(&owner), &owner)
                .unwrap()
                .name,
            "Owner"
        );
    }
    #[test]
    fn lookahead_transfers_before_the_boundary() {
        let mut state = crate::test_state();
        state.session.phase = PhaseCode::Departure;
        state.session.has_departed = true;
        state.session.telemetry = openatc_core::state::Telemetry {
            on_ground: false,
            altitude_feet: 5000.0,
            height_agl_feet: 5000.0,
            longitude: -0.01,
            ground_speed_knots: 240.0,
            ground_track_degrees: Some(90.0),
            ..Default::default()
        };
        let old = station("Old", -1.0, 0.0, 1.0);
        let next = station("Next", 0.0, 1.0, 2.0);
        assert!(old.coverage[0].contains(&state.session.telemetry));
        assert_eq!(
            next_station(&state, &[old.clone(), next], &old)
                .unwrap()
                .name,
            "Next"
        );
    }
    fn arrival_state() -> (EngineState, Station, Station) {
        use openatc_core::{
            airport::{Airport, Procedure, Runway},
            state::{Point, Telemetry},
        };
        let mut state = crate::test_state();
        state.session.plan.destination = "TEST".into();
        state.session.plan.arrival_runway = "36".into();
        state.session.plan.approach = "I36".into();
        state.session.phase = PhaseCode::Arrival;
        state.arrival_airport = Some(Airport {
            icao: "TEST".into(),
            runways: vec![Runway {
                first_name: "36".into(),
                second_name: "18".into(),
                first: Point::default(),
                second: Point {
                    north: 2000.0,
                    ..Default::default()
                },
                width: 45.0,
                ..Default::default()
            }],
            procedures: vec![Procedure {
                kind: "APPCH".into(),
                name: "I36".into(),
                transition: String::new(),
            }],
            ..Default::default()
        });
        state.session.telemetry = Telemetry {
            latitude: -0.15,
            on_ground: false,
            altitude_feet: 2000.0,
            height_agl_feet: 2000.0,
            ground_track_degrees: Some(0.0),
            traffic: Some(vec![]),
            ..Default::default()
        };
        let approach = Station {
            name: "Test Approach".into(),
            service: "Approach".into(),
            ..Default::default()
        };
        let tower = Station {
            name: "Test Tower".into(),
            service: "Tower".into(),
            khz: 118100,
            ..Default::default()
        };
        (state, approach, tower)
    }

    #[test]
    fn cleared_final_descent_does_not_reinstate_old_level() {
        let (mut state, approach, _) = arrival_state();
        state.settings.enforce_clearance_constraints=true;
        state.session.telemetry.latitude=-0.1;
        state.session.clearance=Some(openatc_core::state::Clearance {altitude_feet:5000,acknowledged:true,sequence:1,..Default::default()});
        assert!(arrival_tick(&mut state,&approach));
        state.flow.clearance_sequence=1;
        state.flow.last_correction=Some(std::time::Instant::now()-std::time::Duration::from_secs(31));
        let before=state.session.transcript.len();
        compliance(&mut state,&approach);
        assert_eq!(state.session.transcript.len(),before);
        state.session.telemetry.longitude=0.02;
        state.flow.last_correction=Some(std::time::Instant::now()-std::time::Duration::from_secs(31));
        compliance(&mut state,&approach);
        assert_eq!(state.session.transcript.len(),before+1,"An off-final aircraft still needs level compliance");
    }

    #[test]
    fn approach_clearance_waits_for_published_entry_level_amendment() {
        let (mut state, approach, _) = arrival_state();
        state.session.telemetry.latitude=-0.24;
        state.session.telemetry.altitude_feet=5000.0;
        let mut airport=state.arrival_airport.clone().unwrap();
        airport.navaids.push(openatc_core::airport::Navaid{kind:"GS".into(),runway:"36".into(),glide_angle:3.0,..Default::default()});
        state.arrival_profile=Some(openatc_core::arrival::ArrivalAirport{airport,intercepts:vec![openatc_core::arrival::Intercept{procedure:"I36".into(),altitude_feet:4000.0}],..Default::default()});
        state.session.clearance=Some(openatc_core::state::Clearance{altitude_feet:5000,acknowledged:true,..Default::default()});
        assert!(!arrival_tick(&mut state,&approach));
        assert!(!state.flow.approach_cleared);
        state.session.clearance.as_mut().unwrap().altitude_feet=4000;
        assert!(arrival_tick(&mut state,&approach),"A valid entry clearance allows the approach while descending toward it");
    }

    #[test]
    fn final_handoff_does_not_wait_for_tower_surface_altitude() {
        let (mut state, approach, mut tower) = arrival_state();
        state.session.telemetry.latitude=-0.19;
        state.session.telemetry.altitude_feet=4000.0;
        tower.airport="TEST".into();
        tower.receivable=true;
        tower.coverage=vec![Coverage{floor:0.0,ceiling:2000.0,points:vec![[-1.0,-1.0],[-1.0,1.0],[1.0,1.0],[1.0,-1.0]]}];
        assert!(!tower.coverage[0].contains(&state.session.telemetry));
        assert!(next_station(&state,&[tower.clone()],&approach).is_none());
        assert!(arrival_tick(&mut state,&approach));
        assert_eq!(next_station(&state,&[tower.clone()],&approach).unwrap().khz,tower.khz);
        state.session.telemetry.longitude=0.02;
        assert!(next_station(&state,&[tower.clone()],&approach).is_none(),"Being near the airport is not final establishment");
        state.session.telemetry.longitude=0.0;
        tower.receivable=false;
        assert!(next_station(&state,&[tower],&approach).is_none(),"A handoff still requires radio reception");
    }

    #[test]
    fn approach_tower_checkin_and_landing_are_separate_permissions() {
        let (mut state, approach, tower) = arrival_state();
        state.flow.monitor.speed = Some(190.0);
        assert!(arrival_tick(&mut state, &approach));
        assert!(state.flow.monitor.speed.is_none());
        assert!(
            state
                .session
                .transcript
                .last()
                .unwrap()
                .text
                .contains("normal speed")
        );
        assert!(
            state
                .session
                .transcript
                .last()
                .unwrap()
                .pilot_reply
                .contains("normal speed")
        );
        assert!(state.flow.approach_cleared);
        assert!(!state.flow.landing_cleared);
        assert!(
            state
                .session
                .transcript
                .last()
                .unwrap()
                .text
                .contains("ILS runway 36")
        );
        state
            .session
            .transcript
            .last_mut()
            .unwrap()
            .pilot_reply
            .clear();
        assert!(!arrival_tick(&mut state, &tower));
        state.flow.checked_in_station = station_key(&tower);
        assert!(arrival_tick(&mut state, &tower));
        assert!(state.flow.landing_cleared);
        assert!(
            state
                .session
                .transcript
                .last()
                .unwrap()
                .text
                .contains("cleared to land")
        );
    }

    #[test]
    fn runway_conflict_overrides_pending_landing_readback() {
        let (mut state, _, tower) = arrival_state();
        state.flow.approach_cleared = true;
        state.flow.arrival_requested = true;
        state.flow.checked_in_station = station_key(&tower);
        assert!(arrival_tick(&mut state, &tower));
        assert!(
            !state
                .session
                .transcript
                .last()
                .unwrap()
                .pilot_reply
                .is_empty()
        );
        state.session.telemetry.traffic = Some(vec![openatc_core::state::TrafficTarget {
            latitude: 0.005,
            on_ground: true,
            ..Default::default()
        }]);
        assert!(!protect_final(&mut state, &tower));
        state.session.telemetry.latitude = -0.03;
        state.session.telemetry.radio_busy = true;
        state.session.telemetry.position_valid = true;
        state.session.telemetry.radio_power = true;
        state.session.telemetry.com1_khz = tower.khz;
        state.session.demo = false;
        state.stations = vec![tower.clone()];
        assert!(urgent(&mut state));
        assert!(!state.flow.landing_cleared);
        assert!(state.flow.missed_approach);
        assert!(
            state
                .session
                .transcript
                .last()
                .unwrap()
                .text
                .to_lowercase()
                .contains("go around")
        );
        state.session.telemetry.traffic = Some(vec![]);
        assert!(!arrival_tick(&mut state, &tower));
        assert!(!protect_final(&mut state, &tower));
    }

    #[test]
    fn short_final_go_around_does_not_wait_for_retry_timer() {
        let (mut state, _, tower) = arrival_state();
        state.flow.approach_cleared = true;
        state.flow.arrival_requested = true;
        state.flow.checked_in_station = station_key(&tower);
        state.session.telemetry.traffic = None;
        assert!(arrival_tick(&mut state, &tower));
        assert!(state.flow.landing_retry.is_some());
        state.session.telemetry.latitude = -0.008;
        assert!(protect_final(&mut state, &tower));
        assert!(state.flow.missed_approach);
    }
    #[test]
    fn approach_identifiers_are_spoken_as_approach_names() {
        assert_eq!(approach_label("I19R"), "ILS runway 19R");
        assert_eq!(approach_label("R27"), "RNAV runway 27");
        assert_eq!(approach_label(""), "");
    }

    #[test]
    fn touchdown_exit_and_ground_handoff_are_separate_steps() {
        use openatc_core::{
            airport::{Airport, Runway},
            state::{Point, Telemetry},
        };
        for separate_ground in [false, true] {
            let mut state = crate::test_state();
            state.settings.enforce_clearance_constraints = false;
            state.session.demo = false;
            state.session.has_departed = true;
            state.session.phase = PhaseCode::Approach;
            state.session.plan.destination = "DEST".into();
            state.session.plan.arrival_runway = "36".into();
            state.arrival_airport = Some(Airport {
                icao: "DEST".into(),
                runways: vec![Runway {
                    first_name: "36".into(),
                    second_name: "18".into(),
                    first: Point::default(),
                    second: Point {
                        north: 2000.0,
                        ..Default::default()
                    },
                    width: 45.0,
                    ..Default::default()
                }],
                ..Default::default()
            });
            let tower = Station {
                airport: "DEST".into(),
                name: "Arrival Tower".into(),
                service: "Tower".into(),
                khz: 118100,
                ..Default::default()
            };
            state.stations = vec![tower.clone()];
            if separate_ground {
                state.stations.push(Station {
                    airport: "DEST".into(),
                    name: "Arrival Ground".into(),
                    service: "Ground".into(),
                    khz: 121900,
                    ..Default::default()
                });
            }
            let mut telemetry = Telemetry {
                on_ground: true,
                position_valid: true,
                radio_power: true,
                com1_khz: 118100,
                latitude: 0.005,
                traffic: Some(vec![]),
                ..Default::default()
            };
            for _ in 0..4 {
                openatc_core::ops::update_flight_phase(&mut state.session, &telemetry).unwrap();
            }
            assert_eq!(state.session.phase, PhaseCode::Landed);
            tick(&mut state);
            assert_eq!(state.airport.as_ref().unwrap().icao, "DEST");
            assert_eq!(state.session.transcript.len(), 1);
            assert!(
                state.session.transcript[0]
                    .text
                    .to_lowercase()
                    .contains("vacate")
            );
            for _ in 0..5 {
                tick(&mut state);
            }
            assert_eq!(
                state.session.transcript.len(),
                1,
                "No Ground handoff while on the runway"
            );
            telemetry.longitude = 0.002;
            openatc_core::ops::update_flight_phase(&mut state.session, &telemetry).unwrap();
            for _ in 0..4 {
                tick(&mut state);
            }
            if separate_ground {
                assert_eq!(state.session.recommended_frequency_khz, 121900);
                assert!(
                    state
                        .session
                        .transcript
                        .last()
                        .unwrap()
                        .text
                        .contains("Arrival Ground")
                );
            } else {
                assert_eq!(
                    state.session.transcript.len(),
                    1,
                    "Combined Tower retains the arrival"
                );
            }
        }
    }
}
