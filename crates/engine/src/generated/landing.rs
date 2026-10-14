//! Runway exits use the installed taxi graph; an apron gap is never an exit.
use super::*;
use openatc_core::traffic::motion::{Segment, Stage, Waypoint};

pub(super) struct Exit {
    pub rollout: Vec<Segment>,
    pub vacated: Waypoint,
}

pub(super) fn plan(
    airport: &Airport,
    runway_name: &str,
    touchdown: Waypoint,
    touchdown_knots: f64,
    aircraft: &openatc_core::traffic::profiles::Aircraft,
    handling: &animation::Handling,
    policy: &Operations,
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
    if length < aircraft.runway
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
        || (touchdown.altitude_feet - airport.elevation_feet).abs() > 5.0
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
            let at =
                (entrance.east - start.east) * unit[0] + (entrance.north - start.north) * unit[1];
            let offset =
                (entrance.east - start.east) * unit[1] - (entrance.north - start.north) * unit[0];
            if offset.abs() > policy.surface_sample_meters * 2.0
                || at < along + braking + radius * 2.0
                || at + envelope >= length
            {
                continue;
            }
            if !airport.edges.iter().any(|runway_edge| {
                runway_edge.runway && (runway_edge.first == first || runway_edge.second == first)
            }) {
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
                let geographic = position(airport, point);
                let clear = !openatc_core::airport::within_runway(airport, &point)
                    && openatc_core::traffic::runway_position(
                        airport,
                        runway_name,
                        geographic[0],
                        geographic[1],
                    )
                    .is_some_and(|(_, across, _, _)| across.abs() > runway.width / 2.0 + envelope);
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
            continue;
        };
        let waypoint = |point: Point| {
            let geographic = position(airport, point);
            Waypoint {
                latitude: geographic[0],
                longitude: geographic[1],
                altitude_feet: airport.elevation_feet,
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
            continue;
        }
        let vacated = *points.last().unwrap();
        return Ok(Exit {
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
        });
    }
    Err("No verified forward runway exit within the landing and taxi envelope".into())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let geographic = position(&airport, point);
        let touchdown = Waypoint {
            latitude: geographic[0],
            longitude: geographic[1],
            altitude_feet: airport.elevation_feet,
        };
        let exit = plan(
            &airport, "04L", touchdown, 140.0, aircraft, handling, &policy,
        )
        .unwrap();
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
        openatc_core::traffic::motion::Route::compile(
            exit.rollout,
            aircraft.climb_fpm,
            aircraft.descent_fpm,
        )
        .unwrap();
    }
}
