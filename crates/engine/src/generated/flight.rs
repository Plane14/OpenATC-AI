//! Install a complete checked flight at the departure hold without granting movement.
use super::*;
use openatc_core::stations::Station;
use openatc_core::traffic::motion::{Route, Stage};

#[derive(Clone)]
pub(super) struct Clearance {
    pub station: Station,
    pub instruction: String,
    pub readback: String,
}

#[derive(Clone)]
pub(super) struct Prepared {
    pub route: Arc<Route>,
    pub recovery: BTreeMap<usize, Arc<Route>>,
    pub clearances: BTreeMap<usize, Clearance>,
}

impl Prepared {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .route
            .segment(0)
            .is_none_or(|segment| segment.stage != Stage::LineUp)
            || self
                .route
                .segment(1)
                .is_none_or(|segment| segment.stage != Stage::Takeoff)
        {
            return Err(
                "Generated flight must begin with separate lineup and takeoff limits".into(),
            );
        }
        let mut stages = Vec::new();
        for index in 0..512 {
            let Some(segment) = self.route.segment(index) else {
                break;
            };
            stages.push(segment.stage);
            if segment.stage != Stage::Turnaround {
                let clearance = self
                    .clearances
                    .get(&index)
                    .ok_or_else(|| format!("No generated clearance for segment {index}"))?;
                if !(118000..=136990).contains(&clearance.station.khz)
                    || clearance.instruction.trim().is_empty()
                    || clearance.instruction.len() > 4096
                    || clearance.readback.trim().is_empty()
                    || clearance.readback.len() > 4096
                {
                    return Err(format!("Invalid generated clearance at segment {index}"));
                }
                let services: &[&str] = match segment.stage {
                    Stage::LineUp | Stage::Takeoff | Stage::Rollout => &["Tower"],
                    Stage::TaxiIn | Stage::Crossing => &["Ground", "Tower"],
                    Stage::Climb | Stage::Cruise | Stage::Descent | Stage::Holding => {
                        &["Approach", "Center"]
                    }
                    Stage::Approach | Stage::MissedApproach => &["Approach", "Tower"],
                    _ => &[],
                };
                if !services.contains(&clearance.station.service.as_str()) {
                    return Err(format!(
                        "Wrong controller service for generated segment {index}"
                    ));
                }
            }
            if !segment.stage.on_ground()
                && segment.stage != Stage::Holding
                && !self.recovery.contains_key(&index)
            {
                return Err(format!(
                    "Generated flight has no recovery at segment {index}"
                ));
            }
            if index > 1
                && matches!(
                    segment.stage,
                    Stage::Parked
                        | Stage::TaxiOut
                        | Stage::Pushback
                        | Stage::LineUp
                        | Stage::Takeoff
                )
            {
                return Err("Departure movements reappear after the takeoff segment".into());
            }
        }
        let approach = stages
            .iter()
            .position(|stage| *stage == Stage::Approach)
            .ok_or("Generated flight has no approach")?;
        let rollout = stages
            .iter()
            .position(|stage| *stage == Stage::Rollout)
            .ok_or("Generated flight has no landing roll")?;
        let taxi = stages
            .iter()
            .position(|stage| *stage == Stage::TaxiIn)
            .ok_or("Generated flight has no taxi to parking")?;
        if approach <= 1
            || rollout <= approach
            || taxi <= rollout
            || stages.last() != Some(&Stage::Turnaround)
        {
            return Err("Generated arrival phases are incomplete or out of order".into());
        }
        if self
            .recovery
            .keys()
            .any(|index| self.route.segment(*index).is_none())
        {
            return Err("Generated recovery refers to a missing phase".into());
        }
        Ok(())
    }
}

/// Validate on a clone so a bad recovery cannot leave an aircraft stuck in a half-installed flight.
pub(super) fn install(runtime: &mut Runtime, id: u32, prepared: &Prepared) -> Result<(), String> {
    prepared.validate()?;
    let Some(pose) = runtime
        .world
        .snapshot()
        .into_iter()
        .find(|pose| pose.id == id)
    else {
        return Err("Generated aircraft no longer exists".into());
    };
    if pose.stage != Stage::TaxiOut || !pose.complete || !runtime.tower_ready.contains(&id) {
        return Err("Generated aircraft has not completed taxi and Tower check-in".into());
    }
    let mut world = runtime.world.clone();
    let generation = world.generation();
    if !world.continue_surface(generation, id, prepared.route.clone()) {
        return Err("Generated departure is discontinuous at the holding point".into());
    }
    for (boundary, route) in &prepared.recovery {
        if !world.recovery(generation, id, *boundary, route.clone()) {
            return Err(format!(
                "Generated recovery is discontinuous at segment {boundary}"
            ));
        }
    }
    runtime.world = world;
    runtime
        .approved_orders
        .retain(|(aircraft, _)| *aircraft != id);
    runtime
        .reported_crossings
        .retain(|(aircraft, _)| *aircraft != id);
    runtime
        .reported_holds
        .retain(|(aircraft, _)| *aircraft != id);
    runtime.orders.remove(&id);
    Ok(())
}

pub(super) fn exchange(state: &mut EngineState) -> Option<radio::Exchange> {
    if state.session.telemetry.traffic.is_none() {
        return None;
    }
    let blocked = super::external_surface_cells(&state.generated, &state.session.telemetry);
    for pose in state.generated.world.snapshot() {
        if pose.complete || pose.recovering {
            continue;
        }
        let Some(prepared) = state.generated.flights.get(&pose.id) else {
            continue;
        };
        let index = if pose.awaiting_clearance {
            pose.segment
        } else if !pose.stage.on_ground() || pose.stage == Stage::Takeoff {
            pose.segment + 1
        } else {
            continue;
        };
        if state.generated.approved_orders.contains(&(pose.id, index)) {
            continue;
        }
        let Some(clearance) = prepared.clearances.get(&index) else {
            continue;
        };
        let mut probe = state.generated.world.clone();
        let available = if index == pose.segment {
            probe.authorise(probe.generation(), pose.id, index, &blocked)
        } else {
            probe.authorise_continuation(probe.generation(), pose.id, index, &blocked)
        };
        if !available {
            continue;
        }
        let Some(frame) = state
            .generated
            .frames
            .iter()
            .find(|frame| frame.target.id == pose.id)
        else {
            continue;
        };
        if frame.target.com1_khz != Some(clearance.station.khz) {
            let Some(old) = state
                .stations
                .iter()
                .find(|station| Some(station.khz) == frame.target.com1_khz)
                .cloned()
            else {
                continue;
            };
            let slots = [
                ("callsign", frame.target.callsign.clone()),
                ("station", clearance.station.name.clone()),
                (
                    "frequency",
                    format!("{:.3}", f64::from(clearance.station.khz) / 1000.0),
                ),
            ];
            return Some(radio::Exchange {
                id: pose.id,
                segment: index,
                station: old,
                callsign: frame.target.callsign.clone(),
                messages: vec![
                    (true, openatc_core::dialogue::say("flight_handoff", &slots)),
                    (
                        false,
                        openatc_core::dialogue::say("pilot_frequency_readback", &slots),
                    ),
                ],
                next: 0,
                due: Instant::now(),
                grant: false,
                tune_after: Some(clearance.station.khz),
            });
        }
        return Some(radio::Exchange {
            id: pose.id,
            segment: index,
            station: clearance.station.clone(),
            callsign: frame.target.callsign.clone(),
            messages: vec![
                (true, clearance.instruction.clone()),
                (false, clearance.readback.clone()),
            ],
            next: 0,
            due: Instant::now(),
            grant: true,
            tune_after: None,
        });
    }
    None
}
