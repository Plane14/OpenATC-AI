//! One onward departure from a completed arrival's own stand.
//!
//! The arrival dwell ends with the aircraft parked and every resource retained. A turnaround
//! reuses the ordinary surface, departure, airborne and arrival planners for the reverse
//! direction, keeps the stand until the aircraft moves, then continues through the same
//! controller, readback, runway, wake and corridor-protection gates as any other flight.
use super::*;
use openatc_core::parking::TrafficPreference;
use openatc_core::traffic::motion::Stage;
use openatc_core::traffic::procedures::Catalogue;
use openatc_core::traffic::profiles::Aircraft;

/// Planning inputs retained with each generated aircraft. They describe the direction it flew,
/// so the return leg is planned from the airport it actually parked at.
#[derive(Clone)]
pub(super) struct Input {
    pub profile: Aircraft,
    pub preference: TrafficPreference,
    pub general_aviation: bool,
    pub gate: bool,
    pub origin: Arc<Airport>,
    pub destination: Arc<Airport>,
    pub origin_runway: String,
    pub destination_runway: String,
    pub arrival_stand: String,
    pub departure_procedures: Option<Arc<Catalogue>>,
    pub arrival_procedures: Option<Arc<Catalogue>>,
    pub destination_surface: Option<Arc<runway::Surface>>,
    pub origin_surface: Option<Arc<runway::Surface>>,
}

/// A separately planned return flight and the surface movement that reaches its holding point.
pub(super) struct Plan {
    pub ground: GroundPlan,
    pub prepared: flight::Prepared,
    pub stand: String,
    pub stand_cells: BTreeSet<String>,
    pub corridors: BTreeSet<String>,
}

/// The next parked aircraft that has not already been offered a turnaround.
pub(super) fn eligible(runtime: &Runtime) -> Option<u32> {
    runtime
        .world
        .snapshot()
        .into_iter()
        .find(|pose| {
            pose.complete
                && pose.stage == Stage::Turnaround
                && !runtime.turnarounds_attempted.contains(&pose.id)
        })
        .map(|pose| pose.id)
}

/// Plan the reverse leg from the parked stand with the same planners used for a fresh departure.
pub(super) fn plan(
    runtime: &Runtime,
    id: u32,
    telemetry: &openatc_core::state::Telemetry,
) -> Result<Plan, String> {
    let pose = runtime
        .world
        .snapshot()
        .into_iter()
        .find(|pose| pose.id == id)
        .ok_or("Turnaround aircraft no longer exists")?;
    if !pose.complete || pose.stage != Stage::Turnaround {
        return Err("Turnaround aircraft has not completed its arrival dwell".into());
    }
    let input = runtime
        .turnaround_inputs
        .get(&id)
        .ok_or("Turnaround has no retained planning input")?;
    let handling = runtime
        .movement
        .get(&id)
        .ok_or("Turnaround aircraft has no movement envelope")?;
    let policy = runtime
        .policy
        .as_ref()
        .ok_or("Turnaround planning needs the operations policy")?;
    let callsign = runtime
        .frames
        .iter()
        .find(|frame| frame.target.id == id)
        .map(|frame| frame.target.callsign.clone())
        .ok_or("Turnaround aircraft has no callsign")?;
    let departure_weather = runtime
        .environments
        .get(&input.destination.icao)
        .cloned()
        .ok_or("Turnaround planning needs fresh departure weather")?;
    let arrival_weather = runtime
        .environments
        .get(&input.origin.icao)
        .cloned()
        .ok_or("Turnaround planning needs fresh arrival weather")?;
    if !departure_weather.fresh() || !arrival_weather.fresh() {
        return Err("Turnaround planning needs fresh airport weather".into());
    }
    let destination_surface = input
        .destination_surface
        .as_ref()
        .ok_or("Turnaround planning needs a known departure runway surface")?;
    let origin_surface = input
        .origin_surface
        .as_ref()
        .ok_or("Turnaround planning needs a known arrival runway surface")?;
    let points = runtime
        .terminal_points
        .as_deref()
        .ok_or("Turnaround planning needs published terminal coordinates")?;
    let departures = input
        .departure_procedures
        .as_deref()
        .ok_or("Turnaround planning needs published departure procedures")?;
    let arrivals = input
        .arrival_procedures
        .as_deref()
        .ok_or("Turnaround planning needs published arrival procedures")?;

    let mut observed = planning_observations(telemetry);
    observed.retain(|target| {
        !runtime
            .frames
            .iter()
            .any(|frame| frame.target.id == target.id)
    });
    observed.extend(
        runtime
            .frames
            .iter()
            .filter(|frame| frame.target.id != id && frame.target.on_ground)
            .map(|frame| frame.target.clone()),
    );
    let mut occupied = runtime.occupied_stand_cells.as_ref().clone();
    for other in runtime.world.snapshot() {
        if matches!(other.stage, Stage::Parked | Stage::Turnaround)
            && let Some(segment) = runtime.world.current_segment(other.id)
        {
            occupied.extend(segment.resources.iter().cloned());
        }
    }

    let stand_point = openatc_core::airport::airport_point(
        input.destination.as_ref(),
        pose.position.latitude,
        pose.position.longitude,
    );
    let ground = ground_route(
        input.destination.as_ref(),
        stand_point,
        pose.heading,
        input.gate,
        &input.profile,
        handling,
        &input.destination_runway,
        u64::from(id),
        policy,
        false,
        &observed,
        false,
    )?;
    let hold = ground.route.end_position();
    let surface = departure::plan(
        input.destination.as_ref(),
        &input.destination_runway,
        hold,
        &input.profile,
        handling,
        &departure_weather,
        destination_surface.as_ref(),
        policy,
    )?;
    let candidates = routing::candidates(routing::Inputs {
        origin: input.destination.as_ref(),
        destination: input.origin.as_ref(),
        departure_runway: &input.destination_runway,
        arrival_runway: &input.origin_runway,
        departures,
        arrivals,
        points,
        mora: runtime.mora.as_ref(),
        budget: policy.terminal_candidate_limit,
        direct: input.general_aviation,
    })?;
    let planner = airborne::Planner {
        regions: runtime.regions.as_ref(),
        rotation: surface.rotation,
        origin: input.destination.as_ref(),
        destination: input.origin.as_ref(),
        points,
        mora: runtime.mora.as_ref(),
        departure_weather: &departure_weather,
        arrival_weather: &arrival_weather,
        aircraft: &input.profile,
        handling,
        policy,
        departure_surface: destination_surface.as_ref(),
        arrival_surface: origin_surface.as_ref(),
        observed: &observed,
        reserved: &runtime.reserved_stands,
        occupied_stand_cells: &occupied,
        parking_exclusions: runtime.surface_corridors.as_ref(),
        preference: &input.preference,
        allow_direct: input.general_aviation,
    };
    let (decoded, arrival) = planner.arrival(&candidates, &BTreeMap::new())?;
    let stand = arrival.parking.stand.clone();
    let stand_cells = (0..512)
        .map_while(|index| arrival.parking.surface.route.segment(index))
        .last()
        .map(|segment| segment.resources.iter().cloned().collect())
        .unwrap_or_default();
    let arrival = Arc::new(arrival);
    let terminal = candidates
        .get(decoded.candidate)
        .ok_or("Turnaround selected an unknown terminal candidate")?;
    let controller = controller::Flight {
        origin: input.destination.clone(),
        destination: input.origin.clone(),
        departure_runway: input.destination_runway.clone(),
        arrival_runway: input.origin_runway.clone(),
        terminal: Arc::new(terminal.clone()),
        callsign,
        stations: runtime.stations.clone(),
        regions: runtime.regions.clone(),
        departure_weather,
        arrival_weather,
        missed: decoded.missed_instruction.clone(),
        amendment: decoded.amendment.clone(),
        scope_seconds: 900.0
            - policy.airborne_prediction_seconds
            - policy.flight_clearance_lead_seconds,
    };
    let draft = assembly::Draft::new(
        &surface,
        &decoded,
        arrival,
        &input.origin.icao,
        &input.origin_runway,
        &input.profile,
        handling,
        policy,
        Arc::new(controller),
    )?;
    let terrain = std::cell::RefCell::new(scenery::Terrain::new(std::path::Path::new(
        &runtime.simulator_root,
    ))?);
    let prepared = draft.production(&terrain, runtime.mora.as_ref())?;
    let mut corridors = BTreeSet::new();
    for index in 0..512 {
        if let Some(segment) = ground.route.segment(index) {
            corridors.extend(segment.resources.iter().cloned());
        }
        if let Some(segment) = prepared.route.segment(index)
            && segment.stage.on_ground()
        {
            corridors.extend(segment.resources.iter().cloned());
        }
    }
    Ok(Plan {
        ground,
        prepared,
        stand,
        stand_cells,
        corridors,
    })
}

/// Continue the world from the completed arrival at its stand into the onward ground route.
///
/// The aircraft keeps every resource it already holds, including the stand, until its first
/// movement is granted. The replacement airborne flight is installed at the holding point by
/// the ordinary departure controller path.
pub(super) fn install(runtime: &mut Runtime, id: u32, plan: Plan) -> Result<(), String> {
    let pose = runtime
        .world
        .snapshot()
        .into_iter()
        .find(|pose| pose.id == id)
        .ok_or("Turnaround aircraft no longer exists")?;
    if !pose.complete || pose.stage != Stage::Turnaround {
        return Err("Turnaround aircraft has not completed its arrival dwell".into());
    }
    let input = runtime
        .turnaround_inputs
        .get(&id)
        .cloned()
        .ok_or("Turnaround has no retained planning input")?;
    plan.prepared.validate()?;
    flight::validate_reports(runtime, &plan.prepared)?;
    let departure = plan
        .prepared
        .route
        .segment(0)
        .ok_or("Turnaround departure has no line-up movement")?;
    if departure.stage != Stage::LineUp
        || plan
            .prepared
            .route
            .segment(1)
            .is_none_or(|segment| segment.stage != Stage::Takeoff)
    {
        return Err("Turnaround departure needs separate line-up and takeoff limits".into());
    }
    let lineup = *departure
        .points
        .first()
        .ok_or("Turnaround departure has no start point")?;
    let ground = plan
        .ground
        .route
        .segment(0)
        .ok_or("Turnaround ground route has no movement")?;
    if !matches!(ground.stage, Stage::Pushback | Stage::TaxiOut) {
        return Err("Turnaround must begin with pushback or taxi-out".into());
    }
    let first = *ground
        .points
        .first()
        .ok_or("Turnaround ground route has no start point")?;
    if openatc_core::arrival::distance_nm(
        [first.latitude, first.longitude],
        [pose.position.latitude, pose.position.longitude],
    ) > 0.001
        || (first.altitude_feet - pose.position.altitude_feet).abs() > 1.0
    {
        return Err("Turnaround ground route does not start at the parked aircraft".into());
    }
    let hold = plan.ground.route.end_position();
    if openatc_core::arrival::distance_nm(
        [hold.latitude, hold.longitude],
        [lineup.latitude, lineup.longitude],
    ) > 0.001
    {
        return Err("Turnaround departure does not join its taxi holding point".into());
    }
    let owned = runtime.world.owned_resources(id);
    if owned.is_empty() {
        return Err("Turnaround aircraft does not hold its stand".into());
    }
    if !owned
        .iter()
        .all(|resource| ground.resources.contains(resource))
    {
        return Err("Turnaround would release the stand before the aircraft moves".into());
    }
    if plan.stand_cells.iter().any(|resource| {
        runtime
            .world
            .resource_owner(resource)
            .is_some_and(|owner| owner != id)
    }) {
        return Err("Turnaround arrival stand is already occupied".into());
    }
    if plan.stand != input.arrival_stand && runtime.reserved_stands.contains(&plan.stand) {
        return Err("Turnaround arrival stand is already reserved".into());
    }
    let mut world = runtime.world.clone();
    if !world.continue_surface(world.generation(), id, Arc::new(plan.ground.route.clone())) {
        return Err("Turnaround ground route is discontinuous at the parked aircraft".into());
    }
    runtime.world = world;
    runtime.flights.insert(id, Arc::new(plan.prepared));
    runtime.orders.insert(id, plan.ground.orders);
    runtime.active_flights.remove(&id);
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
    runtime.pending_sector_checkins.remove(&id);
    runtime
        .sector_transfers
        .retain(|(aircraft, _), _| *aircraft != id);
    runtime.handoff_targets.remove(&id);
    runtime.tower_ready.remove(&id);
    runtime.tower_notified.remove(&id);
    runtime.cancelled_takeoffs.remove(&id);
    runtime
        .cancelled_landings
        .retain(|(aircraft, _)| *aircraft != id);
    runtime.rejoins.remove(&id);
    runtime
        .rejoin_attempts
        .retain(|(aircraft, _), _| *aircraft != id);
    runtime.controllers.remove(&id);
    runtime.departure_wake.remove(&id);
    if runtime
        .exchange
        .as_ref()
        .is_some_and(|exchange| exchange.id == id)
    {
        runtime.exchange = None;
    }
    if !input.arrival_stand.is_empty() {
        runtime.reserved_stands.remove(&input.arrival_stand);
    }
    runtime.reserved_stands.insert(plan.stand);
    Arc::make_mut(&mut runtime.occupied_stand_cells).extend(plan.stand_cells);
    Arc::make_mut(&mut runtime.surface_corridors).extend(plan.corridors);
    runtime.turnarounds_attempted.insert(id);
    Ok(())
}

/// Record one bounded attempt. A refused plan leaves the aircraft parked and holds no route.
pub(super) fn settle(runtime: &mut Runtime, id: u32, result: Result<Plan, String>) {
    runtime.turnarounds_attempted.insert(id);
    match result {
        Ok(plan) => {
            if let Err(error) = install(runtime, id, plan)
                && runtime.warnings.len() < 128
            {
                runtime
                    .warnings
                    .push(format!("Turnaround {id} left parked: {error}"));
            }
        }
        Err(error) => {
            if runtime.warnings.len() < 128 {
                runtime
                    .warnings
                    .push(format!("Turnaround {id} could not be planned: {error}"));
            }
        }
    }
}

/// Plan one parked arrival's onward flight on a CPU worker and install it only if the aircraft
/// is still parked and the world has not been replaced meanwhile.
pub(super) async fn prepare(shared: Shared) {
    let (job, id, runtime, telemetry, workers, key, epoch) = {
        let mut state = shared.write().await;
        if !state.settings.generated_traffic
            || state.generated.epoch != Some(state.session_generation)
            || state.generated.turnaround_job.is_some()
            || state.generated.simulation_job.is_some()
            || state.generated.preparation_job.is_some()
        {
            return;
        }
        let Some(id) = eligible(&state.generated) else {
            return;
        };
        let job = next_job();
        state.generated.turnaround_job = Some(job);
        state.generated.turnaround_target = Some(id);
        (
            job,
            id,
            state.generated.clone(),
            state.session.telemetry.clone(),
            state.workers.clone(),
            state.generated.key.clone(),
            state.session_generation,
        )
    };
    let result = workers.run(move || plan(&runtime, id, &telemetry)).await;
    let mut state = shared.write().await;
    let target = state.generated.turnaround_target.take();
    if !finish_job(&mut state.generated.turnaround_job, job) || target != Some(id) {
        return;
    }
    if state.session_generation != epoch
        || state.generated.key != key
        || !state.settings.generated_traffic
    {
        return;
    }
    settle(&mut state.generated, id, result.unwrap_or_else(Err));
}

#[cfg(test)]
mod tests {
    use super::super::flight::{Clearance, Prepared, RunwayReport};
    use super::*;
    use openatc_core::stations::Station;
    use openatc_core::traffic::motion::{Route, Segment, Waypoint};

    fn point(east: f64, altitude: f64) -> Waypoint {
        Waypoint {
            latitude: 0.0,
            longitude: east / 111320.0,
            altitude_feet: altitude,
        }
    }

    fn station(service: &str, khz: i32) -> Station {
        Station {
            name: format!("Demo {service}"),
            service: service.into(),
            airport: "DEMO".into(),
            khz,
            ..Default::default()
        }
    }

    /// A route ending at a stand, with the stand cells still owned by the aircraft.
    fn parked_runtime() -> (EngineState, Arc<Route>) {
        let mut state = crate::test_state();
        state.settings.generated_traffic = true;
        state.settings.hear_traffic_radio = false;
        state.session.telemetry.traffic = Some(vec![]);
        state.session.telemetry.position_valid = true;
        state.session.telemetry.on_ground = true;
        let route = Arc::new(
            Route::compile(
                vec![
                    Segment {
                        stage: Stage::TaxiIn,
                        points: vec![point(0.0, 0.0), point(100.0, 0.0)],
                        knots: 10.0,
                        resources: vec!["TAXI".into()],
                        dwell_seconds: 0.0,
                    },
                    Segment {
                        stage: Stage::Turnaround,
                        points: vec![point(100.0, 0.0)],
                        knots: 0.0,
                        resources: vec!["STAND".into()],
                        dwell_seconds: 30.0,
                    },
                ],
                4000.0,
                4000.0,
            )
            .unwrap(),
        );
        let mut movement = BTreeMap::new();
        movement.insert(
            1,
            animation::builtin()
                .unwrap()
                .into_values()
                .next()
                .expect("bundled handling"),
        );
        let mut turnaround_inputs = BTreeMap::new();
        turnaround_inputs.insert(
            1,
            Input {
                profile: Profiles::builtin()
                    .unwrap()
                    .aircraft_for("B738")
                    .unwrap()
                    .clone(),
                preference: TrafficPreference::default(),
                general_aviation: false,
                gate: true,
                origin: Arc::new(openatc_core::airport::demo_airport()),
                destination: Arc::new(openatc_core::airport::demo_airport()),
                origin_runway: "09".into(),
                destination_runway: "27".into(),
                arrival_stand: "Gate 2".into(),
                departure_procedures: None,
                arrival_procedures: None,
                destination_surface: None,
                origin_surface: None,
            },
        );
        let mut runtime = Runtime {
            epoch: Some(state.session_generation),
            key: "turnaround-fixture".into(),
            policy: Some(
                parse_operations(include_str!("../../../../traffic/operations.toml")).unwrap(),
            ),
            airport: Some(Arc::new(openatc_core::airport::demo_airport())),
            movement,
            turnaround_inputs: Arc::new(turnaround_inputs),
            reserved_stands: BTreeSet::from(["Gate 2".into()]),
            frames: vec![GeneratedTraffic {
                vertical_speed_fpm: 0.0,
                body_heading_degrees: Some(180.0),
                target: TrafficTarget {
                    id: 1,
                    callsign: "TEST123".into(),
                    com1_khz: Some(121_900),
                    ..Default::default()
                },
                model: Arc::new(openatc_core::traffic::csl::Model {
                    id: "fixture".into(),
                    objects: vec![],
                    vertical_offset: None,
                    matches: vec![],
                }),
                animation: Animation::default(),
            }],
            ..Runtime::default()
        };
        assert!(
            runtime
                .world
                .insert(runtime.world.generation(), 1, route.clone())
        );
        state.generated = runtime;
        for second in 0..120 {
            state.session.telemetry.simulation_seconds = Some(f64::from(second));
            advance(&mut state.generated, &state.session.telemetry);
            if state.generated.world.snapshot()[0].complete {
                break;
            }
        }
        let pose = state.generated.world.snapshot()[0].clone();
        assert_eq!(pose.stage, Stage::Turnaround);
        assert!(pose.complete);
        (state, route)
    }

    fn ground_plan(from: Waypoint, via: Waypoint, to: Waypoint) -> GroundPlan {
        let order = |sequence| openatc_core::state::TaxiClearance {
            airport: "DEMO".into(),
            instructions: "Taxi to runway 09 via A.".into(),
            via: "A".into(),
            destination: "RUNWAY".into(),
            hold_short_runway: "09".into(),
            sequence,
            ..Default::default()
        };
        GroundPlan {
            route: Route::compile(
                vec![
                    Segment {
                        stage: Stage::TaxiOut,
                        points: vec![from, via],
                        knots: 10.0,
                        resources: vec!["STAND".into(), "TAXI".into()],
                        dwell_seconds: 0.0,
                    },
                    Segment {
                        stage: Stage::TaxiOut,
                        points: vec![via, to],
                        knots: 10.0,
                        resources: vec!["TAXI".into()],
                        dwell_seconds: 0.0,
                    },
                ],
                4000.0,
                4000.0,
            )
            .unwrap(),
            orders: vec![(0, order(1)), (1, order(2))],
        }
    }

    /// A separately cleared departure from the taxi hold. Clearance wording is only evidence
    /// here; the controller contracts it models are validated by `Prepared::validate`.
    fn return_flight(hold: Waypoint) -> Prepared {
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
            hold,
            point(400.0, 0.0),
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
                    clearances.insert(
                        index,
                        Clearance {
                            amended_route: None,
                            station: station(service, khz),
                            transfers: Vec::new(),
                            coordinated_recovery: Vec::new(),
                            instruction: format!("Controller phase {index}."),
                            readback: format!("Pilot phase {index}."),
                            initial_climb: (index == 1).then_some(2),
                            taxi_through: None,
                            airborne_through: None,
                            landing_rollout: None,
                            missed: None,
                            holding: None,
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
                    resources: if matches!(stage, Stage::LineUp | Stage::Takeoff | Stage::Rollout) {
                        vec!["RUNWAY/DEMO/09/27".into()]
                    } else if stage.on_ground() {
                        vec![format!("PHASE/{index}")]
                    } else {
                        vec![]
                    },
                    dwell_seconds: 0.0,
                }
            })
            .collect();
        Prepared {
            amendment: None,
            airspeeds: BTreeMap::new(),
            route: Arc::new(Route::compile(segments, 4000.0, 4000.0).unwrap()),
            recovery,
            clearances,
            vacated_reports: BTreeMap::from([(
                7,
                RunwayReport {
                    airport: "DEMO".into(),
                    runway: "09".into(),
                },
            )]),
        }
    }

    fn plan_for(pose: &openatc_core::traffic::motion::Pose) -> Plan {
        Plan {
            stand: "Gate 1".into(),
            stand_cells: BTreeSet::from(["RETURN-STAND".into()]),
            corridors: BTreeSet::from(["TAXI".into(), "RETURN-STAND".into()]),
            ground: ground_plan(pose.position, point(200.0, 0.0), point(300.0, 0.0)),
            prepared: return_flight(point(300.0, 0.0)),
        }
    }

    #[test]
    fn a_parked_arrival_turns_around_and_releases_its_stand_on_departure() {
        let (mut state, arrival_route) = parked_runtime();
        assert!(state.generated.world.follows(1, &arrival_route));
        assert!(state.generated.world.owned_resources(1).contains("STAND"));
        assert!(eligible(&state.generated).is_some());

        let pose = state.generated.world.snapshot()[0].clone();
        settle(&mut state.generated, 1, Ok(plan_for(&pose)));
        assert!(
            state
                .generated
                .warnings
                .iter()
                .all(|warning| !warning.contains("parked")),
            "{:?}",
            state.generated.warnings
        );
        assert!(state.generated.turnarounds_attempted.contains(&1));
        assert!(eligible(&state.generated).is_none());

        let pose = state.generated.world.snapshot()[0].clone();
        assert_eq!(pose.stage, Stage::TaxiOut);
        assert!(!state.generated.active_flights.contains(&1));
        assert!(state.generated.orders.contains_key(&1));
        assert!(state.generated.flights.contains_key(&1));
        assert!(state.generated.reserved_stands.contains("Gate 1"));
        assert!(!state.generated.reserved_stands.contains("Gate 2"));

        let mut cleared = false;
        let mut moving = false;
        let mut moving_with_stand = false;
        let mut released = false;
        let start = state.generated.previous_time.unwrap_or(0.0) as u32 + 1;
        for second in start..start + 300 {
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            radio_tick(&mut state);
            if let Some(exchange) = &state.generated.exchange
                && exchange.station.service == "Ground"
                && exchange.grant
            {
                assert!(
                    exchange
                        .messages
                        .iter()
                        .any(|(controller, text)| *controller
                            && text.contains("Taxi to runway 09 via A."))
                );
                assert!(
                    exchange.messages.len() >= 3
                        && exchange
                            .messages
                            .last()
                            .is_some_and(|(controller, _)| !*controller),
                    "the departure clearance needs a pilot readback"
                );
                cleared = true;
            }
            state.session.telemetry.simulation_seconds = Some(f64::from(second));
            advance(&mut state.generated, &state.session.telemetry);
            let pose = state.generated.world.snapshot()[0].clone();
            let held = state.generated.world.owned_resources(1);
            if pose.knots > 0.0 {
                moving = true;
                if held.contains("STAND") {
                    moving_with_stand = true;
                }
            }
            if !held.contains("STAND") {
                released = true;
                break;
            }
        }
        assert!(cleared, "no departure clearance was issued");
        assert!(moving, "the aircraft never began taxi-out");
        assert!(
            moving_with_stand,
            "the stand was not held until the aircraft moved"
        );
        assert!(released, "the stand was not released after departure");
        assert!(!state.generated.world.owned_resources(1).contains("STAND"));
    }

    #[test]
    fn an_unplannable_turnaround_leaves_the_aircraft_parked_and_holds_nothing() {
        let (mut state, arrival_route) = parked_runtime();
        let result = plan(&state.generated, 1, &state.session.telemetry);
        assert!(result.is_err(), "an unplannable turnaround must be refused");
        settle(&mut state.generated, 1, result);
        assert!(
            state
                .generated
                .warnings
                .iter()
                .any(|warning| warning.contains("could not be planned"))
        );
        assert!(state.generated.world.follows(1, &arrival_route));
        let pose = state.generated.world.snapshot()[0].clone();
        assert_eq!(pose.stage, Stage::Turnaround);
        assert!(pose.complete);
        assert!(state.generated.orders.is_empty());
        assert!(state.generated.flights.is_empty());
        assert!(state.generated.active_flights.is_empty());
        assert!(state.generated.approved_orders.is_empty());
        assert_eq!(
            state.generated.world.owned_resources(1),
            BTreeSet::from(["STAND".into()]),
            "a refused turnaround may hold only the stand it physically occupies"
        );
        assert!(state.generated.reserved_stands.contains("Gate 2"));
        assert!(scene_settled(&state.generated.world));
        assert!(eligible(&state.generated).is_none());
    }

    #[test]
    fn a_turnaround_that_would_drop_the_stand_is_refused() {
        let (mut state, arrival_route) = parked_runtime();
        let pose = state.generated.world.snapshot()[0].clone();
        let mut plan = plan_for(&pose);
        plan.ground.route = Route::compile(
            vec![Segment {
                stage: Stage::TaxiOut,
                points: vec![pose.position, point(300.0, 0.0)],
                knots: 10.0,
                resources: vec!["TAXI".into()],
                dwell_seconds: 0.0,
            }],
            4000.0,
            4000.0,
        )
        .unwrap();
        settle(&mut state.generated, 1, Ok(plan));
        assert!(
            state
                .generated
                .warnings
                .iter()
                .any(|warning| warning.contains("release the stand"))
        );
        assert!(state.generated.world.follows(1, &arrival_route));
        assert_eq!(state.generated.world.snapshot()[0].stage, Stage::Turnaround);
        assert!(state.generated.orders.is_empty());
        assert_eq!(
            state.generated.world.owned_resources(1),
            BTreeSet::from(["STAND".into()])
        );
    }
}
