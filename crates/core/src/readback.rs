//! Conservative spoken readback matching; never repairs incorrect operational values.
use crate::state::{Request, State};
fn word(token: &str) -> &str {
    match token {
        "alpha" | "alfa" => "a",
        "bravo" => "b",
        "charlie" => "c",
        "delta" => "d",
        "echo" => "e",
        "foxtrot" => "f",
        "golf" => "g",
        "hotel" => "h",
        "india" => "i",
        "juliet" | "juliett" => "j",
        "kilo" => "k",
        "lima" => "l",
        "mike" => "m",
        "november" => "n",
        "oscar" => "o",
        "papa" => "p",
        "quebec" => "q",
        "romeo" => "r",
        "sierra" => "s",
        "tango" => "t",
        "uniform" => "u",
        "victor" => "v",
        "whiskey" => "w",
        "xray" => "x",
        "yankee" => "y",
        "zulu" => "z",
        "zero" | "oh" => "0",
        "one" => "1",
        "two" => "2",
        "three" | "tree" => "3",
        "four" | "fower" => "4",
        "five" | "fife" => "5",
        "six" => "6",
        "seven" => "7",
        "eight" => "8",
        "nine" | "niner" => "9",
        _ => token,
    }
}
fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
fn compact(text: &str) -> String {
    tokens(text).iter().map(|s| word(s)).collect()
}
fn altitude(text: &str, expected: i32) -> bool {
    let t = tokens(text);
    for end in 0..t.len() {
        if t[end] != "feet" && t[end] != "foot" {
            continue;
        }
        let mut start = end;
        while start > 0
            && (word(&t[start - 1]).chars().all(|c| c.is_ascii_digit())
                || matches!(t[start - 1].as_str(), "thousand" | "hundred"))
        {
            start -= 1;
        }
        let mut value = 0;
        let mut run = String::new();
        for token in &t[start..end] {
            if token == "thousand" || token == "hundred" {
                value +=
                    run.parse::<i32>().unwrap_or(0) * if token == "thousand" { 1000 } else { 100 };
                run.clear();
            } else {
                run.push_str(word(token));
            }
        }
        value += run.parse::<i32>().unwrap_or(0);
        if value == expected {
            return true;
        }
    }
    false
}
/// Check identifiers, numeric values and safety words in a short instruction reply.
pub fn matches_pilot_reply(actual: &str, expected: &str, callsign: &str) -> bool {
    let normalize = |text: &str| {
        tokens(text)
            .into_iter()
            .map(|token| {
                if token.len() > 1 && token[..token.len() - 1].chars().all(|c| c.is_ascii_digit()) {
                    let suffix = match token.chars().last().unwrap() {
                        'l' => "left",
                        'r' => "right",
                        'c' => "centre",
                        _ => "",
                    };
                    if !suffix.is_empty() {
                        return format!("{}{suffix}", &token[..token.len() - 1]);
                    }
                }
                word(&token).to_owned()
            })
            .collect::<String>()
    };
    let check_identifiers = expected.to_lowercase().contains("direct")
        || expected.to_lowercase().contains("approach")
        || expected.to_lowercase().contains("expect");
    let identifiers: Vec<_> = expected
        .split_whitespace()
        .map(|v| v.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-'))
        .filter(|v| {
            v.len() > 1
                && v.chars().any(|c| c.is_ascii_alphabetic())
                && v.chars()
                    .filter(|c| c.is_ascii_alphabetic())
                    .all(|c| c.is_ascii_uppercase())
        })
        .map(normalize)
        .collect();
    let actual = normalize(actual);
    let expected = normalize(expected);
    if !actual.contains(&compact(callsign)) {
        return false;
    }
    for word in ["left", "right", "cross", "holdshort"] {
        if expected.contains(word) && !actual.contains(word) {
            return false;
        }
        if !expected.contains(word) && actual.contains(word) {
            return false;
        }
    }
    let numbers = |text: &str| {
        let mut values: Vec<_> = text
            .split(|c: char| !c.is_ascii_digit())
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect();
        values.sort();
        values
    };
    if numbers(&actual) != numbers(&expected) {
        return false;
    }
    !check_identifiers || identifiers.iter().all(|id| actual.contains(id))
}

/// Recognize pending spoken readbacks before general conversation classification.
/// A structured readback button already supplies an explicit clearance sequence.
pub fn prepare(state: &State, request: &mut Request) {
    if request.clearance_sequence != 0 {
        return;
    }
    let taxi = state.taxi_clearance.pending_readback;
    let clearance = state.clearance.as_ref();
    if !taxi && clearance.is_none_or(|c| c.acknowledged) {
        if let Some(last) = state
            .transcript
            .iter()
            .rev()
            .find(|e| e.is_controller_instruction())
            && !last.pilot_reply.is_empty()
            && matches_pilot_reply(&request.text, &last.pilot_reply, &state.plan.callsign)
        {
            request.intent = "acknowledge".into();
            request.clearance_sequence = last.sequence;
        }
        return;
    }
    let text = compact(&request.text);
    // The generic parser can interpret "initial altitude 5000" as an altitude
    // request. Pending readback evidence takes priority over that classification.
    let amendment = clearance.is_some_and(|c| !c.readback_scope.is_empty());
    let evidence = if amendment {
        ["climb", "descend", "maintain", "feet", "direct", "runway"]
            .iter()
            .any(|word| text.contains(word))
    } else if taxi {
        text.contains("holdshort") || text.contains("taxivia") || text.contains("crossrunway")
    } else {
        [
            "cleared",
            "clearto",
            "route",
            "initialaltitude",
            "squawk",
            "feet",
            "meters",
        ]
        .iter()
        .filter(|word| text.contains(**word))
        .count()
            >= 2
    };
    if request.intent != "readback" && !evidence {
        return;
    }
    let spoken = compact(&request.text);
    let mut missing = Vec::new();
    if !spoken.contains(&compact(&state.plan.callsign)) {
        missing.push(crate::dialogue::say("readback_field_callsign", &[]));
    }
    if taxi {
        let hold = if state.taxi_clearance.hold_short_runway.is_empty() {
            &state.taxi_clearance.destination
        } else {
            &state.taxi_clearance.hold_short_runway
        };
        if !spoken.contains(&compact(hold)) {
            missing.push(crate::dialogue::say(
                "readback_field_assigned_runway_stand",
                &[],
            ));
        }
        if !state.taxi_clearance.crossing_runway.is_empty() && !spoken.contains("cross") {
            missing.push(crate::dialogue::say("readback_field_runway_crossing", &[]));
        }
        if state.taxi_clearance.runway_taxi
            && (!spoken.contains("backtrack") || !spoken.contains("holdposition"))
        {
            missing.push(crate::dialogue::say("readback_field_runway_taxi", &[]));
        }
        if !state.taxi_clearance.hold_short_runway.is_empty() && !spoken.contains("holdshort") {
            missing.push(crate::dialogue::say(
                "readback_field_hold_short_instruction",
                &[],
            ));
        }
        // Validate structured route facts, never reverse-parse editable speech.
        for name in state
            .taxi_clearance
            .via
            .split_whitespace()
            .filter(|s| !matches!(*s, "and" | "then"))
        {
            if !spoken.contains(&compact(name)) {
                missing.push(crate::dialogue::say("readback_field_taxiway_route", &[]));
                break;
            }
        }
        request.clearance_sequence = state.taxi_clearance.sequence;
        request
            .waypoint
            .clone_from(&state.taxi_clearance.instructions);
    } else {
        let Some(c) = clearance else {
            return;
        };
        if !c.readback_scope.is_empty() {
            match c.readback_scope.as_str() {
                "altitude" if !altitude(&request.text, c.altitude_feet) => {
                    missing.push(crate::dialogue::say("readback_field_altitude", &[]))
                }
                "direct" if !spoken.contains(&compact(&c.route)) => {
                    missing.push(crate::dialogue::say("readback_field_route", &[]))
                }
                "runway" if !spoken.contains(&format!("runway{}", compact(&c.runway))) => {
                    missing.push(crate::dialogue::say("readback_field_runway", &[]))
                }
                _ => {}
            }
            request.clearance_sequence = c.sequence;
            request.altitude_feet = c.altitude_feet;
            request.waypoint.clone_from(&c.route);
            request.intent = "readback".into();
            if !missing.is_empty() {
                request.waypoint = format!("READBACK ERROR:{}", missing.join(", "));
            }
            return;
        }
        if !spoken.contains(&compact(&state.plan.destination)) {
            missing.push(crate::dialogue::say("readback_field_destination", &[]));
        }
        let required_route = if c.as_filed {
            &c.departure_readback
        } else {
            &c.route
        };
        let full_route = c
            .route
            .split_whitespace()
            .filter(|s| *s != "DCT")
            .all(|s| spoken.contains(&compact(s)))
            && spoken.contains("route");
        if !(full_route
            || required_route
                .split_whitespace()
                .filter(|s| *s != "DCT")
                .all(|s| spoken.contains(&compact(s)))
                && (!c.as_filed
                    || spoken.contains("asfiled")
                    || spoken.contains("flightplannedroute")))
        {
            missing.push(crate::dialogue::say("readback_field_route", &[]));
        }
        if !altitude(&request.text, c.altitude_feet) {
            missing.push(crate::dialogue::say("readback_field_altitude", &[]));
        }
        if !spoken.contains(&format!("squawk{}", c.squawk)) {
            missing.push(crate::dialogue::say("readback_field_squawk", &[]));
        }
        if !spoken.contains(&format!("runway{}", compact(&c.runway))) {
            missing.push(crate::dialogue::say("readback_field_runway", &[]));
        }
        request.clearance_sequence = c.sequence;
        request.altitude_feet = c.altitude_feet;
        request.waypoint.clone_from(&c.route);
    }
    "readback".clone_into(&mut request.intent);
    if !missing.is_empty() {
        request.waypoint = "READBACK ERROR:".to_owned()
            + &crate::dialogue::say(
                "readback_error_say_again_your_readback_i",
                &[
                    (
                        "clearance_kind",
                        (if taxi { "taxi" } else { "IFR" }).to_string(),
                    ),
                    ("missing_items", (missing.join(", ")).clone()),
                ],
            );
    }
}

/// Construct the pilot's complete reply from the active clearance, never radio chatter.
#[must_use]
pub fn auto_reply(state: &State) -> Option<Request> {
    if state.taxi_clearance.stage() == crate::state::SurfaceStage::CrossingComplete
        && state.telemetry.on_ground
    {
        return Some(Request {
            intent: "vacated".into(),
            text: crate::dialogue::say(
                "pilot_runway_vacated",
                &[
                    ("callsign", state.plan.callsign.clone()),
                    ("runway", state.taxi_clearance.crossing_runway.clone()),
                ],
            ),
            ..Default::default()
        });
    }
    let last = state
        .transcript
        .iter()
        .rev()
        .find(|entry| entry.is_controller_instruction())?;
    if last.position.is_empty() {
        return None;
    }
    let mut request = Request {
        intent: "readback".to_owned(),
        ..Default::default()
    };
    if state.taxi_clearance.pending_readback {
        let taxi = &state.taxi_clearance;
        "taxi".clone_into(&mut request.readback_kind);
        request.clearance_sequence = taxi.sequence;
        request.waypoint.clone_from(&taxi.instructions);
        request.text = if !taxi.crossing_runway.is_empty() {
            crate::dialogue::say(
                "pilot_crossing_readback",
                &[
                    ("runway", taxi.crossing_runway.clone()),
                    ("callsign", state.plan.callsign.clone()),
                ],
            )
        } else if taxi.runway_taxi {
            crate::dialogue::say(
                "pilot_runway_taxi_readback",
                &[
                    ("entry_runway", taxi.entry_runway.clone()),
                    ("runway", taxi.destination.clone()),
                    ("callsign", state.plan.callsign.clone()),
                ],
            )
        } else if taxi.via.is_empty() && !taxi.hold_short_runway.is_empty() {
            crate::dialogue::say(
                "pilot_taxi_holding_point_readback",
                &[
                    ("runway", taxi.hold_short_runway.clone()),
                    ("callsign", state.plan.callsign.clone()),
                ],
            )
        } else if !taxi.hold_short_runway.is_empty() {
            crate::dialogue::say(
                "pilot_taxi_hold_readback",
                &[
                    ("via", taxi.via.clone()),
                    ("runway", taxi.hold_short_runway.clone()),
                    ("callsign", state.plan.callsign.clone()),
                ],
            )
        } else {
            crate::dialogue::say(
                "pilot_taxi_readback",
                &[
                    ("instructions", taxi.instructions.clone()),
                    ("callsign", state.plan.callsign.clone()),
                ],
            )
        };
    } else {
        let Some(clearance) = state.clearance.as_ref().filter(|c| !c.acknowledged) else {
            if last.pilot_reply.is_empty() {
                return None;
            }
            return Some(Request {
                intent: "acknowledge".to_owned(),
                text: last.pilot_reply.clone(),
                clearance_sequence: last.sequence,
                ..Default::default()
            });
        };
        request.clearance_sequence = clearance.sequence;
        "ifr".clone_into(&mut request.readback_kind);
        request.altitude_feet = clearance.altitude_feet;
        request.waypoint.clone_from(&clearance.route);
        request.text = if !clearance.readback_scope.is_empty() {
            crate::dialogue::say(
                &format!("pilot_{}_amendment", clearance.readback_scope),
                &[
                    ("callsign", state.plan.callsign.clone()),
                    (
                        "altitude",
                        crate::altitude_text(
                            f64::from(clearance.altitude_feet),
                            crate::UnitSystem::Imperial,
                            true,
                        ),
                    ),
                    ("route", clearance.route.clone()),
                    ("runway", clearance.runway.clone()),
                ],
            )
        } else {
            crate::dialogue::say(
                "pilot_ifr_readback",
                &[
                    ("destination", state.plan.destination.clone()),
                    (
                        "route",
                        if clearance.as_filed {
                            if clearance.departure_readback.is_empty() {
                                crate::dialogue::say("filed_routing", &[])
                            } else {
                                crate::dialogue::say(
                                    "departure_then_filed",
                                    &[("sid", clearance.departure_readback.clone())],
                                )
                            }
                        } else {
                            clearance.route.clone()
                        },
                    ),
                    ("altitude", clearance.altitude_feet.to_string()),
                    ("squawk", clearance.squawk.clone()),
                    ("runway", clearance.runway.clone()),
                    ("callsign", state.plan.callsign.clone()),
                ],
            )
        };
    }
    Some(request)
}

/// Build a check-in from the aircraft's current altitude and active clearance.
#[must_use]
pub fn checkin_text(state: &State, station: &str) -> String {
    let altitude = crate::altitude_text(
        state.telemetry.reported_altitude_feet(),
        crate::UnitSystem::Imperial,
        true,
    );
    let target = state
        .clearance
        .as_ref()
        .map_or(state.plan.initial_altitude_feet, |c| c.altitude_feet);
    let climb = if !state.telemetry.on_ground
        && f64::from(target) > state.telemetry.reported_altitude_feet() + 100.0
    {
        crate::dialogue::say(
            "pilot_checkin_climbing",
            &[(
                "altitude",
                crate::altitude_text(f64::from(target), crate::UnitSystem::Imperial, true),
            )],
        )
    } else {
        String::new()
    };
    crate::dialogue::say(
        if state.telemetry.on_ground {
            "pilot_ground_checkin"
        } else {
            "pilot_controller_checkin"
        },
        &[
            ("station", station.to_owned()),
            ("callsign", state.plan.callsign.clone()),
            ("altitude", altitude),
            ("climb", climb),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_replies_do_not_accept_wrong_runways_directions_or_levels() {
        assert!(matches_pilot_reply(
            "Turn left heading 270, C-GTLT",
            "Turn left heading 270, C-GTLT",
            "C-GTLT"
        ));
        assert!(!matches_pilot_reply(
            "Turn right heading 270, C-GTLT",
            "Turn left heading 270, C-GTLT",
            "C-GTLT"
        ));
        assert!(!matches_pilot_reply(
            "Cross runway 25L, C-GTLT",
            "Cross runway 25R, C-GTLT",
            "C-GTLT"
        ));
        assert!(!matches_pilot_reply(
            "Maintain 7000 feet, C-GTLT",
            "Maintain 8000 feet, C-GTLT",
            "C-GTLT"
        ));
        assert!(!matches_pilot_reply(
            "Direct WRONG, C-GTLT",
            "Direct RIGHT, C-GTLT",
            "C-GTLT"
        ));
        assert!(matches_pilot_reply(
            "Cross runway two five right, Charlie Golf Tango Lima Tango",
            "Cross runway 25R, C-GTLT",
            "C-GTLT"
        ));
    }
    #[test]
    fn rejects_wrong_altitude_and_matches_phonetics() {
        let mut s = State::default();
        s.plan.callsign = "C-GTLT".into();
        s.plan.destination = "VTBS".into();
        s.clearance = Some(crate::state::Clearance {
            altitude_feet: 5000,
            route: "CHU1C CMP W21 NOBER NORT2D".into(),
            runway: "31".into(),
            squawk: "2105".into(),
            sequence: 2,
            ..Default::default()
        });
        let text = "Charlie Golf Tango Lima Tango cleared Victor Tango Bravo Sierra CHU1C CMP W21 NOBER NORT2D five thousand feet squawk two one zero five runway three one";
        let mut r = Request {
            text: text.into(),
            intent: "conversation".into(),
            ..Default::default()
        };
        prepare(&s, &mut r);
        assert_eq!(r.intent, "readback");
        assert!(!r.waypoint.starts_with("READBACK ERROR"));
        let mut wrong = Request {
            text: text.replace("five thousand", "five hundred"),
            intent: "conversation".into(),
            ..Default::default()
        };
        prepare(&s, &mut wrong);
        assert!(wrong.waypoint.contains("altitude"));
    }
    #[test]
    fn abbreviated_clearance_reads_back_departure_not_entire_filed_route() {
        let mut state = State::default();
        state.plan.destination = "LIMC".into();
        state.plan.callsign = "C-GTLT".into();
        state.clearance = Some(crate::state::Clearance {
            altitude_feet: 5000,
            route: "BASI8A BASIP Y15 EGHIN DCT ANAKI DCT DEVOX DEVO3E".into(),
            departure_readback: "BASI8A BASIP".into(),
            as_filed: true,
            runway: "04R".into(),
            squawk: "4517".into(),
            sequence: 2,
            ..Default::default()
        });
        let text = "Cleared to LIMC BASI8A BASIP then as filed initial altitude 5000 feet squawk 4517 runway 04R C-GTLT";
        let mut request = Request {
            text: text.into(),
            ..Default::default()
        };
        prepare(&state, &mut request);
        assert!(
            !request.waypoint.starts_with("READBACK ERROR"),
            "{}",
            request.waypoint
        );
        assert_eq!(request.waypoint, state.clearance.as_ref().unwrap().route);
        for bad in [
            text.replace("4517", "4516"),
            text.replace("BASI8A", "WRONG1"),
            text.replace("as filed", ""),
        ] {
            let mut request = Request {
                text: bad,
                ..Default::default()
            };
            prepare(&state, &mut request);
            assert!(request.waypoint.starts_with("READBACK ERROR"));
        }
    }
    #[test]
    fn typed_clearance_and_garbled_readback_override_generic_parser() {
        let mut state = State::default();
        state.plan.callsign = "C-GTLT".into();
        state.plan.destination = "VTBS".into();
        state.clearance = Some(crate::state::Clearance {
            altitude_feet: 5000,
            route: "CHU1C CMP W21 NOBER NORT2D".into(),
            runway: "31".into(),
            squawk: "2105".into(),
            sequence: 2,
            ..Default::default()
        });
        let typed = "Cleared to VTBS, route CHU1C CMP W21 NOBER NORT2D, initial altitude 5000 feet, squawk 2105, Runway 31, C-GTLT";
        let parsed = crate::identify::interpret_text(typed);
        let mut request = Request {
            intent: parsed.intent,
            altitude_feet: parsed.altitude_feet,
            waypoint: parsed.waypoint,
            text: typed.into(),
            ..Default::default()
        };
        assert_ne!(
            request.intent, "readback",
            "reproduce the generic-parser conflict"
        );
        prepare(&state, &mut request);
        assert_eq!(request.intent, "readback");
        assert_eq!(request.waypoint, state.clearance.as_ref().unwrap().route);
        let garbled = "Charlie Gough, 10-go, clear to 30 BS, route 21 CCMP, W21, no bird, nor 2D, initial altitude 5000 feet, or 2.105 runway 31 foot departure.";
        let parsed = crate::identify::interpret_text(garbled);
        let mut request = Request {
            intent: parsed.intent,
            altitude_feet: parsed.altitude_feet,
            waypoint: parsed.waypoint,
            text: garbled.into(),
            ..Default::default()
        };
        prepare(&state, &mut request);
        assert_eq!(request.intent, "readback");
        assert!(request.waypoint.starts_with("READBACK ERROR:"));
        assert!(request.waypoint.contains("callsign"));
        let mut taxi = Request {
            intent: "conversation".into(),
            text: "Your Quest Taxi 2 runway 31".into(),
            ..Default::default()
        };
        prepare(&state, &mut taxi);
        assert_eq!(taxi.intent, "conversation", "taxi is not a readback");
    }
}
