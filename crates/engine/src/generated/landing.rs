//! Runway exits use the installed taxi graph; an apron gap is never an exit.
use super::*;
use openatc_core::traffic::{
    curves,
    motion::{Segment, Stage, Waypoint},
};

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
    let heading = unit[0].atan2(unit[1]).to_degrees().rem_euclid(360.0);
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
            let (Some(entrance), Some(exit)) = (nodes.get(&first), nodes.get(&second)) else {
                continue;
            };
            let at =
                (entrance.east - start.east) * unit[0] + (entrance.north - start.north) * unit[1];
            let offset =
                (entrance.east - start.east) * unit[1] - (entrance.north - start.north) * unit[0];
            if offset.abs() > 2.0
                || at < along + braking + radius * 2.0
                || at + envelope >= length
                || openatc_core::airport::within_runway(airport, exit)
            {
                continue;
            }
            if !airport.edges.iter().any(|runway_edge| {
                runway_edge.runway && (runway_edge.first == first || runway_edge.second == first)
            }) {
                continue;
            }
            candidates.push((at, *entrance, *exit));
        }
    }
    candidates.sort_by(|left, right| left.0.total_cmp(&right.0));
    for (at, entrance, exit) in candidates.into_iter().take(32) {
        let turn_start = Point {
            east: start.east + (at - radius * 2.0) * unit[0],
            north: start.north + (at - radius * 2.0) * unit[1],
            height: 0.0,
        };
        let exit_heading = (exit.east - entrance.east)
            .atan2(exit.north - entrance.north)
            .to_degrees()
            .rem_euclid(360.0);
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
        let Ok(turn) = curves::connect(
            curves::Pose {
                point: turn_start,
                heading,
            },
            curves::Pose {
                point: exit,
                heading: exit_heading,
            },
            &curves::Limits {
                radius,
                spacing: policy.surface_sample_meters,
                length: 1000.0,
                initial_turn: None,
            },
            |a, b| protected(a.point) && protected(b.point),
        ) else {
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
