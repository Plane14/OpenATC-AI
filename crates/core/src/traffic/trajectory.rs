//! Bounded airborne geometry and crossing levels prepared on planning workers.
use super::{
    curves,
    motion::Waypoint,
    vertical::{self, Crossing},
};
use crate::{
    arrival::{bearing, distance_nm},
    state::Point,
};

/// A resolved crossing after decoding its published leg and pressure reference.
#[derive(Clone)]
pub struct Fix {
    /// Latitude and longitude in degrees.
    pub coordinate: [f64; 2],
    /// Required true arrival course, including fly-over and course-to-fix legs.
    pub arrival_heading: Option<f64>,
    /// Crossing floor converted to geometric feet MSL.
    pub minimum_feet: f64,
    /// Crossing ceiling converted to geometric feet MSL.
    pub maximum_feet: f64,
    /// Desired geometric level before restrictions are applied.
    pub preferred_feet: f64,
    /// Ground speed after applying the aircraft envelope, wind and published IAS limit.
    pub ground_knots: f64,
    /// Published turn direction, when specified.
    pub required_turn: Option<curves::Turn>,
}

/// Performance and sampling limits used during planning.
pub struct Envelope {
    /// Maximum bank for planned turns.
    pub bank_degrees: f64,
    /// Maximum angular turn rate in degrees per second.
    pub turn_rate_degrees_sec: f64,
    /// Maximum climb rate in feet per minute.
    pub climb_fpm: f64,
    /// Maximum descent rate in feet per minute.
    pub descent_fpm: f64,
    /// Maximum spacing between protection samples.
    pub spacing_meters: f64,
    /// Permitted curve length beyond the direct connection.
    pub maximum_detour_meters: f64,
}

/// A sampled connection ending at its crossing restriction.
pub struct Leg {
    /// Geographic path with geometric heights.
    pub points: Vec<Waypoint>,
    /// Planned speed over the ground.
    pub ground_knots: f64,
    /// Duration at the planned speed.
    pub seconds: f64,
}

/// Build heading-continuous fly-over connectors; every sample needs corridor protection.
/// This does not grant flight or turn an unresolved procedure into a direct clearance.
pub fn compile(
    start: Waypoint,
    heading_degrees: f64,
    fixes: &[Fix],
    final_heading_degrees: f64,
    envelope: &Envelope,
    protected: impl Fn(Waypoint, Waypoint) -> bool,
) -> Result<Vec<Leg>, String> {
    if fixes.is_empty()
        || fixes.len() > 128
        || ![heading_degrees, final_heading_degrees]
            .iter()
            .all(|v| v.is_finite())
        || !(5.0..=30.0).contains(&envelope.bank_degrees)
        || !(0.5..=3.0).contains(&envelope.turn_rate_degrees_sec)
        || !(10.0..=250.0).contains(&envelope.spacing_meters)
        || !(0.0..=100000.0).contains(&envelope.maximum_detour_meters)
        || ![start.latitude, start.longitude, start.altitude_feet]
            .iter()
            .all(|v| v.is_finite())
        || start.latitude.abs() >= 85.0
        || start.longitude.abs() > 180.0
    {
        return Err("Invalid generated airborne envelope".into());
    }
    let origin = [start.latitude, start.longitude];
    let projection = |coordinate: [f64; 2]| Point {
        east: crate::navigation::turn(origin[1], coordinate[1])
            * 111320.0
            * origin[0].to_radians().cos(),
        north: (coordinate[0] - origin[0]) * 111320.0,
        height: 0.0,
    };
    let geographic = |point: Point, altitude_feet: f64| Waypoint {
        latitude: origin[0] + point.north / 111320.0,
        longitude: (origin[1] + point.east / (111320.0 * origin[0].to_radians().cos()) + 180.0)
            .rem_euclid(360.0)
            - 180.0,
        altitude_feet,
    };
    let mut from = curves::Pose {
        point: projection(origin),
        heading: heading_degrees,
    };
    let mut geometry = Vec::with_capacity(fixes.len());
    let mut crossings = Vec::with_capacity(fixes.len());
    let mut count = 0;
    for (index, fix) in fixes.iter().enumerate() {
        if !fix.coordinate.iter().all(|value| value.is_finite())
            || fix.coordinate[0].abs() >= 85.0
            || fix.coordinate[1].abs() > 180.0
            || !(40.0..=550.0).contains(&fix.ground_knots)
        {
            return Err(format!("Invalid airborne crossing at {index}"));
        }
        let speed = fix.ground_knots * (1852.0 / 3600.0);
        let radius = (speed.powi(2) / (9.80665 * envelope.bank_degrees.to_radians().tan()))
            .max(speed / envelope.turn_rate_degrees_sec.to_radians());
        let heading = fix.arrival_heading.unwrap_or_else(|| {
            fixes.get(index + 1).map_or(final_heading_degrees, |next| {
                bearing(fix.coordinate, next.coordinate)
            })
        });
        if !heading.is_finite() {
            return Err(format!("Invalid arrival course at {index}"));
        }
        let to = curves::Pose {
            point: projection(fix.coordinate),
            heading,
        };
        let length = (to.point.east - from.point.east).hypot(to.point.north - from.point.north);
        let samples = curves::connect(
            from,
            to,
            &curves::Limits {
                radius,
                spacing: envelope.spacing_meters,
                length: length + envelope.maximum_detour_meters,
                initial_turn: fix.required_turn,
            },
            |_, _| true,
        )?;
        count += samples.len();
        if count > 4096 {
            return Err("Generated airborne route exceeds its sample budget".into());
        }
        let meters: f64 = samples
            .windows(2)
            .map(|pair| {
                let a = geographic(pair[0].point, 0.0);
                let b = geographic(pair[1].point, 0.0);
                distance_nm([a.latitude, a.longitude], [b.latitude, b.longitude]) * 1852.0
            })
            .sum();
        let seconds = meters / speed;
        crossings.push(Crossing {
            minimum_feet: fix.minimum_feet,
            maximum_feet: fix.maximum_feet,
            preferred_feet: fix.preferred_feet,
            seconds,
        });
        geometry.push((samples, meters));
        from = to;
    }
    let levels = vertical::levels(
        start.altitude_feet,
        &crossings,
        envelope.climb_fpm,
        envelope.descent_fpm,
    )?;
    let mut altitude = start.altitude_feet;
    let mut result = Vec::with_capacity(fixes.len());
    for (((samples, meters), crossing), level) in geometry.into_iter().zip(crossings).zip(levels) {
        let mut along = 0.0;
        let mut points = Vec::with_capacity(samples.len());
        for (index, sample) in samples.iter().enumerate() {
            if index > 0 {
                let previous = geographic(samples[index - 1].point, 0.0);
                let current = geographic(sample.point, 0.0);
                along += distance_nm(
                    [previous.latitude, previous.longitude],
                    [current.latitude, current.longitude],
                ) * 1852.0;
            }
            points.push(geographic(
                sample.point,
                altitude + (level - altitude) * (along / meters).clamp(0.0, 1.0),
            ));
        }
        if points.windows(2).any(|pair| !protected(pair[0], pair[1])) {
            return Err("Generated airborne path lacks verified corridor protection".into());
        }
        let nm: f64 = points
            .windows(2)
            .map(|pair| {
                distance_nm(
                    [pair[0].latitude, pair[0].longitude],
                    [pair[1].latitude, pair[1].longitude],
                )
            })
            .sum();
        let seconds = nm / fixes[result.len()].ground_knots * 3600.0;
        let rate = (level - altitude) * 60.0 / seconds;
        if rate > envelope.climb_fpm + 1.0 || -rate > envelope.descent_fpm + 1.0 {
            return Err("Geographic projection exceeds the vertical envelope".into());
        }
        altitude = level;
        result.push(Leg {
            points,
            ground_knots: fixes[result.len()].ground_knots,
            seconds: crossing.seconds,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_ceiling_shapes_a_continuous_turning_descent() {
        let start = Waypoint {
            latitude: 44.0,
            longitude: 8.0,
            altitude_feet: 18000.0,
        };
        let fixes = [
            Fix {
                coordinate: [44.2, 8.0],
                arrival_heading: None,
                minimum_feet: 0.0,
                maximum_feet: 18000.0,
                preferred_feet: 18000.0,
                ground_knots: 200.0,
                required_turn: None,
            },
            Fix {
                coordinate: [44.2, 8.1],
                arrival_heading: None,
                minimum_feet: 12000.0,
                maximum_feet: 12000.0,
                preferred_feet: 12000.0,
                ground_knots: 180.0,
                required_turn: None,
            },
        ];
        let envelope = Envelope {
            bank_degrees: 20.0,
            turn_rate_degrees_sec: 3.0,
            climb_fpm: 1500.0,
            descent_fpm: 2000.0,
            spacing_meters: 150.0,
            maximum_detour_meters: 20000.0,
        };
        let path = compile(start, 0.0, &fixes, 90.0, &envelope, |a, b| {
            a.altitude_feet >= 10000.0 && b.altitude_feet >= 10000.0
        })
        .unwrap();
        assert!(path[0].points.last().unwrap().altitude_feet < 18000.0);
        assert_eq!(path[0].points.last(), path[1].points.first());
        assert!((path[1].points.last().unwrap().altitude_feet - 12000.0).abs() < 0.01);
        assert!(compile(start, 0.0, &fixes, 90.0, &envelope, |_, _| false).is_err());
    }
}
