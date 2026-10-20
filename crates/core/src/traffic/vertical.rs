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
    levels_selected(
        initial_feet,
        crossings,
        climb_fpm,
        descent_fpm,
        |_, lower, upper, preferred| Some(preferred.clamp(lower, upper)),
    )
}

/// Select clearance-compatible levels without relaxing the reachable crossing envelope.
pub fn levels_selected(
    initial_feet: f64,
    crossings: &[Crossing],
    climb_fpm: f64,
    descent_fpm: f64,
    select: impl Fn(usize, f64, f64, f64) -> Option<f64>,
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
        let (lower, upper) = intersection(lower, upper, crossing).ok_or_else(|| {
            format!(
                "Unreachable crossing restriction at {index}: need {:.0}..{:.0} ft, reachable {:.0}..{:.0} ft in {:.1} s",
                crossing.minimum_feet,
                crossing.maximum_feet,
                previous.0 - descent_fpm * minutes,
                previous.1 + climb_fpm * minutes,
                crossing.seconds
            )
        })?;
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
    // Discrete levels constrain neighbouring crossings too. Propagate their actual
    // attainable extrema before choosing one preferred level along the route.
    let snap = |index: usize, lower: f64, upper: f64| -> Result<(f64, f64), String> {
        let first = select(index, lower, upper, lower)
            .filter(|level| level.is_finite() && *level >= lower && *level <= upper);
        let last = select(index, lower, upper, upper)
            .filter(|level| level.is_finite() && *level >= lower && *level <= upper);
        match (first, last) {
            (Some(first), Some(last)) if first <= last => Ok((first, last)),
            _ => Err(format!(
                "No clearance-compatible envelope within crossing {index}: {lower:.3}..{upper:.3}"
            )),
        }
    };
    let mut converged = false;
    for _ in 0..crossings.len() * 2 + 1 {
        let mut changed = false;
        let mut previous = (initial_feet, initial_feet);
        for index in 0..crossings.len() {
            let minutes = crossings[index].seconds / 60.0;
            let (lower, upper) = intersection(
                bounds[index].0.max(previous.0 - descent_fpm * minutes),
                bounds[index].1.min(previous.1 + climb_fpm * minutes),
                &crossings[index],
            )
            .ok_or_else(|| format!("Unreachable discrete crossing {index}"))?;
            let next = snap(index, lower, upper)?;
            changed |=
                (next.0 - bounds[index].0).abs() > 1e-6 || (next.1 - bounds[index].1).abs() > 1e-6;
            bounds[index] = next;
            previous = next;
        }
        for index in (0..crossings.len() - 1).rev() {
            let next = bounds[index + 1];
            let minutes = crossings[index + 1].seconds / 60.0;
            let (lower, upper) = intersection(
                bounds[index].0.max(next.0 - climb_fpm * minutes),
                bounds[index].1.min(next.1 + descent_fpm * minutes),
                &crossings[index],
            )
            .ok_or_else(|| format!("Conflicting discrete crossing {index}"))?;
            let next = snap(index, lower, upper)?;
            changed |=
                (next.0 - bounds[index].0).abs() > 1e-6 || (next.1 - bounds[index].1).abs() > 1e-6;
            bounds[index] = next;
        }
        if !changed {
            converged = true;
            break;
        }
    }
    if !converged {
        return Err("Clearance-level envelope did not converge".into());
    }
    let mut selected = Vec::with_capacity(crossings.len());
    let mut previous = initial_feet;
    for (index, (crossing, (lower, upper))) in crossings.iter().zip(bounds).enumerate() {
        let minutes = crossing.seconds / 60.0;
        let lower = lower.max(previous - descent_fpm * minutes);
        let upper = upper.min(previous + climb_fpm * minutes);
        let (lower, upper) = intersection(lower, upper, crossing)
            .ok_or("Generated vertical profile lost its reachable envelope")?;
        previous = select(index, lower, upper, crossing.preferred_feet)
            .filter(|level| level.is_finite() && *level >= lower && *level <= upper)
            .ok_or_else(|| format!("No clearance-compatible level within crossing {index}"))?;
        selected.push(previous);
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_future_envelopes_start_descent_before_a_continuous_gap() {
        let crossings = [
            Crossing {
                minimum_feet: 10900.0,
                maximum_feet: 11100.0,
                preferred_feet: 11100.0,
                seconds: 60.0,
            },
            Crossing {
                minimum_feet: 10900.0,
                maximum_feet: 10960.0,
                preferred_feet: 10960.0,
                seconds: 60.0,
            },
        ];
        let grid = |_: usize, lower: f64, upper: f64, preferred: f64| {
            let low = (lower / 100.0).ceil();
            let high = (upper / 100.0).floor();
            (low <= high).then(|| (preferred / 100.0).round().clamp(low, high) * 100.0)
        };
        // The continuous solution fits 11060 then 10960; rounding only the first
        // target would leave no reachable standard level at the second fix.
        assert_eq!(
            levels_selected(10900.0, &crossings, 200.0, 100.0, grid).unwrap(),
            [11000.0, 10900.0]
        );
        let constrained = [
            crossings[0],
            Crossing {
                seconds: 30.0,
                ..crossings[1]
            },
        ];
        assert_eq!(
            levels_selected(10900.0, &constrained, 200.0, 100.0, grid).unwrap(),
            [10900.0, 10900.0]
        );
    }

    #[test]
    fn selected_levels_remain_inside_future_restrictions_and_reachable_rates() {
        let crossings = [
            Crossing {
                minimum_feet: 0.0,
                maximum_feet: 2000.0,
                preferred_feet: 1450.0,
                seconds: 120.0,
            },
            Crossing {
                minimum_feet: 500.0,
                maximum_feet: 500.0,
                preferred_feet: 500.0,
                seconds: 60.0,
            },
        ];
        let grid = |_: usize, lower: f64, upper: f64, preferred: f64| {
            let chosen = (preferred.clamp(lower, upper) / 100.0).floor() * 100.0;
            (chosen >= lower && chosen <= upper).then_some(chosen)
        };
        assert_eq!(
            levels_selected(0.0, &crossings, 1000.0, 1000.0, grid).unwrap(),
            [1400.0, 500.0]
        );
        assert_eq!(
            levels(0.0, &crossings, 1000.0, 1000.0).unwrap(),
            [1450.0, 500.0]
        );
        assert!(
            levels_selected(0.0, &crossings, 1000.0, 1000.0, |_, _, upper, _| Some(
                upper + 1.0
            ))
            .is_err()
        );
        let narrow = [Crossing {
            minimum_feet: 1550.0,
            maximum_feet: 1590.0,
            preferred_feet: 1570.0,
            seconds: 120.0,
        }];
        assert!(levels_selected(0.0, &narrow, 1000.0, 1000.0, grid).is_err());
    }

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
