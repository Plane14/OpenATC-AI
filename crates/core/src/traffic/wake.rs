//! FAA consolidated wake rules. Aircraft categories come from JO 7360.1K.

/// Departure relationship to the preceding aircraft's flight path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Departure {
    /// Same-direction full-length departure, or the equivalent intersecting path.
    FullLength,
    /// Departure from an intersection beyond the preceding departure point.
    Intersection,
    /// Opposite-direction departure behind a departure or missed approach.
    Opposite,
}

/// Published CWT category for a supported aircraft. Unknown is never guessed from wingspan.
#[must_use]
pub fn category(aircraft: &str) -> Option<char> {
    super::profiles::aircraft(aircraft)?.cwt
}

/// Apply these rules only within the represented US procedure region.
#[must_use]
pub fn applies(airport: &str) -> bool {
    airport.starts_with('K')
        || ["PA", "PF", "PH", "PO", "PP"]
            .iter()
            .any(|p| airport.starts_with(p))
}

/// Required departure interval. The observer may use liftoff as a conservative timing origin.
/// JO 7110.65 3-9-6 and 3-9-7. Runway geometry must qualify before calling this function.
#[must_use]
pub fn departure_seconds(leader: char, follower: char, kind: Departure) -> Option<u16> {
    if !(('A'..='I').contains(&leader) && ('A'..='I').contains(&follower)) {
        return None;
    }
    let heavy_pair = follower != 'A'
        && (leader == 'A'
            || "BD".contains(leader)
            || (leader == 'C' && ('E'..='I').contains(&follower)));
    let base = if heavy_pair {
        if leader == 'A' { 180 } else { 120 }
    } else if leader == 'E' && follower == 'I' {
        120
    } else {
        0
    };
    Some(match kind {
        Departure::FullLength => base,
        Departure::Intersection if follower == 'I' && "EFGH".contains(leader) => 180,
        Departure::Intersection | Departure::Opposite if base > 0 => base + 60,
        Departure::Opposite if follower == 'I' && "FG".contains(leader) => 180,
        _ => 0,
    })
}

/// Whether observed departure wake can affect this runway without an inferred flight path.
/// Close parallel runways share wake protection; more distant crossing paths need route geometry.
#[must_use]
pub fn related_runways(
    airport: &crate::airport::Airport,
    leader: &str,
    follower: &str,
    category: Option<char>,
) -> bool {
    let find = |name: &str| {
        airport
            .runways
            .iter()
            .find(|r| r.first_name == name || r.second_name == name)
    };
    let (Some(a), Some(b)) = (find(leader), find(follower)) else {
        return false;
    };
    if a.first_name == b.first_name {
        return true;
    }
    let (dx, dn) = (b.second.east - b.first.east, b.second.north - b.first.north);
    let length = dx.hypot(dn);
    if length < 100.0 {
        return false;
    }
    let (ax, an) = (a.second.east - a.first.east, a.second.north - a.first.north);
    let angle = crate::navigation::turn(dx.atan2(dn).to_degrees(), ax.atan2(an).to_degrees()).abs();
    if angle.min((180.0 - angle).abs()) > 10.0 {
        return false;
    }
    let cross = |p: crate::state::Point| {
        ((p.east - b.first.east) * dn - (p.north - b.first.north) * dx) / length
    };
    let distance = cross(a.first).abs().max(cross(a.second).abs());
    let limit = match category {
        Some('E') => 213.36,
        Some('F'..='I') => 0.0,
        _ => 762.0,
    };
    distance < limit
}

/// Resolve the observed final to the closest aligned runway in the shared wake area.
#[must_use]
pub fn approach_runway<'a>(
    airport: &'a crate::airport::Airport,
    own: &str,
    target: &crate::state::TrafficTarget,
) -> Option<&'a str> {
    let own_position = super::runway_position(airport, own, target.latitude, target.longitude)?;
    airport
        .runways
        .iter()
        .filter(|r| related_runways(airport, &r.first_name, own, None))
        .flat_map(|r| [&r.first_name, &r.second_name])
        .filter_map(|name| {
            let position =
                super::approach_position(airport, name, target.latitude, target.longitude)?;
            (position.1.abs() <= 250.0
                && crate::navigation::turn(own_position.3, position.3).abs() < 20.0
                && crate::navigation::turn(target.track_degrees, position.3).abs() < 20.0)
                .then_some((name.as_str(), position.1.abs()))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(name, _)| name)
}

/// Wake-only approach spacing at the leader's threshold passage; radar minima still apply.
/// JO 7110.65 5-5-4, Wake Turbulence Separation for On Approach.
#[must_use]
pub fn approach_nm(leader: char, follower: char) -> Option<f64> {
    if !(('A'..='I').contains(&leader) && ('A'..='I').contains(&follower)) {
        return None;
    }
    let row: [f64; 9] = match leader {
        'A' => [0.0, 5.0, 6.0, 6.0, 7.0, 7.0, 7.0, 8.0, 8.0],
        'B' => [0.0, 3.0, 4.0, 4.0, 5.0, 5.0, 5.0, 5.0, 6.0],
        'C' => [0.0, 0.0, 0.0, 0.0, 3.5, 3.5, 3.5, 5.0, 6.0],
        'D' => [0.0, 3.0, 4.0, 4.0, 5.0, 5.0, 5.0, 6.0, 6.0],
        'E' | 'F' => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 4.0],
        _ => [0.0; 9],
    };
    Some(row[follower as usize - 'A' as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heavy_parallel_departures_share_protection_but_remote_runways_do_not() {
        use crate::{
            airport::{Airport, Runway},
            state::Point,
        };
        let mut airport = Airport {
            runways: vec![
                Runway {
                    first_name: "09L".into(),
                    second_name: "27R".into(),
                    first: Point::default(),
                    second: Point {
                        east: 3000.0,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                Runway {
                    first_name: "09R".into(),
                    second_name: "27L".into(),
                    first: Point {
                        north: 230.0,
                        ..Default::default()
                    },
                    second: Point {
                        north: 230.0,
                        east: 3000.0,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert!(related_runways(&airport, "09L", "09R", Some('B')));
        assert!(related_runways(&airport, "27R", "09R", Some('B')));
        assert!(!related_runways(&airport, "09L", "09R", Some('E')));
        assert!(!related_runways(&airport, "09L", "09R", Some('F')));
        assert!(related_runways(&airport, "09L", "27R", Some('F')));
        airport.runways[1].first.north = 800.0;
        airport.runways[1].second.north = 800.0;
        assert!(!related_runways(&airport, "09L", "09R", Some('B')));
    }
    #[test]
    fn approach_category_and_icao_weight_are_not_cwt_categories() {
        assert_eq!(category("A320"), Some('F'));
        assert_eq!(category("P28A"), Some('I'));
        assert_eq!(category("A339"), Some('D'));
        assert_eq!(category("B763"), Some('C'));
        assert_eq!(category("unknown"), None);
    }
    #[test]
    fn aircraft_pair_and_departure_point_change_the_wake_interval() {
        assert_eq!(departure_seconds('F', 'I', Departure::FullLength), Some(0));
        assert_eq!(
            departure_seconds('F', 'I', Departure::Intersection),
            Some(180)
        );
        assert_eq!(
            departure_seconds('B', 'F', Departure::FullLength),
            Some(120)
        );
        assert_eq!(
            departure_seconds('A', 'F', Departure::Intersection),
            Some(240)
        );
        assert_eq!(departure_seconds('C', 'B', Departure::FullLength), Some(0));
        assert_eq!(departure_seconds('C', 'F', Departure::Opposite), Some(180));
        assert_eq!(departure_seconds('?', 'F', Departure::FullLength), None);
    }
    #[test]
    fn approach_pairs_are_directional_and_zero_does_not_waive_radar_separation() {
        assert_eq!(approach_nm('F', 'I'), Some(4.0));
        assert_eq!(approach_nm('I', 'F'), Some(0.0));
        assert_eq!(approach_nm('A', 'I'), Some(8.0));
        assert_eq!(approach_nm('B', 'I'), Some(6.0));
        assert_eq!(approach_nm('C', 'F'), Some(3.5));
        assert_eq!(approach_nm('?', 'I'), None);
    }
}
