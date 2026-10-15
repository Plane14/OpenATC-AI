//! Explicit airborne test positions. These do not issue operational permissions.
use crate::{
    arrival::{bearing, destination, distance_nm},
    state::{FlightPlan, PhaseCode},
};
use serde::{Deserialize, Serialize};

/// Route checkpoint passed between the engine and simulator adapter.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    /// Identifies the requested test stage.
    pub kind: String,
    /// Geographic latitude.
    pub latitude: f64,
    /// Geographic longitude.
    pub longitude: f64,
    /// Nominal MSL level; the adapter preserves the current barometric offset.
    pub altitude_feet: f64,
    /// Physical SDK height, distinct from the assigned barometric level.
    pub geometric_feet: f64,
    /// Course towards the next route fix, degrees true.
    pub track: f64,
    /// Next unflown route fix.
    pub next_fix: usize,
    /// ATC planning stage, independent of approach/landing permission.
    pub phase: PhaseCode,
}

/// Select straight, known legs only. Never invent vector legs, arcs or a final clearance.
pub fn route(plan: &FlightPlan, kind: &str) -> Result<Checkpoint, String> {
    let fixes = &plan.fixes;
    let straight = |index: usize| {
        index > 0
            && matches!(fixes[index].path_terminator.as_str(), "" | "TF" | "DF")
            && distance_nm(
                [fixes[index - 1].latitude, fixes[index - 1].longitude],
                [fixes[index].latitude, fixes[index].longitude],
            ) >= 2.0
    };
    let selected = match kind {
        "sid" => (1..fixes.len())
            .find(|&i| straight(i) && fixes[i].constraint_source.starts_with("SID ")),
        "cruise" => (1..fixes.len())
            .filter(|&i| {
                straight(i)
                    && fixes[i].constraint_source.is_empty()
                    && fixes[i - 1].constraint_source.is_empty()
                    && fixes[i].altitude_feet >= f64::from(plan.cruise_feet) * 0.9
            })
            .max_by(|&a, &b| {
                distance_nm(
                    [fixes[a - 1].latitude, fixes[a - 1].longitude],
                    [fixes[a].latitude, fixes[a].longitude],
                )
                .total_cmp(&distance_nm(
                    [fixes[b - 1].latitude, fixes[b - 1].longitude],
                    [fixes[b].latitude, fixes[b].longitude],
                ))
            }),
        "star" => (1..fixes.len())
            .find(|&i| straight(i) && fixes[i].constraint_source.starts_with("STAR ")),
        "approach" => {
            // Start at least 20 route miles before the first approach fix so the
            // normal planner must issue the final descent and approach permission.
            let entry = fixes
                .iter()
                .position(|f| f.constraint_source.starts_with("APPCH "))
                .ok_or("Import and expand a published approach first.")?;
            let mut remaining = 20.0;
            let mut selected = None;
            for i in (1..=entry).rev() {
                let length = distance_nm(
                    [fixes[i - 1].latitude, fixes[i - 1].longitude],
                    [fixes[i].latitude, fixes[i].longitude],
                );
                if length >= remaining && straight(i) {
                    selected = Some((i, (1.0 - remaining / length).clamp(0.05, 0.95)));
                    break;
                }
                remaining -= length;
                if remaining <= 0.0 {
                    break;
                }
            }
            let (i, fraction) =
                selected.ok_or("No straight route leg available 20 miles before approach.")?;
            return on_leg(plan, kind, i, fraction, PhaseCode::Arrival);
        }
        _ => return Err("Unknown airborne checkpoint.".into()),
    }
    .ok_or("No suitable expanded route leg for that checkpoint.")?;
    on_leg(
        plan,
        kind,
        selected,
        0.5,
        match kind {
            "sid" => PhaseCode::Departure,
            "cruise" => PhaseCode::Cruise,
            _ => PhaseCode::Arrival,
        },
    )
}
fn on_leg(
    plan: &FlightPlan,
    kind: &str,
    i: usize,
    fraction: f64,
    phase: PhaseCode,
) -> Result<Checkpoint, String> {
    let a = &plan.fixes[i - 1];
    let b = &plan.fixes[i];
    if ![
        a.latitude,
        a.longitude,
        b.latitude,
        b.longitude,
        a.altitude_feet,
        b.altitude_feet,
    ]
    .iter()
    .all(|v| v.is_finite())
        || a.latitude.abs() > 85.0
        || b.latitude.abs() > 85.0
        || a.longitude.abs() > 180.0
        || b.longitude.abs() > 180.0
    {
        return Err(
            "Checkpoint coordinates are invalid or outside the supported latitude range.".into(),
        );
    }
    let start = [a.latitude, a.longitude];
    let end = [b.latitude, b.longitude];
    let point = destination(
        start,
        bearing(start, end),
        distance_nm(start, end) * fraction,
    );
    let mut level = if kind == "cruise" {
        f64::from(plan.cruise_feet)
    } else {
        a.altitude_feet + (b.altitude_feet - a.altitude_feet) * fraction
    };
    if b.minimum_altitude_feet
        .zip(b.maximum_altitude_feet)
        .is_some_and(|(minimum, maximum)| minimum > maximum)
    {
        return Err("Checkpoint has conflicting published altitude limits.".into());
    }
    level = (level / 100.0).round() * 100.0;
    if let Some(min) = b.minimum_altitude_feet {
        level = level.max(f64::from(min));
    }
    if let Some(max) = b.maximum_altitude_feet {
        level = level.min(f64::from(max));
    }
    if !(1500.0..=45000.0).contains(&level) {
        return Err("Checkpoint requires a usable planned/published airborne level.".into());
    }
    Ok(Checkpoint {
        kind: kind.into(),
        latitude: point[0],
        longitude: point[1],
        altitude_feet: level,
        geometric_feet: level,
        track: bearing(point, end),
        next_fix: i,
        phase,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RouteFix;
    #[test]
    fn checkpoints_use_expanded_legs_and_leave_time_for_approach() {
        let mut plan = FlightPlan {
            cruise_feet: 18000,
            ..Default::default()
        };
        plan.fixes = (0..8)
            .map(|i| RouteFix {
                latitude: 44.0 + f64::from(i) * 0.25,
                longitude: 8.0,
                altitude_feet: if i < 4 { 18000.0 } else { 5000.0 },
                constraint_source: match i {
                    1 => "SID TEST",
                    4 | 5 => "STAR TEST",
                    6 | 7 => "APPCH I35RY",
                    _ => "",
                }
                .into(),
                path_terminator: "TF".into(),
                ..Default::default()
            })
            .collect();
        assert_eq!(route(&plan, "sid").unwrap().next_fix, 1);
        assert_eq!(route(&plan, "cruise").unwrap().altitude_feet, 18000.0);
        assert_eq!(route(&plan, "star").unwrap().next_fix, 4);
        let before = route(&plan, "approach").unwrap();
        assert_eq!(before.next_fix, 5);
        assert_eq!(before.phase, PhaseCode::Arrival);
        assert_eq!(before.altitude_feet, 5000.0);
        plan.fixes[5].path_terminator = "RF".into();
        assert!(route(&plan, "approach").is_err());
        assert!(route(&plan, "final").is_err());
    }
}
