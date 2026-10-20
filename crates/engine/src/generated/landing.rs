//! Runway exits use the installed taxi graph; an apron gap is never an exit.
use super::*;
use openatc_core::traffic::motion::{Segment, Stage, Waypoint};

#[derive(Clone)]
pub(super) struct Exit {
    pub rollout: Vec<Segment>,
    pub vacated: Waypoint,
}

#[derive(Clone)]
pub(super) struct Arrival {
    pub flare: Segment,
    pub parking: Parking,
}
pub(super) struct Request<'a> {
    pub airport: &'a Airport,
    pub surface: &'a super::runway::Surface,
    pub crossing: Waypoint,
    pub angle_degrees: f64,
    pub airspeed: openatc_core::traffic::trajectory::Airspeed,
    pub aircraft: &'a openatc_core::traffic::profiles::Aircraft,
    pub handling: &'a animation::Handling,
    pub policy: &'a Operations,
    pub observed: &'a [TrafficTarget],
    pub reserved: &'a BTreeSet<String>,
    pub occupied_stand_cells: &'a BTreeSet<String>,
    // A new parked footprint must not obstruct an earlier surface route.
    pub parking_exclusions: &'a BTreeSet<String>,
    pub preference: &'a openatc_core::parking::TrafficPreference,
}
pub(super) fn prepare(input: Request<'_>) -> Result<Arrival, String> {
    let knots = input
        .airspeed
        .ground_knots(input.crossing.altitude_feet, input.surface.heading())?;
    let flare = input.surface.flare(
        input.airport,
        input.crossing,
        input.angle_degrees,
        knots,
        input.handling,
        input.policy.surface_sample_meters,
    )?;
    let touchdown = *flare.points.last().ok_or("Flare has no touchdown")?;
    let mut selected = None;
    let mut attempts = 0;
    let mut reason = None;
    let result = plan_select(
        input.airport,
        &input.surface.runway,
        touchdown,
        knots,
        input.aircraft,
        input.handling,
        input.policy,
        input.surface,
        |exit| {
            // Bound complete exit-to-stand searches independently of the runway's graph size.
            if attempts >= 8 {
                return false;
            }
            attempts += 1;
            let candidate = (|| {
                let parking = parking(
                    input.airport,
                    exit.clone(),
                    input.aircraft,
                    input.handling,
                    input.policy,
                    input.observed,
                    input.reserved,
                    input.occupied_stand_cells,
                    input.parking_exclusions,
                    input.preference,
                )?;
                let mut segments = vec![flare.clone()];
                segments.extend(parking.exit.rollout.clone());
                segments.extend(
                    (0..512).map_while(|index| parking.surface.route.segment(index).cloned()),
                );
                openatc_core::traffic::motion::Route::compile(
                    segments,
                    input.aircraft.climb_fpm,
                    input.aircraft.descent_fpm,
                )?;
                Ok::<_, String>(parking)
            })();
            match candidate {
                Ok(parking) => {
                    selected = Some(parking);
                    true
                }
                Err(error) => {
                    reason = Some(error);
                    false
                }
            }
        },
    );
    if let Err(error) = result {
        return Err(match reason {
            Some(reason) => format!("{error}; {attempts} exit-to-stand searches: {reason}"),
            None => error,
        });
    }
    let parking = selected.ok_or("Landing exit lost its checked parking route")?;
    Ok(Arrival { flare, parking })
}

fn clear_of_runways(
    airport: &Airport,
    from: Point,
    to: Point,
    aircraft: &openatc_core::traffic::profiles::Aircraft,
) -> bool {
    let from = position(airport, from);
    let to = position(airport, to);
    !openatc_core::airport::aircraft_on_runway_for(
        airport,
        &openatc_core::state::Telemetry {
            latitude: to[0],
            longitude: to[1],
            heading_degrees: openatc_core::arrival::bearing(from, to),
            position_valid: true,
            on_ground: true,
            wingspan_meters: Some(aircraft.wingspan),
            ..Default::default()
        },
        aircraft,
    )
}

/// Decide whether an aircraft can leave a runway through a verified forward exit.
/// The same screening and turn construction used by [`plan`] is applied to a nominal
/// threshold crossing, so runway assignment and landing planning share one source of truth.
pub(super) fn vacatable(
    airport: &Airport,
    runway_name: &str,
    aircraft: &openatc_core::traffic::profiles::Aircraft,
    handling: &animation::Handling,
    policy: &Operations,
    surface: &super::runway::Surface,
    environment: &super::environment::Environment,
) -> Result<(), String> {
    if !surface.matches(airport, runway_name) {
        return Err("Landing runway surface is unavailable".into());
    }
    let threshold = surface.threshold;
    let height = surface.height(surface.point(threshold)) + handling.flare_height_feet.max(50.0);
    let knots = environment.runway_speed(airport, runway_name, aircraft.approach_knots, height)?;
    vacatable_at_speed(
        airport,
        runway_name,
        aircraft,
        handling,
        policy,
        surface,
        knots,
    )
}

/// Run the shared exit screen with an explicit final-approach ground speed.
pub(super) fn vacatable_at_speed(
    airport: &Airport,
    runway_name: &str,
    aircraft: &openatc_core::traffic::profiles::Aircraft,
    handling: &animation::Handling,
    policy: &Operations,
    surface: &super::runway::Surface,
    ground_knots: f64,
) -> Result<(), String> {
    let runway = airport
        .runways
        .iter()
        .find(|runway| runway.first_name == runway_name || runway.second_name == runway_name)
        .ok_or("Landing runway is unavailable")?;
    let (start, end) = if runway.first_name == runway_name {
        (runway.first, runway.second)
    } else {
        (runway.second, runway.first)
    };
    let length = (end.east - start.east).hypot(end.north - start.north);
    if !surface.matches(airport, runway_name) || length - surface.threshold < aircraft.runway {
        return Err("Landing runway or stopping envelope is insufficient".into());
    }
    let point = surface.point(surface.threshold);
    let coordinate = position(airport, point);
    let crossing = Waypoint {
        latitude: coordinate[0],
        longitude: coordinate[1],
        altitude_feet: surface.height(point) + handling.flare_height_feet.max(50.0),
    };
    let flare = surface.flare(
        airport,
        crossing,
        -3.0,
        ground_knots,
        handling,
        policy.surface_sample_meters,
    )?;
    let touchdown = *flare.points.last().ok_or("Flare has no touchdown")?;
    plan(
        airport,
        runway_name,
        touchdown,
        ground_knots,
        aircraft,
        handling,
        policy,
        surface,
    )
    .map(|_| ())
}

/// Choose the first runway end, in the supplied preference order, that the type can leave.
/// The error names every candidate and the reason it was rejected.
pub(super) fn vacatable_runway(
    airport: &Airport,
    candidates: &[(String, super::runway::Surface)],
    aircraft: &openatc_core::traffic::profiles::Aircraft,
    handling: &animation::Handling,
    policy: &Operations,
    ground_knots: f64,
) -> Result<String, String> {
    let mut rejected = Vec::new();
    for (name, surface) in candidates {
        match vacatable_at_speed(
            airport,
            name,
            aircraft,
            handling,
            policy,
            surface,
            ground_knots,
        ) {
            Ok(()) => return Ok(name.clone()),
            Err(reason) => rejected.push(format!("{name}: {reason}")),
        }
    }
    Err(format!(
        "No arrival runway can be vacated by {}: {}",
        aircraft.name,
        if rejected.is_empty() {
            "no runway surfaces are available".to_owned()
        } else {
            rejected.join("; ")
        }
    ))
}

// This joins the landing geometry and the aircraft envelope; both are needed verbatim.
#[allow(clippy::too_many_arguments)]
pub(super) fn plan(
    airport: &Airport,
    runway_name: &str,
    touchdown: Waypoint,
    touchdown_knots: f64,
    aircraft: &openatc_core::traffic::profiles::Aircraft,
    handling: &animation::Handling,
    policy: &Operations,
    surface: &super::runway::Surface,
) -> Result<Exit, String> {
    plan_select(
        airport,
        runway_name,
        touchdown,
        touchdown_knots,
        aircraft,
        handling,
        policy,
        surface,
        |_| true,
    )
}

#[allow(clippy::too_many_arguments)]
fn plan_select(
    airport: &Airport,
    runway_name: &str,
    touchdown: Waypoint,
    touchdown_knots: f64,
    aircraft: &openatc_core::traffic::profiles::Aircraft,
    handling: &animation::Handling,
    policy: &Operations,
    surface: &super::runway::Surface,
    mut onward: impl FnMut(&Exit) -> bool,
) -> Result<Exit, String> {
    let runway = airport
        .runways
        .iter()
        .find(|runway| runway.first_name == runway_name || runway.second_name == runway_name)
        .ok_or("Landing runway is unavailable")?;
    let (start, end) = if runway.first_name == runway_name {
        (runway.first, runway.second)
    } else {
        (runway.second, runway.first)
    };
    let length = (end.east - start.east).hypot(end.north - start.north);
    if !surface.matches(airport, runway_name)
        || length - surface.threshold < aircraft.runway
        || !(35.0..=230.0).contains(&touchdown_knots)
        || handling.braking_knots_sec <= 0.0
    {
        return Err("Landing runway or stopping envelope is insufficient".into());
    }
    let unit = [
        (end.east - start.east) / length,
        (end.north - start.north) / length,
    ];
    let local =
        openatc_core::airport::airport_point(airport, touchdown.latitude, touchdown.longitude);
    let along = (local.east - start.east) * unit[0] + (local.north - start.north) * unit[1];
    let across = (local.east - start.east) * unit[1] - (local.north - start.north) * unit[0];
    if along < 0.0
        || along >= length
        || across.abs() > 2.0
        || (touchdown.altitude_feet - surface.height(local)).abs() > 5.0
    {
        return Err("Touchdown is not aligned with the nominal runway surface".into());
    }
    let braking = (touchdown_knots.powi(2) - handling.taxi_turn_knots.powi(2)).max(0.0) * 0.514444
        / (2.0 * handling.braking_knots_sec);
    let radius = handling.taxi_turn_radius_meters;
    let envelope = aircraft.wingspan.max(aircraft.length) / 2.0;
    let nodes = airport
        .nodes
        .iter()
        .map(|node| (node.id, node.point))
        .collect::<BTreeMap<_, _>>();
    let mut candidates = Vec::new();
    let mut screened = [0usize; 4];
    let landing_runway_edges = BTreeSet::from([
        runway.first_name.clone(),
        runway.second_name.clone(),
        format!("{}/{}", runway.first_name, runway.second_name),
        format!("{}/{}", runway.second_name, runway.first_name),
    ]);
    // A node is an exit entrance only where a taxiway meets this runway.
    // Unnamed runway edges stay eligible and are resolved by the alignment test.
    let on_landing_runway = |id: i64| {
        airport.edges.iter().any(|edge| {
            edge.runway
                && (edge.first == id || edge.second == id)
                && (edge.name.is_empty() || landing_runway_edges.contains(&edge.name))
        })
    };
    for edge in airport
        .edges
        .iter()
        .filter(|edge| !edge.runway && edge.size >= aircraft.parking_size())
    {
        for (first, second) in [(edge.first, edge.second), (edge.second, edge.first)] {
            if edge.one_way && first != edge.first {
                continue;
            }
            let (Some(entrance), Some(_exit)) = (nodes.get(&first), nodes.get(&second)) else {
                continue;
            };
            if !on_landing_runway(first) {
                screened[3] += 1;
                continue;
            }
            let at =
                (entrance.east - start.east) * unit[0] + (entrance.north - start.north) * unit[1];
            let offset =
                (entrance.east - start.east) * unit[1] - (entrance.north - start.north) * unit[0];
            if offset.abs() > policy.surface_sample_meters * 2.0 {
                screened[0] += 1;
                continue;
            }
            if at < along + braking + radius * 2.0 {
                screened[1] += 1;
                continue;
            }
            if at + envelope >= length {
                screened[2] += 1;
                continue;
            }
            let mut queue = std::collections::VecDeque::from([(second, vec![first, second])]);
            let mut visits = 0;
            while let Some((node, visited)) = queue.pop_front() {
                visits += 1;
                if visits > 32 {
                    break;
                }
                let point = nodes[&node];
                let previous = nodes[&visited[visited.len() - 2]];
                let clear = clear_of_runways(airport, previous, point, aircraft);
                if clear {
                    candidates.push((at, visited.iter().map(|id| nodes[id]).collect::<Vec<_>>()));
                    break;
                }
                if visited.len() >= 8 {
                    continue;
                }
                for connection in airport
                    .edges
                    .iter()
                    .filter(|edge| !edge.runway && edge.size >= aircraft.parking_size())
                {
                    let next = if connection.first == node {
                        Some(connection.second)
                    } else if !connection.one_way && connection.second == node {
                        Some(connection.first)
                    } else {
                        None
                    };
                    if let Some(next) = next
                        && nodes.contains_key(&next)
                        && !visited.contains(&next)
                    {
                        let mut path = visited.clone();
                        path.push(next);
                        queue.push_back((next, path));
                    }
                }
            }
        }
    }
    candidates.sort_by(|left, right| left.0.total_cmp(&right.0));
    let candidate_count = candidates.len();
    let mut rejected = [0; 4];
    for (at, path) in candidates.into_iter().take(32) {
        let turn_start = Point {
            east: start.east + (at - radius * 2.0) * unit[0],
            north: start.north + (at - radius * 2.0) * unit[1],
            height: 0.0,
        };
        let protected = |point: Point| {
            openatc_core::airport::on_pavement(airport, point)
                && airport
                    .runways
                    .iter()
                    .filter(|other| {
                        other.first_name != runway.first_name
                            || other.second_name != runway.second_name
                    })
                    .all(|other| {
                        let geographic = position(airport, point);
                        openatc_core::traffic::runway_position(
                            airport,
                            &other.first_name,
                            geographic[0],
                            geographic[1],
                        )
                        .is_none_or(|(along, across, length, _)| {
                            along < -envelope
                                || along > length + envelope
                                || across.abs() > other.width / 2.0 + envelope
                        })
                    })
        };
        let mut route = vec![turn_start];
        route.extend(path);
        let minimum_radius =
            handling.wheelbase_meters / handling.maximum_steering_degrees.to_radians().tan();
        let result = openatc_core::traffic::surface::rounded(
            &route,
            radius,
            minimum_radius.max(0.5).min(radius),
            policy.surface_sample_meters,
            protected,
        );
        let Ok(turn) = result else {
            rejected[0] += 1;
            continue;
        };
        // Rounded exits must end beyond the same holding line used by live radio reports.
        if turn.len() < 2
            || !clear_of_runways(
                airport,
                turn[turn.len() - 2].point,
                turn.last().unwrap().point,
                aircraft,
            )
        {
            rejected[1] += 1;
            continue;
        }
        let waypoint = |point: Point| {
            let geographic = position(airport, point);
            Waypoint {
                latitude: geographic[0],
                longitude: geographic[1],
                altitude_feet: surface.height(point),
            }
        };
        let mut points = vec![waypoint(turn_start)];
        points.extend(turn.iter().skip(1).map(|sample| waypoint(sample.point)));
        let mut resources = BTreeSet::from([format!(
            "RUNWAY/{}/{}/{}",
            airport.icao, runway.first_name, runway.second_name
        )]);
        for sample in &turn {
            resources.extend(cells(
                airport,
                sample.point,
                envelope,
                policy.conflict_cell_meters,
            ));
        }
        if resources.len() > 32 {
            rejected[2] += 1;
            continue;
        }
        let vacated = *points.last().unwrap();
        let exit = Exit {
            rollout: vec![
                Segment {
                    stage: Stage::Rollout,
                    points: vec![touchdown, waypoint(turn_start)],
                    knots: touchdown_knots,
                    resources: resources
                        .iter()
                        .filter(|resource| resource.starts_with("RUNWAY/"))
                        .cloned()
                        .collect(),
                    dwell_seconds: 0.0,
                },
                Segment {
                    stage: Stage::Rollout,
                    points,
                    knots: handling.taxi_turn_knots,
                    resources: resources.into_iter().collect(),
                    dwell_seconds: 0.0,
                },
            ],
            vacated,
        };
        if onward(&exit) {
            return Ok(exit);
        }
        rejected[3] += 1;
    }
    Err(format!(
        "No verified forward runway exit within the landing and taxi envelope ({candidate_count} mapped candidates; {} turn, {} runway-clearance, {} resource-budget rejections, {} onward-route rejections; {} off-axis, {} before stopping limit, {} past runway end, {} disconnected; touchdown {:.0} m, braking {:.0} m at {:.1} kt)",
        rejected[0],
        rejected[1],
        rejected[2],
        rejected[3],
        screened[0],
        screened[1],
        screened[2],
        screened[3],
        along,
        braking,
        touchdown_knots
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed airport scenery"]
    fn louisville_forward_exits_keep_the_aircraft_stopping_envelope() {
        let root = std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap();
        let airport = super::super::airport(&root, "KSDF").unwrap();
        let policy = parse_operations(include_str!("../../../../traffic/operations.toml")).unwrap();
        let data = openatc_core::arrival::ArrivalAirport::load(&root, airport.clone()).unwrap();
        let handling = animation::builtin().unwrap();
        let mut missing = Vec::new();
        for code in ["B738", "B752", "C172"] {
            let aircraft = openatc_core::traffic::profiles::aircraft(code).unwrap();
            let handling = &handling[&aircraft.handling.unwrap()];
            let mut accepted = 0;
            for name in ["17L", "17R"] {
                let runway = airport
                    .runways
                    .iter()
                    .find(|runway| runway.first_name == name || runway.second_name == name)
                    .unwrap();
                let (first, second) = if runway.first_name == name {
                    (runway.first, runway.second)
                } else {
                    (runway.second, runway.first)
                };
                let length = (second.east - first.east).hypot(second.north - first.north);
                let point = Point {
                    east: first.east + (second.east - first.east) * 300.0 / length,
                    north: first.north + (second.north - first.north) * 300.0 / length,
                    height: 0.0,
                };
                let surface = super::super::runway::Surface::new(
                    &airport,
                    name,
                    &data.thresholds,
                    policy.runway_slope_limit_percent,
                )
                .unwrap();
                let location = position(&airport, point);
                let touchdown = Waypoint {
                    latitude: location[0],
                    longitude: location[1],
                    altitude_feet: surface.height(point),
                };
                match plan(
                    &airport,
                    name,
                    touchdown,
                    aircraft.approach_knots,
                    aircraft,
                    handling,
                    &policy,
                    &surface,
                ) {
                    Ok(exit) => {
                        let points = &exit.rollout.last().unwrap().points;
                        assert!(clear_of_runways(
                            &airport,
                            openatc_core::airport::airport_point(
                                &airport,
                                points[points.len() - 2].latitude,
                                points[points.len() - 2].longitude
                            ),
                            openatc_core::airport::airport_point(
                                &airport,
                                exit.vacated.latitude,
                                exit.vacated.longitude
                            ),
                            aircraft
                        ));
                        accepted += 1;
                        eprintln!("KSDF {name} {code}: verified forward exit");
                    }
                    Err(error) => eprintln!("KSDF {name} {code}: {error}"),
                }
            }
            if accepted == 0 {
                missing.push(code);
            }
        }
        assert!(missing.is_empty(), "No forward exit for {missing:?}");
    }

    #[test]
    fn exit_screening_references_only_the_landing_runway() {
        let point = |east, north| Point {
            east,
            north,
            height: 0.0,
        };
        let node = |id, east, north| openatc_core::airport::TaxiNode {
            id,
            point: point(east, north),
        };
        let edge = |first, second, runway, name: &str| openatc_core::airport::TaxiEdge {
            first,
            second,
            runway,
            name: name.into(),
            size: 'F',
            ..Default::default()
        };
        let airport = Airport {
            icao: "TEST".into(),
            // Narrow pavement makes the aligned candidate's turn fail after screening,
            // so the diagnostic counters remain observable.
            runways: vec![openatc_core::airport::Runway {
                first_name: "09".into(),
                second_name: "27".into(),
                width: 5.0,
                first: point(0.0, 0.0),
                second: point(2000.0, 0.0),
                ..Default::default()
            }],
            nodes: vec![
                node(1, 0.0, 0.0),
                node(2, 2000.0, 0.0),
                node(3, 1500.0, 0.0),
                node(4, 1500.0, 100.0),
                node(5, 200.0, 0.0),
                node(6, 200.0, 100.0),
                node(7, 1500.0, 300.0),
                node(8, 0.0, 300.0),
                node(9, 500.0, 900.0),
                node(10, 600.0, 900.0),
                node(11, 1500.0, 400.0),
                node(13, 1500.0, 20.0),
                node(14, 1500.0, 80.0),
                node(15, 1996.0, 0.0),
                node(16, 1996.0, 60.0),
                node(17, 1600.0, 20.0),
            ],
            edges: vec![
                edge(1, 5, true, "09/27"),
                edge(5, 3, true, "09/27"),
                edge(3, 15, true, "09/27"),
                edge(15, 2, true, "09/27"),
                edge(13, 17, true, "09/27"),
                edge(7, 8, true, "09L/27R"),
                edge(3, 4, false, "T1"),
                edge(5, 6, false, "T2"),
                edge(7, 11, false, "T3"),
                edge(9, 10, false, "T4"),
                edge(13, 14, false, "T5"),
                edge(15, 16, false, "T6"),
            ],
            ..Default::default()
        };
        let thresholds = [
            openatc_core::arrival::Threshold {
                runway: "09".into(),
                elevation_feet: 0.0,
            },
            openatc_core::arrival::Threshold {
                runway: "27".into(),
                elevation_feet: 0.0,
            },
        ];
        let surface = super::super::runway::Surface::new(&airport, "09", &thresholds, 2.0).unwrap();
        let coordinate = position(&airport, surface.point(50.0));
        let touchdown = Waypoint {
            latitude: coordinate[0],
            longitude: coordinate[1],
            altitude_feet: surface.height(surface.point(50.0)),
        };
        let aircraft = openatc_core::traffic::profiles::aircraft("C172").unwrap();
        let builtin = animation::builtin().unwrap();
        let policy = parse_operations(include_str!("../../../../traffic/operations.toml")).unwrap();
        let error = match plan(
            &airport,
            "09",
            touchdown,
            65.0,
            aircraft,
            &builtin[&animation::Kind::Piston],
            &policy,
            &surface,
        ) {
            Ok(_) => panic!("Expected the exit screening to reject the fixture"),
            Err(error) => error,
        };
        // The aligned entrance beyond the stopping distance is still a candidate.
        assert!(error.contains("1 mapped candidates"), "{error}");
        // The parallel-runway and remote taxi nodes are not landing-runway entrances.
        assert!(error.contains("1 off-axis"), "{error}");
        assert!(error.contains("1 before stopping limit"), "{error}");
        assert!(error.contains("1 past runway end"), "{error}");
        assert!(error.contains("8 disconnected"), "{error}");
    }

    #[test]
    fn arrival_runway_selection_requires_a_reachable_forward_exit() {
        let point = |east, north| Point {
            east,
            north,
            height: 0.0,
        };
        let node = |id, east, north| openatc_core::airport::TaxiNode {
            id,
            point: point(east, north),
        };
        let edge = |first, second, runway, name: &str| openatc_core::airport::TaxiEdge {
            first,
            second,
            runway,
            name: name.into(),
            size: 'F',
            ..Default::default()
        };
        // The first runway's only entrance is inside the runway-end envelope. The second
        // runway repeats that geometry or offers an entrance beyond the stopping limit.
        let fixture = |second_feasible: bool| {
            let airport = Airport {
                icao: "TEST".into(),
                runways: vec![
                    openatc_core::airport::Runway {
                        first_name: "09".into(),
                        second_name: "27".into(),
                        width: 45.0,
                        first: point(0.0, 0.0),
                        second: point(2969.0, 0.0),
                        ..Default::default()
                    },
                    openatc_core::airport::Runway {
                        first_name: "09L".into(),
                        second_name: "27R".into(),
                        width: 45.0,
                        first: point(0.0, 500.0),
                        second: point(2969.0, 500.0),
                        ..Default::default()
                    },
                ],
                nodes: vec![
                    node(1, 0.0, 0.0),
                    node(2, 2955.0, 0.0),
                    node(3, 2969.0, 0.0),
                    node(4, 2955.0, 100.0),
                    node(5, 0.0, 500.0),
                    node(6, if second_feasible { 2600.0 } else { 2955.0 }, 500.0),
                    node(7, 2969.0, 500.0),
                    node(8, if second_feasible { 2600.0 } else { 2955.0 }, 600.0),
                ],
                edges: vec![
                    edge(1, 2, true, "09/27"),
                    edge(2, 3, true, "09/27"),
                    edge(2, 4, false, "T1"),
                    edge(5, 6, true, "09L/27R"),
                    edge(6, 7, true, "09L/27R"),
                    edge(6, 8, false, "T2"),
                ],
                pavement_triangles: vec![
                    [
                        point(-100.0, -100.0),
                        point(3100.0, -100.0),
                        point(3100.0, 800.0),
                    ],
                    [
                        point(-100.0, -100.0),
                        point(3100.0, 800.0),
                        point(-100.0, 800.0),
                    ],
                ],
                ..Default::default()
            };
            let thresholds = [
                openatc_core::arrival::Threshold {
                    runway: "09".into(),
                    elevation_feet: 0.0,
                },
                openatc_core::arrival::Threshold {
                    runway: "27".into(),
                    elevation_feet: 0.0,
                },
                openatc_core::arrival::Threshold {
                    runway: "09L".into(),
                    elevation_feet: 0.0,
                },
                openatc_core::arrival::Threshold {
                    runway: "27R".into(),
                    elevation_feet: 0.0,
                },
            ];
            let policy =
                parse_operations(include_str!("../../../../traffic/operations.toml")).unwrap();
            let surfaces = ["09", "09L"]
                .into_iter()
                .map(|name| {
                    (
                        name.to_owned(),
                        super::super::runway::Surface::new(
                            &airport,
                            name,
                            &thresholds,
                            policy.runway_slope_limit_percent,
                        )
                        .unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            (airport, surfaces, policy)
        };
        let aircraft = openatc_core::traffic::profiles::aircraft("B763").unwrap();
        let handling = animation::builtin().unwrap();
        let handling = &handling[&aircraft.handling.unwrap()];
        let (airport, surfaces, policy) = fixture(true);
        assert_eq!(
            vacatable_runway(
                &airport,
                &surfaces,
                aircraft,
                handling,
                &policy,
                aircraft.approach_knots
            )
            .unwrap(),
            "09L"
        );
        let (airport, surfaces, policy) = fixture(false);
        let error = vacatable_runway(
            &airport,
            &surfaces,
            aircraft,
            handling,
            &policy,
            aircraft.approach_knots,
        )
        .unwrap_err();
        assert!(
            error.contains("No arrival runway can be vacated"),
            "{error}"
        );
        assert!(error.contains("09:"), "{error}");
        assert!(error.contains("09L:"), "{error}");
        assert!(error.contains("past runway end"), "{error}");
    }

    #[test]
    fn runway_exit_extends_beyond_the_holding_line_with_the_whole_aircraft() {
        let point = |east, north| Point {
            east,
            north,
            height: 0.0,
        };
        let airport = Airport {
            runways: vec![openatc_core::airport::Runway {
                first_name: "09".into(),
                second_name: "27".into(),
                width: 45.0,
                first: point(0.0, 0.0),
                second: point(2000.0, 0.0),
                ..Default::default()
            }],
            taxi_hold_lines: vec![[point(700.0, 90.0), point(1300.0, 90.0)]],
            ..Default::default()
        };
        let mut aircraft = openatc_core::traffic::profiles::aircraft("C172")
            .unwrap()
            .clone();
        aircraft.length = 20.0;
        aircraft.wingspan = 12.0;
        // The centre and wings are outside the pavement, but the holding line is ahead.
        assert!(!clear_of_runways(
            &airport,
            point(1000.0, 50.0),
            point(1000.0, 60.0),
            &aircraft
        ));
        assert!(!clear_of_runways(
            &airport,
            point(1000.0, 90.0),
            point(1000.0, 100.0),
            &aircraft
        ));
        assert!(clear_of_runways(
            &airport,
            point(1000.0, 110.0),
            point(1000.0, 120.0),
            &aircraft
        ));
    }

    #[test]
    #[ignore = "requires installed airport scenery"]
    fn piston_arrival_uses_a_connected_propeller_stand() {
        let root = std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap();
        let airport = super::super::airport(&root, "LFMD").unwrap();
        let aircraft = openatc_core::traffic::profiles::aircraft("C172").unwrap();
        let handling = &animation::builtin().unwrap()[&animation::Kind::Piston];
        let policy = parse_operations(include_str!("../../../../traffic/operations.toml")).unwrap();
        let runway = airport
            .runways
            .iter()
            .max_by(|a, b| {
                (a.second.east - a.first.east)
                    .hypot(a.second.north - a.first.north)
                    .total_cmp(
                        &(b.second.east - b.first.east).hypot(b.second.north - b.first.north),
                    )
            })
            .unwrap();
        let length = (runway.second.east - runway.first.east)
            .hypot(runway.second.north - runway.first.north);
        let point = Point {
            east: runway.first.east + (runway.second.east - runway.first.east) * 150.0 / length,
            north: runway.first.north + (runway.second.north - runway.first.north) * 150.0 / length,
            height: 0.0,
        };
        let data = openatc_core::arrival::ArrivalAirport::load(&root, airport.clone()).unwrap();
        let surface = super::super::runway::Surface::new(
            &airport,
            &runway.first_name,
            &data.thresholds,
            policy.runway_slope_limit_percent,
        )
        .unwrap();
        let position = position(&airport, point);
        let touchdown = Waypoint {
            latitude: position[0],
            longitude: position[1],
            altitude_feet: surface.height(point),
        };
        let exit = plan(
            &airport,
            &runway.first_name,
            touchdown,
            65.0,
            aircraft,
            handling,
            &policy,
            &surface,
        )
        .unwrap();
        let arrival = parking(
            &airport,
            exit,
            aircraft,
            handling,
            &policy,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &openatc_core::parking::TrafficPreference::default(),
        )
        .unwrap();
        let stand = airport
            .parking
            .iter()
            .find(|stand| stand.name == arrival.stand)
            .unwrap();
        assert!(openatc_core::parking::moving_supports(stand, aircraft));
        assert_eq!(arrival.exit.rollout[1].knots, handling.taxi_turn_knots);
        assert!(
            arrival
                .surface
                .route
                .segment(0)
                .is_some_and(|segment| segment.stage == Stage::TaxiIn)
        );
    }

    #[test]
    #[ignore = "requires installed airport scenery"]
    fn nice_exit_stays_on_pavement_and_slows_before_turning() {
        let root = std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap();
        let airport = super::super::airport(&root, "LFMN").unwrap();
        let aircraft = openatc_core::traffic::profiles::aircraft("A320").unwrap();
        let handling = &animation::builtin().unwrap()[&animation::Kind::Jet];
        let policy = parse_operations(include_str!("../../../../traffic/operations.toml")).unwrap();
        let runway = airport
            .runways
            .iter()
            .find(|runway| runway.first_name == "04L" || runway.second_name == "04L")
            .unwrap();
        let (first, second) = if runway.first_name == "04L" {
            (runway.first, runway.second)
        } else {
            (runway.second, runway.first)
        };
        let length = (second.east - first.east).hypot(second.north - first.north);
        let point = Point {
            east: first.east + (second.east - first.east) * 300.0 / length,
            north: first.north + (second.north - first.north) * 300.0 / length,
            height: 0.0,
        };
        let data = openatc_core::arrival::ArrivalAirport::load(&root, airport.clone()).unwrap();
        let surface = super::super::runway::Surface::new(
            &airport,
            "04L",
            &data.thresholds,
            policy.runway_slope_limit_percent,
        )
        .unwrap();
        let geographic = position(&airport, point);
        let touchdown = Waypoint {
            latitude: geographic[0],
            longitude: geographic[1],
            altitude_feet: surface.height(point),
        };
        let exit = plan(
            &airport, "04L", touchdown, 140.0, aircraft, handling, &policy, &surface,
        )
        .unwrap();
        let end = &exit.rollout.last().unwrap().points;
        assert!(clear_of_runways(
            &airport,
            openatc_core::airport::airport_point(
                &airport,
                end[end.len() - 2].latitude,
                end[end.len() - 2].longitude
            ),
            openatc_core::airport::airport_point(
                &airport,
                exit.vacated.latitude,
                exit.vacated.longitude
            ),
            aircraft,
        ));
        assert_eq!(exit.rollout.len(), 2);
        assert_eq!(exit.rollout[1].knots, handling.taxi_turn_knots);
        assert!(!openatc_core::airport::within_runway(
            &airport,
            &openatc_core::airport::airport_point(
                &airport,
                exit.vacated.latitude,
                exit.vacated.longitude
            )
        ));
        assert!(exit.rollout.iter().all(|segment| {
            segment
                .resources
                .iter()
                .any(|resource| resource.starts_with("RUNWAY/LFMN/"))
        }));
        let arrival = parking(
            &airport,
            exit,
            aircraft,
            handling,
            &policy,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &openatc_core::parking::TrafficPreference::default(),
        )
        .unwrap();
        assert!(!arrival.stand.is_empty());
        assert!(
            arrival
                .surface
                .route
                .segment(0)
                .is_some_and(|segment| segment.stage == Stage::TaxiIn)
        );
        assert!(!arrival.surface.orders.is_empty());
        let stand = airport
            .parking
            .iter()
            .find(|stand| stand.name == arrival.stand)
            .unwrap();
        let coordinate = position(&airport, stand.point);
        let end = arrival.surface.route.end_position();
        assert!(
            openatc_core::arrival::distance_nm(coordinate, [end.latitude, end.longitude]) * 1852.0
                < 1.0
        );
        assert!(
            arrival
                .surface
                .orders
                .iter()
                .any(|(_, order)| !order.crossing_runway.is_empty())
        );
        let reserved = BTreeSet::from([arrival.stand.clone()]);
        let alternative = parking(
            &airport,
            arrival.exit.clone(),
            aircraft,
            handling,
            &policy,
            &[],
            &reserved,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &openatc_core::parking::TrafficPreference::default(),
        )
        .unwrap();
        assert_ne!(alternative.stand, arrival.stand);
        let mut branded = airport.clone();
        for stand in &mut branded.parking {
            stand.airlines = if stand.name == alternative.stand {
                vec!["SIA".into()]
            } else {
                vec!["QFA".into()]
            };
        }
        let preferred = parking(
            &branded,
            arrival.exit.clone(),
            aircraft,
            handling,
            &policy,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &openatc_core::parking::TrafficPreference {
                operator: "sia".into(),
                operation: "airline".into(),
            },
        )
        .unwrap();
        assert_eq!(preferred.stand, alternative.stand);
        let occupied_cells = (0..512)
            .map_while(|index| arrival.surface.route.segment(index))
            .last()
            .unwrap()
            .resources
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let separated = parking(
            &airport,
            arrival.exit.clone(),
            aircraft,
            handling,
            &policy,
            &[],
            &BTreeSet::new(),
            &occupied_cells,
            &BTreeSet::new(),
            &openatc_core::parking::TrafficPreference::default(),
        )
        .unwrap();
        assert_ne!(separated.stand, arrival.stand);
        assert!(
            (0..512)
                .map_while(|index| separated.surface.route.segment(index))
                .all(|segment| segment
                    .resources
                    .iter()
                    .all(|resource| !occupied_cells.contains(resource)))
        );
        let corridor_safe = parking(
            &airport,
            arrival.exit.clone(),
            aircraft,
            handling,
            &policy,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new(),
            &occupied_cells,
            &openatc_core::parking::TrafficPreference::default(),
        )
        .unwrap();
        assert_ne!(corridor_safe.stand, arrival.stand);
        let goal = (0..512)
            .map_while(|index| corridor_safe.surface.route.segment(index))
            .last()
            .unwrap();
        assert!(
            goal.resources
                .iter()
                .all(|resource| !occupied_cells.contains(resource))
        );
        let occupied = TrafficTarget {
            id: 123,
            latitude: coordinate[0],
            longitude: coordinate[1],
            on_ground: true,
            aircraft: "A320".into(),
            wingspan_meters: Some(aircraft.wingspan),
            ..Default::default()
        };
        let alternative = parking(
            &airport,
            arrival.exit.clone(),
            aircraft,
            handling,
            &policy,
            &[occupied],
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            &openatc_core::parking::TrafficPreference::default(),
        )
        .unwrap();
        assert_ne!(alternative.stand, arrival.stand);
        let all_reserved = airport
            .parking
            .iter()
            .map(|stand| stand.name.clone())
            .collect();
        assert!(
            parking(
                &airport,
                arrival.exit.clone(),
                aircraft,
                handling,
                &policy,
                &[],
                &all_reserved,
                &BTreeSet::new(),
                &BTreeSet::new(),
                &openatc_core::parking::TrafficPreference::default(),
            )
            .is_err()
        );
        openatc_core::traffic::motion::Route::compile(
            arrival.exit.rollout,
            aircraft.climb_fpm,
            aircraft.descent_fpm,
        )
        .unwrap();
    }
}

#[derive(Clone)]
pub(super) struct Parking {
    pub exit: Exit,
    pub surface: GroundPlan,
    pub stand: String,
}

/// Reserve only after the caller has committed the complete flight and its gate resource.
// Stand selection needs the airport, aircraft, policy and all three exclusion sets separately.
#[allow(clippy::too_many_arguments)]
pub(super) fn parking(
    airport: &Airport,
    exit: Exit,
    aircraft: &openatc_core::traffic::profiles::Aircraft,
    handling: &animation::Handling,
    policy: &Operations,
    observed: &[TrafficTarget],
    reserved: &BTreeSet<String>,
    occupied_stand_cells: &BTreeSet<String>,
    parking_exclusions: &BTreeSet<String>,
    preference: &openatc_core::parking::TrafficPreference,
) -> Result<Parking, String> {
    let from = openatc_core::airport::airport_point(
        airport,
        exit.vacated.latitude,
        exit.vacated.longitude,
    );
    let last = exit
        .rollout
        .last()
        .ok_or("Arrival has no protected runway exit")?;
    let previous = last
        .points
        .iter()
        .rev()
        .nth(1)
        .ok_or("Arrival exit has no heading")?;
    let heading = openatc_core::arrival::bearing(
        [previous.latitude, previous.longitude],
        [exit.vacated.latitude, exit.vacated.longitude],
    );
    let mut blocked_cells = occupied_stand_cells.clone();
    for target in observed.iter().filter(|target| target.on_ground) {
        if openatc_core::arrival::distance_nm(
            [target.latitude, target.longitude],
            [airport.reference_latitude, airport.reference_longitude],
        ) <= policy.surface_monitor_radius_nm
        {
            blocked_cells.extend(super::cells(
                airport,
                openatc_core::airport::airport_point(airport, target.latitude, target.longitude),
                target.wingspan_meters.unwrap_or(80.0) / 2.0,
                policy.conflict_cell_meters,
            ));
        }
    }
    let mut candidates = airport
        .parking
        .iter()
        .filter(|stand| {
            !reserved.contains(&stand.name)
                && openatc_core::parking::moving_supports(stand, aircraft)
                && super::cells(
                    airport,
                    stand.point,
                    aircraft.wingspan.max(aircraft.length) / 2.0,
                    policy.conflict_cell_meters,
                )
                .is_disjoint(&blocked_cells)
                && super::cells(
                    airport,
                    stand.point,
                    aircraft.wingspan.max(aircraft.length) / 2.0,
                    policy.conflict_cell_meters,
                )
                .is_disjoint(parking_exclusions)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        let distance = |stand: &openatc_core::airport::Parking| {
            (stand.point.east - from.east).hypot(stand.point.north - from.north)
        };
        preference
            .rank(left, aircraft.parking_size())
            .cmp(&preference.rank(right, aircraft.parking_size()))
            .then_with(|| distance(left).total_cmp(&distance(right)))
            .then_with(|| left.name.cmp(&right.name))
    });
    let nominal_airport = Airport {
        elevation_feet: exit.vacated.altitude_feet,
        ..airport.clone()
    };
    for stand in candidates.into_iter().take(32) {
        let result = super::ground_route(
            &nominal_airport,
            from,
            heading,
            false,
            aircraft,
            handling,
            &stand.name,
            0,
            policy,
            true,
            observed,
            true,
        );
        if let Ok(surface) = result {
            if (0..512)
                .map_while(|index| surface.route.segment(index))
                .any(|segment| {
                    segment
                        .resources
                        .iter()
                        .any(|resource| blocked_cells.contains(resource))
                })
            {
                continue;
            }
            return Ok(Parking {
                exit,
                surface,
                stand: stand.name.clone(),
            });
        }
    }
    Err("No free aircraft-compatible stand with a verified taxi-in route".into())
}
