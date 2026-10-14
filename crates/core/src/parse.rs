//! Request matching and altitude, route and emergency phrase parsing.

/// Match requests before falling back to catalogue titles. Emergency calls take priority.
pub(crate) fn interpret(text: &str) -> super::identify::Interpreted {
    let normalized = text.to_lowercase();
    let mut request = super::identify::Interpreted {
        intent: "conversation".to_owned(),
        ..Default::default()
    };
    request.intent =
        super::catalog::matching_intent(&normalized).unwrap_or_else(|| "conversation".into());
    if request.intent == "emergency" {
        return request;
    }
    let grammar = super::catalog::parser();
    let altitude = regex::Regex::new(&grammar.altitude).ok();
    if let Some(expression) = altitude
        && let Some(found) = expression.captures(&normalized)
        && let Some(digits) = found.get(2)
        && let Ok(value) = digits.as_str().parse::<i32>()
    {
        let fl = found.get(1).is_some();
        if grammar
            .descent_markers
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            "descent".clone_into(&mut request.intent);
        } else {
            "altitude".clone_into(&mut request.intent);
        }
        request.altitude_feet = value * if fl { 100 } else { 1 };
        if grammar
            .metric_markers
            .iter()
            .any(|marker| normalized.contains(marker))
            && !fl
        {
            request.altitude_feet = super::meters_to_feet(request.altitude_feet);
        }
    }
    let direct = regex::Regex::new(&grammar.direct).ok();
    if let Some(expression) = direct
        && let Some(found) = expression.captures(&normalized)
        && let Some(waypoint) = found.get(1)
    {
        "direct".clone_into(&mut request.intent);
        request.waypoint = waypoint.as_str().to_ascii_uppercase();
    }
    for amendment in &grammar.amendments {
        let pattern = &amendment.expression;
        let intent = amendment.intent.as_str();
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
