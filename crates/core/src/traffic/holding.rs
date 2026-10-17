//! A level racetrack already aligned at its fix; entry and exit still need clearances.
use super::motion::{Segment, Stage, Waypoint};
pub mod published;

/// Controller-approved values after applying the regional level and speed rules.
pub struct Clearance {
    /// Indicated/standard-pressure level used to check the published restriction.
    pub level_feet: i32,
    /// Geometric altitude used by the position renderer.
    pub geometric_feet: f64,
    /// Indicated airspeed for the published holding limit.
    pub indicated_knots: f64,
    /// Planned ground speed after the airspeed/wind conversion.
    pub ground_knots: f64,
    /// Planned bank within the aircraft envelope.
    pub bank_degrees: f64,
    /// Regional entry/holding turn-rate limit, degrees per second.
    pub turn_rate_degrees_sec: f64,
}

/// Fully specified hold after resolving the published magnetic reference and vertical limits.
#[derive(Clone)]
pub struct Pattern {
    /// Fix coordinate and cleared geometric drawing altitude.
    pub fix: Waypoint,
    /// Inbound track, degrees true.
    pub inbound_degrees: f64,
    /// Right-hand turns when true.
    pub right: bool,
    /// Planned ground speed; the caller must respect the applicable IAS limit and wind.
    pub ground_knots: f64,
    /// Straight leg duration for this nominal ground-speed profile, seconds.
    pub leg_seconds: f64,
    /// Nominal bank used to choose the turn radius.
    pub bank_degrees: f64,
    /// Rate cap applied together with the aircraft's bank limit.
    pub turn_rate_degrees_sec: f64,
}

/// Holding geometry awaiting its complete terrain/airspace check.
#[derive(Clone)]
pub struct Draft {
    segment: Segment,
}
impl Draft {
    /// No movement segment is returned if any part of the racetrack is unprotected.
    pub fn protect(
        self,
        protected: impl Fn(Waypoint, Waypoint) -> bool,
    ) -> Result<Segment, String> {
        if self
            .segment
            .points
            .windows(2)
            .any(|pair| !protected(pair[0], pair[1]))
        {
            return Err("Holding path lacks verified terrain/airspace protection".into());
        }
        Ok(self.segment)
    }
}

impl Pattern {
    /// Decode an explicit published HM leg; this does not approve entry or resolve an unspecified variation.
    pub fn published(
        leg: &super::procedures::Leg,
        position: super::coordinates::Position,
        clearance: &Clearance,
    ) -> Result<Self, String> {
        if leg.terminator != "HM" || !matches!(leg.turn.as_str(), "L" | "R") {
            return Err("Published hold needs an explicit HM leg and turn direction".into());
        }
        let (minimum, maximum) = leg.altitude_limits()?;
        if minimum.is_some_and(|level| clearance.level_feet < level)
            || maximum.is_some_and(|level| clearance.level_feet > level)
            || !(40.0..=300.0).contains(&clearance.indicated_knots)
            || !(-2000..=60000).contains(&clearance.level_feet)
            || !matches!(leg.speed_description.as_str(), "" | "-")
            || leg
                .speed_knots
                .is_some_and(|limit| clearance.indicated_knots > limit)
        {
            return Err(
                "Holding clearance conflicts with a published level or speed restriction".into(),
            );
        }
        let course = leg.course.ok_or("Published holding course is missing")?;
        let variation = position
            .magnetic_variation_degrees
            .ok_or("Published holding magnetic reference is unresolved")?;
        let leg_seconds = if let Some(time) = leg.distance_time.strip_prefix('T') {
            time.parse::<f64>()
                .map_err(|_| "Invalid published holding time")?
                * 6.0
        } else {
            let distance = leg
                .distance_time
                .parse::<f64>()
                .map_err(|_| "Published holding distance is missing")?
                / 10.0;
            distance / clearance.ground_knots * 3600.0
        };
        Ok(Self {
            fix: Waypoint {
                latitude: position.coordinate[0],
                longitude: position.coordinate[1],
                altitude_feet: clearance.geometric_feet,
            },
            inbound_degrees: (course + variation).rem_euclid(360.0),
            right: leg.turn == "R",
            ground_knots: clearance.ground_knots,
            leg_seconds,
            bank_degrees: clearance.bank_degrees,
            turn_rate_degrees_sec: clearance.turn_rate_degrees_sec,
        })
    }
    /// Build a closed path only after every sampled segment passes the terrain/airspace check.
    pub fn compile(
        &self,
        protected: impl Fn(Waypoint, Waypoint) -> bool,
    ) -> Result<Segment, String> {
        self.draft()?.protect(protected)
    }
    /// Prepare bounded geometry without releasing it as a movement segment.
    pub fn draft(&self) -> Result<Draft, String> {
        if !(-90.0..=90.0).contains(&self.fix.latitude)
            || !(-180.0..=180.0).contains(&self.fix.longitude)
            || !(-2000.0..=60000.0).contains(&self.fix.altitude_feet)
            || !(0.0..360.0).contains(&self.inbound_degrees)
            || !(40.0..=300.0).contains(&self.ground_knots)
            || !(30.0..=180.0).contains(&self.leg_seconds)
            || !(5.0..=30.0).contains(&self.bank_degrees)
            || !(0.5..=3.0).contains(&self.turn_rate_degrees_sec)
        {
            return Err("Invalid holding speed, level or geometry".into());
        }
        let speed = self.ground_knots * 0.514444;
        let radius = (speed.powi(2) / (9.80665 * self.bank_degrees.to_radians().tan()))
            .max(speed / self.turn_rate_degrees_sec.to_radians());
        let heading = self.inbound_degrees.to_radians();
        let u = [heading.sin(), heading.cos()];
        let normal = if self.right {
            [u[1], -u[0]]
        } else {
            [-u[1], u[0]]
        };
        let sign = if self.right { -1.0 } else { 1.0 };
        let length = speed * self.leg_seconds;
        let turn_samples = (std::f64::consts::PI * radius / speed).ceil().max(90.0) as usize;
        let mut local = vec![[0.0, 0.0]];
        for turn in 0..2 {
            let start = if turn == 0 {
                [0.0, 0.0]
            } else {
                [
                    normal[0] * 2.0 * radius - u[0] * length,
                    normal[1] * 2.0 * radius - u[1] * length,
                ]
            };
            let direction = if turn == 0 { 1.0 } else { -1.0 };
            let centre = [
                start[0] + normal[0] * radius * direction,
                start[1] + normal[1] * radius * direction,
            ];
            let initial = (start[1] - centre[1]).atan2(start[0] - centre[0]);
            for index in 1..=turn_samples {
                let angle =
                    initial + sign * std::f64::consts::PI * index as f64 / turn_samples as f64;
                local.push([
                    centre[0] + radius * angle.cos(),
                    centre[1] + radius * angle.sin(),
                ]);
            }
            let from = *local.last().unwrap();
            let end = if turn == 0 {
                [
                    normal[0] * 2.0 * radius - u[0] * length,
                    normal[1] * 2.0 * radius - u[1] * length,
                ]
            } else {
                [0.0, 0.0]
            };
            let steps = (length / 500.0).ceil().max(1.0) as usize;
            for index in 1..=steps {
                let amount = index as f64 / steps as f64;
                local.push([
                    from[0] + (end[0] - from[0]) * amount,
                    from[1] + (end[1] - from[1]) * amount,
                ]);
            }
        }
        if local.len() > 4096 {
            return Err("Holding geometry exceeds its sample budget".into());
        }
        let mut points: Vec<_> = local
            .into_iter()
            .map(|point| {
                let coordinate = crate::arrival::destination(
                    [self.fix.latitude, self.fix.longitude],
                    point[0].atan2(point[1]).to_degrees().rem_euclid(360.0),
                    point[0].hypot(point[1]) / 1852.0,
                );
                Waypoint {
                    latitude: coordinate[0],
                    longitude: coordinate[1],
                    altitude_feet: self.fix.altitude_feet,
                }
            })
            .collect();
        points[0] = self.fix;
        *points.last_mut().unwrap() = self.fix;
        Ok(Draft {
            segment: Segment {
                stage: Stage::Holding,
                points,
                knots: self.ground_knots,
                resources: vec![],
                dwell_seconds: 0.0,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_aircraft_hold_respects_standard_rate_instead_of_full_bank() {
        let pattern = Pattern {
            fix: Waypoint {
                latitude: 45.0,
                longitude: 9.0,
                altitude_feet: 4000.0,
            },
            inbound_degrees: 0.0,
            right: true,
            ground_knots: 80.0,
            leg_seconds: 60.0,
            bank_degrees: 25.0,
            turn_rate_degrees_sec: 3.0,
        };
        let hold = pattern.compile(|_, _| true).unwrap();
        assert!(pattern.draft().unwrap().protect(|_, _| false).is_err());

        let first_turn_seconds: f64 = hold.points[..=90]
            .windows(2)
            .map(|pair| {
                crate::arrival::distance_nm(
                    [pair[0].latitude, pair[0].longitude],
                    [pair[1].latitude, pair[1].longitude],
                ) / pattern.ground_knots
                    * 3600.0
            })
            .sum();
        assert!((first_turn_seconds - 60.0).abs() < 0.02);
        assert!(
            Pattern {
                turn_rate_degrees_sec: 3.1,
                ..pattern
            }
            .compile(|_, _| true)
            .is_err()
        );
    }
    #[test]
    fn level_racetrack_keeps_turns_continuous_and_rejects_unprotected_paths() {
        let pattern = Pattern {
            fix: Waypoint {
                latitude: 45.645972,
                longitude: 9.021583,
                altitude_feet: 6000.0,
            },
            inbound_degrees: 136.0,
            right: true,
            ground_knots: 180.0,
            leg_seconds: 60.0,
            bank_degrees: 25.0,
            turn_rate_degrees_sec: 3.0,
        };
        let hold = pattern.compile(|_, _| true).unwrap();
        assert_eq!(hold.points[0], *hold.points.last().unwrap());
        let first = crate::arrival::bearing(
            [hold.points[0].latitude, hold.points[0].longitude],
            [hold.points[1].latitude, hold.points[1].longitude],
        );
        assert!(crate::navigation::turn(pattern.inbound_degrees, first).abs() < 3.0);
        let last = hold.points.len() - 1;
        let inbound = crate::arrival::bearing(
            [
                hold.points[last - 1].latitude,
                hold.points[last - 1].longitude,
            ],
            [hold.points[last].latitude, hold.points[last].longitude],
        );
        assert!(crate::navigation::turn(pattern.inbound_degrees, inbound).abs() < 1.0);
        assert!(super::super::motion::Route::compile(vec![hold], 1000.0, 1000.0).is_ok());
        assert!(pattern.compile(|_, _| false).is_err());
    }
}
