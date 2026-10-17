//! Continuous relative-motion checks between consecutive route predictions.
use super::motion::Waypoint;

/// Check horizontal separation only during the interval with insufficient vertical separation.
/// The caller supplies the applicable minima and a sufficiently short prediction interval.
pub fn breach(
    a: [Waypoint; 2],
    b: [Waypoint; 2],
    horizontal_nm: f64,
    vertical_feet: f64,
) -> Result<bool, String> {
    if !(0.1..=1000.0).contains(&horizontal_nm)
        || !(100.0..=1000000.0).contains(&vertical_feet)
        || a.iter().chain(&b).any(|p| {
            !p.latitude.is_finite()
                || p.latitude.abs() >= 85.0
                || !p.longitude.is_finite()
                || p.longitude.abs() > 180.0
                || !p.altitude_feet.is_finite()
        })
    {
        return Err("Invalid airborne separation coordinates or minima".into());
    }
    let z = b[0].altitude_feet - a[0].altitude_feet;
    let dz = (b[1].altitude_feet - a[1].altitude_feet) - z;
    let (low, high) = if dz.abs() < 1e-10 {
        if z.abs() >= vertical_feet {
            return Ok(false);
        }
        (0.0, 1.0)
    } else {
        let first = (-vertical_feet - z) / dz;
        let last = (vertical_feet - z) / dz;
        (first.min(last).max(0.0), first.max(last).min(1.0))
    };
    if high - low <= 1e-12 {
        return Ok(false);
    }
    let latitude = (a[0].latitude + a[1].latitude + b[0].latitude + b[1].latitude) / 4.0;
    let relative = |a: Waypoint, b: Waypoint| {
        [
            ((b.longitude - a.longitude + 180.0).rem_euclid(360.0) - 180.0)
                * 60.0
                * latitude.to_radians().cos(),
            (b.latitude - a.latitude) * 60.0,
        ]
    };
    let p = relative(a[0], b[0]);
    let end = relative(a[1], b[1]);
    let v = [end[0] - p[0], end[1] - p[1]];
    let squared = v[0] * v[0] + v[1] * v[1];
    let closest = if squared > 0.0 {
        (-(p[0] * v[0] + p[1] * v[1]) / squared).clamp(low, high)
    } else {
        low
    };
    let distance = (p[0] + v[0] * closest).hypot(p[1] + v[1] * closest);
    Ok(distance < horizontal_nm)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crossings_are_checked_between_samples_only_while_vertical_separation_is_lost() {
        let p = |east, north, altitude| Waypoint {
            latitude: north / 60.0,
            longitude: east / 60.0,
            altitude_feet: altitude,
        };
        let a = [p(-1.0, 0.0, 5000.0), p(1.0, 0.0, 5000.0)];
        let crossing = [p(0.0, -1.0, 5000.0), p(0.0, 1.0, 5000.0)];
        assert!(breach(a, crossing, 0.2, 1000.0).unwrap());
        assert!(!breach(a, [p(0.0, -1.0, 7000.0), p(0.0, 1.0, 7000.0)], 0.2, 1000.0).unwrap());
        // The aircraft cross horizontally at t=.5; vertical separation is lost only after t=.9.
        assert!(!breach(a, [p(0.0, -1.0, 15000.0), p(0.0, 1.0, 5000.0)], 0.2, 1000.0).unwrap());
        assert!(breach(a, [p(0.0, -1.0, 7000.0), p(0.0, 1.0, 3000.0)], 0.2, 1000.0).unwrap());
        let wrapped = |lon| Waypoint {
            longitude: lon,
            latitude: 0.0,
            altitude_feet: 5000.0,
        };
        assert!(
            breach(
                [wrapped(179.999), wrapped(-179.999)],
                [wrapped(180.0); 2],
                0.2,
                1000.0
            )
            .unwrap()
        );
        assert!(breach(a, crossing, f64::NAN, 1000.0).is_err());
        assert!(breach([p(f64::NAN, 0.0, 5000.0); 2], crossing, 0.2, 1000.0).is_err());
    }
}
