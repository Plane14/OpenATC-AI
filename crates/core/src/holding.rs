//! Conservative runway protection using actual simulator targets.
use crate::{
    airport::{Airport, airport_point},
    state::Telemetry,
};
/// Spoken response ID when runway entry/takeoff is blocked.
#[must_use]
pub fn blocked(airport: &Airport, runway: &str, telemetry: &Telemetry) -> Option<&'static str> {
    let Some(targets) = &telemetry.traffic else {
        return Some("holding_standby");
    };
    blocked_targets(airport, runway, targets.iter().take(63))
}

/// Check a bounded traffic view without copying telemetry or aircraft lists.
pub fn blocked_targets<'a>(
    airport: &Airport,
    runway: &str,
    targets: impl Iterator<Item = &'a crate::state::TrafficTarget>,
) -> Option<&'static str> {
    let Some(r) = airport
        .runways
        .iter()
        .find(|r| r.first_name == runway || r.second_name == runway)
    else {
        return Some("holding_standby");
    };
    let (start, end) = if r.first_name == runway {
        (r.first, r.second)
    } else {
        (r.second, r.first)
    };
    let dx = end.east - start.east;
    let dn = end.north - start.north;
    let length = dx.hypot(dn);
    if length < 100.0 {
        return Some("holding_standby");
    }
    let heading = dx.atan2(dn).to_degrees().rem_euclid(360.0);
    for t in targets {
        if ![t.latitude, t.longitude, t.altitude_feet, t.track_degrees]
            .iter()
            .all(|v| v.is_finite())
            || t.latitude.abs() > 90.0
            || t.longitude.abs() > 180.0
        {
            return Some("holding_standby");
        }
        let p = airport_point(airport, t.latitude, t.longitude);
        let east = p.east - start.east;
        let north = p.north - start.north;
        let along = (east * dx + north * dn) / length;
        let across = (east * dn - north * dx).abs() / length;
        let agl = t.altitude_feet - airport.elevation_feet;
        let (fore_aft, lateral) =
            crate::traffic::footprint(&t.aircraft, t.wingspan_meters, t.track_degrees, heading);
        if (-fore_aft - 10.0..=length + fore_aft + 10.0).contains(&along)
            && across < r.width / 2.0 + lateral + 10.0
            && (t.on_ground || agl < 150.0)
        {
            return Some("holding_runway_occupied");
        }
        let heading_difference = (t.track_degrees - heading + 180.0).rem_euclid(360.0) - 180.0;
        // Protect both approach ends of the same physical runway.
        let inbound = (along < 0.0 && heading_difference.abs() < 35.0)
            || (along > length && heading_difference.abs() > 145.0);
        let distance = if along < 0.0 { -along } else { along - length };
        if !t.on_ground
            && inbound
            && distance < 9260.0
            && across < 150.0 + distance * 0.08
            && (-100.0..2500.0).contains(&agl)
        {
            return Some("holding_traffic_final");
        }
    }
    None
}

/// Protect the time needed for a particular runway movement, not just a fixed radius.
pub fn blocked_during<'a>(
    airport: &Airport,
    runway: &str,
    targets: impl Iterator<Item = &'a crate::state::TrafficTarget> + Clone,
    seconds: f64,
) -> Option<&'static str> {
    if !seconds.is_finite() || !(0.0..=600.0).contains(&seconds) {
        return Some("holding_standby");
    }
    if let Some(reason) = blocked_targets(airport, runway, targets.clone()) {
        return Some(reason);
    }
    for target in targets.filter(|target| !target.on_ground) {
        let Some((along, across, length, heading)) =
            crate::traffic::runway_position(airport, runway, target.latitude, target.longitude)
        else {
            return Some("holding_standby");
        };
        let turn = crate::navigation::turn(heading, target.track_degrees).abs();
        let inbound = along < 0.0 && turn < 35.0 || along > length && turn > 145.0;
        let distance = if along < 0.0 { -along } else { along - length };
        if !inbound
            || across.abs() > 150.0 + distance * 0.08
            || !(-100.0..=5000.0).contains(&(target.altitude_feet - airport.elevation_feet))
        {
            continue;
        }
        let Some(speed) = target
            .ground_speed_knots
            .filter(|speed| (20.0..=600.0).contains(speed))
        else {
            if distance <= seconds * 600.0 * 0.514_444 {
                return Some("holding_standby");
            }
            continue;
        };
        if distance / (speed * 0.514_444) <= seconds {
            return Some("holding_traffic_final");
        }
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Point, TrafficTarget};
    #[test]
    fn a_long_crossing_protects_an_arrival_outside_the_fixed_final_radius() {
        let airport = crate::airport::demo_airport();
        let mut traffic = TrafficTarget {
            latitude: 0.0,
            longitude: -0.18,
            altitude_feet: 1200.0,
            track_degrees: 90.0,
            ground_speed_knots: Some(200.0),
            on_ground: false,
            ..Default::default()
        };
        assert!(blocked_targets(&airport, "09", std::iter::once(&traffic)).is_none());
        assert_eq!(
            blocked_during(&airport, "09", std::iter::once(&traffic), 240.0),
            Some("holding_traffic_final")
        );
        assert!(blocked_during(&airport, "09", std::iter::once(&traffic), 60.0).is_none());
        traffic.ground_speed_knots = None;
        assert_eq!(
            blocked_during(&airport, "09", std::iter::once(&traffic), 240.0),
            Some("holding_standby")
        );
        traffic.longitude = 0.18;
        traffic.track_degrees = 270.0;
        traffic.ground_speed_knots = Some(200.0);
        assert_eq!(
            blocked_during(&airport, "09", std::iter::once(&traffic), 240.0),
            Some("holding_traffic_final")
        );
    }
    #[test]
    fn crossing_tail_and_arrival_exit_stay_protected_beyond_the_painted_line() {
        let airport = Airport {
            runways: vec![crate::airport::Runway {
                first_name: "09".into(),
                second_name: "27".into(),
                width: 45.0,
                second: Point {
                    east: 2000.0,
                    ..Default::default()
                },
                ..Default::default()
            }],
            taxi_hold_lines: vec![[
                Point {
                    east: 900.0,
                    north: 75.0,
                    height: 0.0,
                },
                Point {
                    east: 1100.0,
                    north: 75.0,
                    height: 0.0,
                },
            ]],
            ..Default::default()
        };
        let route = crate::state::TaxiClearance {
            crossing_runway: "09".into(),
            points: vec![
                Point {
                    east: 1000.0,
                    north: -100.0,
                    height: 0.0,
                },
                Point {
                    east: 1000.0,
                    north: 130.0,
                    height: 0.0,
                },
            ],
            ..Default::default()
        };
        let mut t = Telemetry {
            aircraft_icao: "A320".into(),
            longitude: 1000.0 / 111_320.0,
            latitude: 100.0 / 111_320.0,
            heading_degrees: 0.0,
            on_ground: true,
            position_valid: true,
            ..Default::default()
        };
        assert!(!crate::airport::crossing_vacated(&airport, &route, &t));
        assert!(crate::airport::aircraft_on_runway(&airport, &t));
        t.latitude = 125.0 / 111_320.0;
        assert!(crate::airport::crossing_vacated(&airport, &route, &t));
        assert!(!crate::airport::aircraft_on_runway(&airport, &t));
        let target = TrafficTarget {
            aircraft: "B77W".into(),
            longitude: 1000.0 / 111_320.0,
            latitude: 65.0 / 111_320.0,
            on_ground: true,
            track_degrees: 0.0,
            ..Default::default()
        };
        assert_eq!(
            blocked_targets(&airport, "09", std::iter::once(&target)),
            Some("holding_runway_occupied")
        );
    }
    #[test]
    fn distinguishes_clear_runway_incoming_occupied_and_unavailable() {
        let a = Airport {
            runways: vec![crate::airport::Runway {
                first_name: "09".into(),
                second_name: "27".into(),
                first: Point::default(),
                second: Point {
                    east: 2000.0,
                    ..Default::default()
                },
                width: 45.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut t = Telemetry::default();
        assert_eq!(blocked(&a, "09", &t), Some("holding_standby"));
        t.traffic = Some(vec![]);
        assert_eq!(blocked(&a, "09", &t), None);
        let target = TrafficTarget {
            longitude: -0.02,
            altitude_feet: 500.0,
            track_degrees: 90.0,
            ..Default::default()
        };
        t.traffic = Some(vec![target]);
        assert_eq!(blocked(&a, "09", &t), Some("holding_traffic_final"));
        t.traffic.as_mut().unwrap()[0].track_degrees = 270.0;
        assert_eq!(blocked(&a, "09", &t), None);
        t.traffic = Some(vec![TrafficTarget {
            longitude: 0.01,
            on_ground: true,
            ..Default::default()
        }]);
        assert_eq!(blocked(&a, "09", &t), Some("holding_runway_occupied"));
    }
}
