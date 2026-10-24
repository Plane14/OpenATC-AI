//! Check proposed airborne grants against the movement world and current external observations.
#[allow(unused_imports, clippy::wildcard_imports)]
use super::*;
use super::{BTreeSet, Runtime, TrafficTarget, flight};
use openatc_core::{
    state::Telemetry,
    traffic::motion::{Pose, Stage, Waypoint, World},
};

fn apply(world: &mut World, id: u32, index: usize, clearance: &flight::Clearance) -> bool {
    let Some(pose) = world.snapshot().into_iter().find(|pose| pose.id == id) else {
        return false;
    };
    let generation = world.generation();
    if let Some(end) = clearance.initial_climb
        && !world.authorise_airborne_until(generation, id, 2, end)
    {
        return false;
    }
    if let Some(end) = clearance.airborne_through {
        return world.authorise_airborne_until(generation, id, index, end);
    }
    if index == pose.segment {
        world.authorise(generation, id, index, &BTreeSet::new())
    } else {
        world.authorise_continuation(generation, id, index, &BTreeSet::new())
    }
}

fn external(target: &TrafficTarget, rate: f64, time: f64) -> Waypoint {
    let coordinate = openatc_core::arrival::destination(
        [target.latitude, target.longitude],
        target.track_degrees,
        target.ground_speed_knots.unwrap_or(0.0) * time / 3600.0,
    );
    Waypoint {
        latitude: coordinate[0],
        longitude: coordinate[1],
        altitude_feet: target.altitude_feet + rate * time / 60.0,
    }
}

pub(super) fn accepted_world(runtime: &Runtime) -> World {
    let mut world = runtime.world.clone();
    for &(id, index) in &runtime.approved_orders {
        if let Some(contract) = runtime
            .flights
            .get(&id)
            .and_then(|plan| plan.clearances.get(&index))
        {
            let _ = apply(&mut world, id, index, contract);
        }
    }
    world
}

/// True when an airborne target is flying parallel to a different runway rather than crossing
/// the departure path. A 04L arrival must not withhold a 04R departure.
fn parallel_other_runway(
    airport: &Airport,
    runway: &str,
    latitude: f64,
    longitude: f64,
    track_degrees: f64,
) -> bool {
    let Some((_, across, _, heading)) =
        openatc_core::traffic::runway_position(airport, runway, latitude, longitude)
    else {
        return false;
    };
    let angle = openatc_core::navigation::turn(track_degrees, heading).abs();
    angle.min((180.0 - angle).abs()) < 20.0 && across.abs() > 250.0
}

/// The forecast includes existing accepted scopes, their fallback holds and the proposed grant.
pub(super) fn blocked(
    runtime: &Runtime,
    telemetry: &Telemetry,
    id: u32,
    index: usize,
    clearance: &flight::Clearance,
) -> bool {
    let Some(plan) = runtime.flights.get(&id) else {
        return true;
    };
    let Some(segment) = plan.route.segment(index) else {
        return true;
    };
    if segment.stage.on_ground() && segment.stage != Stage::Takeoff {
        return false;
    }
    let runway = segment.resources.iter().find_map(|resource| {
        resource
            .strip_prefix("RUNWAY/")
            .and_then(|rest| rest.split('/').nth(1))
    });
    let airport = runtime.airport.as_deref();
    let Some(policy) = runtime.policy.as_ref() else {
        return true;
    };
    if telemetry.traffic.is_none() || !telemetry.position_valid {
        return true;
    }
    let mut world = accepted_world(runtime);
    if !apply(&mut world, id, index, clearance) {
        return true;
    }
    let Some(pose) = runtime
        .world
        .snapshot()
        .into_iter()
        .find(|pose| pose.id == id)
    else {
        return true;
    };
    let end = clearance
        .initial_climb
        .or(clearance.airborne_through)
        .unwrap_or(index);
    let remaining = if end > pose.segment {
        plan.route
            .nominal_seconds(pose.segment + 1, end)
            .unwrap_or(f64::INFINITY)
    } else {
        0.0
    };
    let horizon = pose.seconds_to_limit + remaining + policy.airborne_prediction_seconds;
    if horizon > 900.0 {
        return true;
    }
    !forecast_clear(runtime, telemetry, id, &world, horizon, runway).unwrap_or(false)
}

pub(super) fn rejoin_blocked(
    runtime: &Runtime,
    telemetry: &Telemetry,
    id: u32,
    plan: &flight::Prepared,
) -> bool {
    rejoin_check(runtime, telemetry, id, plan).is_err()
}

pub(super) fn rejoin_check(
    runtime: &Runtime,
    telemetry: &Telemetry,
    id: u32,
    plan: &flight::Prepared,
) -> Result<(), String> {
    let policy = runtime
        .policy
        .as_ref()
        .ok_or("Missing onward prediction policy")?;
    let mut world = accepted_world(runtime);
    let through = plan.clearances[&0].airborne_through.unwrap_or(0);
    world.queue_hold_exit(
        world.generation(),
        id,
        plan.route.clone(),
        plan.recovery.clone(),
        through,
    )?;
    let horizon = world.holding_exit_seconds(id).unwrap_or(f64::INFINITY)
        + plan
            .route
            .nominal_seconds(0, through)
            .unwrap_or(f64::INFINITY)
        + policy.airborne_prediction_seconds;
    if horizon > 900.0 {
        return Err(format!(
            "Onward forecast needs {horizon:.1} seconds, above its 900-second budget"
        ));
    }
    if forecast_clear(runtime, telemetry, id, &world, horizon, None)? {
        Ok(())
    } else {
        Err("Onward forecast conflicts with another aircraft".into())
    }
}

pub(super) fn forecast_clear(
    runtime: &Runtime,
    telemetry: &Telemetry,
    id: u32,
    world: &World,
    seconds: f64,
    runway: Option<&str>,
) -> Result<bool, String> {
    let policy = runtime
        .policy
        .as_ref()
        .ok_or("Missing airborne prediction policy")?;
    let aircraft_airport = runtime.airport.as_deref();
    let parallel_other = |latitude: f64, longitude: f64, track_degrees: f64| {
        runway
            .zip(aircraft_airport)
            .is_some_and(|(runway, airport)| {
                parallel_other_runway(airport, runway, latitude, longitude, track_degrees)
            })
    };
    let generated: BTreeSet<_> = world.snapshot().iter().map(|pose| pose.id).collect();
    let mut observed = telemetry
        .traffic
        .as_ref()
        .ok_or("Unknown traffic observations")?
        .iter()
        .filter(|target| !generated.contains(&target.id))
        .take(126)
        .map(|target| {
            (
                target.clone(),
                (target.id != 0)
                    .then(|| runtime.observed_climbs.get(&target.id).copied())
                    .flatten(),
            )
        })
        .collect::<Vec<_>>();
    observed.push((
        TrafficTarget {
            id: 0,
            latitude: telemetry.latitude,
            longitude: telemetry.longitude,
            altitude_feet: telemetry.altitude_feet,
            track_degrees: telemetry
                .ground_track_degrees
                .unwrap_or(telemetry.heading_degrees),
            ground_speed_knots: Some(telemetry.ground_speed_knots),
            on_ground: telemetry.on_ground,
            ..Default::default()
        },
        Some(telemetry.vertical_speed_fpm),
    ));
    if observed
        .iter()
        .any(|(target, _)| !openatc_core::traffic::advisory::valid(target))
    {
        return Err("Invalid external airborne observation".into());
    }
    let mut previous: Option<(f64, Vec<Pose>)> = None;
    let mut error = None;
    let clear = world.forecast(seconds, |time, poses| {
        let Some((before, last)) = previous.as_ref() else {
            previous = Some((time, poses.to_vec()));
            return true;
        };
        let Some(own) = poses.iter().find(|pose| pose.id == id) else {
            return false;
        };
        let Some(old) = last.iter().find(|pose| pose.id == id) else {
            return false;
        };
        if own.stage.on_ground() && old.stage.on_ground() {
            previous = Some((time, poses.to_vec()));
            return true;
        }
        let path = [old.position, own.position];
        for other in poses
            .iter()
            .filter(|other| other.id != id && !other.stage.on_ground())
            .filter(|other| {
                !parallel_other(
                    other.position.latitude,
                    other.position.longitude,
                    other.heading,
                )
            })
        {
            let Some(old) = last.iter().find(|pose| pose.id == other.id) else {
                return false;
            };
            match openatc_core::traffic::separation::breach(
                path,
                [old.position, other.position],
                policy.airborne_separation_nm + 0.01,
                policy.airborne_separation_feet,
            ) {
                Ok(false) => {}
                Ok(true) => return false,
                Err(reason) => {
                    error = Some(reason);
                    return false;
                }
            }
        }
        for (target, rate) in observed
            .iter()
            .filter(|(target, _)| !target.on_ground)
            .filter(|(target, _)| {
                !parallel_other(target.latitude, target.longitude, target.track_degrees)
            })
        {
            let vertical = policy.airborne_separation_feet
                + if rate.is_none() {
                    15000.0 * time / 60.0
                } else {
                    0.0
                };
            let horizontal = policy.airborne_separation_nm
                + 0.01
                + if target.ground_speed_knots.is_none() {
                    2000.0 * time / 3600.0
                } else {
                    0.0
                };
            let other = [
                external(target, rate.unwrap_or(0.0), *before),
                external(target, rate.unwrap_or(0.0), time),
            ];
            match openatc_core::traffic::separation::breach(path, other, horizontal, vertical) {
                Ok(false) => {}
                Ok(true) => return false,
                Err(reason) => {
                    error = Some(reason);
                    return false;
                }
            }
        }
        previous = Some((time, poses.to_vec()));
        true
    })?;
    if let Some(error) = error {
        return Err(error);
    }
    Ok(clear)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openatc_core::{
        stations::Station,
        traffic::{
            holding::Pattern,
            motion::{Route, Segment},
        },
    };
    fn fixture(other_feet: f64, other_authorised: bool) -> (Runtime, Telemetry, flight::Clearance) {
        let point = |east, north, altitude| Waypoint {
            latitude: north / 60.0,
            longitude: east / 60.0,
            altitude_feet: altitude,
        };
        let mut runtime = Runtime {
            policy: Some(
                parse_operations(include_str!("../../../../traffic/operations.toml")).unwrap(),
            ),
            ..Default::default()
        };
        let clearance = flight::Clearance {
            amended_route: None,
            transfers: Vec::new(),
            coordinated_recovery: Vec::new(),
            station: Station {
                service: "Center".into(),
                khz: 118_675,
                ..Default::default()
            },
            instruction: "Maintain the cleared route and level".into(),
            readback: "Maintaining the cleared route and level".into(),
            initial_climb: None,
            taxi_through: None,
            airborne_through: None,
            landing_rollout: None,
            missed: None,
            holding: None,
        };
        for (id, start, end, inbound) in [
            (1, point(-6.0, 0.0, 5000.0), point(6.0, 0.0, 5000.0), 90.0),
            (
                2,
                point(0.0, 6.0, other_feet),
                point(0.0, -6.0, other_feet),
                180.0,
            ),
        ] {
            let hold = Pattern {
                fix: end,
                inbound_degrees: inbound,
                right: true,
                ground_knots: 300.0,
                leg_seconds: 60.0,
                bank_degrees: 25.0,
                turn_rate_degrees_sec: 3.0,
            }
            .compile(|_, _| true)
            .unwrap();
            let recovery = Arc::new(Route::compile(vec![hold.clone()], 3000.0, 3000.0).unwrap());
            let route = Arc::new(
                Route::compile(
                    vec![
                        Segment {
                            stage: Stage::Cruise,
                            points: vec![start, end],
                            knots: 300.0,
                            resources: vec![],
                            dwell_seconds: 0.0,
                        },
                        hold,
                    ],
                    3000.0,
                    3000.0,
                )
                .unwrap(),
            );
            assert!(runtime.world.insert(0, id, route.clone()));
            assert!(runtime.world.recovery(0, id, 0, recovery.clone()));
            runtime.flights.insert(
                id,
                Arc::new(flight::Prepared {
                    spacing_floor: None,
                    airspeeds: BTreeMap::new(),
                    route,
                    recovery: BTreeMap::from([(0, recovery)]),
                    clearances: BTreeMap::from([(0, clearance.clone())]),
                    vacated_reports: BTreeMap::new(),
                    amendment: None,
                }),
            );
        }
        if other_authorised {
            assert!(runtime.world.authorise(0, 2, 0, &BTreeSet::new()));
        }
        runtime.world.advance(0.0, false);
        let telemetry = Telemetry {
            position_valid: true,
            on_ground: true,
            traffic: Some(vec![]),
            ..Default::default()
        };
        (runtime, telemetry, clearance)
    }

    #[test]
    fn proposed_airborne_grants_check_crossing_routes_and_pending_grants_without_moving_live_aircraft()
     {
        let (runtime, telemetry, clearance) = fixture(5000.0, true);
        let before = runtime.world.snapshot();
        assert!(blocked(&runtime, &telemetry, 1, 0, &clearance));
        for (old, new) in before.iter().zip(runtime.world.snapshot()) {
            assert_eq!(old.position, new.position);
            assert_eq!(old.awaiting_clearance, new.awaiting_clearance);
        }
        let (mut runtime, telemetry, clearance) = fixture(5000.0, false);
        assert!(!blocked(&runtime, &telemetry, 1, 0, &clearance));
        runtime.approved_orders.insert((2, 0));
        assert!(blocked(&runtime, &telemetry, 1, 0, &clearance));
        assert!(
            runtime
                .world
                .snapshot()
                .iter()
                .all(|pose| pose.awaiting_clearance)
        );
        let (runtime, mut telemetry, clearance) = fixture(7000.0, true);
        assert!(!blocked(&runtime, &telemetry, 1, 0, &clearance));
        telemetry.traffic = None;
        assert!(blocked(&runtime, &telemetry, 1, 0, &clearance));
    }

    #[test]
    fn external_motion_uses_recent_climb_observations_and_unknown_identity_is_not_the_player() {
        let (mut runtime, mut telemetry, clearance) = fixture(7000.0, true);
        let crossing = TrafficTarget {
            id: 99,
            latitude: 0.1,
            longitude: 0.0,
            altitude_feet: 5000.0,
            ground_speed_knots: Some(300.0),
            track_degrees: 180.0,
            on_ground: false,
            ..Default::default()
        };
        telemetry.traffic = Some(vec![crossing.clone()]);
        assert!(blocked(&runtime, &telemetry, 1, 0, &clearance));
        runtime.observed_climbs.insert(99, 6000.0);
        assert!(!blocked(&runtime, &telemetry, 1, 0, &clearance));
        telemetry.traffic.as_mut().unwrap()[0].id = 0;
        telemetry.vertical_speed_fpm = 6000.0;
        assert!(blocked(&runtime, &telemetry, 1, 0, &clearance));
        telemetry.traffic = Some(vec![TrafficTarget {
            ground_speed_knots: None,
            ..crossing
        }]);
        assert!(blocked(&runtime, &telemetry, 1, 0, &clearance));
    }
}
