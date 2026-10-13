//! Airborne instructions, route progress and compliance monitoring.
use crate::EngineState;
use openatc_core::{
    apply::add_transmission,
    dialogue::say,
    navigation,
    state::{PhaseCode, Request, SpeechTag},
    stations::Station,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Monitor {
    pub next_fix: usize,
    pub route: String,
    pub direct_origin: Option<[f64; 2]>,
    pub heading: Option<f64>,
    pub speed: Option<f64>,
    pub deviation: Option<(String, f64)>,
    pub last_call: Option<Instant>,
    pub warning: openatc_core::compliance::WarningCount,
    pub previous: Option<openatc_core::state::Telemetry>,
    pub step_target: Option<i32>,
    pub checked_fix: usize,
    pub pending_sequence: u32,
    pub pending_since: Option<Instant>,
}
fn transmit(state: &mut EngineState, station: &Station, text: String, reply: Option<String>) {
    let tag = crate::flight::tag(state, station);
    add_transmission(&mut state.session, "ATC", &text, &tag);
    if let Some(reply) = reply
        && let Some(last) = state.session.transcript.last_mut()
    {
        last.pilot_reply = reply;
    }
    state.flow.monitor.last_call = Some(Instant::now());
}
fn reply(
    state: &mut EngineState,
    request: &Request,
    tag: &SpeechTag,
    accepted: bool,
    text: String,
    pilot: Option<String>,
) -> Value {
    if !request.controller_initiated {
        let call = state.session.plan.callsign.clone();
        add_transmission(
            &mut state.session,
            &call,
            &request.text,
            &SpeechTag::default(),
        );
    }
    add_transmission(&mut state.session, "ATC", &text, tag);
    if let Some(pilot) = pilot
        && let Some(last) = state.session.transcript.last_mut()
    {
        last.pilot_reply = pilot;
    }
    json!({"accepted":accepted,"message":text})
}
/// Remaining imported-route distance; a direct distance is only a fallback.
pub fn remaining_nm(state: &EngineState) -> Option<f64> {
    let t = &state.session.telemetry;
    let mut point = [t.latitude, t.longitude];
    let mut distance = 0.0;
    let next = state.flow.monitor.next_fix.max(1);
    for fix in state.session.plan.fixes.iter().skip(next) {
        let target = [fix.latitude, fix.longitude];
        distance += openatc_core::arrival::distance_nm(point, target);
        point = target;
    }
    if let Some(airport) = &state.arrival_airport {
        distance += openatc_core::arrival::distance_nm(
            point,
            [airport.reference_latitude, airport.reference_longitude],
        );
    }
    (distance > 0.0).then_some(distance)
}
/// A nominal three-degree descent with ten miles left for final approach.
pub fn descent_feasible(state: &EngineState, altitude: i32) -> bool {
    let Some(distance) = remaining_nm(state) else {
        return f64::from(altitude) <= state.session.telemetry.altitude_feet;
    };
    let elevation = state
        .arrival_airport
        .as_ref()
        .map_or(0.0, |a| a.elevation_feet);
    altitude_within_descent(distance, elevation, altitude)
}
fn altitude_within_descent(distance: f64, elevation: f64, altitude: i32) -> bool {
    let ceiling = elevation + (distance - 10.0).max(0.0) * openatc_core::arrival::gradient(3.0);
    distance.is_finite()
        && elevation.is_finite()
        && f64::from(altitude) <= ceiling.max(elevation + 2000.0)
}
/// Reject amendments that cannot meet the next published crossing window.
pub fn altitude_allowed(state: &EngineState, altitude: i32) -> bool {
    if !state.settings.enforce_clearance_constraints {
        return true;
    }
    let next = state.flow.monitor.next_fix.max(1);
    state.session.plan.fixes.get(next).is_none_or(|f| {
        let t = &state.session.telemetry;
        let distance =
            openatc_core::ops::distance_nm(t.latitude, t.longitude, f.latitude, f.longitude);
        f.minimum_altitude_feet.is_none_or(|v| altitude >= v)
            && f.maximum_altitude_feet.is_none_or(|v| {
                f64::from(altitude)
                    <= f64::from(v)
                        + (distance - 5.0).max(0.0) * openatc_core::arrival::gradient(3.0)
            })
    })
}
pub fn request(
    state: &mut EngineState,
    station: &Station,
    request: &Request,
    tag: &SpeechTag,
) -> Option<Value> {
    if !["speed", "deviation", "resume_course"].contains(&request.intent.as_str()) {
        return None;
    }
    if !openatc_core::ops::request_available(&state.session, &request.intent) {
        return Some(reply(
            state,
            request,
            tag,
            false,
            say("amendment_unavailable", &[]),
            None,
        ));
    }
    let call = state.session.plan.callsign.clone();
    let value = request.waypoint.trim().to_lowercase();
    let (text, pilot) = match request.intent.as_str() {
        "speed" => {
            let speed = value
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && (50.0..=400.0).contains(v));
            let Some(speed) = speed else {
                return Some(reply(
                    state,
                    request,
                    tag,
                    false,
                    say("speed_request_invalid", &[]),
                    None,
                ));
            };
            if state.session.telemetry.indicated_speed_knots.is_none() {
                return Some(reply(
                    state,
                    request,
                    tag,
                    false,
                    say("speed_indication_unavailable", &[]),
                    None,
                ));
            }
            let limit = state
                .session
                .plan
                .fixes
                .get(state.flow.monitor.next_fix.max(1))
                .and_then(|f| f.maximum_speed_knots);
            let us = station.airport.starts_with('K')
                || station.airport.starts_with("PA")
                || station.airport.starts_with("PH");
            if limit.is_some_and(|v| speed > v)
                || (us && state.session.telemetry.altitude_feet < 10000.0 && speed > 250.0)
            {
                return Some(reply(
                    state,
                    request,
                    tag,
                    false,
                    say("speed_restriction_unable", &[]),
                    None,
                ));
            }
            state.flow.monitor.speed = Some(speed);
            let slots = [("callsign", call), ("speed", format!("{speed:.0}"))];
            (
                say("assigned_speed", &slots),
                say("pilot_speed_readback", &slots),
            )
        }
        "deviation" => {
            let parts: Vec<_> = value.split_whitespace().collect();
            let Some(degrees) = parts
                .get(1)
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite() && (1.0..=45.0).contains(v))
            else {
                return Some(reply(
                    state,
                    request,
                    tag,
                    false,
                    say("deviation_request_invalid", &[]),
                    None,
                ));
            };
            let direction = parts.first().copied().unwrap_or("");
            if !["left", "right"].contains(&direction) {
                return Some(reply(
                    state,
                    request,
                    tag,
                    false,
                    say("deviation_request_invalid", &[]),
                    None,
                ));
            }
            state.flow.monitor.deviation = Some((direction.into(), degrees));
            state.flow.monitor.heading = None;
            state.flow.monitor.warning.resolved();
            let slots = [
                ("callsign", call),
                ("direction", direction.into()),
                ("degrees", format!("{degrees:.0}")),
            ];
            (
                say("deviation_approved", &slots),
                say("pilot_deviation_readback", &slots),
            )
        }
        _ => {
            state.flow.monitor.deviation = None;
            state.flow.monitor.heading = None;
            let Some(fix) = state
                .session
                .plan
                .fixes
                .get(state.flow.monitor.next_fix.max(1))
            else {
                return Some(reply(
                    state,
                    request,
                    tag,
                    false,
                    say("route_geometry_unavailable", &[]),
                    None,
                ));
            };
            let slots = [("callsign", call), ("fix", fix.identifier.clone())];
            (
                say("deviation_resume", &slots),
                say("pilot_resume_readback", &slots),
            )
        }
    };
    state.flow.monitor.last_call = Some(Instant::now());
    Some(reply(state, request, tag, true, text, Some(pilot)))
}
/// One transmission per tick, after handoffs and pending readbacks have cleared.
/// Keep handoffs and new instructions behind the current readback.
pub fn awaiting_reply(state: &mut EngineState, station: &Station) -> bool {
    let pending = state
        .session
        .clearance
        .as_ref()
        .is_some_and(|c| !c.acknowledged)
        || state
            .session
            .transcript
            .iter()
            .rev()
            .find(|e| e.is_controller_instruction())
            .is_some_and(|e| !e.pilot_reply.is_empty());
    if !pending {
        state.flow.monitor.pending_since = None;
        return false;
    }
    let sequence = state
        .session
        .transcript
        .iter()
        .rev()
        .find(|e| e.is_controller_instruction())
        .map_or(0, |e| e.sequence);
    let monitor = &mut state.flow.monitor;
    if monitor.pending_since.is_none() || monitor.pending_sequence != sequence {
        monitor.pending_sequence = sequence;
        monitor.pending_since = Some(Instant::now());
    }
    if monitor
        .pending_since
        .is_some_and(|t| t.elapsed() >= Duration::from_secs(45))
    {
        let text = say(
            "instruction_readback_reminder",
            &[("callsign", state.session.plan.callsign.clone())],
        );
        // Preserve the actionable reply on the reminder rather than losing it.
        let reply = state
            .session
            .transcript
            .iter()
            .rev()
            .find(|e| e.is_controller_instruction())
            .and_then(|e| (!e.pilot_reply.is_empty()).then(|| e.pilot_reply.clone()));
        transmit(state, station, text, reply);
        state.flow.monitor.pending_since = Some(Instant::now());
        state.flow.monitor.pending_sequence =
            state.session.transcript.last().map_or(0, |e| e.sequence);
    }
    true
}
pub fn tick(state: &mut EngineState, station: &Station) {
    let t = state.session.telemetry.clone();
    if t.on_ground
        || !["Approach", "Departure", "Center"].contains(&station.service.as_str())
        || state.session.clearance_cancelled
        || state
            .session
            .clearance
            .as_ref()
            .is_none_or(|c| !c.acknowledged)
        || state
            .session
            .transcript
            .iter()
            .rev()
            .find(|e| e.is_controller_instruction())
            .is_some_and(|e| !e.pilot_reply.is_empty())
    {
        return;
    }
    let previous = state.flow.monitor.previous.replace(t.clone());
    let c = state.session.clearance.as_ref().unwrap().clone();
    let own = [t.latitude, t.longitude];
    if state.flow.monitor.route != c.route {
        state.flow.monitor.route = c.route.clone();
        state.flow.monitor.heading = None;
        state.flow.monitor.next_fix = state
            .session
            .plan
            .fixes
            .iter()
            .position(|f| f.identifier == c.route)
            .unwrap_or(1)
            .max(1);
        state.flow.monitor.direct_origin = (c.readback_scope == "direct").then_some(own);
    }
    let mut fixes = state.session.plan.fixes.clone();
    let next = state.flow.monitor.next_fix.max(1);
    if let Some(origin) = state.flow.monitor.direct_origin
        && next > 0
        && let Some(f) = fixes.get_mut(next - 1)
    {
        f.latitude = origin[0];
        f.longitude = origin[1];
    }
    let leg = navigation::leg(&fixes, next, &t);
    let call = state.session.plan.callsign.clone();
    let ready = state
        .flow
        .monitor
        .last_call
        .is_none_or(|last| last.elapsed() >= Duration::from_secs(30));
    if let Some(leg) = &leg
        && (leg.distance_nm < 0.05 || (leg.along >= 1.0 && leg.cross_track_nm < 1.5))
    {
        if next != state.flow.monitor.checked_fix {
            state.flow.monitor.checked_fix = next;
            if let Some(fix) = fixes.get(next)
                && state.settings.enforce_clearance_constraints
            {
                let interpolation = previous
                    .as_ref()
                    .and_then(|p| navigation::leg(&fixes, next, p).map(|l| (p, l)))
                    .filter(|(_, l)| l.along < 1.0 && leg.along >= 1.0 && leg.along > l.along)
                    .map(|(p, l)| (p, ((1.0 - l.along) / (leg.along - l.along)).clamp(0.0, 1.0)));
                let altitude = interpolation.map_or(t.altitude_feet, |(p, f)| {
                    p.altitude_feet + (t.altitude_feet - p.altitude_feet) * f
                });
                let speed = interpolation
                    .and_then(|(p, f)| {
                        p.indicated_speed_knots
                            .zip(t.indicated_speed_knots)
                            .map(|(a, b)| a + (b - a) * f)
                    })
                    .or(t.indicated_speed_knots);
                let violation = fix
                    .minimum_altitude_feet
                    .is_some_and(|v| altitude < f64::from(v) - 200.0)
                    || fix
                        .maximum_altitude_feet
                        .is_some_and(|v| altitude > f64::from(v) + 200.0)
                    || matches!((fix.maximum_speed_knots,speed),(Some(v),Some(speed)) if speed>v+10.0);
                if violation {
                    let text = say(
                        "crossing_restriction_missed",
                        &[("callsign", call.clone()), ("fix", fix.identifier.clone())],
                    );
                    correct(state, station, text, None);
                    state.flow.monitor.next_fix = next + 1;
                    state.flow.monitor.direct_origin = None;
                    return;
                }
            }
        }
        state.flow.monitor.step_target = fixes.get(next).and_then(|f| f.step_altitude_feet);
        state.flow.monitor.next_fix = next + 1;
        state.flow.monitor.direct_origin = None;
        state.flow.monitor.heading = None;
    }
    if !ready {
        return;
    }
    if let Some(heading) = state.flow.monitor.heading {
        if previous.as_ref().is_some_and(|p| {
            navigation::turn(p.heading_degrees, heading).abs()
                > navigation::turn(t.heading_degrees, heading).abs() + 1.0
        }) {
            return;
        }
        if navigation::turn(t.heading_degrees, heading).abs() > 15.0
            && state.settings.enforce_clearance_constraints
        {
            let turn = navigation::turn(t.heading_degrees, heading);
            let slots = [
                ("callsign", call.clone()),
                (
                    "direction",
                    if turn < 0.0 { "left" } else { "right" }.into(),
                ),
                ("degrees", format!("{:.0}", turn.abs())),
            ];
            correct(
                state,
                station,
                say("course_correction_relative", &slots),
                Some(say("pilot_turn_relative", &slots)),
            );
            return;
        }
    }
    if let Some(speed) = state.flow.monitor.speed
        && let Some(ias) = t.indicated_speed_knots
        && (ias - speed).abs() > 20.0
        && state.settings.enforce_clearance_constraints
    {
        let slots = [("callsign", call.clone()), ("speed", format!("{speed:.0}"))];
        correct(
            state,
            station,
            say("speed_correction", &slots),
            Some(say("pilot_speed_readback", &slots)),
        );
        return;
    }
    // Do not infer a straight airport-to-fix path through an unresolved SID.
    if let Some(leg) = leg
        && (next > 1 || state.session.plan.sid.is_empty() || c.readback_scope == "direct")
        && leg.cross_track_nm > 2.0
        && t.height_agl_feet > 1500.0
        && !matches!(state.session.phase, PhaseCode::Approach | PhaseCode::Landed)
    {
        let track = t.ground_track_degrees.unwrap_or(t.heading_degrees);
        let course = fixes
            .get(next - 1)
            .zip(fixes.get(next))
            .map(|(a, b)| {
                openatc_core::arrival::bearing([a.latitude, a.longitude], [b.latitude, b.longitude])
            })
            .unwrap_or(leg.bearing);
        let deviation = navigation::turn(course, track);
        if state
            .flow
            .monitor
            .deviation
            .as_ref()
            .is_some_and(|(side, limit)| {
                deviation.abs() <= *limit + 5.0 && (deviation < 0.0) == (side == "left")
            })
        {
            return;
        }
        let turn = navigation::turn(t.heading_degrees, leg.bearing);
        if turn.abs() > 5.0 {
            let destination = [fixes[next].latitude, fixes[next].longitude];
            if state
                .mora
                .corridor(own, destination)
                .is_none_or(|floor| t.altitude_feet < f64::from(floor))
            {
                transmit(
                    state,
                    station,
                    say("off_course_vector_unavailable", &[("callsign", call)]),
                    None,
                );
                return;
            }
            state.flow.monitor.heading = Some(leg.bearing);
            let direction = if turn < 0.0 { "left" } else { "right" };
            let slots = [
                ("callsign", call.clone()),
                ("direction", direction.into()),
                ("degrees", format!("{:.0}", turn.abs())),
                ("fix", fixes[next].identifier.clone()),
            ];
            if let Some(magnetic) = t.magnetic_heading_degrees {
                let heading =
                    (leg.bearing - navigation::turn(magnetic, t.heading_degrees)).rem_euclid(360.0);
                let mut slots = slots.to_vec();
                slots.push((
                    "heading",
                    format!("{:03.0}", if heading < 0.5 { 360.0 } else { heading }),
                ));
                correct(
                    state,
                    station,
                    say("course_correction_heading", &slots),
                    Some(say("pilot_heading_readback", &slots)),
                );
            } else {
                correct(
                    state,
                    station,
                    say("course_correction_relative", &slots),
                    Some(say("pilot_turn_relative", &slots)),
                );
            }
            return;
        }
    } else {
        state.flow.monitor.warning.resolved();
    }
    if descend(state, station) {
        return;
    }
    climb(state, station);
}
fn correct(state: &mut EngineState, station: &Station, text: String, pilot: Option<String>) {
    if !state.settings.enforce_clearance_constraints {
        state.flow.monitor.warning.resolved();
    }
    let repeated = state.flow.monitor.warning.continuing()
        && state.settings.enforce_clearance_constraints
        && state.settings.terminate_after_warnings
        && !state.session.emergency_declared;
    if repeated {
        let assistance = say(
            "compliance_assistance",
            &[("callsign", state.session.plan.callsign.clone())],
        );
        transmit(state, station, format!("{text} {assistance}"), pilot);
    } else {
        transmit(state, station, text, pilot);
    }
}
fn climb(state: &mut EngineState, station: &Station) {
    let t = &state.session.telemetry;
    let c = state.session.clearance.as_ref().unwrap();
    if !matches!(
        state.session.phase,
        PhaseCode::Departure | PhaseCode::Cruise
    ) || t.altitude_feet < f64::from(c.altitude_feet) - 500.0
        || state.flow.monitor.deviation.is_some()
    {
        return;
    }
    let ceiling = station
        .coverage
        .iter()
        .filter(|v| v.contains(t))
        .map(|v| v.ceiling)
        .reduce(f64::max)
        .unwrap_or(f64::from(state.session.plan.cruise_feet));
    let requested = state
        .flow
        .monitor
        .step_target
        .unwrap_or(state.session.plan.cruise_feet);
    let mut target = requested.min(((ceiling - 500.0) / 1000.0).floor() as i32 * 1000);
    if state.settings.enforce_clearance_constraints
        && let Some(limit) = state
            .session
            .plan
            .fixes
            .get(state.flow.monitor.next_fix.max(1))
            .and_then(|f| f.maximum_altitude_feet)
    {
        target = target.min(limit);
    }
    if target <= c.altitude_feet
        || !altitude_allowed(state, target)
        || !descent_feasible(state, target)
    {
        return;
    }
    let phrase = if !state.session.plan.sid.is_empty()
        && !state.session.plan.sid_has_vectors
        && state
            .session
            .plan
            .fixes
            .iter()
            .any(|f| f.constraint_source.starts_with("SID"))
    {
        "automatic_climb_via_sid"
    } else if t.vertical_speed_fpm > 200.0 {
        "automatic_continue_climb"
    } else {
        "automatic_climb"
    };
    let slots = [
        ("sid", state.session.plan.sid.clone()),
        ("callsign", state.session.plan.callsign.clone()),
        (
            "altitude",
            openatc_core::altitude_text(
                f64::from(target),
                openatc_core::UnitSystem::Imperial,
                true,
            ),
        ),
    ];
    let text = say(phrase, &slots);
    if state.flow.monitor.step_target.is_some() && target == requested {
        state.session.plan.cruise_feet = target;
        state.flow.monitor.step_target = None;
    }
    let sequence = state.session.next_sequence;
    let c = state.session.clearance.as_mut().unwrap();
    c.altitude_feet = target;
    c.sequence = sequence;
    c.acknowledged = false;
    c.readback_scope = "altitude".into();
    transmit(state, station, text, None);
}

fn descend(state: &mut EngineState, station: &Station) -> bool {
    if !matches!(
        state.session.phase,
        PhaseCode::Cruise | PhaseCode::Arrival | PhaseCode::Departure
    ) {
        return false;
    }
    let t = &state.session.telemetry;
    let c = state.session.clearance.as_ref().unwrap();
    let next = state.flow.monitor.next_fix.max(1);
    let Some(fix) = state.session.plan.fixes.get(next) else {
        return false;
    };
    let distance =
        openatc_core::ops::distance_nm(t.latitude, t.longitude, fix.latitude, fix.longitude);
    let Some(upper) = fix.maximum_altitude_feet else {
        return false;
    };
    if upper >= c.altitude_feet
        || distance > (f64::from(c.altitude_feet - upper) / 300.0 + 10.0).max(10.0)
    {
        return false;
    }
    let target = upper.max(fix.minimum_altitude_feet.unwrap_or(upper));
    let slots = [
        ("callsign", state.session.plan.callsign.clone()),
        (
            "altitude",
            openatc_core::altitude_text(
                f64::from(target),
                openatc_core::UnitSystem::Imperial,
                true,
            ),
        ),
    ];
    let phrase = if t.vertical_speed_fpm < -200.0 {
        "automatic_continue_descent"
    } else {
        "automatic_descent"
    };
    let mut text = say(phrase, &slots);
    let crossing = say(
        if fix.minimum_altitude_feet == fix.maximum_altitude_feet {
            "crossing_at"
        } else {
            "crossing_below"
        },
        &[
            ("callsign", state.session.plan.callsign.clone()),
            ("fix", fix.identifier.clone()),
            ("altitude", slots[1].1.clone()),
        ],
    );
    text.push(' ');
    text.push_str(&crossing);
    let c = state.session.clearance.as_mut().unwrap();
    c.altitude_feet = target;
    c.sequence = state.session.next_sequence;
    c.readback_scope = "altitude".into();
    c.acknowledged = false;
    state.session.phase = PhaseCode::Arrival;
    transmit(state, station, text, None);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_corrections_keep_clearance_and_required_readback() {
        let mut state = crate::test_state();
        state.settings.enforce_clearance_constraints = true;
        state.settings.terminate_after_warnings = true;
        state.session.clearance = Some(openatc_core::state::Clearance {
            altitude_feet: 5000,
            acknowledged: true,
            ..Default::default()
        });
        state.flow.approach_cleared = true;
        state.flow.landing_cleared = true;
        let station = Station {
            service: "Approach".into(),
            ..Default::default()
        };
        for _ in 0..5 {
            correct(
                &mut state,
                &station,
                "Turn left heading 270.".into(),
                Some("Left heading 270, TEST.".into()),
            );
            assert!(!state.session.clearance_cancelled);
            assert!(state.flow.approach_cleared && state.flow.landing_cleared);
            assert_eq!(
                state.session.clearance.as_ref().unwrap().altitude_feet,
                5000
            );
            assert_eq!(
                state.session.transcript.last().unwrap().pilot_reply,
                "Left heading 270, TEST."
            );
        }
        assert!(
            state.session.transcript.last().unwrap().text.len() > "Turn left heading 270.".len()
        );
    }
    #[test]
    fn pending_readback_blocks_new_instructions_and_keeps_the_reply() {
        let mut state = crate::test_state();
        let station = Station {
            name: "Test".into(),
            service: "Center".into(),
            ..Default::default()
        };
        state.session.clearance = Some(openatc_core::state::Clearance {
            acknowledged: false,
            ..Default::default()
        });
        transmit(
            &mut state,
            &station,
            "Climb to 8000 feet".into(),
            Some("8000 feet, TEST".into()),
        );
        assert!(awaiting_reply(&mut state, &station));
        let count = state.session.transcript.len();
        assert!(awaiting_reply(&mut state, &station));
        assert_eq!(state.session.transcript.len(), count);
        state.flow.monitor.pending_since = Some(Instant::now() - Duration::from_secs(46));
        assert!(awaiting_reply(&mut state, &station));
        assert_eq!(
            state.session.transcript.last().unwrap().pilot_reply,
            "8000 feet, TEST"
        );
        state.session.clearance.as_mut().unwrap().acknowledged = true;
        state
            .session
            .transcript
            .last_mut()
            .unwrap()
            .pilot_reply
            .clear();
        assert!(!awaiting_reply(&mut state, &station));
    }
    #[test]
    fn higher_altitudes_need_enough_track_miles_to_descend() {
        assert!(altitude_within_descent(100.0, 50.0, 28000));
        assert!(!altitude_within_descent(100.0, 50.0, 38000));
        assert!(altitude_within_descent(30.0, 50.0, 6000));
        assert!(!altitude_within_descent(30.0, 50.0, 10000));
        assert!(!altitude_within_descent(f64::NAN, 50.0, 5000));
        assert!(altitude_within_descent(20.0, 6000.0, 8000));
    }
}
