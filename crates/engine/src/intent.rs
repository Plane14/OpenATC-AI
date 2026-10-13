//! Only recognised pilot requests can cross from model output into flight logic.
use openatc_core::state::Request;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Intent {
    RadioCheck,
    Repeat,
    Standby,
    Unable,
    Clearance,
    Taxi,
    Progressive,
    Start,
    Pushback,
    StartPushback,
    Ready,
    GoAround,
    CancelIfr,
    Altitude,
    Descent,
    Direct,
    Approach,
    DepartureRunway,
    ArrivalRunway,
    ChangeApproach,
    Speed,
    Deviation,
    ResumeCourse,
    Gate,
    Frequency,
    Emergency,
    Conversation,
    Checkin,
    CrossRunway,
    Vacated,
    Readback,
    Landing,
    Position,
    TrafficSighted,
    TrafficNotSighted,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Classification {
    intent: Intent,
    #[serde(default)]
    altitude_feet: i32,
    #[serde(default)]
    waypoint: String,
}

pub fn parse(content: &str, original: &Request, role: &str) -> Result<Request, String> {
    let value: Classification =
        serde_json::from_str(content).map_err(|e| format!("Invalid pilot-request JSON: {e}"))?;
    if !(0..=60000).contains(&value.altitude_feet) || value.waypoint.len() > 120 {
        return Err("Pilot request contains an unsupported value".into());
    }
    let intent = match value.intent {
        Intent::RadioCheck => "radio_check",
        Intent::Repeat => "repeat",
        Intent::Standby => "standby",
        Intent::Unable => "unable",
        Intent::Clearance => "clearance",
        Intent::Taxi => "taxi",
        Intent::Progressive => "progressive",
        Intent::Start => "start",
        Intent::Pushback => "pushback",
        Intent::StartPushback => "start_pushback",
        Intent::Ready => "ready",
        Intent::GoAround => "go_around",
        Intent::CancelIfr => "cancel_ifr",
        Intent::Altitude => "altitude",
        Intent::Descent => "descent",
        Intent::Direct => "direct",
        Intent::Approach => "approach",
        Intent::DepartureRunway => "departure_runway",
        Intent::ArrivalRunway => "arrival_runway",
        Intent::ChangeApproach => "change_approach",
        Intent::Speed => "speed",
        Intent::Deviation => "deviation",
        Intent::ResumeCourse => "resume_course",
        Intent::Gate => "gate",
        Intent::Frequency => "frequency",
        Intent::Emergency => "emergency",
        Intent::Conversation => "conversation",
        Intent::Checkin => "checkin",
        Intent::CrossRunway => "cross_runway",
        Intent::Vacated => "vacated",
        Intent::Readback => "readback",
        Intent::Landing => "landing",
        Intent::Position => "position",
        Intent::TrafficSighted => "traffic_sighted",
        Intent::TrafficNotSighted => "traffic_not_sighted",
    };
    Ok(Request {
        intent: intent.into(),
        altitude_feet: value.altitude_feet,
        waypoint: value.waypoint,
        text: original.text.clone(),
        role: role.into(),
        ..Default::default()
    })
}

pub fn parse_or_clarify(content: &str, original: &Request, role: &str) -> Request {
    match parse(content, original, role) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("ATC intent classification rejected: {error}");
            Request {
                intent: "clarify".into(),
                text: original.text.clone(),
                role: role.into(),
                ..Default::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crew_shaped_output_requests_clarification_without_performing_actions() {
        let original = Request {
            text: "yes".into(),
            ..Default::default()
        };
        let parsed = parse_or_clarify(
            r#"{"actions":[{"target":"altitude","value":5000}]}"#,
            &original,
            "atc",
        );
        assert_eq!(parsed.intent, "clarify");
        assert_eq!(parsed.text, "yes");
        assert_eq!(parsed.altitude_feet, 0);
        assert_eq!(parsed.clearance_sequence, 0);
        assert!(!parsed.controller_initiated);
    }
    #[test]
    fn model_cannot_supply_permissions_or_readback_proof() {
        let original = Request {
            text: "request flight level one eight zero".into(),
            ..Default::default()
        };
        let parsed = parse(
            r#"{"intent":"altitude","altitudeFeet":18000,"waypoint":""}"#,
            &original,
            "atc",
        )
        .unwrap();
        assert_eq!(parsed.text, original.text);
        assert!(!parsed.controller_initiated);
        assert_eq!(parsed.clearance_sequence, 0);
        for invalid in [
            r#"{"intent":"set_dataref"}"#,
            r#"{"intent":"readback","clearanceSequence":12}"#,
            r#"{"intent":"taxi","controllerInitiated":true}"#,
            r#"{"intent":"altitude","altitudeFeet":-5000}"#,
            r#"{"intent":"altitude","altitudeFeet":"FL180"}"#,
        ] {
            assert!(parse(invalid, &original, "atc").is_err(), "{invalid}");
        }
    }
}
