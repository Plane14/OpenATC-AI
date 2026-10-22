//! Heading-constrained connectors, prepared once and checked before movement is granted.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
use crate::state::Point;
use dubins_paths::{
    PathType, SegmentType,
    f64::{DubinsPath, PosRot},
};

/// Local coordinates and a compass heading in degrees true.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    /// Airport/route-local point; the caller owns its geographic projection.
    pub point: Point,
    /// Clockwise from north.
    pub heading: f64,
}
/// Required first turn where a procedure specifies its direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// Begin with a left turn.
    Left,
    /// Begin with a right turn.
    Right,
}
/// Bounds supplied by the aircraft envelope and the planning policy.
pub struct Limits {
    /// Radius within the steering or bank envelope, meters.
    pub radius: f64,
    /// Maximum distance between collision/terrain samples, meters.
    pub spacing: f64,
    /// Reject excessive detours rather than adding unplanned orbits.
    pub length: f64,
    /// Optional published direction for the first turn.
    pub initial_turn: Option<Turn>,
}
/// Analytic heading retained for footprint checks and continuity at the endpoints.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    /// Local position and interpolated height.
    pub point: Point,
    /// Compass heading, degrees true.
    pub heading: f64,
}

/// Choose the shortest permitted curve whose complete sampled path is protected.
/// # Panics
///
/// Panics if the connection limits produce an invalid rotation.
pub fn connect(
    from: Pose,
    to: Pose,
    limits: &Limits,
    protected: impl Fn(Sample, Sample) -> bool,
) -> Result<Vec<Sample>, String> {
    let valid = |pose: Pose| {
        [
            pose.point.east,
            pose.point.north,
            pose.point.height,
            pose.heading,
        ]
        .iter()
        .all(|value| value.is_finite())
            && pose.point.east.abs() <= 10_000_000.0
            && pose.point.north.abs() <= 10_000_000.0
            && (-2000.0..=20000.0).contains(&pose.point.height)
    };
    if !valid(from)
        || !valid(to)
        || !(1.0..=25000.0).contains(&limits.radius)
        || !(0.25..=250.0).contains(&limits.spacing)
        || !(1.0..=300_000.0).contains(&limits.length)
    {
        return Err("Invalid heading-constrained connection".into());
    }
    let rotation = |heading: f64| {
        (90.0 - heading)
            .to_radians()
            .rem_euclid(std::f64::consts::TAU)
    };
    let start = PosRot::from_floats(0.0, 0.0, rotation(from.heading));
    let end = PosRot::from_floats(
        to.point.east - from.point.east,
        to.point.north - from.point.north,
        rotation(to.heading),
    );
    let mut candidates: Vec<_> = PathType::ALL
        .into_iter()
        .filter(|kind| match limits.initial_turn {
            None => true,
            Some(Turn::Left) => kind.to_segment_types()[0] == SegmentType::L,
            Some(Turn::Right) => kind.to_segment_types()[0] == SegmentType::R,
        })
        .filter_map(|kind| DubinsPath::new(start, end, limits.radius, kind).ok())
        .filter(|path| {
            path.length().is_finite() && path.length() >= 0.01 && path.length() <= limits.length
        })
        .collect();
    candidates.sort_by(|first, second| first.length().total_cmp(&second.length()));
    for path in candidates {
        let length = path.length();
        let types = path.path_type.to_segment_types();
        let steps = std::array::from_fn::<_, 3, _>(|index| {
            let spacing = if types[index] == SegmentType::S {
                limits.spacing
            } else {
                limits.spacing.min(limits.radius * 2.0_f64.to_radians())
            };
            (path.segment_length(index) / spacing).ceil() as usize
        });
        if steps.iter().sum::<usize>() > 8191 {
            continue;
        }
        // Straight portions need the distance bound, while turns also need the
        // angular bound. A long straight must not inherit the turn's tiny step.
        let mut distances = vec![0.0];
        let mut offset = 0.0;
        for (index, steps) in steps.into_iter().enumerate() {
            let part = path.segment_length(index);
            for step in 1..=steps {
                let distance = offset + part * step as f64 / steps as f64;
                if distance - distances.last().unwrap() >= 0.025 {
                    distances.push(distance);
                }
            }
            offset += part;
        }
        if length - distances.last().unwrap() >= 0.025 || distances.len() == 1 {
            distances.push(length);
        } else {
            *distances.last_mut().unwrap() = length;
        }
        let endpoint = path.endpoint();
        if (endpoint.x() - end.x()).hypot(endpoint.y() - end.y()) > 0.05
            || crate::navigation::turn(90.0 - endpoint.rot().to_degrees(), to.heading).abs() > 0.01
        {
            continue;
        }
        let mut samples = Vec::with_capacity(distances.len());
        for distance in distances {
            let fraction = distance / length;
            let pose = path.sample((length * fraction).min((length - 1e-8).max(0.0)));
            samples.push(Sample {
                point: Point {
                    east: from.point.east + pose.x(),
                    north: from.point.north + pose.y(),
                    height: from.point.height + (to.point.height - from.point.height) * fraction,
                },
                heading: (90.0 - pose.rot().to_degrees()).rem_euclid(360.0),
            });
        }
        samples[0] = Sample {
            point: from.point,
            heading: from.heading.rem_euclid(360.0),
        };
        *samples.last_mut().unwrap() = Sample {
            point: to.point,
            heading: to.heading.rem_euclid(360.0),
        };
        if samples.windows(2).all(|pair| protected(pair[0], pair[1])) {
            return Ok(samples);
        }
    }
    Err("No protected connection fits the heading and turn envelope".into())
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    #[test]
    fn long_straights_do_not_consume_the_turn_sampling_budget() {
        let from = Pose {
            point: Point::default(),
            heading: 0.0,
        };
        let to = Pose {
            point: Point {
                north: 100_000.0,
                ..Default::default()
            },
            heading: 0.0,
        };
        let samples = connect(
            from,
            to,
            &Limits {
                radius: 1000.0,
                spacing: 250.0,
                length: 100_100.0,
                initial_turn: None,
            },
            |_, _| true,
        )
        .unwrap();
        assert!(samples.len() <= 402);
        assert_eq!(samples.first().unwrap().point, from.point);
        assert_eq!(samples.last().unwrap().point, to.point);
        assert!(samples.windows(2).all(|pair| {
            (pair[1].point.east - pair[0].point.east)
                .hypot(pair[1].point.north - pair[0].point.north)
                <= 250.001
                && crate::navigation::turn(pair[0].heading, pair[1].heading).abs() <= 2.01
        }));
        let turned = Pose {
            point: Point {
                east: 100_000.0,
                north: 100_000.0,
                height: 0.0,
            },
            heading: 90.0,
        };
        let samples = connect(
            from,
            turned,
            &Limits {
                radius: 1000.0,
                spacing: 250.0,
                length: 150_000.0,
                initial_turn: None,
            },
            |_, _| true,
        )
        .unwrap();
        assert!(samples.len() < 1000);
        assert!(samples.windows(2).all(|pair| {
            (pair[1].point.east - pair[0].point.east)
                .hypot(pair[1].point.north - pair[0].point.north)
                <= 250.001
                && crate::navigation::turn(pair[0].heading, pair[1].heading).abs() <= 2.01
        }));
    }

    #[test]
    fn reverse_connector_preserves_the_nose_direction_when_taxi_starts() {
        let from = Pose {
            point: Point {
                east: 0.0,
                north: 0.0,
                height: 0.0,
            },
            heading: 180.0,
        };
        let to = Pose {
            point: Point {
                east: -20.0,
                north: -60.0,
                height: 0.0,
            },
            heading: 270.0,
        };
        let limits = Limits {
            radius: 20.0,
            spacing: 5.0,
            length: 150.0,
            initial_turn: None,
        };
        let curve = connect(from, to, &limits, |first, second| {
            [first, second].iter().all(|sample| {
                (-50.0..=10.0).contains(&sample.point.east)
                    && (-90.0..=1.0).contains(&sample.point.north)
            })
        })
        .unwrap();
        assert_eq!(curve[0].point, from.point);
        assert_eq!(curve.last().unwrap().point, to.point);
        assert_eq!(
            (curve.last().unwrap().heading + 180.0).rem_euclid(360.0),
            90.0
        );
        for pair in curve.windows(2) {
            assert!(crate::navigation::turn(pair[0].heading, pair[1].heading).abs() <= 2.01);
            assert!(
                (pair[0].point.east - pair[1].point.east)
                    .hypot(pair[0].point.north - pair[1].point.north)
                    <= 5.01
            );
        }
        assert!(connect(from, to, &limits, |_, _| false).is_err());
        assert!(
            connect(
                from,
                to,
                &Limits {
                    length: 30.0,
                    ..limits
                },
                |_, _| true
            )
            .is_err()
        );
    }
}
