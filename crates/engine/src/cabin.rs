//! Simulated passenger activity; aircraft controls still require simulator readback.
use crate::EngineState;
use openatc_core::{
    dialogue::{phrases, say},
    state::PhaseCode,
};
use serde::Serialize;
use std::time::Instant;

#[derive(Default, Serialize)]
pub struct Cabin {
    /// Planned passenger count; zero means no manifest is available.
    passengers: i32,
    /// Simulated occupants, not a reading from an aircraft lavatory sensor.
    lavatory: u32,
    pub preparing_for_turbulence: bool,
    #[serde(skip)]
    last_update: Option<Instant>,
    cruise_seconds: f64,
    preparation_seconds: f64,
    #[serde(skip)]
    report_due: bool,
    preparation_reported: bool,
}

impl Cabin {
    pub fn tick(&mut self, phase: PhaseCode, passengers: i32, paused: bool, belts: Option<bool>) {
        let now = Instant::now();
        let elapsed = self
            .last_update
            .replace(now)
            .map_or(0.0, |last| now.duration_since(last).as_secs_f64().min(2.0));
        self.advance(phase, passengers, if paused { 0.0 } else { elapsed }, belts);
    }

    fn advance(&mut self, phase: PhaseCode, passengers: i32, seconds: f64, belts: Option<bool>) {
        if passengers != self.passengers {
            self.passengers = passengers.max(0);
            self.lavatory = 0;
            self.cruise_seconds = 0.0;
        }
        if self.passengers <= 0 {
            self.lavatory = 0;
            return;
        }
        if self.preparing_for_turbulence {
            self.preparation_seconds += seconds;
            if self.preparation_seconds >= 45.0 {
                self.lavatory = 0;
                if belts == Some(true) && !self.preparation_reported {
                    self.report_due = true;
                }
            }
        } else if phase == PhaseCode::Cruise && belts == Some(false) {
            self.cruise_seconds += seconds;
            // One passenger uses the lavatory during the middle of each service cycle.
            self.lavatory = u32::from(
                self.cruise_seconds % 600.0 >= 120.0 && self.cruise_seconds % 600.0 < 210.0,
            );
        } else {
            self.lavatory = 0;
        }
    }
}

fn belts(state: &EngineState) -> Option<bool> {
    state
        .crew
        .observed_at
        .filter(|at| at.elapsed().as_secs() < 3)?;
    state.crew.values.get("seatbelts").map(|v| *v > 0.0)
}

pub fn tick(state: &mut EngineState) {
    let belts = belts(state);
    state.cabin.tick(
        state.session.phase,
        state.session.plan.passengers,
        state.session.telemetry.paused,
        belts,
    );
}

/// Answer cabin reports and turbulence briefings without granting aircraft permissions.
pub fn report(state: &mut EngineState, text: &str) -> Option<String> {
    let text = text.to_lowercase();
    let matches = |key| phrases(key).iter().any(|p| text.contains(p));
    let warning = matches("cabin_request_turbulence");
    let clear = matches("cabin_request_turbulence_clear");
    let report = matches("cabin_request_passengers");
    if !warning && !clear && !report {
        return None;
    }
    if !state.session.mission.incident.is_empty() {
        return Some(state.session.mission.incident.clone());
    }
    tick(state);
    if clear {
        state.cabin.preparing_for_turbulence = false;
        state.cabin.preparation_seconds = 0.0;
        state.cabin.report_due = false;
        state.cabin.preparation_reported = false;
        return Some(say("cabin_turbulence_clear", &[]));
    }
    let mut reply = String::new();
    if warning {
        if !state.cabin.preparing_for_turbulence {
            state.cabin.preparation_seconds = 0.0;
            state.cabin.report_due = false;
            state.cabin.preparation_reported = false;
        }
        state.cabin.preparing_for_turbulence = true;
        reply.push_str(&say("cabin_turbulence_preparing", &[]));
        reply.push(' ');
        reply.push_str(&say(
            if belts(state) == Some(true) {
                "cabin_belts_on"
            } else {
                "cabin_belts_pending"
            },
            &[],
        ));
    }
    if report || warning {
        if !reply.is_empty() {
            reply.push(' ');
        }
        if state.cabin.passengers <= 0 {
            reply.push_str(&say("cabin_manifest_unknown", &[]));
        } else {
            let phase = if state.cabin.preparing_for_turbulence {
                "preparing"
            } else {
                match state.session.phase {
                    PhaseCode::Parked | PhaseCode::Clearance => "boarding",
                    PhaseCode::Pushback | PhaseCode::Taxi | PhaseCode::Departure => "departure",
                    PhaseCode::Cruise
                        if state
                            .crew
                            .values
                            .get("cabin_brightness")
                            .is_some_and(|v| *v <= 30.0) =>
                    {
                        "resting"
                    }
                    PhaseCode::Cruise => "cruise",
                    PhaseCode::Arrival | PhaseCode::Approach => "arrival",
                    PhaseCode::Landed | PhaseCode::TaxiIn => "landed",
                    PhaseCode::Finished => "finished",
                }
            };
            reply.push_str(&say(
                &format!("cabin_passengers_{phase}"),
                &[("passengers", state.cabin.passengers.to_string())],
            ));
            if state.cabin.lavatory > 0 {
                reply.push(' ');
                reply.push_str(&say("cabin_passenger_lavatory", &[]));
            }
        }
    }
    Some(reply)
}

pub fn follow_up(state: &mut EngineState) {
    if !state.cabin.report_due
        || belts(state) != Some(true)
        || !state.session.crew_actions.is_empty()
    {
        return;
    }
    state.cabin.report_due = false;
    // Keep the warning active until the captain reports clear of the rough air.
    state.cabin.preparation_reported = true;
    crate::crew::speak(state, "cabin", &say("cabin_passengers_seated", &[]));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lavatory_report_persists_until_preparation_finishes_and_sign_is_on() {
        let mut cabin = Cabin::default();
        cabin.advance(PhaseCode::Cruise, 142, 150.0, Some(false));
        assert_eq!(cabin.lavatory, 1);
        cabin.preparing_for_turbulence = true;
        cabin.advance(PhaseCode::Cruise, 142, 20.0, Some(false));
        assert_eq!(cabin.lavatory, 1);
        assert!(!cabin.report_due);
        cabin.advance(PhaseCode::Cruise, 142, 25.0, Some(false));
        assert_eq!(cabin.lavatory, 0);
        assert!(!cabin.report_due);
        cabin.advance(PhaseCode::Cruise, 142, 1.0, Some(true));
        assert!(cabin.report_due);
    }
    #[test]
    fn unknown_manifest_does_not_invent_passengers_or_sleeping_during_startup() {
        let mut state = crate::test_state();
        assert!(
            report(&mut state, "how are the passengers")
                .unwrap()
                .contains("count")
        );
        state.session.plan.passengers = 142;
        let text = report(&mut state, "how are the passengers").unwrap();
        assert!(text.contains("142"));
        assert!(!text.contains("sleep") && !text.contains("landing"));
    }
}
