//! Install a complete checked flight at the departure hold without granting movement.
use super::*;
use openatc_core::stations::Station;
use openatc_core::traffic::motion::{Route, Stage};

#[derive(Clone)]
pub(super) struct Clearance {
    pub station: Station,
    pub instruction: String,
    pub readback: String,
    /// The takeoff instruction also clears the first planned climb segment.
    pub initial_climb: bool,
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
        if self
            .route
            .segment(2)
            .is_none_or(|segment| segment.stage != Stage::Climb)
            || self
                .clearances
                .get(&1)
                .is_none_or(|clearance| !clearance.initial_climb)
            || self
                .clearances
                .iter()
                .any(|(index, clearance)| *index != 1 && clearance.initial_climb)
        {
            return Err("Takeoff needs an explicit initial-climb permission".into());
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

pub(super) fn acknowledge(runtime: &mut Runtime, id: u32, segment: usize) {
    if runtime.active_flights.contains(&id)
        && segment == 1
        && runtime.flights.get(&id).is_some_and(|flight| {
            flight
                .clearances
                .get(&1)
                .is_some_and(|clearance| clearance.initial_climb)
        })
    {
        runtime.approved_orders.insert((id, 2));
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
    runtime
        .pending_checkins
        .retain(|(aircraft, _)| *aircraft != id);
    runtime.orders.remove(&id);
    runtime.active_flights.insert(id);
    runtime.handoff_targets.insert(id, 0);
    Ok(())
}

pub(super) fn exchange(state: &mut EngineState) -> Option<radio::Exchange> {
    if state.session.telemetry.traffic.is_none() {
        return None;
    }
    let blocked = super::external_surface_cells(&state.generated, &state.session.telemetry);
    let mut poses = state.generated.world.snapshot();
    poses.sort_by(|left, right| {
        let deadline = |pose: &openatc_core::traffic::motion::Pose| {
            if pose.awaiting_clearance {
                0.0
            } else {
                pose.seconds_to_limit
            }
        };
        deadline(left)
            .total_cmp(&deadline(right))
            .then_with(|| left.id.cmp(&right.id))
    });
    for pose in poses {
        if pose.complete || pose.recovering || !state.generated.active_flights.contains(&pose.id) {
            continue;
        }
        let Some(prepared) = state.generated.flights.get(&pose.id) else {
            continue;
        };
        let Some(frame) = state
            .generated
            .frames
            .iter()
            .find(|frame| frame.target.id == pose.id)
        else {
            continue;
        };
        if let Some(index) = state
            .generated
            .pending_checkins
            .iter()
            .find_map(|(id, index)| (*id == pose.id).then_some(*index))
            && let Some(clearance) = prepared.clearances.get(&index)
            && frame.target.com1_khz == Some(clearance.station.khz)
        {
            let slots = [
                ("station", clearance.station.name.clone()),
                ("callsign", frame.target.callsign.clone()),
            ];
            let exchange = radio::Exchange {
                id: pose.id,
                segment: index,
                station: clearance.station.clone(),
                callsign: frame.target.callsign.clone(),
                messages: vec![
                    (
                        false,
                        openatc_core::dialogue::say("pilot_station_checkin", &slots),
                    ),
                    (true, openatc_core::dialogue::say("flight_checkin", &slots)),
                ],
                next: 0,
                due: Instant::now(),
                grant: false,
                tune_after: None,
            };
            state.generated.pending_checkins.remove(&(pose.id, index));
            return Some(exchange);
        }
        let current_handoff = state
            .generated
            .handoff_targets
            .get(&pose.id)
            .is_none_or(|index| *index < pose.segment)
            && prepared
                .clearances
                .get(&pose.segment)
                .is_some_and(|clearance| Some(clearance.station.khz) != frame.target.com1_khz);
        let index = if current_handoff || pose.awaiting_clearance {
            pose.segment
        } else if !pose.stage.on_ground() || pose.stage == Stage::Takeoff {
            if pose.seconds_to_limit
                > state
                    .generated
                    .policy
                    .as_ref()?
                    .flight_clearance_lead_seconds
            {
                continue;
            }
            pose.segment + 1
        } else {
            continue;
        };
        if !current_handoff && state.generated.approved_orders.contains(&(pose.id, index)) {
            continue;
        }
        let Some(clearance) = prepared.clearances.get(&index) else {
            continue;
        };
        let mut probe = state.generated.world.clone();
        let available = if current_handoff {
            true
        } else if index == pose.segment {
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
            let Some(old) = prepared
                .clearances
                .range(..=pose.segment)
                .rev()
                .map(|(_, clearance)| &clearance.station)
                .find(|station| Some(station.khz) == frame.target.com1_khz)
                .or_else(|| {
                    state
                        .stations
                        .iter()
                        .find(|station| Some(station.khz) == frame.target.com1_khz)
                })
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

#[cfg(test)]
mod tests {
    use super::*;
    use openatc_core::traffic::motion::{Segment, Waypoint};

    fn point(east: f64, altitude: f64) -> Waypoint {
        Waypoint {
            latitude: 0.0,
            longitude: east / 111320.0,
            altitude_feet: altitude,
        }
    }
    fn fixture() -> (EngineState, Prepared) {
        let mut state = crate::test_state();
        state.settings.generated_traffic = true;
        state.settings.hear_traffic_radio = false;
        state.session.telemetry.traffic = Some(vec![]);
        state.session.telemetry.position_valid = true;
        state.session.telemetry.longitude = -0.02;
        state.session.telemetry.on_ground = true;
        let phases = [
            Stage::LineUp,
            Stage::Takeoff,
            Stage::Climb,
            Stage::Cruise,
            Stage::Descent,
            Stage::Approach,
            Stage::Rollout,
            Stage::TaxiIn,
            Stage::Turnaround,
        ];
        let vertices = [
            point(100.0, 0.0),
            point(200.0, 0.0),
            point(1000.0, 0.0),
            point(5000.0, 1000.0),
            point(8000.0, 1000.0),
            point(11000.0, 500.0),
            point(13000.0, 0.0),
            point(14000.0, 0.0),
            point(14500.0, 0.0),
        ];
        let mut clearances = BTreeMap::new();
        let mut recovery = BTreeMap::new();
        let segments = phases
            .iter()
            .enumerate()
            .map(|(index, stage)| {
                let end = vertices[(index + 1).min(vertices.len() - 1)];
                if *stage != Stage::Turnaround {
                    let (service, khz) = match stage {
                        Stage::LineUp | Stage::Takeoff | Stage::Approach | Stage::Rollout => {
                            ("Tower", 120950)
                        }
                        Stage::Climb | Stage::Descent => ("Approach", 124500),
                        Stage::Cruise => ("Center", 118675),
                        Stage::TaxiIn => ("Ground", 121705),
                        _ => unreachable!(),
                    };
                    let station = Station {
                        name: format!("Test {service}"),
                        service: service.into(),
                        airport: "TEST".into(),
                        khz,
                        ..Default::default()
                    };
                    state.stations.push(station.clone());
                    clearances.insert(
                        index,
                        Clearance {
                            station,
                            instruction: format!("Controller phase {index}."),
                            readback: format!("Pilot phase {index}."),
                            initial_climb: index == 1,
                        },
                    );
                }
                if !stage.on_ground() {
                    let mut north = end;
                    north.latitude += 1000.0 / 111320.0;
                    let mut east = end;
                    east.longitude += 1000.0 / 111320.0;
                    let mut corner = east;
                    corner.latitude = north.latitude;
                    recovery.insert(
                        index,
                        Arc::new(
                            Route::compile(
                                vec![Segment {
                                    stage: Stage::Holding,
                                    points: vec![end, east, corner, north, end],
                                    knots: 80.0,
                                    resources: vec![],
                                    dwell_seconds: 0.0,
                                }],
                                4000.0,
                                4000.0,
                            )
                            .unwrap(),
                        ),
                    );
                }
                Segment {
                    stage: *stage,
                    points: if *stage == Stage::Turnaround {
                        vec![end]
                    } else {
                        vec![vertices[index], end]
                    },
                    knots: if matches!(stage, Stage::LineUp | Stage::TaxiIn) {
                        10.0
                    } else if *stage == Stage::Turnaround {
                        0.0
                    } else {
                        80.0
                    },
                    resources: if stage.on_ground() {
                        vec![format!("PHASE/{index}")]
                    } else {
                        vec![]
                    },
                    dwell_seconds: 0.0,
                }
            })
            .collect();
        let prepared = Prepared {
            route: Arc::new(Route::compile(segments, 4000.0, 4000.0).unwrap()),
            recovery,
            clearances,
        };
        let mut runtime = Runtime::default();
        runtime.epoch = Some(state.session_generation);
        runtime.policy =
            Some(parse_operations(include_str!("../../../../traffic/operations.toml")).unwrap());
        runtime.airport = Some(Arc::new(Airport {
            icao: "TEST".into(),
            ..Default::default()
        }));
        runtime.world.insert(
            0,
            1,
            Arc::new(
                Route::compile(
                    vec![Segment {
                        stage: Stage::TaxiOut,
                        points: vec![point(0.0, 0.0), vertices[0]],
                        knots: 10.0,
                        resources: vec!["HOLD".into()],
                        dwell_seconds: 0.0,
                    }],
                    4000.0,
                    4000.0,
                )
                .unwrap(),
            ),
        );
        runtime.world.authorise(0, 1, 0, &BTreeSet::new());
        runtime.world.advance(0.0, false);
        runtime.world.advance(30.0, false);
        runtime.tower_ready.insert(1);
        runtime.frames.push(GeneratedTraffic {
            target: TrafficTarget {
                id: 1,
                callsign: "TEST123".into(),
                com1_khz: Some(120950),
                ..Default::default()
            },
            vertical_speed_fpm: 0.0,
            body_heading_degrees: None,
            model: Arc::new(openatc_core::traffic::csl::Model {
                id: "fixture".into(),
                objects: vec![],
                vertical_offset: None,
                matches: vec![],
            }),
            animation: Animation::default(),
        });
        state.generated = runtime;
        (state, prepared)
    }

    #[test]
    fn complete_flight_permissions_reach_parking_and_bad_recovery_is_atomic() {
        let (mut state, prepared) = fixture();
        let mut broken = prepared.clone();
        broken.recovery.remove(&2);
        assert!(install(&mut state.generated, 1, &broken).is_err());
        assert_eq!(state.generated.world.snapshot()[0].stage, Stage::TaxiOut);
        assert!(state.generated.world.snapshot()[0].complete);
        install(&mut state.generated, 1, &prepared).unwrap();
        assert!(state.generated.world.snapshot()[0].awaiting_clearance);
        state.generated.flights.insert(1, Arc::new(prepared));
        let mut seen = BTreeSet::new();
        let mut frequencies = BTreeSet::new();
        let mut checkins = BTreeSet::new();
        for second in 31..900 {
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            super::super::radio_tick(&mut state);
            if let Some(exchange) = &state.generated.exchange
                && !exchange.grant
                && exchange.tune_after.is_none()
                && exchange
                    .messages
                    .first()
                    .is_some_and(|(controller, _)| !controller)
            {
                checkins.insert(exchange.station.khz);
            }
            state.session.telemetry.simulation_seconds = Some(f64::from(second));
            super::super::advance(&mut state.generated, &state.session.telemetry);
            let pose = &state.generated.world.snapshot()[0];
            seen.insert(format!("{:?}", pose.stage));
            frequencies.insert(state.generated.frames[0].target.com1_khz.unwrap());
            assert!(
                !pose.recovering,
                "Normal flight entered recovery at {:?}",
                pose.stage
            );
            if pose.stage == Stage::Turnaround && pose.complete {
                break;
            }
        }
        let pose = &state.generated.world.snapshot()[0];
        assert_eq!(pose.stage, Stage::Turnaround);
        assert!(
            pose.complete,
            "Generated flight did not reach parking: {pose:?}"
        );
        assert!(frequencies.contains(&124500));
        assert!(frequencies.contains(&118675));
        assert!(frequencies.contains(&121705));
        for station in [124500, 118675, 120950, 121705] {
            assert!(
                checkins.contains(&station),
                "Missing generated check-in on {station}"
            );
        }
        for stage in [
            "Takeoff", "Climb", "Cruise", "Descent", "Approach", "Rollout", "TaxiIn",
        ] {
            assert!(seen.contains(stage), "Skipped generated phase {stage}");
        }
    }
}
