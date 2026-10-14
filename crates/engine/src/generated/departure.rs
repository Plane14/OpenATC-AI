//! Protected runway entry and a profile-specific roll, before any airborne grant is considered.
use super::*;
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

pub(super) fn plan(
    airport: &Airport,
    runway_name: &str,
    hold: Waypoint,
    aircraft: &openatc_core::traffic::profiles::Aircraft,
    handling: &animation::Handling,
    rotation_ground_knots: f64,
    policy: &Operations,
) -> Result<Surface, String> {
    if !(35.0..=230.0).contains(&rotation_ground_knots) {
        return Err("Departure rotation ground speed is unresolved".into());
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
    let route = openatc_core::airport::calculate_runway_taxi_route(
        airport,
        &telemetry,
        runway_name,
        aircraft.parking_size(),
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
    let speed = rotation_ground_knots * 0.514444;
    let rotation_time = (handling.rotation_pitch_degrees / handling.pitch_rate_degrees_sec
        - 0.1 * rotation_ground_knots / handling.acceleration_knots_sec)
        .max(0.0);
    let roll =
        speed.powi(2) / (2.0 * handling.acceleration_knots_sec * 0.514444) + speed * rotation_time;
    if !remaining.is_finite()
        || entry_along < 0.0
        || remaining < aircraft.runway
        || roll + aircraft.length >= remaining
    {
        return Err("Remaining runway cannot support the planned departure roll".into());
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
    let supported = |point| {
        openatc_core::airport::on_pavement(airport, point)
            && !openatc_core::airport::within_runway(&other, &point)
    };
    let rounded = openatc_core::traffic::surface::rounded(
        &points,
        handling.taxi_turn_radius_meters,
        handling.wheelbase_meters / handling.maximum_steering_degrees.to_radians().tan(),
        policy.surface_sample_meters,
        supported,
    )?;
    if rounded.windows(2).any(|pair| {
        let from = pair[0].point;
        let to = pair[1].point;
        let count = ((to.east - from.east).hypot(to.north - from.north)
            / policy.surface_sample_meters)
            .ceil()
            .max(1.0) as usize;
        (0..=count).any(|index| {
            !supported(Point {
                east: from.east + (to.east - from.east) * index as f64 / count as f64,
                north: from.north + (to.north - from.north) * index as f64 / count as f64,
                height: 0.0,
            })
        })
    }) {
        return Err("Runway entry connector lacks verified pavement".into());
    }
    let waypoint = |point| {
        let coordinate = position(airport, point);
        Waypoint {
            latitude: coordinate[0],
            longitude: coordinate[1],
            altitude_feet: airport.elevation_feet,
        }
    };
    let resource = format!(
        "RUNWAY/{}/{}/{}",
        airport.icao, runway.first_name, runway.second_name
    );
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
    let surface = Surface {
        rotation,
        remaining_meters: remaining,
        segments: vec![
            Segment {
                stage: Stage::LineUp,
                points: rounded
                    .iter()
                    .map(|sample| waypoint(sample.point))
                    .collect(),
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
