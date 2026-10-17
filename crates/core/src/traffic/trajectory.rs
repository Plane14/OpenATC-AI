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

/// Retain the IAS requirement when vertical scheduling changes a crossing level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Airspeed {
    /// Indicated speed chosen within the published limits.
    pub indicated_knots: f64,
    /// Sampled airport pressure/temperature reference.
    pub atmosphere: super::air::Atmosphere,
    /// True direction from which the planning wind blows.
    pub wind_from: f64,
    /// Planning wind speed in knots.
    pub wind_knots: f64,
}
impl Airspeed {
    /// Resolve movement speed after the crossing height and track are known.
    pub fn ground_knots(&self, geometric_feet: f64, track: f64) -> Result<f64, String> {
        super::air::wind(
            self.atmosphere
                .true_knots(self.indicated_knots, geometric_feet)?,
            track,
            self.wind_from,
            self.wind_knots,
        )
        .map(|(ground, _)| ground)
    }
}

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
    /// Original IAS and weather; None explicitly represents a fixed ground-speed path.
    pub airspeed: Option<Airspeed>,
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
#[derive(Clone)]
pub struct Leg {
    /// Geographic path with geometric heights.
    pub points: Vec<Waypoint>,
    /// Planned speed over the ground.
    pub ground_knots: f64,
    /// Duration at the planned speed.
    pub seconds: f64,
}

/// Solved geometry and levels awaiting an independent terrain/airspace check.
/// Its movement samples are unavailable until every connection passes that check.
#[derive(Clone)]
pub struct Draft {
    legs: Vec<Leg>,
}
impl Draft {
    /// Planning diagnostics; this does not make movement samples available.
    pub fn leg_count(&self) -> usize {
        self.legs.len()
    }

    /// Release movement samples only when the complete solved path is protected.
    pub fn protect(
        self,
        protected: impl Fn(Waypoint, Waypoint) -> bool,
    ) -> Result<Vec<Leg>, String> {
        if self.legs.iter().any(|leg| {
            leg.points
                .windows(2)
                .any(|pair| !protected(pair[0], pair[1]))
        }) {
            return Err("Generated airborne path lacks verified corridor protection".into());
        }
        Ok(self.legs)
    }
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
    draft(
        start,
        heading_degrees,
        fixes,
        final_heading_degrees,
        envelope,
    )?
    .protect(protected)
}

/// Solve bounded geometry, levels and nominal airspeed without granting movement.
pub fn draft(
    start: Waypoint,
    heading_degrees: f64,
    fixes: &[Fix],
    final_heading_degrees: f64,
    envelope: &Envelope,
) -> Result<Draft, String> {
    if fixes.is_empty() || fixes.len() > 128 {
        return Err("Generated airborne crossing budget is invalid".into());
    }
    let mut scheduled = fixes.to_vec();
    let mut previous = [start.latitude, start.longitude];
    // Preferred cruise heights are not a valid initial TAS estimate for the
    // first SID turn. Seed from the actual starting height, then solve the
    // complete level/speed/geometry profile to convergence below.
    for fix in &mut scheduled {
        if let Some(airspeed) = fix.airspeed {
            let track = fix
                .arrival_heading
                .unwrap_or_else(|| bearing(previous, fix.coordinate));
            let seed_height = start
                .altitude_feet
                .min(fix.preferred_feet)
                .max(fix.minimum_feet)
                .min(fix.maximum_feet);
            fix.ground_knots = airspeed.ground_knots(seed_height, track)?;
        }
        previous = fix.coordinate;
    }
    // A changed level changes TAS, turn radius and time available to climb or
    // descend. Rebuild all three together; never publish an unconverged path.
    for _ in 0..12 {
        let legs = compile_once(
            start,
            heading_degrees,
            &scheduled,
            final_heading_degrees,
            envelope,
        )?;
        let mut settled = true;
        for (fix, leg) in scheduled.iter_mut().zip(&legs) {
            let Some(airspeed) = fix.airspeed else {
                continue;
            };
            let end = leg
                .points
                .last()
                .ok_or("Airborne crossing has no endpoint")?;
            let before = leg
                .points
                .iter()
                .rev()
                .nth(1)
                .ok_or("Airborne crossing has no incoming track")?;
            let track = bearing(
                [before.latitude, before.longitude],
                [end.latitude, end.longitude],
            );
            let ground = airspeed.ground_knots(end.altitude_feet, track)?;
            settled &= (ground - fix.ground_knots).abs() < 0.01;
            fix.ground_knots = ground;
        }
        if settled {
            return Ok(Draft { legs });
        }
    }
    Err("Joint altitude and airspeed planning did not converge".into())
}

fn compile_once(
    start: Waypoint,
    heading_degrees: f64,
    fixes: &[Fix],
    final_heading_degrees: f64,
    envelope: &Envelope,
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
        )
        .map_err(|error| format!("Airborne crossing {index}, radius {radius:.0} m, length {length:.0} m, headings {:.1}/{heading:.1}: {error}", from.heading))?;
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
    fn solved_levels_preserve_piston_turboprop_and_jet_indicated_speeds() {
        let atmosphere = super::super::air::Atmosphere::surface(1013.25, 15.0, 0.0).unwrap();
        let start = Waypoint {
            latitude: 44.0,
            longitude: 8.0,
            altitude_feet: 18000.0,
        };
        for (indicated, climb, descent) in [
            (65.0, 650.0, 500.0),
            (110.0, 1500.0, 1200.0),
            (170.0, 2500.0, 1000.0),
        ] {
            let airspeed = Airspeed {
                indicated_knots: indicated,
                atmosphere,
                wind_from: 280.0,
                wind_knots: 12.0,
            };
            let fixes = [
                Fix {
                    coordinate: [44.3, 8.0],
                    arrival_heading: Some(0.0),
                    minimum_feet: 0.0,
                    maximum_feet: 18000.0,
                    preferred_feet: 18000.0,
                    ground_knots: airspeed.ground_knots(18000.0, 0.0).unwrap(),
                    airspeed: Some(airspeed),
                    required_turn: None,
                },
                Fix {
                    coordinate: [44.3, 8.08],
                    arrival_heading: Some(90.0),
                    minimum_feet: 12000.0,
                    maximum_feet: 12000.0,
                    preferred_feet: 12000.0,
                    ground_knots: airspeed.ground_knots(12000.0, 90.0).unwrap(),
                    airspeed: Some(airspeed),
                    required_turn: None,
                },
            ];
            let envelope = Envelope {
                bank_degrees: 20.0,
                turn_rate_degrees_sec: 3.0,
                climb_fpm: climb,
                descent_fpm: descent,
                spacing_meters: 150.0,
                maximum_detour_meters: 20000.0,
            };
            let fixed_speed = compile_once(start, 0.0, &fixes, 90.0, &envelope).unwrap();
            let draft = draft(start, 0.0, &fixes, 90.0, &envelope).unwrap();
            assert_eq!(draft.leg_count(), fixes.len());
            assert!(draft.clone().protect(|_, _| false).is_err());
            assert!(
                draft
                    .protect(|a, b| a.altitude_feet >= 10000.0 && b.altitude_feet >= 10000.0)
                    .is_ok()
            );
            let end = fixed_speed[0].points.last().unwrap();
            let before = fixed_speed[0].points.iter().rev().nth(1).unwrap();
            let required = airspeed
                .ground_knots(
                    end.altitude_feet,
                    bearing(
                        [before.latitude, before.longitude],
                        [end.latitude, end.longitude],
                    ),
                )
                .unwrap();
            assert!(
                (fixed_speed[0].ground_knots - required).abs() > 1.0,
                "Preferred-level speed must reproduce the old IAS mismatch"
            );
            let legs = compile(start, 0.0, &fixes, 90.0, &envelope, |a, b| {
                a.altitude_feet >= 10000.0 && b.altitude_feet >= 10000.0
            })
            .unwrap();
            assert!(
                legs[0].points.last().unwrap().altitude_feet < 17900.0,
                "{indicated}: downstream limit must force earlier descent"
            );
            assert!(
                (legs[0].ground_knots - fixes[0].ground_knots).abs() > 1.0,
                "Fixed preferred-level ground speed must change"
            );
            for leg in &legs {
                let end = leg.points.last().unwrap();
                let before = leg.points.iter().rev().nth(1).unwrap();
                let expected = airspeed
                    .ground_knots(
                        end.altitude_feet,
                        bearing(
                            [before.latitude, before.longitude],
                            [end.latitude, end.longitude],
                        ),
                    )
                    .unwrap();
                assert!(
                    (leg.ground_knots - expected).abs() < 0.011,
                    "{indicated}: IAS changed when the vertical solution changed"
                );
            }
            assert!(compile(start, 0.0, &fixes, 90.0, &envelope, |_, _| false).is_err());
        }
    }
    #[test]
    fn crossing_ceiling_shapes_a_continuous_turning_descent() {
        let start = Waypoint {
            latitude: 44.0,
            longitude: 8.0,
            altitude_feet: 18000.0,
        };
        let fixes = [
            Fix {
                airspeed: None,
                coordinate: [44.2, 8.0],
                arrival_heading: None,
                minimum_feet: 0.0,
                maximum_feet: 18000.0,
                preferred_feet: 18000.0,
                ground_knots: 200.0,
                required_turn: None,
            },
            Fix {
                airspeed: None,
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
