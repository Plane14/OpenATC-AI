//! Route geometry and published crossing limits, separate from planned altitudes.
use crate::state::{FlightPlan, RouteFix, Telemetry};

/// A known route leg near the aircraft.
#[derive(Clone, Debug)]
pub struct Leg {
    /// Destination index in the plan's fix list.
    pub next: usize,
    /// Distance from the leg's centreline in nautical miles.
    pub cross_track_nm: f64,
    /// Projection along the leg; one is the destination fix.
    pub along: f64,
    /// Bearing from the aircraft to that fix, degrees true.
    pub bearing: f64,
    /// Distance to the destination fix.
    pub distance_nm: f64,
}
/// Project onto a straight imported leg. Procedure arcs and vector legs are not inferred.
pub fn leg(fixes: &[RouteFix], next: usize, t: &Telemetry) -> Option<Leg> {
    let a = fixes.get(next.checked_sub(1)?)?;
    let b = fixes.get(next)?;
    if !b.path_terminator.is_empty() && !["TF", "DF"].contains(&b.path_terminator.as_str()) {
        return None;
    }
    let cosine = t.latitude.to_radians().cos().abs().max(0.05);
    let lon = |v: f64| (v - t.longitude + 180.0).rem_euclid(360.0) - 180.0;
    let p = [
        lon(a.longitude) * cosine * 60.0,
        (a.latitude - t.latitude) * 60.0,
    ];
    let q = [
        lon(b.longitude) * cosine * 60.0,
        (b.latitude - t.latitude) * 60.0,
    ];
    let d = [q[0] - p[0], q[1] - p[1]];
    let len = d[0] * d[0] + d[1] * d[1];
    if len < 0.01 {
        return None;
    }
    let along = -(p[0] * d[0] + p[1] * d[1]) / len;
    let cross = (p[0] + along.clamp(0.0, 1.0) * d[0]).hypot(p[1] + along.clamp(0.0, 1.0) * d[1]);
    Some(Leg {
        next,
        cross_track_nm: cross,
        along,
        bearing: crate::arrival::bearing([t.latitude, t.longitude], [b.latitude, b.longitude]),
        distance_nm: crate::ops::distance_nm(t.latitude, t.longitude, b.latitude, b.longitude),
    })
}
/// Parse a published altitude in feet or a coded flight level.
fn altitude(value: &str) -> Option<i32> {
    let value = value.trim();
    let (number, multiplier) = value.strip_prefix("FL").map_or((value, 1), |v| (v, 100));
    number
        .parse::<i32>()
        .ok()
        .map(|v| v * multiplier)
        .filter(|v| (0..=60000).contains(v))
}
/// Merge the selected runway and procedure transitions only.
pub fn load_constraints(plan: &mut FlightPlan, kind: &str, text: &str) {
    let (name, runway, transition) = match kind {
        "SID" => (&plan.sid, &plan.runway, &plan.sid_transition),
        "STAR" => (&plan.star, &plan.arrival_runway, &plan.star_transition),
        "APPCH" => (&plan.approach, &plan.arrival_runway, &String::new()),
        _ => return,
    };
    if name.is_empty() {
        return;
    }
    for line in text.lines() {
        let Some((record, data)) = line.split_once(':') else {
            continue;
        };
        if record != kind {
            continue;
        }
        let f: Vec<_> = data.split(',').map(str::trim).collect();
        if f.len() < 28 || f[2] != name {
            continue;
        }
        let shared = format!("RW{}B", runway.trim_end_matches(['L', 'R', 'C']));
        if !f[3].is_empty() && f[3] != transition && f[3] != format!("RW{runway}") && f[3] != shared
        {
            continue;
        }
        if kind == "SID" && ["VM", "VR", "VA", "VD", "VI", "FM"].contains(&f[11]) {
            plan.sid_has_vectors = true;
        }
        let first = altitude(f[23]);
        let second = altitude(f[24]);
        let (min, max) = match f[22] {
            "+" => (first, None),
            "-" => (None, first),
            "B" if first.is_some() && second.is_some() => (
                Some(first.unwrap().min(second.unwrap())),
                Some(first.unwrap().max(second.unwrap())),
            ),
            " " | "" => (first, first),
            _ => (None, None),
        };
        let index = if kind == "SID" {
            plan.fixes.iter().position(|fix| fix.identifier == f[4])
        } else {
            plan.fixes.iter().rposition(|fix| fix.identifier == f[4])
        };
        if let Some(index) = index {
            let fix = &mut plan.fixes[index];
            fix.path_terminator = f[11].into();

            if let Some(min) = min {
                fix.minimum_altitude_feet =
                    Some(fix.minimum_altitude_feet.map_or(min, |old| old.max(min)));
            }
            if let Some(max) = max {
                fix.maximum_altitude_feet =
                    Some(fix.maximum_altitude_feet.map_or(max, |old| old.min(max)));
            }
            if let Ok(speed) = f[27].parse::<f64>()
                && speed > 0.0
            {
                fix.maximum_speed_knots = Some(speed);
            }
            if min.is_some() || max.is_some() || fix.maximum_speed_knots.is_some() {
                fix.constraint_source = format!("{kind} {name} / installed CIFP");
            }
        }
    }
}
/// Load terminal constraints from the same custom/default navdata priority as X-Plane.
pub fn enrich(plan: &mut FlightPlan, root: &str) {
    plan.sid_has_vectors = false;
    for fix in &mut plan.fixes {
        if fix.constraint_source.ends_with("/ installed CIFP") {
            fix.minimum_altitude_feet = None;
            fix.maximum_altitude_feet = None;
            fix.maximum_speed_knots = None;
            fix.path_terminator.clear();
            fix.constraint_source.clear();
        }
    }
    for (id, kind) in [
        (plan.departure.clone(), "SID"),
        (plan.destination.clone(), "STAR"),
        (plan.destination.clone(), "APPCH"),
    ] {
        let custom = format!("{root}/Custom Data/CIFP/{id}.dat");
        let path = if std::path::Path::new(&custom).is_file() {
            custom
        } else {
            format!("{root}/Resources/default data/CIFP/{id}.dat")
        };
        if let Ok(text) = std::fs::read_to_string(path) {
            load_constraints(plan, kind, &text);
        }
    }
}
/// Smallest signed turn from one heading to another.
pub fn turn(from: f64, to: f64) -> f64 {
    (to - from + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_and_turn_wrap() {
        let fixes = vec![
            RouteFix {
                latitude: 0.0,
                longitude: 0.0,
                ..Default::default()
            },
            RouteFix {
                latitude: 0.0,
                longitude: 1.0,
                ..Default::default()
            },
        ];
        let t = Telemetry {
            latitude: 0.1,
            longitude: 0.5,
            ..Default::default()
        };
        let l = leg(&fixes, 1, &t).unwrap();
        assert!((l.cross_track_nm - 6.0).abs() < 0.1);
        assert!((l.along - 0.5).abs() < 0.01);
        assert_eq!(turn(350.0, 10.0), 20.0);
    }
    #[test]
    fn planned_altitude_is_not_a_restriction_and_other_runways_are_excluded() {
        let mut p = FlightPlan {
            sid: "TEST1".into(),
            runway: "25L".into(),
            fixes: vec![RouteFix {
                identifier: "FIX".into(),
                altitude_feet: 5000.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut f = vec![""; 38];
        f[2] = "TEST1";
        f[3] = "RW25B";
        f[4] = "FIX";
        f[22] = "B";
        f[23] = "09000";
        f[24] = "FL050";
        load_constraints(&mut p, "SID", &format!("SID:{};", f.join(",")));
        assert_eq!(p.fixes[0].minimum_altitude_feet, Some(5000));
        assert_eq!(p.fixes[0].maximum_altitude_feet, Some(9000));
        f[3] = "RW07B";
        f[22] = "-";
        f[23] = "03000";
        load_constraints(&mut p, "SID", &format!("SID:{};", f.join(",")));
        assert_eq!(p.fixes[0].maximum_altitude_feet, Some(9000));
    }
}

/// Installed one-degree grid minimum off-route altitudes, in feet.
#[derive(Clone, Debug, Default)]
pub struct Mora(pub std::collections::BTreeMap<(i32, i32), i32>);
impl Mora {
    /// Custom navdata takes priority over the bundled grid.
    pub fn load(root: &str) -> Self {
        let custom = format!("{root}/Custom Data/earth_mora.dat");
        let path = if std::path::Path::new(&custom).is_file() {
            custom
        } else {
            format!("{root}/Resources/default data/earth_mora.dat")
        };
        std::fs::read_to_string(path).map_or_else(|_| Self::default(), |text| Self::parse(&text))
    }
    /// Rows contain thirty consecutive longitude cells, each in hundreds of feet.
    pub fn parse(text: &str) -> Self {
        let mut cells = std::collections::BTreeMap::new();
        for line in text.lines() {
            let f: Vec<_> = line.split_whitespace().collect();
            if f.len() != 32 {
                continue;
            }
            let (Ok(lat), Ok(lon)) = (f[0].parse::<i32>(), f[1].parse::<i32>()) else {
                continue;
            };
            for (index, value) in f[2..].iter().enumerate() {
                if let Ok(height) = value.parse::<i32>()
                    && (1..600).contains(&height)
                {
                    cells.insert((lat, lon + index as i32), height * 100);
                }
            }
        }
        Self(cells)
    }
    /// Conservative floor along a prospective off-route vector; missing cells stay unknown.
    pub fn corridor(&self, from: [f64; 2], to: [f64; 2]) -> Option<i32> {
        let distance = crate::arrival::distance_nm(from, to);
        let count = (distance / 2.0).ceil().clamp(1.0, 500.0) as usize;
        let bearing = crate::arrival::bearing(from, to);
        (0..=count)
            .map(|i| {
                let p =
                    crate::arrival::destination(from, bearing, distance * i as f64 / count as f64);
                self.0
                    .get(&(p[0].floor() as i32, p[1].floor() as i32))
                    .copied()
            })
            .collect::<Option<Vec<_>>>()
            .and_then(|v| v.into_iter().max())
    }
}
