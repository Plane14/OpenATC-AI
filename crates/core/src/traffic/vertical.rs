//! Choose reachable crossing levels before committing a generated flight to its route.

/// A crossing restriction expressed in geometric feet MSL by the caller.
#[derive(Clone, Copy, Debug)]
pub struct Crossing {
    /// Lowest permitted geometric level, feet MSL.
    pub minimum_feet: f64,
    /// Highest permitted geometric level, feet MSL.
    pub maximum_feet: f64,
    /// Desired level before future restrictions are considered.
    pub preferred_feet: f64,
    /// Travel time from the preceding crossing, including the initial position.
    pub seconds: f64,
}

fn intersection(lower: f64, upper: f64, crossing: &Crossing) -> Option<(f64, f64)> {
    if lower <= upper {
        return Some((lower, upper));
    }
    // Forward/backward propagation can invert equal bounds by a few ULPs.
    // Snap only that roundoff and retain the original published altitude band.
    let roundoff = 8.0 * f64::EPSILON * lower.abs().max(upper.abs()).max(1.0);
    if lower - upper > roundoff {
        return None;
    }
    let level = ((lower + upper) * 0.5).clamp(crossing.minimum_feet, crossing.maximum_feet);
    Some((level, level))
}

/// Propagate restrictions in both directions so a later ceiling starts the descent early.
/// Pressure references must be converted before calling this function.
pub fn levels(
    initial_feet: f64,
    crossings: &[Crossing],
    climb_fpm: f64,
    descent_fpm: f64,
) -> Result<Vec<f64>, String> {
    if !initial_feet.is_finite()
        || crossings.is_empty()
        || crossings.len() > 512
        || !(100.0..=10000.0).contains(&climb_fpm)
        || !(100.0..=10000.0).contains(&descent_fpm)
    {
        return Err("Invalid generated vertical profile".into());
    }
    let mut bounds = Vec::with_capacity(crossings.len());
    let mut previous = (initial_feet, initial_feet);
    for (index, crossing) in crossings.iter().enumerate() {
        if ![
            crossing.minimum_feet,
            crossing.maximum_feet,
            crossing.preferred_feet,
            crossing.seconds,
        ]
        .iter()
        .all(|value| value.is_finite())
            || crossing.minimum_feet > crossing.maximum_feet
            || !(0.0..=86400.0).contains(&crossing.seconds)
        {
            return Err(format!("Invalid crossing restriction at {index}"));
        }
        let minutes = crossing.seconds / 60.0;
        let lower = crossing
            .minimum_feet
            .max(previous.0 - descent_fpm * minutes);
        let upper = crossing.maximum_feet.min(previous.1 + climb_fpm * minutes);
        let (lower, upper) = intersection(lower, upper, crossing)
            .ok_or_else(|| format!("Unreachable crossing restriction at {index}"))?;
        bounds.push((lower, upper));
        previous = (lower, upper);
    }
    for index in (0..crossings.len() - 1).rev() {
        let next = bounds[index + 1];
        let minutes = crossings[index + 1].seconds / 60.0;
        bounds[index].0 = bounds[index].0.max(next.0 - climb_fpm * minutes);
        bounds[index].1 = bounds[index].1.min(next.1 + descent_fpm * minutes);
        bounds[index] = intersection(bounds[index].0, bounds[index].1, &crossings[index])
            .ok_or_else(|| format!("Conflicting crossing restrictions at {index}"))?;
    }
    let mut selected = Vec::with_capacity(crossings.len());
    let mut previous = initial_feet;
    for (crossing, (lower, upper)) in crossings.iter().zip(bounds) {
        let minutes = crossing.seconds / 60.0;
        let lower = lower.max(previous - descent_fpm * minutes);
        let upper = upper.min(previous + climb_fpm * minutes);
        let (lower, upper) = intersection(lower, upper, crossing)
            .ok_or("Generated vertical profile lost its reachable envelope")?;
        previous = crossing.preferred_feet.clamp(lower, upper);
        selected.push(previous);
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_crossings_tolerate_roundoff_without_relaxing_the_published_level() {
        let level: f64 = 6000.0;
        let crossing = Crossing {
            minimum_feet: level,
            maximum_feet: level,
            preferred_feet: level,
            seconds: 0.0,
        };
        let adjacent = f64::from_bits(level.to_bits() + 1);
        assert_eq!(
            levels(adjacent, &[crossing], 2000.0, 1000.0).unwrap(),
            [level]
        );
        assert!(levels(level + 0.001, &[crossing], 2000.0, 1000.0).is_err());
        assert!(levels(level - 0.001, &[crossing], 2000.0, 1000.0).is_err());
    }

    #[test]
    fn future_ceiling_moves_descent_before_the_restricted_leg() {
        let route = [
            Crossing {
                minimum_feet: 0.0,
                maximum_feet: 18000.0,
                preferred_feet: 18000.0,
                seconds: 120.0,
            },
            Crossing {
                minimum_feet: 0.0,
                maximum_feet: 18000.0,
                preferred_feet: 18000.0,
                seconds: 120.0,
            },
            Crossing {
                minimum_feet: 0.0,
                maximum_feet: 12000.0,
                preferred_feet: 12000.0,
                seconds: 60.0,
            },
        ];
        assert_eq!(
            levels(18000.0, &route, 1500.0, 2000.0).unwrap(),
            [18000.0, 14000.0, 12000.0]
        );
        let mut impossible = route;
        impossible[1].minimum_feet = 15000.0;
        assert!(levels(18000.0, &impossible, 1500.0, 2000.0).is_err());
    }

    #[test]
    fn climb_limits_and_exact_crossings_are_preserved() {
        let route = [
            Crossing {
                minimum_feet: 2000.0,
                maximum_feet: 3000.0,
                preferred_feet: 5000.0,
                seconds: 120.0,
            },
            Crossing {
                minimum_feet: 4000.0,
                maximum_feet: 4000.0,
                preferred_feet: 9000.0,
                seconds: 120.0,
            },
        ];
        assert_eq!(
            levels(1000.0, &route, 1000.0, 1500.0).unwrap(),
            [3000.0, 4000.0]
        );
        assert!(levels(1000.0, &route, 400.0, 1500.0).is_err());
    }
}
