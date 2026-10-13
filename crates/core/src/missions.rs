//! Editable flight stories. Events describe the simulated cabin, not aircraft systems.
use crate::state::PhaseCode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
/// One portable mission definition.
pub struct Mission {
    /// Content format version.
    pub version: u32,
    /// Stable file and event namespace.
    pub id: String,
    /// Browser title.
    pub title: String,
    /// Preflight story and objectives.
    pub briefing: String,
    /// ICAO aircraft types, not parking width classes.
    pub aircraft: Vec<String>,
    /// Suggested origin; loading a mission does not replace a dispatch plan.
    pub departure: String,
    /// Suggested destination.
    pub destination: String,
    /// Approximate flying time, excluding ground preparation.
    pub duration_minutes: u32,
    /// Optional future briefing media; all missions also have a text briefing.
    #[serde(default)]
    pub media: Vec<Media>,
    /// Ordered, conditional story events.
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
/// Portable media reference. Media is optional and never executes code.
pub struct Media {
    /// image, audio or video.
    pub kind: String,
    /// Relative asset path or HTTPS URL.
    pub source: String,
    /// Accessible description, caption or transcript.
    pub transcript: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
/// An event can fire once, after its phase delay and optional earlier outcome.
pub struct Event {
    /// Unique within the mission.
    pub id: String,
    /// Phase name: parked, clearance, pushback, taxi, departure, cruise, arrival, approach, landed or taxi_in.
    pub phase: String,
    /// Unpaused seconds spent in the selected phase.
    pub delay_seconds: u32,
    /// Probability of this event on this flight, from zero to one hundred.
    pub probability: u8,
    /// Optional prerequisite event ID.
    #[serde(default)]
    pub after_event: String,
    /// Required choice outcome from that event.
    #[serde(default)]
    pub after_outcome: String,
    /// normal, urgent or emergency; suppresses cabin jokes while urgent events are open.
    pub severity: String,
    /// Optional fictional passenger seat, such as 12A.
    #[serde(default)]
    pub seat: String,
    /// Alternative attendant lines; {seat} and {passengers} are the only slots.
    pub say: Vec<String>,
    /// Responses the captain can give; these never grant an ATC clearance.
    pub choices: Vec<Choice>,
    /// Optional future event media.
    #[serde(default)]
    pub media: Vec<Media>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
/// A branch chosen by the captain.
pub struct Choice {
    /// Unique within the event, also used by the request API.
    pub id: String,
    /// Button label and captain's transmission.
    pub label: String,
    /// Unambiguous spoken/typed aliases for this choice.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Branch value retained for later conditions and the debrief.
    pub outcome: String,
    /// Alternative attendant acknowledgements.
    pub say: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
/// Small live snapshot for the mission page and crew context.
pub struct Status {
    /// Selected mission ID.
    pub id: String,
    /// Display title.
    pub title: String,
    /// True until stopped or the flight ends.
    pub running: bool,
    /// Current story event, if one needs a captain's response.
    pub active_event: String,
    /// Current event severity.
    pub severity: String,
    /// Most recent urgent incident report, retained until mission completion.
    #[serde(default)]
    pub incident: String,
    /// Available responses to the current event.
    pub choices: Vec<Choice>,
    /// Ordered record of event outcomes; no scoring of medical treatment.
    pub debrief: Vec<String>,
}

/// Convert an editable phase name to the engine's phase.
#[must_use]
pub fn phase(name: &str) -> Option<PhaseCode> {
    Some(match name {
        "parked" => PhaseCode::Parked,
        "clearance" => PhaseCode::Clearance,
        "pushback" => PhaseCode::Pushback,
        "taxi" => PhaseCode::Taxi,
        "departure" => PhaseCode::Departure,
        "cruise" => PhaseCode::Cruise,
        "arrival" => PhaseCode::Arrival,
        "approach" => PhaseCode::Approach,
        "landed" => PhaseCode::Landed,
        "taxi_in" => PhaseCode::TaxiIn,
        _ => return None,
    })
}

/// File-safe mission identifier.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn valid_media(media: &[Media]) -> bool {
    media.len() <= 8
        && media.iter().all(|m| {
            ["image", "audio", "video"].contains(&m.kind.as_str())
                && !m.transcript.trim().is_empty()
                && m.transcript.len() <= 8000
                && m.source.len() <= 1024
                && !m.source.is_empty()
                && (m.source.starts_with("https://")
                    || (!m.source.starts_with('/')
                        && !m.source.contains([':', '\\'])
                        && !m.source.split('/').any(|p| p == "..")))
        })
}

fn lines_valid(lines: &[String]) -> bool {
    !lines.is_empty()
        && lines.len() <= 24
        && lines.iter().all(|line| {
            let plain = line.replace("{seat}", "").replace("{passengers}", "");
            !line.trim().is_empty() && line.len() <= 1500 && !plain.contains(['{', '}'])
        })
}

impl Mission {
    /// Validate before saving or starting a mission.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 || !valid_id(&self.id) {
            return Err("Mission needs version 1 and an ID using lowercase letters, digits, hyphens or underscores.".into());
        }
        if self.title.trim().is_empty()
            || self.title.len() > 160
            || self.briefing.trim().is_empty()
            || self.briefing.len() > 8000
        {
            return Err("Supply a title and a briefing (up to 8,000 characters).".into());
        }
        if self.aircraft.is_empty()
            || self.aircraft.len() > 32
            || self.aircraft.iter().any(|a| {
                !(2..=4).contains(&a.len())
                    || !a
                        .bytes()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            })
        {
            return Err(
                "Aircraft must be ICAO types such as A320 or A20N, not a parking size.".into(),
            );
        }
        if [&self.departure, &self.destination].iter().any(|a| {
            a.len() != 4
                || !a
                    .bytes()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        }) || !(1..=720).contains(&self.duration_minutes)
        {
            return Err("Supply airport identifiers and a duration of 1–720 minutes.".into());
        }
        if self.events.is_empty() || self.events.len() > 64 || !valid_media(&self.media) {
            return Err(
                "Supply 1–64 events and valid optional media references with transcripts.".into(),
            );
        }
        for (i, event) in self.events.iter().enumerate() {
            if !valid_id(&event.id)
                || self.events[..i].iter().any(|e| e.id == event.id)
                || phase(&event.phase).is_none()
                || event.delay_seconds > 43200
                || event.probability > 100
                || !["normal", "urgent", "emergency"].contains(&event.severity.as_str())
            {
                return Err(format!(
                    "Invalid ID, phase, timing, probability or severity for event {}.",
                    event.id
                ));
            }
            if !event.after_event.is_empty()
                && !self.events[..i].iter().any(|e| {
                    e.id == event.after_event
                        && e.choices.iter().any(|c| c.outcome == event.after_outcome)
                })
            {
                return Err(format!(
                    "Event {} must depend on an earlier event's defined outcome.",
                    event.id
                ));
            }
            if event.seat.len() > 5
                || !event
                    .seat
                    .bytes()
                    .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
                || (event.seat.is_empty() && event.say.iter().any(|s| s.contains("{seat}")))
            {
                return Err(format!("Invalid or missing seat for {}.", event.id));
            }
            if !lines_valid(&event.say)
                || !valid_media(&event.media)
                || event.choices.is_empty()
                || event.choices.len() > 8
            {
                return Err(format!(
                    "Event {} needs valid lines and 1–8 choices.",
                    event.id
                ));
            }
            for (j, c) in event.choices.iter().enumerate() {
                if !valid_id(&c.id)
                    || !valid_id(&c.outcome)
                    || event.choices[..j].iter().any(|p| p.id == c.id)
                    || c.label.trim().is_empty()
                    || c.label.len() > 500
                    || c.aliases.len() > 12
                    || c.aliases
                        .iter()
                        .any(|a| a.trim().len() < 4 || a.len() > 200)
                    || !lines_valid(&c.say)
                {
                    return Err(format!("Invalid choice in event {}.", event.id));
                }
            }
        }
        Ok(())
    }

    /// Match the actual aircraft, with dispatch type as a fallback for standalone testing.
    #[must_use]
    pub fn compatible(&self, aircraft: &str) -> bool {
        self.aircraft
            .iter()
            .any(|a| a.eq_ignore_ascii_case(aircraft.trim()))
    }
}

/// Read a single editable mission.
pub fn parse(text: &str) -> Result<Mission, String> {
    if text.len() > 256 * 1024 {
        return Err("Mission exceeds 256 KiB.".into());
    }
    let mission: Mission = toml::from_str(text).map_err(|e| e.to_string())?;
    mission.validate()?;
    Ok(mission)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_paths_unsupported_slots_and_missing_branches() {
        let original = parse(include_str!("../../../missions/coastal-medical.toml")).unwrap();
        assert!(original.compatible("A20N"));
        assert!(!original.compatible("C"));
        let mut mission = original.clone();
        mission.media.push(Media {
            kind: "video".into(),
            source: "../private.mp4".into(),
            transcript: "Briefing".into(),
        });
        assert!(mission.validate().is_err());
        mission = original.clone();
        mission.events[0].say = vec!["{execute}".into()];
        assert!(mission.validate().is_err());
        mission = original.clone();
        mission.events[1].after_event = "cabin_call".into();
        mission.events[1].after_outcome = "undefined".into();
        assert!(mission.validate().is_err());
        mission.events[1].after_outcome = "coordinating".into();
        assert!(mission.validate().is_ok());
        mission = original;
        mission.aircraft = vec!["C".into()];
        assert!(mission.validate().is_err());
    }
}
