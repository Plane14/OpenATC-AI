//! Choose reachable crossing levels before committing a generated flight to its route.

/// A crossing restriction expressed in geometric feet MSL by the caller.
#[derive(Clone, Copy, Debug)]
pub struct Crossing {
    pub minimum_feet: f64,
    pub maximum_feet: f64,
    pub preferred_feet: f64,
    /// Travel time from the preceding crossing, including the initial position.
    pub seconds: f64,
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
        if lower > upper {
            return Err(format!("Unreachable crossing restriction at {index}"));
        }
        bounds.push((lower, upper));
        previous = (lower, upper);
    }
    for index in (0..crossings.len() - 1).rev() {
        let next = bounds[index + 1];
        let minutes = crossings[index + 1].seconds / 60.0;
        bounds[index].0 = bounds[index].0.max(next.0 - climb_fpm * minutes);
        bounds[index].1 = bounds[index].1.min(next.1 + descent_fpm * minutes);
        if bounds[index].0 > bounds[index].1 {
            return Err(format!("Conflicting crossing restrictions at {index}"));
        }
    }
    let mut selected = Vec::with_capacity(crossings.len());
    let mut previous = initial_feet;
    for (crossing, (lower, upper)) in crossings.iter().zip(bounds) {
        let minutes = crossing.seconds / 60.0;
        let lower = lower.max(previous - descent_fpm * minutes);
        let upper = upper.min(previous + climb_fpm * minutes);
        if lower > upper {
            return Err("Generated vertical profile lost its reachable envelope".into());
        }
        previous = crossing.preferred_feet.clamp(lower, upper);
        selected.push(previous);
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

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
