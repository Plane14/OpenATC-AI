//! A nominal runway slope from published threshold elevations, independent of airport maximum height.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
#[allow(unused_imports, clippy::wildcard_imports)]
use super::*;
use super::{Airport, Point, animation, position};
use openatc_core::{
    arrival::Threshold,
    traffic::motion::{Segment, Stage, Waypoint},
};

pub(super) struct Surface {
    airport: String,
    pub runway: String,
    pub resource: String,
    start: Point,
    unit: [f64; 2],
    pub length: f64,
    pub threshold: f64,
    threshold_feet: f64,
    slope: f64,
}
impl Surface {
    pub fn new(
        airport: &Airport,
        name: &str,
        thresholds: &[Threshold],
        maximum_slope_percent: f64,
    ) -> Result<Self, String> {
        let runway = airport
            .runways
            .iter()
            .find(|r| r.first_name == name || r.second_name == name)
            .ok_or("Runway surface needs a known runway end")?;
        let elevation = |name: &str| -> Result<f64, String> {
            let heights: Vec<_> = thresholds
                .iter()
                .filter(|threshold| threshold.runway == name)
                .map(|threshold| threshold.elevation_feet)
                .collect();
            let first = *heights
                .first()
                .ok_or("Published runway threshold elevation is missing")?;
            if heights.iter().any(|height| {
                !height.is_finite()
                    || !(-2000.0..=20000.0).contains(height)
                    || (*height - first).abs() > 0.01
            }) {
                return Err("Published runway threshold elevations conflict or are invalid".into());
            }
            Ok(first)
        };
        let (start, end, near, far, threshold, opposite_displacement) = if runway.first_name == name
        {
            (
                runway.first,
                runway.second,
                elevation(&runway.first_name)?,
                elevation(&runway.second_name)?,
                runway.first_displaced_meters,
                runway.second_displaced_meters,
            )
        } else {
            (
                runway.second,
                runway.first,
                elevation(&runway.second_name)?,
                elevation(&runway.first_name)?,
                runway.second_displaced_meters,
                runway.first_displaced_meters,
            )
        };
        let length = (end.east - start.east).hypot(end.north - start.north);
        let span = length - threshold - opposite_displacement;
        if !length.is_finite()
            || length < 100.0
            || !threshold.is_finite()
            || !opposite_displacement.is_finite()
            || threshold < 0.0
            || opposite_displacement < 0.0
            || span < 100.0
            || !(0.1..=10.0).contains(&maximum_slope_percent)
        {
            return Err("Runway surface has invalid threshold geometry".into());
        }
        let slope = (far - near) / span;
        if slope.abs() * 0.3048 * 100.0 > maximum_slope_percent {
            return Err("Runway slope exceeds the configured nominal envelope".into());
        }
        Ok(Self {
            airport: airport.icao.clone(),
            runway: name.into(),
            resource: format!(
                "RUNWAY/{}/{}/{}",
                airport.icao, runway.first_name, runway.second_name
            ),
            start,
            unit: [
                (end.east - start.east) / length,
                (end.north - start.north) / length,
            ],
            length,
            threshold,
            threshold_feet: near,
            slope,
        })
    }
    pub fn matches(&self, airport: &Airport, runway: &str) -> bool {
        self.airport == airport.icao && self.runway == runway
    }
    pub fn along(&self, point: Point) -> f64 {
        (point.east - self.start.east) * self.unit[0]
            + (point.north - self.start.north) * self.unit[1]
    }
    pub fn point(&self, along: f64) -> Point {
        Point {
            east: self.start.east + self.unit[0] * along,
            north: self.start.north + self.unit[1] * along,
            height: 0.0,
        }
    }
    /// Outside the runway, retain its projected height as a nominal exit datum until terrain placement.
    pub fn height(&self, point: Point) -> f64 {
        self.threshold_feet
            + self.slope * (self.along(point).clamp(0.0, self.length) - self.threshold)
    }
    pub fn heading(&self) -> f64 {
        self.unit[0]
            .atan2(self.unit[1])
            .to_degrees()
            .rem_euclid(360.0)
    }

    pub fn flare(
        &self,
        airport: &Airport,
        crossing: Waypoint,
        angle_degrees: f64,
        ground_knots: f64,
        handling: &animation::Handling,
        spacing: f64,
    ) -> Result<Segment, String> {
        if !self.matches(airport, &self.runway)
            || !(-8.0..=-1.0).contains(&angle_degrees)
            || !(35.0..=230.0).contains(&ground_knots)
            || !(1.0..=10.0).contains(&spacing)
        {
            return Err("Flare needs a published descent angle, speed and runway surface".into());
        }
        let local =
            openatc_core::airport::airport_point(airport, crossing.latitude, crossing.longitude);
        let along = self.along(local);
        let across = (local.east - self.start.east) * self.unit[1]
            - (local.north - self.start.north) * self.unit[0];
        let height = crossing.altitude_feet - self.height(local);
        if !height.is_finite()
            || (along - self.threshold).abs() > 2.0
            || across.abs() > 2.0
            || !(handling.flare_height_feet..=200.0).contains(&height)
        {
            return Err("Approach endpoint is not a usable threshold crossing".into());
        }
        let incoming = (-angle_degrees).to_radians().tan() / 0.3048 + self.slope;
        let contact = handling.touchdown_sink_fpm / (ground_knots * 0.514_444 * 60.0);
        if !incoming.is_finite() || incoming <= contact || contact <= 0.0 {
            return Err("Flare cannot reduce the planned sink rate".into());
        }
        let straight = (height - handling.flare_height_feet) / incoming;
        let roundout = 2.0 * handling.flare_height_feet / (incoming + contact);
        let distance = straight + roundout;
        if along + distance >= self.length {
            return Err("Flare would touch down beyond the runway".into());
        }
        let count = (distance / spacing).ceil() as usize;
        if count == 0 || count > 1023 {
            return Err("Flare exceeds the geometry budget".into());
        }
        let mut points = vec![crossing];
        for index in 1..=count {
            let x = distance * index as f64 / count as f64;
            let relative = if x <= straight {
                height - incoming * x
            } else {
                let t = x - straight;
                handling.flare_height_feet - incoming * t
                    + (incoming - contact) * t * t / (2.0 * roundout)
            };
            let point = self.point(along + x);
            let coordinate = position(airport, point);
            points.push(Waypoint {
                latitude: coordinate[0],
                longitude: coordinate[1],
                altitude_feet: self.height(point)
                    + if index == count {
                        0.0
                    } else {
                        relative.max(0.0)
                    },
            });
        }
        Ok(Segment {
            stage: Stage::Approach,
            points,
            knots: ground_knots,
            resources: vec![self.resource.clone()],
            dwell_seconds: 0.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Airport, Vec<Threshold>) {
        (
            Airport {
                icao: "TEST".into(),
                elevation_feet: 900.0,
                runways: vec![openatc_core::airport::Runway {
                    first_name: "09".into(),
                    second_name: "27".into(),
                    width: 45.0,
                    first: Point::default(),
                    second: Point {
                        north: 4000.0,
                        ..Default::default()
                    },
                    first_displaced_meters: 100.0,
                    second_displaced_meters: 200.0,
                }],
                ..Default::default()
            },
            vec![
                Threshold {
                    runway: "09".into(),
                    elevation_feet: 691.0,
                },
                Threshold {
                    runway: "27".into(),
                    elevation_feet: 745.0,
                },
            ],
        )
    }
    #[test]
    fn reciprocal_thresholds_define_one_surface_and_conflicting_data_rejects() {
        let (airport, mut heights) = fixture();
        let first = Surface::new(&airport, "09", &heights, 2.0).unwrap();
        let reverse = Surface::new(&airport, "27", &heights, 2.0).unwrap();
        assert!((first.height(first.point(100.0)) - 691.0).abs() < 1e-8);
        assert!((first.height(first.point(3800.0)) - 745.0).abs() < 1e-8);
        for along in [0.0, 100.0, 1000.0, 3800.0, 4000.0] {
            let point = first.point(along);
            assert!((first.height(point) - reverse.height(point)).abs() < 1e-8);
        }
        assert!(Surface::new(&airport, "09", &heights, 0.1).is_err());
        heights.push(Threshold {
            runway: "09".into(),
            elevation_feet: 700.0,
        });
        assert!(Surface::new(&airport, "09", &heights, 2.0).is_err());
        assert!(Surface::new(&airport, "09", &heights[..1], 2.0).is_err());
    }
    #[test]
    fn aircraft_roundouts_reach_the_sloped_surface_without_an_altitude_snap() {
        let (airport, heights) = fixture();
        let surface = Surface::new(&airport, "09", &heights, 2.0).unwrap();
        let point = surface.point(surface.threshold);
        let coordinate = position(&airport, point);
        let crossing = Waypoint {
            latitude: coordinate[0],
            longitude: coordinate[1],
            altitude_feet: surface.height(point) + 57.0,
        };
        let handling = animation::builtin().unwrap();
        let mut distances = Vec::new();
        for (kind, knots) in [
            (animation::Kind::Piston, 65.0),
            (animation::Kind::Turboprop, 110.0),
            (animation::Kind::RegionalJet, 130.0),
            (animation::Kind::Jet, 140.0),
            (animation::Kind::HeavyJet, 155.0),
        ] {
            let rates = &handling[&kind];
            let flare = surface
                .flare(&airport, crossing, -3.0, knots, rates, 5.0)
                .unwrap();
            assert_eq!(flare.points[0], crossing);
            assert!(!flare.stage.on_ground());
            assert_eq!(flare.resources, vec![surface.resource.clone()]);
            let relative = |waypoint: &Waypoint| {
                waypoint.altitude_feet
                    - surface.height(openatc_core::airport::airport_point(
                        &airport,
                        waypoint.latitude,
                        waypoint.longitude,
                    ))
            };
            assert!(
                flare
                    .points
                    .windows(2)
                    .all(|pair| relative(&pair[0]) > relative(&pair[1])
                        && relative(&pair[1]) >= -1e-8)
            );
            let last = flare.points.last().unwrap();
            assert!(relative(last).abs() < 1e-8);
            let previous = &flare.points[flare.points.len() - 2];
            let seconds = openatc_core::arrival::distance_nm(
                [previous.latitude, previous.longitude],
                [last.latitude, last.longitude],
            ) / knots
                * 3600.0;
            let sink = (relative(previous) - relative(last)) * 60.0 / seconds;
            assert!(
                (sink - rates.touchdown_sink_fpm).abs() < 25.0,
                "Unexpected contact rate {sink} for {kind:?}"
            );
            distances.push(surface.along(openatc_core::airport::airport_point(
                &airport,
                last.latitude,
                last.longitude,
            )));
        }
        assert!(
            distances
                .windows(2)
                .all(|pair| (pair[0] - pair[1]).abs() > 1.0)
        );
        let mut high = crossing;
        high.altitude_feet += 300.0;
        assert!(
            surface
                .flare(
                    &airport,
                    high,
                    -3.0,
                    140.0,
                    &handling[&animation::Kind::Jet],
                    5.0
                )
                .is_err()
        );
        let mut off = crossing;
        off.longitude += 0.001;
        assert!(
            surface
                .flare(
                    &airport,
                    off,
                    -3.0,
                    140.0,
                    &handling[&animation::Kind::Jet],
                    5.0
                )
                .is_err()
        );
    }
}
