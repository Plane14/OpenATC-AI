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
    /// Last taxi segment covered by this one instruction; never spans a runway crossing.
    pub taxi_through: Option<usize>,
}

/// The runway actually used, retained independently of controller wording.
#[derive(Clone)]
pub(super) struct RunwayReport {
    pub airport: String,
    pub runway: String,
}

#[derive(Clone)]
pub(super) struct Prepared {
    pub route: Arc<Route>,
    pub recovery: BTreeMap<usize, Arc<Route>>,
    pub clearances: BTreeMap<usize, Clearance>,
    /// Keyed by the first segment after the runway exit/crossing.
    pub vacated_reports: BTreeMap<usize, RunwayReport>,
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
        for (&start, clearance) in &self.clearances {
            let Some(end) = clearance.taxi_through else {
                continue;
            };
            if end <= start || end >= stages.len() || stages[start] != Stage::TaxiIn {
                return Err("Taxi clearance has an invalid movement limit".into());
            }
            for index in start + 1..=end {
                let next = self
                    .clearances
                    .get(&index)
                    .ok_or("Taxi clearance has a missing movement")?;
                if stages[index] != Stage::TaxiIn
                    || self.vacated_reports.contains_key(&index)
                    || next.taxi_through.is_some()
                    || next.station.airport != clearance.station.airport
                    || next.station.khz != clearance.station.khz
                    || next.station.service != clearance.station.service
                    || next.station.name != clearance.station.name
                    || next.instruction != clearance.instruction
                    || next.readback != clearance.readback
                {
                    return Err(
                        "Taxi clearance crosses a separate permission or controller limit".into(),
                    );
                }
            }
        }
        for (index, pair) in stages.windows(2).enumerate() {
            if pair[0] == Stage::Crossing
                && !matches!(pair[1], Stage::TaxiIn | Stage::Crossing | Stage::Turnaround)
            {
                return Err("Runway crossing has no surface exit limit".into());
            }
            if matches!(pair[0], Stage::Rollout | Stage::Crossing)
                && matches!(pair[1], Stage::TaxiIn | Stage::Crossing | Stage::Turnaround)
            {
                let report = self
                    .vacated_reports
                    .get(&(index + 1))
                    .ok_or("Runway exit needs a typed vacated report")?;
                if report.runway.trim().is_empty()
                    || report.airport.trim().is_empty()
                    || self
                        .clearances
                        .get(&index)
                        .is_none_or(|clearance| clearance.station.airport != report.airport)
                {
                    return Err("Runway report does not match its surface controller".into());
                }
            }
        }
        if self.vacated_reports.keys().any(|&index| {
            index == 0
                || index >= stages.len()
                || !matches!(stages[index - 1], Stage::Rollout | Stage::Crossing)
                || !matches!(
                    stages[index],
                    Stage::TaxiIn | Stage::Crossing | Stage::Turnaround
                )
        }) {
            return Err("Runway report is not at a completed surface runway movement".into());
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
    if !runtime.active_flights.contains(&id) || !runtime.approved_orders.contains(&(id, segment)) {
        return;
    }
    let Some(clearance) = runtime
        .flights
        .get(&id)
        .and_then(|flight| flight.clearances.get(&segment))
    else {
        return;
    };
    if segment == 1 && clearance.initial_climb {
        runtime.approved_orders.insert((id, 2));
    }
    if let Some(end) = clearance.taxi_through {
        for index in segment + 1..=end {
            runtime.approved_orders.insert((id, index));
        }
    }
}

/// Validate on a clone so a bad recovery cannot leave an aircraft stuck in a half-installed flight.
pub(super) fn install(runtime: &mut Runtime, id: u32, prepared: &Prepared) -> Result<(), String> {
    prepared.validate()?;
    for (index, report) in &prepared.vacated_reports {
        let airport = runtime
            .surface_airport(&report.airport)
            .ok_or("Runway report airport is unavailable")?;
        let runway = airport
            .runways
            .iter()
            .find(|runway| {
                runway.first_name == report.runway || runway.second_name == report.runway
            })
            .ok_or("Runway report names an unknown runway")?;
        let movement = prepared
            .route
            .segment(index - 1)
            .ok_or("Runway report movement is unavailable")?;
        if !movement.resources.contains(&format!(
            "RUNWAY/{}/{}/{}",
            airport.icao, runway.first_name, runway.second_name
        )) {
            return Err("Runway report movement does not protect the named runway".into());
        }
    }
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
    runtime.flights.insert(id, Arc::new(prepared.clone()));
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
    let profiles = state.traffic_profiles.current();
    let targets = super::observed_targets(&state.generated, &state.session.telemetry);
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
        if pose.recovering || !state.generated.active_flights.contains(&pose.id) {
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
        if let Some(report) = prepared.vacated_reports.get(&pose.segment) {
            let previous = pose.segment - 1;
            if !state
                .generated
                .reported_crossings
                .contains(&(pose.id, previous))
            {
                let Some(airport) = state.generated.surface_airport(&report.airport) else {
                    continue;
                };
                let Some(clearance) = prepared.clearances.get(&previous) else {
                    continue;
                };
                let observed = openatc_core::state::Telemetry {
                    aircraft_icao: frame.target.aircraft.clone(),
                    latitude: pose.position.latitude,
                    longitude: pose.position.longitude,
                    altitude_feet: pose.position.altitude_feet,
                    position_valid: true,
                    on_ground: pose.stage.on_ground(),
                    heading_degrees: pose.heading,
                    wingspan_meters: frame.target.wingspan_meters,
                    ..Default::default()
                };
                if !observed.on_ground
                    || frame.target.com1_khz != Some(clearance.station.khz)
                    || !airport.runways.iter().any(|runway| {
                        runway.first_name == report.runway || runway.second_name == report.runway
                    })
                    || profiles
                        .as_ref()
                        .and_then(|profiles| profiles.aircraft_for(&frame.target.aircraft))
                        .map_or_else(
                            || openatc_core::airport::aircraft_on_runway(airport, &observed),
                            |profile| {
                                openatc_core::airport::aircraft_on_runway_for(
                                    airport, &observed, profile,
                                )
                            },
                        )
                {
                    continue;
                }
                let exchange = radio::Exchange {
                    id: pose.id,
                    segment: pose.segment,
                    station: clearance.station.clone(),
                    callsign: frame.target.callsign.clone(),
                    messages: vec![(
                        false,
                        openatc_core::dialogue::say(
                            "pilot_runway_vacated",
                            &[
                                ("callsign", frame.target.callsign.clone()),
                                ("runway", report.runway.clone()),
                            ],
                        ),
                    )],
                    next: 0,
                    due: Instant::now(),
                    grant: false,
                    tune_after: None,
                };
                state
                    .generated
                    .reported_crossings
                    .insert((pose.id, previous));
                return Some(exchange);
            }
        }
        if pose.complete {
            continue;
        }
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
        let Some(clearance) = prepared.clearances.get(&index) else {
            continue;
        };
        if state.generated.crossing_blocked(pose.id, index, &targets) {
            if state.generated.approved_orders.remove(&(pose.id, index)) {
                let runway = prepared
                    .vacated_reports
                    .get(&(index + 1))
                    .map(|report| report.runway.as_str());
                return Some(radio::cancel_crossing(
                    radio::Exchange {
                        id: pose.id,
                        segment: index,
                        station: clearance.station.clone(),
                        callsign: frame.target.callsign.clone(),
                        messages: vec![],
                        next: 0,
                        due: Instant::now(),
                        grant: false,
                        tune_after: None,
                    },
                    runway,
                ));
            }
            continue;
        }
        if !current_handoff && state.generated.approved_orders.contains(&(pose.id, index)) {
            continue;
        }
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
        if let Some(end) = clearance.taxi_through
            && !state.generated.world.reserve_surface_until(
                state.generated.world.generation(),
                pose.id,
                index,
                end,
                &blocked,
            )
        {
            continue;
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
                            taxi_through: None,
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
                    resources: if *stage == Stage::Rollout {
                        vec!["RUNWAY/TEST/09/27".into()]
                    } else if stage.on_ground() {
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
            vacated_reports: BTreeMap::from([(
                7,
                RunwayReport {
                    airport: "TEST".into(),
                    runway: "09".into(),
                },
            )]),
        };
        let mut runtime = Runtime::default();
        runtime.epoch = Some(state.session_generation);
        runtime.policy =
            Some(parse_operations(include_str!("../../../../traffic/operations.toml")).unwrap());
        runtime.airport = Some(Arc::new(Airport {
            icao: "TEST".into(),
            runways: vec![openatc_core::airport::Runway {
                first_name: "09".into(),
                second_name: "27".into(),
                width: 45.0,
                first: Point {
                    east: 12000.0,
                    ..Default::default()
                },
                second: Point {
                    east: 13500.0,
                    ..Default::default()
                },
                ..Default::default()
            }],
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
    fn one_taxi_readback_covers_only_its_explicit_contiguous_route() {
        let (mut state, mut prepared) = fixture();
        let mut segments: Vec<_> = (0..32)
            .map_while(|index| prepared.route.segment(index).cloned())
            .collect();
        segments[7].points = vec![point(14000.0, 0.0), point(14250.0, 0.0)];
        segments.insert(
            8,
            Segment {
                stage: Stage::TaxiIn,
                points: vec![point(14250.0, 0.0), point(14500.0, 0.0)],
                knots: 10.0,
                resources: vec!["PHASE/8".into()],
                dwell_seconds: 0.0,
            },
        );
        prepared.route = Arc::new(Route::compile(segments, 4000.0, 4000.0).unwrap());
        prepared
            .clearances
            .insert(8, prepared.clearances[&7].clone());
        prepared.clearances.get_mut(&7).unwrap().taxi_through = Some(8);
        prepared.validate().unwrap();
        let mut invalid = prepared.clone();
        invalid.clearances.get_mut(&7).unwrap().taxi_through = Some(9);
        assert!(
            invalid.validate().is_err(),
            "Taxi permission cannot include parking completion"
        );
        invalid = prepared.clone();
        invalid.clearances.get_mut(&8).unwrap().station.khz = 121950;
        assert!(
            invalid.validate().is_err(),
            "Taxi permission cannot bypass a controller change"
        );
        invalid = prepared.clone();
        invalid.clearances.get_mut(&8).unwrap().instruction = "A separate taxi instruction".into();
        assert!(
            invalid.validate().is_err(),
            "Taxi permission cannot absorb another instruction"
        );
        install(&mut state.generated, 1, &prepared).unwrap();
        acknowledge(&mut state.generated, 1, 7);
        assert!(
            !state.generated.approved_orders.contains(&(1, 8)),
            "Preparing a readback must not grant its route"
        );
        let mut taxi_seen = false;
        let mut corridor_reserved = false;
        for second in 31..1100 {
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            super::super::radio_tick(&mut state);
            if state
                .generated
                .exchange
                .as_ref()
                .is_some_and(|exchange| exchange.segment == 7 && exchange.grant)
            {
                corridor_reserved = true;
                assert!(!state.generated.approved_orders.contains(&(1, 7)));
                assert!(state.generated.world.snapshot()[0].awaiting_clearance);
                let mut probe = state.generated.world.clone();
                let other = Route::compile(
                    vec![Segment {
                        stage: Stage::TaxiIn,
                        points: vec![point(14250.0, 0.0), point(14500.0, 0.0)],
                        knots: 10.0,
                        resources: vec!["PHASE/8".into()],
                        dwell_seconds: 0.0,
                    }],
                    4000.0,
                    4000.0,
                )
                .unwrap();
                assert!(probe.insert(0, 2, Arc::new(other)));
                assert!(
                    !probe.reserve(0, 2, 0, &BTreeSet::new()),
                    "Full taxi corridor must be reserved while its readback is pending"
                );
            }
            if state
                .generated
                .exchange
                .as_ref()
                .is_some_and(|exchange| exchange.segment == 8 && exchange.grant)
            {
                panic!("A covered taxi edge generated a redundant clearance");
            }
            state.session.telemetry.simulation_seconds = Some(f64::from(second));
            super::super::advance(&mut state.generated, &state.session.telemetry);
            let pose = &state.generated.world.snapshot()[0];
            if pose.segment == 8 {
                taxi_seen = true;
                assert!(state.generated.approved_orders.contains(&(1, 7)));
                assert!(state.generated.approved_orders.contains(&(1, 8)));
            }
            if pose.complete && pose.stage == Stage::Turnaround {
                break;
            }
        }
        let pose = &state.generated.world.snapshot()[0];
        assert!(corridor_reserved && taxi_seen && pose.complete && pose.stage == Stage::Turnaround);
    }

    #[test]
    fn landing_permission_checks_player_and_native_runway_occupants() {
        let (mut state, prepared) = fixture();
        install(&mut state.generated, 1, &prepared).unwrap();
        for second in 31..900 {
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            super::super::radio_tick(&mut state);
            state.session.telemetry.simulation_seconds = Some(f64::from(second));
            super::super::advance(&mut state.generated, &state.session.telemetry);
            let pose = &state.generated.world.snapshot()[0];
            if pose.stage == Stage::Approach
                && pose.seconds_to_limit
                    <= state
                        .generated
                        .policy
                        .as_ref()
                        .unwrap()
                        .flight_clearance_lead_seconds
            {
                break;
            }
        }
        assert_eq!(state.generated.world.snapshot()[0].stage, Stage::Approach);
        assert!(!state.generated.approved_orders.contains(&(1, 6)));
        state.generated.exchange = None;
        let occupant = TrafficTarget {
            id: 88,
            aircraft: "B77W".into(),
            callsign: "ARR88".into(),
            longitude: 12700.0 / 111320.0,
            track_degrees: 90.0,
            on_ground: true,
            ..Default::default()
        };
        state.session.telemetry.traffic = Some(vec![occupant.clone()]);
        let blocked =
            super::super::external_surface_cells(&state.generated, &state.session.telemetry);
        assert!(blocked.contains("RUNWAY/TEST/09/27"));
        assert!(
            !state
                .generated
                .world
                .clone()
                .authorise_continuation(0, 1, 6, &blocked)
        );
        assert!(
            exchange(&mut state).is_none(),
            "Native runway occupant must prevent landing permission"
        );
        state.session.telemetry.traffic = Some(vec![]);
        assert!(
            exchange(&mut state).is_some_and(|exchange| exchange.segment == 6 && exchange.grant)
        );
        state.session.telemetry.longitude = occupant.longitude;
        state.session.telemetry.heading_degrees = occupant.track_degrees;
        assert!(
            exchange(&mut state).is_none(),
            "Player runway occupant must prevent landing permission"
        );
    }

    #[test]
    fn active_taxi_edges_cannot_inherit_a_previous_segment_permission() {
        let (mut state, _) = fixture();
        state.generated.world = Default::default();
        let route = Route::compile(
            (0..2)
                .map(|index| Segment {
                    stage: Stage::TaxiIn,
                    points: vec![
                        point(f64::from(index) * 100.0, 0.0),
                        point(f64::from(index + 1) * 100.0, 0.0),
                    ],
                    knots: 10.0,
                    resources: vec![],
                    dwell_seconds: 0.0,
                })
                .collect(),
            4000.0,
            4000.0,
        )
        .unwrap();
        assert!(state.generated.world.insert(0, 1, Arc::new(route)));
        state.generated.active_flights.insert(1);
        state.generated.approved_orders.insert((1, 0));
        for second in 0..80 {
            state.session.telemetry.simulation_seconds = Some(f64::from(second));
            super::super::advance(&mut state.generated, &state.session.telemetry);
        }
        let pose = &state.generated.world.snapshot()[0];
        assert_eq!(pose.segment, 1);
        assert!(pose.awaiting_clearance);
        assert_eq!(pose.position, point(100.0, 0.0));
    }

    #[test]
    fn complete_arrival_crossing_waits_for_readback_and_resumes_without_legacy_orders() {
        let (mut state, mut prepared) = fixture();
        let mut segments: Vec<_> = (0..32)
            .map_while(|index| prepared.route.segment(index).cloned())
            .collect();
        segments.pop();
        segments.extend([
            Segment {
                stage: Stage::Crossing,
                points: vec![point(14500.0, 0.0), point(15000.0, 0.0)],
                knots: 10.0,
                resources: vec!["RUNWAY/TEST/18/36".into()],
                dwell_seconds: 0.0,
            },
            Segment {
                stage: Stage::TaxiIn,
                points: vec![point(15000.0, 0.0), point(15500.0, 0.0)],
                knots: 10.0,
                resources: vec![],
                dwell_seconds: 0.0,
            },
            Segment {
                stage: Stage::Turnaround,
                points: vec![point(15500.0, 0.0)],
                knots: 0.0,
                resources: vec![],
                dwell_seconds: 0.0,
            },
        ]);
        prepared.route = Arc::new(Route::compile(segments, 4000.0, 4000.0).unwrap());
        for index in [8, 9] {
            prepared
                .clearances
                .insert(index, prepared.clearances[&7].clone());
        }
        let mut crossing_span = prepared.clone();
        crossing_span.clearances.get_mut(&7).unwrap().taxi_through = Some(8);
        assert!(
            crossing_span
                .validate()
                .unwrap_err()
                .contains("permission or controller limit"),
            "A taxi clearance cannot authorise a runway crossing"
        );
        prepared.vacated_reports.insert(
            9,
            RunwayReport {
                airport: "TEST".into(),
                runway: "18".into(),
            },
        );
        Arc::make_mut(state.generated.airport.as_mut().unwrap())
            .runways
            .push(openatc_core::airport::Runway {
                first_name: "18".into(),
                second_name: "36".into(),
                width: 45.0,
                first: Point {
                    east: 14750.0,
                    north: 1000.0,
                    height: 0.0,
                },
                second: Point {
                    east: 14750.0,
                    north: -1000.0,
                    height: 0.0,
                },
                ..Default::default()
            });
        install(&mut state.generated, 1, &prepared).unwrap();
        assert!(!state.generated.orders.contains_key(&1));
        for second in 31..1200 {
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            super::super::radio_tick(&mut state);
            state.session.telemetry.simulation_seconds = Some(f64::from(second));
            super::super::advance(&mut state.generated, &state.session.telemetry);
            if state.generated.world.snapshot()[0].segment == 8 {
                break;
            }
        }
        let stopped = state.generated.world.snapshot()[0].clone();
        assert_eq!(stopped.segment, 8);
        assert!(stopped.awaiting_clearance);
        for _ in 0..8 {
            state.session.telemetry.simulation_seconds =
                Some(state.session.telemetry.simulation_seconds.unwrap() + 1.0);
            super::super::advance(&mut state.generated, &state.session.telemetry);
            assert_eq!(
                state.generated.world.snapshot()[0].position,
                stopped.position
            );
        }
        assert!(!state.generated.approved_orders.contains(&(1, 8)));
        let final_traffic = TrafficTarget {
            id: 88,
            aircraft: "A320".into(),
            callsign: "ARR88".into(),
            latitude: 1500.0 / 111320.0,
            longitude: 14750.0 / 111320.0,
            altitude_feet: 200.0,
            track_degrees: 180.0,
            ground_speed_knots: Some(120.0),
            on_ground: false,
            ..Default::default()
        };
        state.session.telemetry.traffic = Some(vec![final_traffic.clone()]);
        for _ in 0..8 {
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            super::super::radio_tick(&mut state);
        }
        assert!(
            !state.generated.approved_orders.contains(&(1, 8)),
            "Predicted arrival must block the crossing instruction"
        );
        state.session.telemetry.traffic = Some(vec![]);
        // Let the controller and pilot speak, then introduce final traffic before accepting the readback.
        for _ in 0..8 {
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            super::super::radio_tick(&mut state);
            if state.generated.exchange.as_ref().is_some_and(|exchange| {
                exchange.segment == 8 && exchange.grant && exchange.next == exchange.messages.len()
            }) {
                break;
            }
        }
        assert!(
            state
                .generated
                .exchange
                .as_ref()
                .is_some_and(|exchange| exchange.segment == 8 && exchange.grant)
        );
        state.session.telemetry.traffic = Some(vec![final_traffic.clone()]);
        state.generated.exchange.as_mut().unwrap().due = Instant::now();
        super::super::radio_tick(&mut state);
        assert!(!state.generated.approved_orders.contains(&(1, 8)));
        assert!(
            !state.generated.exchange.as_ref().unwrap().grant,
            "A new conflict must replace the grant with cancellation/hold"
        );
        state.session.telemetry.traffic = Some(vec![]);
        for _ in 0..12 {
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            super::super::radio_tick(&mut state);
            if state.generated.approved_orders.contains(&(1, 8)) {
                break;
            }
        }
        assert!(state.generated.approved_orders.contains(&(1, 8)));
        state.session.telemetry.traffic = Some(vec![final_traffic]);
        state.session.telemetry.simulation_seconds =
            Some(state.session.telemetry.simulation_seconds.unwrap() + 1.0);
        super::super::advance(&mut state.generated, &state.session.telemetry);
        assert_eq!(
            state.generated.world.snapshot()[0].position,
            stopped.position
        );
        assert!(state.generated.world.snapshot()[0].awaiting_clearance);
        super::super::radio_tick(&mut state);
        assert!(
            !state.generated.approved_orders.contains(&(1, 8)),
            "A new conflict before entry must revoke an accepted crossing"
        );
        assert!(!state.generated.exchange.as_ref().unwrap().grant);
        state.session.telemetry.traffic = Some(vec![]);
        for _ in 0..12 {
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            super::super::radio_tick(&mut state);
            if state.generated.approved_orders.contains(&(1, 8)) {
                break;
            }
        }
        assert!(state.generated.approved_orders.contains(&(1, 8)));
        for _ in 0..300 {
            state.session.telemetry.simulation_seconds =
                Some(state.session.telemetry.simulation_seconds.unwrap() + 1.0);
            super::super::advance(&mut state.generated, &state.session.telemetry);
            if state.generated.world.snapshot()[0].segment == 9 {
                break;
            }
        }
        let pose = &state.generated.world.snapshot()[0];
        assert_eq!(
            pose.segment, 9,
            "Acknowledged complete-flight crossing remained blocked without legacy orders"
        );
        assert!(pose.awaiting_clearance);
        assert!(!state.generated.approved_orders.contains(&(1, 9)));
        super::super::radio_tick(&mut state);
        assert!(state.generated.reported_crossings.contains(&(1, 8)));
    }

    #[test]
    fn custom_profile_dimensions_do_not_use_an_unknown_aircraft_fallback() {
        let (state, _) = fixture();
        let airport = state.generated.airport.as_ref().unwrap();
        let observed = openatc_core::state::Telemetry {
            aircraft_icao: "CUSTOM".into(),
            latitude: 60.0 / 111320.0,
            longitude: 12700.0 / 111320.0,
            heading_degrees: 90.0,
            position_valid: true,
            on_ground: true,
            ..Default::default()
        };
        let mut aircraft = openatc_core::traffic::profiles::aircraft("C172")
            .unwrap()
            .clone();
        aircraft.length = 20.0;
        aircraft.wingspan = 12.0;
        assert!(openatc_core::airport::aircraft_on_runway(
            airport, &observed
        ));
        assert!(!openatc_core::airport::aircraft_on_runway_for(
            airport, &observed, &aircraft
        ));
        aircraft.length = f64::NAN;
        assert!(openatc_core::airport::aircraft_on_runway_for(
            airport, &observed, &aircraft
        ));
    }
    #[test]
    fn runway_vacated_waits_for_observed_clearance_before_ground_handoff() {
        let (mut state, prepared) = fixture();
        install(&mut state.generated, 1, &prepared).unwrap();
        Arc::make_mut(state.generated.airport.as_mut().unwrap()).runways[0]
            .second
            .east = 15000.0;
        for second in 31..900 {
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            super::super::radio_tick(&mut state);
            state.session.telemetry.simulation_seconds = Some(f64::from(second));
            super::super::advance(&mut state.generated, &state.session.telemetry);
            if state.generated.world.snapshot()[0].stage == Stage::TaxiIn {
                break;
            }
        }
        assert_eq!(state.generated.world.snapshot()[0].stage, Stage::TaxiIn);
        let stopped = state.generated.world.snapshot()[0].position;
        for offset in 1..=8 {
            super::super::radio_tick(&mut state);
            let seconds = state.session.telemetry.simulation_seconds.unwrap();
            state.session.telemetry.simulation_seconds = Some(seconds + f64::from(offset));
            super::super::advance(&mut state.generated, &state.session.telemetry);
            let pose = &state.generated.world.snapshot()[0];
            assert!(
                pose.awaiting_clearance,
                "Taxi-in moved without its readback"
            );
            assert_eq!(pose.position, stopped);
        }
        assert!(!state.generated.reported_crossings.contains(&(1, 6)));
        assert_eq!(state.generated.frames[0].target.com1_khz, Some(120950));
        assert!(!state.generated.approved_orders.contains(&(1, 7)));
        Arc::make_mut(state.generated.airport.as_mut().unwrap()).runways[0]
            .second
            .east = 13500.0;
        super::super::radio_tick(&mut state);
        assert!(state.generated.reported_crossings.contains(&(1, 6)));
        let exchange = state.generated.exchange.as_ref().unwrap();
        assert_eq!(exchange.station.khz, 120950);
        assert!(!exchange.grant && exchange.tune_after.is_none());
    }
    #[test]
    fn complete_flight_permissions_reach_parking_and_bad_recovery_is_atomic() {
        let (mut state, prepared) = fixture();
        let mut missing_report = prepared.clone();
        missing_report.vacated_reports.clear();
        assert!(install(&mut state.generated, 1, &missing_report).is_err());
        let mut wrong_controller = prepared.clone();
        wrong_controller
            .vacated_reports
            .get_mut(&7)
            .unwrap()
            .airport = "OTHER".into();
        assert!(install(&mut state.generated, 1, &wrong_controller).is_err());
        let mut unknown_runway = prepared.clone();
        unknown_runway.vacated_reports.get_mut(&7).unwrap().runway = "99".into();
        assert!(install(&mut state.generated, 1, &unknown_runway).is_err());
        let mut broken = prepared.clone();
        broken.recovery.remove(&2);
        assert!(install(&mut state.generated, 1, &broken).is_err());
        assert!(state.generated.flights.is_empty());
        assert!(state.generated.active_flights.is_empty());
        assert_eq!(state.generated.world.snapshot()[0].stage, Stage::TaxiOut);
        assert!(state.generated.world.snapshot()[0].complete);
        install(&mut state.generated, 1, &prepared).unwrap();
        assert!(state.generated.world.snapshot()[0].awaiting_clearance);
        assert!(state.generated.flights.contains_key(&1));
        state.settings.hear_traffic_radio = true;
        state.session.telemetry.com1_khz = 120950;
        let mut report_text = None;
        let mut vacated_reports = 0;
        let mut seen = BTreeSet::new();
        let mut frequencies = BTreeSet::new();
        let mut checkins = BTreeSet::new();
        for second in 31..900 {
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            let report_seen = state.generated.reported_crossings.contains(&(1, 6));
            super::super::radio_tick(&mut state);
            if !report_seen && state.generated.reported_crossings.contains(&(1, 6)) {
                vacated_reports += 1;
                report_text = Some(
                    state.generated.exchange.as_ref().unwrap().messages[0]
                        .1
                        .clone(),
                );
                assert_eq!(
                    state.generated.frames[0].target.com1_khz,
                    Some(120950),
                    "Vacated report must precede Ground transfer"
                );
                assert!(
                    !state.generated.approved_orders.contains(&(1, 7)),
                    "A vacated report cannot grant taxi-in"
                );
            }
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
        assert_eq!(vacated_reports, 1);
        let report_text = report_text.unwrap();
        let heard: Vec<_> = state
            .session
            .transcript
            .iter()
            .filter(|entry| entry.speaker == "TEST123" && entry.text == report_text)
            .collect();
        assert_eq!(heard.len(), 1, "Runway vacated must be heard exactly once");
        assert!(heard[0].background);
        assert_eq!(heard[0].radio, Some(openatc_core::state::Radio::Com1));
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
