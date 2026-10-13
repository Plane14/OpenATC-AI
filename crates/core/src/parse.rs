//! Request matching and altitude, route and emergency phrase parsing.

const DEFINITIONS: [(&str, &str); 26] = [
    ("landing", "request landing clearance"),
    ("cross_runway", "request runway crossing"),
    ("checkin", "check in"),
    ("clearance", "request ifr clearance"),
    ("start", "request start-up"),
    ("start_pushback", "request start-up and pushback"),
    ("pushback", "request pushback"),
    ("taxi", "request taxi"),
    ("backtrack", "request runway backtrack"),
    ("ready", "ready for departure"),
    ("gate", "taxi to parking"),
    ("progressive", "repeat taxi instructions"),
    ("radio_check", "radio check"),
    ("readback", "read back clearance"),
    ("repeat", "say again"),
    ("standby", "stand by"),
    ("unable", "unable"),
    ("frequency", "request frequency change"),
    ("position", "confirm position"),
    ("altitude", "request altitude..."),
    ("direct", "request direct-to..."),
    ("descent", "request descent..."),
    ("cancel_ifr", "cancel ifr"),
    ("approach", "brief arrival approach"),
    ("go_around", "going around"),
    ("emergency", "declare emergency"),
];

/// Match requests before falling back to catalogue titles. Emergency calls take priority.
pub(crate) fn interpret(text: &str) -> super::identify::Interpreted {
    let normalized = text.to_ascii_lowercase();
    let mut request = super::identify::Interpreted {
        intent: "conversation".to_owned(),
        ..Default::default()
    };
    match normalized.as_str() {
        "radio check" => "radio_check".clone_into(&mut request.intent),
        "say again" | "repeat" => "repeat".clone_into(&mut request.intent),
        "unable" => "unable".clone_into(&mut request.intent),
        "stand by" => "standby".clone_into(&mut request.intent),
        "request clearance" | "request ifr clearance" => {
            "clearance".clone_into(&mut request.intent);
        }
        "request taxi" => "taxi".clone_into(&mut request.intent),
        "request start" | "request startup" | "request start up" => {
            "start".clone_into(&mut request.intent);
        }
        "request start and pushback" | "request pushback and start" | "request push and start" => {
            "start_pushback".clone_into(&mut request.intent);
        }
        "request pushback" => "pushback".clone_into(&mut request.intent),
        "ready for departure" => "ready".clone_into(&mut request.intent),
        "going around" | "go around" => "go_around".clone_into(&mut request.intent),
        "cancel ifr" => "cancel_ifr".clone_into(&mut request.intent),
        "request altimeter" => "altimeter".clone_into(&mut request.intent),
        "request weather" => "weather".clone_into(&mut request.intent),
        _ => {}
    }
    // Callsign-bearing transmissions use the same deterministic workflow parser.
    for (phrase, intent) in [
        ("traffic not in sight", "traffic_not_sighted"),
        ("traffic not sighted", "traffic_not_sighted"),
        ("negative contact", "traffic_not_sighted"),
        ("traffic in sight", "traffic_sighted"),
        ("traffic sighted", "traffic_sighted"),
        ("request landing clearance", "landing"),
        ("vacated", "vacated"),
        ("vacted", "vacated"),
        ("runway vacated", "vacated"),
        ("runway clear", "vacated"),
        ("clear of runway", "vacated"),
        ("request runway crossing", "cross_runway"),
        ("request crossing", "cross_runway"),
        ("checking in", "checkin"),
        ("check-in", "checkin"),
        ("check in", "checkin"),
        ("with you", "checkin"),
        ("request start-up and pushback", "start_pushback"),
        ("request start and pushback", "start_pushback"),
        ("request pushback and start", "start_pushback"),
        ("request push and start", "start_pushback"),
        ("request start up", "start"),
        ("request startup", "start"),
        ("request start", "start"),
        ("request pushback", "pushback"),
        ("request ifr clearance", "clearance"),
        ("request clearance", "clearance"),
        ("request taxi", "taxi"),
        ("request runway backtrack", "backtrack"),
        ("request backtrack", "backtrack"),
    ] {
        if normalized.contains(phrase) {
            intent.clone_into(&mut request.intent);
            break;
        }
    }
    if normalized.contains("mayday")
        || normalized.contains("pan pan")
        || normalized.contains("pan-pan")
        || normalized == "emergency"
        || normalized == "declare emergency"
    {
        "emergency".clone_into(&mut request.intent);
        return request;
    }
    let altitude =
        regex::Regex::new(r"(?:request )?(?:altitude |climb |descend |descent )(?:to )?(fl\s*)?([0-9]{2,5})(?: feet| meters)?")
            .ok();
    if let Some(expression) = altitude
        && let Some(found) = expression.captures(&normalized)
        && let Some(digits) = found.get(2)
        && let Ok(value) = digits.as_str().parse::<i32>()
    {
        let fl = found.get(1).is_some();
        if normalized.contains("desc") {
            "descent".clone_into(&mut request.intent);
        } else {
            "altitude".clone_into(&mut request.intent);
        }
        request.altitude_feet = value * if fl { 100 } else { 1 };
        if normalized.contains("meter") && !fl {
            request.altitude_feet = super::meters_to_feet(request.altitude_feet);
        }
    }
    let direct = regex::Regex::new(r"(?:request )?direct(?: to)? ([a-z0-9]{2,10})").ok();
    if let Some(expression) = direct
        && let Some(found) = expression.captures(&normalized)
        && let Some(waypoint) = found.get(1)
    {
        "direct".clone_into(&mut request.intent);
        request.waypoint = waypoint.as_str().to_ascii_uppercase();
    }
    for (pattern, intent) in [
        (
            r"(?:change|request|use)(?: the)? (?:departure|departing) runway ([0-9]{1,2}[lrc]?)",
            "departure_runway",
        ),
        (
            r"(?:change|request|use)(?: the)? (?:arrival|landing) runway ([0-9]{1,2}[lrc]?)",
            "arrival_runway",
        ),
        (
            r"(?:change|request)(?: the)? approach(?: to)? ([a-z0-9]+)",
            "change_approach",
        ),
        (
            r"(?:request |maintain )?(?:speed |airspeed )([0-9]{2,3})(?: knots)?",
            "speed",
        ),
        (
            r"(?:request |can we |may we )?(?:deviate|deviation)(?: up to)? ([0-9]{1,2})(?: degrees)? (left|right)",
            "deviation",
        ),
    ] {
        if let Ok(expression) = regex::Regex::new(pattern)
            && let Some(found) = expression.captures(&normalized)
        {
            request.intent = intent.into();
            request.waypoint = found.get(1).map_or("", |m| m.as_str()).to_uppercase();
            if intent == "deviation" {
                request.waypoint = format!(
                    "{} {}",
                    found.get(2).map_or("", |m| m.as_str()),
                    request.waypoint
                );
            }
        }
    }
    if normalized.contains("clear of weather")
        || normalized.contains("resume planned route")
        || normalized.contains("back on course")
    {
        request.intent = "resume_course".into();
    }
    if request.intent == "conversation" {
        for (intent, title) in DEFINITIONS {
            if title == normalized {
                intent.clone_into(&mut request.intent);
                break;
            }
        }
    }
    request
}

#[cfg(test)]
mod checkin_tests {
    #[test]
    fn emergency_keeps_priority_over_an_embedded_amendment() {
        for text in [
            "mayday, request direct SLI",
            "pan pan, request descent to 3000",
            "pan-pan, request descent to 3000",
        ] {
            assert_eq!(super::interpret(text).intent, "emergency");
        }
    }
    #[test]
    fn accepts_natural_controller_checkins() {
        for text in [
            "SOCAL Approach, C-GTLT, checking in",
            "SOCAL Approach, C-GTLT, with you",
            "check-in",
        ] {
            assert_eq!(super::interpret(text).intent, "checkin");
        }
    }
    #[test]
    fn traffic_reports_keep_the_negation() {
        for text in [
            "Negative contact",
            "SOCAL Approach, C-GTLT, traffic not in sight",
            "traffic not sighted",
        ] {
            assert_eq!(super::interpret(text).intent, "traffic_not_sighted");
        }
        assert_eq!(
            super::interpret("C-GTLT, traffic in sight").intent,
            "traffic_sighted"
        );
    }
}
