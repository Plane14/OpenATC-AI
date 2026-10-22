//! Protected runway entry and a profile-specific roll, before any airborne grant is considered.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
#[allow(unused_imports, clippy::wildcard_imports)]
use super::*;
use super::{Airport, BTreeSet, Operations, Point, animation, cells, position};
use openatc_core::traffic::motion::{Segment, Stage, Waypoint};

#[derive(Clone)]
pub(super) struct Surface {
    pub segments: Vec<Segment>,
    pub rotation: Waypoint,
    pub remaining_meters: f64,
}
impl Surface {
    fn validate(&self, aircraft: &openatc_core::traffic::profiles::Aircraft) -> Result<(), String> {
        if !self.remaining_meters.is_finite()
            || self.remaining_meters < aircraft.runway
            || self
                .segments
                .last()
                .and_then(|segment| segment.points.last())
                != Some(&self.rotation)
        {
            return Err("Departure surface has an invalid runway limit".into());
        }
        openatc_core::traffic::motion::Route::compile(
            self.segments.clone(),
            aircraft.climb_fpm,
            aircraft.descent_fpm,
        )?;
        Ok(())
    }
}

// Each value is an independent planning input; the call sites already name them.
#[allow(clippy::too_many_arguments)]
pub(super) fn plan(
    airport: &Airport,
    runway_name: &str,
    hold: Waypoint,
    aircraft: &openatc_core::traffic::profiles::Aircraft,
    handling: &animation::Handling,
    weather: &super::environment::Environment,
    surface: &super::runway::Surface,
    policy: &Operations,
) -> Result<Surface, String> {
    match plan_entry(
        airport,
        runway_name,
        hold,
        aircraft,
        handling,
        weather,
        surface,
        policy,
        false,
    ) {
        Ok(surface) => Ok(surface),
        Err(first) => plan_entry(
            airport,
            runway_name,
            hold,
            aircraft,
            handling,
            weather,
            surface,
            policy,
            true,
        )
        .map_err(|second| format!("{first}; full-length retry: {second}")),
    }
}

#[allow(clippy::too_many_arguments)]
fn plan_entry(
    airport: &Airport,
    runway_name: &str,
    hold: Waypoint,
    aircraft: &openatc_core::traffic::profiles::Aircraft,
    handling: &animation::Handling,
    weather: &super::environment::Environment,
    surface: &super::runway::Surface,
    policy: &Operations,
    full_length: bool,
) -> Result<Surface, String> {
    if !surface.matches(airport, runway_name) {
        return Err("Departure surface belongs to another runway".into());
    }
    let runway = airport
        .runways
        .iter()
        .find(|runway| runway.first_name == runway_name || runway.second_name == runway_name)
        .ok_or("Departure runway is unavailable")?;
    let (start, end) = if runway.first_name == runway_name {
        (runway.first, runway.second)
    } else {
        (runway.second, runway.first)
    };
    let length = (end.east - start.east).hypot(end.north - start.north);
    if !length.is_finite() || length < 100.0 {
        return Err("Departure runway geometry is invalid".into());
    }
    let u = [
        (end.east - start.east) / length,
        (end.north - start.north) / length,
    ];
    let telemetry = openatc_core::state::Telemetry {
        latitude: hold.latitude,
        longitude: hold.longitude,
        position_valid: true,
        on_ground: true,
        ..Default::default()
    };
    let route = openatc_core::airport::calculate_runway_taxi_route_with(
        airport,
        &telemetry,
        runway_name,
        aircraft.parking_size(),
        full_length,
    )?;
    if route.points.len() < 2 {
        return Err("No verified runway entry path".into());
    }
    let mut points = route.points;
    points.pop(); // Keep the actual entry instead of backtracking to an assumed full-length start.
    let entry = *points.last().ok_or("Runway entry point is missing")?;
    let entry_along = (entry.east - start.east) * u[0] + (entry.north - start.north) * u[1];
    let aligned = entry_along + (handling.taxi_turn_radius_meters * 2.0).max(aircraft.length * 2.0);
    let remaining = length - aligned;
    let roll_at = |knots: f64| {
        let speed = knots * 0.514_444;
        let rotation_time = (handling.rotation_pitch_degrees / handling.pitch_rate_degrees_sec
            - 0.1 * knots / handling.acceleration_knots_sec)
            .max(0.0);
        speed.powi(2) / (2.0 * handling.acceleration_knots_sec * 0.514_444) + speed * rotation_time
    };
    let mut rotation_ground_knots = weather.runway_speed(
        airport,
        runway_name,
        handling.rotation_knots,
        surface.height(surface.point(aligned)),
    )?;
    let mut converged = false;
    for _ in 0..12 {
        let height = surface.height(surface.point(aligned + roll_at(rotation_ground_knots)));
        let next = weather.runway_speed(airport, runway_name, handling.rotation_knots, height)?;
        if (next - rotation_ground_knots).abs() < 0.001 {
            rotation_ground_knots = next;
            converged = true;
            break;
        }
        rotation_ground_knots = next;
    }
    if !converged || !(35.0..=230.0).contains(&rotation_ground_knots) {
        return Err("Departure rotation ground speed is unresolved".into());
    }
    let roll = roll_at(rotation_ground_knots);
    if !remaining.is_finite()
        || entry_along < 0.0
        || remaining < aircraft.runway
        || roll + aircraft.length >= remaining
    {
        return Err(format!(
            "Remaining runway cannot support the planned departure roll (entry {entry_along:.0} m, aligned {aligned:.0} m, remaining {remaining:.0} m, need {:.0} m, roll {roll:.0} m)",
            aircraft.runway
        ));
    }
    let point = |along| Point {
        east: start.east + u[0] * along,
        north: start.north + u[1] * along,
        height: 0.0,
    };
    points.push(point(aligned));
    points.insert(
        0,
        openatc_core::airport::airport_point(airport, hold.latitude, hold.longitude),
    );
    let other = Airport {
        runways: airport
            .runways
            .iter()
            .filter(|other| {
                other.first_name != runway.first_name || other.second_name != runway.second_name
            })
            .cloned()
            .collect(),
        ..Default::default()
    };
    let refused = std::cell::Cell::new(None);
    let supported = |point| {
        let ok = openatc_core::airport::on_pavement(airport, point)
            && !openatc_core::airport::within_runway(&other, &point);
        if !ok {
            refused.set(Some(point));
        }
        ok
    };
    let rounded = openatc_core::traffic::surface::rounded(
        &points,
        handling.taxi_turn_radius_meters,
        handling.wheelbase_meters / handling.maximum_steering_degrees.to_radians().tan(),
        policy.surface_sample_meters,
        supported,
    )
    .map_err(|error| {
        if let Some(point) = refused.get() {
            let coordinate = position(airport, point);
            format!("{error} at {:.7}, {:.7}", coordinate[0], coordinate[1])
        } else {
            error
        }
    })?;
    let failure = rounded.windows(2).find_map(|pair| {
        let from = pair[0].point;
        let to = pair[1].point;
        let count = ((to.east - from.east).hypot(to.north - from.north)
            / policy.surface_sample_meters)
            .ceil()
            .max(1.0) as usize;
        (0..=count)
            .map(|index| Point {
                east: from.east + (to.east - from.east) * index as f64 / count as f64,
                north: from.north + (to.north - from.north) * index as f64 / count as f64,
                height: 0.0,
            })
            .find(|point| !supported(*point))
    });
    if let Some(point) = failure {
        let coordinate = position(airport, point);
        let blocking = other.runways.iter().find(|runway| {
            openatc_core::airport::within_runway(
                &openatc_core::airport::Airport {
                    runways: vec![(*runway).clone()],
                    ..Default::default()
                },
                &point,
            )
        });
        if let Some(runway) = blocking {
            return Err(format!(
                "Runway entry connector enters the {}/{} protected strip at {:.7}, {:.7}",
                runway.first_name, runway.second_name, coordinate[0], coordinate[1]
            ));
        }
        return Err(format!(
            "Runway entry connector lacks verified pavement at {:.7}, {:.7}",
            coordinate[0], coordinate[1]
        ));
    }
    let waypoint = |point| {
        let coordinate = position(airport, point);
        Waypoint {
            latitude: coordinate[0],
            longitude: coordinate[1],
            altitude_feet: surface.height(point),
        }
    };
    let resource = surface.resource.clone();
    let mut protected = BTreeSet::from([resource.clone()]);
    for sample in &rounded {
        protected.extend(cells(
            airport,
            sample.point,
            aircraft.wingspan.max(aircraft.length) / 2.0,
            policy.conflict_cell_meters,
        ));
    }
    if protected.len() > 32 {
        return Err("Runway entry exceeds the protected resource budget".into());
    }
    let rotation = waypoint(point(aligned + roll));
    let mut roll_resources = BTreeSet::from([resource]);
    let count = (roll / policy.surface_sample_meters).ceil().max(1.0) as usize;
    if count > 4096 {
        return Err("Departure roll exceeds the sample budget".into());
    }
    for index in 0..=count {
        let sample = waypoint(point(aligned + roll * index as f64 / count as f64));
        for other in &airport.runways {
            if let Some((along, across, length, _)) = openatc_core::traffic::runway_position(
                airport,
                &other.first_name,
                sample.latitude,
                sample.longitude,
            ) {
                let envelope = aircraft.wingspan.max(aircraft.length) / 2.0;
                if along >= -envelope
                    && along <= length + envelope
                    && across.abs() <= other.width / 2.0 + envelope
                {
                    roll_resources.insert(format!(
                        "RUNWAY/{}/{}/{}",
                        airport.icao, other.first_name, other.second_name
                    ));
                }
            }
        }
    }
    let mut lineup: Vec<_> = rounded
        .iter()
        .map(|sample| waypoint(sample.point))
        .collect();
    let mut distances = vec![0.0];
    for pair in lineup.windows(2) {
        distances.push(
            distances.last().unwrap()
                + openatc_core::arrival::distance_nm(
                    [pair[0].latitude, pair[0].longitude],
                    [pair[1].latitude, pair[1].longitude],
                ),
        );
    }
    let distance = *distances.last().ok_or("Runway entry has no samples")?;
    if distance <= 0.0 {
        return Err("Runway entry has no length".into());
    }
    let aligned_height = surface.height(point(aligned));
    // Taxi nodes have no surveyed heights. Preserve the hold datum and join the
    // selected runway surface gradually; native terrain placement remains separate.
    for (sample, along) in lineup.iter_mut().zip(distances) {
        sample.altitude_feet =
            hold.altitude_feet + (aligned_height - hold.altitude_feet) * along / distance;
    }
    lineup[0] = hold;
    *lineup.last_mut().unwrap() = waypoint(point(aligned));
    let surface = Surface {
        rotation,
        remaining_meters: remaining,
        segments: vec![
            Segment {
                stage: Stage::LineUp,
                points: lineup,
                knots: handling.taxi_turn_knots,
                resources: protected.into_iter().collect(),
                dwell_seconds: 0.0,
            },
            Segment {
                stage: Stage::Takeoff,
                points: vec![waypoint(point(aligned)), rotation],
                knots: rotation_ground_knots,
                resources: roll_resources.into_iter().collect(),
                dwell_seconds: 0.0,
            },
        ],
    };
    surface.validate(aircraft)?;
    Ok(surface)
}
