//! Aircraft size and stand availability from simulator telemetry and scenery metadata.
use crate::{
    airport::{Airport, Parking, airport_point},
    state::{State, Telemetry},
};

/// X-Plane taxi and ramp width class, using the loaded aircraft rather than its start location.
#[must_use]
pub fn aircraft_size(state: &State) -> Option<char> {
    if let Some(span) = state.telemetry.wingspan_meters {
        return size_for_span(span);
    }
    let icao = if state.telemetry.aircraft_icao.is_empty() {
        &state.plan.aircraft
    } else {
        &state.telemetry.aircraft_icao
    };
    if let Some(aircraft) = crate::traffic::profiles::aircraft(icao) {
        return size_for_span(aircraft.wingspan);
    }
    // Conservative class for common fixed-wing types when the simulator omits wingspan.
    match icao.trim().to_uppercase().as_str() {
        "C152" | "C172" | "C182" | "C208" | "PA28" | "BE58" | "SR20" | "SR22" => Some('A'),
        "BE20" | "BE30" | "BE9L" | "C25A" | "C25B" | "C25C" | "C550" | "C560" | "PC12" | "DHC6" => {
            Some('B')
        }
        "A318" | "A319" | "A320" | "A321" | "A19N" | "A20N" | "A21N" | "BCS1" | "BCS3" | "B731"
        | "B732" | "B733" | "B734" | "B735" | "B736" | "B737" | "B738" | "B739" | "B37M"
        | "B38M" | "B39M" | "B3XM" | "AT72" | "AT76" | "DH8D" | "E170" | "E175" | "E190"
        | "E195" | "E290" | "E295" => Some('C'),
        "A306" | "A310" | "B752" | "B753" | "B762" | "B763" | "B764" | "MD11" => Some('D'),
        "A332" | "A333" | "A338" | "A339" | "A342" | "A343" | "A345" | "A346" | "A359" | "A35K"
        | "B741" | "B742" | "B743" | "B744" | "B772" | "B773" | "B77L" | "B77W" | "B788"
        | "B789" | "B78X" => Some('E'),
        "A388" | "B748" => Some('F'),
        _ => None,
    }
}

/// Width classes published in X-Plane's ATC taxi route specification.
#[must_use]
pub fn size_for_span(span: f64) -> Option<char> {
    if !span.is_finite() || span <= 0.0 {
        return None;
    }
    [
        (15.0, 'A'),
        (24.0, 'B'),
        (36.0, 'C'),
        (52.0, 'D'),
        (65.0, 'E'),
        (80.0, 'F'),
    ]
    .into_iter()
    .find(|(limit, _)| span < *limit)
    .map(|(_, size)| size)
}

/// Scenery stand width, absent when the author did not supply it.
#[must_use]
pub fn stand_size(stand: &Parking) -> Option<char> {
    stand
        .size
        .chars()
        .next()
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| ('A'..='F').contains(c))
}

/// Whether scenery explicitly provides enough width for this aircraft.
#[must_use]
pub fn fits(stand: &Parking, size: char) -> bool {
    stand_size(stand).is_some_and(|capacity| capacity >= size)
}

/// Fixed-wing traffic must not occupy scenery positions reserved for helicopters.
#[must_use]
pub fn fixed_wing_stand(stand: &Parking) -> bool {
    stand
        .equipment
        .split('|')
        .any(|equipment| matches!(equipment, "jets" | "heavy" | "turboprops" | "props"))
}

/// Equipment eligibility is separate from the stand's wingspan limit.
#[must_use]
pub fn supports_aircraft(stand: &Parking, aircraft: &crate::traffic::profiles::Aircraft) -> bool {
    let equipment = if aircraft.handling == Some(crate::traffic::animation::Kind::Piston) {
        "props"
    } else if aircraft.propeller {
        "turboprops"
    } else {
        "jets"
    };
    fits(stand, aircraft.parking_size())
        && stand.equipment.split('|').any(|value| {
            value == equipment || value == "heavy" && matches!(aircraft.wake, 'H' | 'J')
        })
}

fn span_limit(size: char) -> f64 {
    match size {
        'A' => 15.0,
        'B' => 24.0,
        'C' => 36.0,
        'D' => 52.0,
        'E' => 65.0,
        _ => 80.0,
    }
}

/// None means no traffic scan. Static scenery objects are not part of this feed.
#[must_use]
pub fn occupied(
    airport: &Airport,
    stand: &Parking,
    telemetry: &Telemetry,
    size: char,
) -> Option<bool> {
    let traffic = telemetry.traffic.as_ref()?;
    Some(traffic.iter().filter(|t| t.on_ground).any(|t| {
        if !t.latitude.is_finite() || !t.longitude.is_finite() {
            return true;
        }
        let point = airport_point(airport, t.latitude, t.longitude);
        let span = t
            .wingspan_meters
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(80.0);
        let clearance = span_limit(size) / 2.0 + span / 2.0 + 10.0;
        (point.east - stand.point.east).hypot(point.north - stand.point.north) < clearance
    }))
}

/// Preferred stand first, then gates of the smallest suitable class, nearest first.
#[must_use]
pub fn candidates<'a>(airport: &'a Airport, state: &State, size: char) -> Vec<&'a Parking> {
    let position = airport_point(airport, state.telemetry.latitude, state.telemetry.longitude);
    let distance =
        |s: &Parking| (s.point.east - position.east).hypot(s.point.north - position.north);
    let mut stands: Vec<_> = airport
        .parking
        .iter()
        .filter(|p| {
            fits(p, size)
                && (state.demo || occupied(airport, p, &state.telemetry, size) == Some(false))
        })
        .collect();
    stands.sort_by(|a, b| {
        (a.name != state.plan.arrival_stand)
            .cmp(&(b.name != state.plan.arrival_stand))
            .then_with(|| (a.kind != "gate").cmp(&(b.kind != "gate")))
            .then_with(|| stand_size(a).cmp(&stand_size(b)))
            .then_with(|| distance(a).total_cmp(&distance(b)))
    });
    stands
}

/// Recheck the destination before marking a taxi-in route complete.
#[must_use]
pub fn endpoint_available(state: &State, telemetry: &Telemetry) -> Option<bool> {
    let route = &state.taxi_clearance;
    if !route.to_parking || !route.hold_short_runway.is_empty() || !route.crossing_runway.is_empty()
    {
        return None;
    }
    let stand = Parking {
        point: *route.points.last()?,
        ..Default::default()
    };
    let airport = Airport {
        reference_latitude: route.reference_latitude,
        reference_longitude: route.reference_longitude,
        ..Default::default()
    };
    occupied(&airport, &stand, telemetry, aircraft_size(state)?).map(|busy| !busy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Point, TrafficTarget};
    #[test]
    fn actual_aircraft_overrides_dispatch_and_never_uses_departure_stand() {
        let mut state = State::default();
        state.plan.aircraft = "A20N".into();
        state.plan.departure_stand = "Tiny GA ramp".into();
        assert_eq!(aircraft_size(&state), Some('C'));
        state.telemetry.aircraft_icao = "B748".into();
        assert_eq!(aircraft_size(&state), Some('F'));
        state.telemetry.wingspan_meters = Some(35.8);
        assert_eq!(aircraft_size(&state), Some('C'));
        assert_eq!(size_for_span(36.0), Some('D'));
        assert_eq!(size_for_span(f64::NAN), None);
    }
    #[test]
    fn excludes_small_occupied_and_unknown_stands_and_checks_alternatives() {
        let airport = Airport {
            parking: vec![
                Parking {
                    name: "Small".into(),
                    size: "B".into(),
                    ..Default::default()
                },
                Parking {
                    name: "Occupied".into(),
                    size: "C".into(),
                    ..Default::default()
                },
                Parking {
                    name: "C2".into(),
                    size: "C".into(),
                    point: Point {
                        east: 150.0,
                        north: 0.0,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                Parking {
                    name: "Large".into(),
                    size: "E".into(),
                    point: Point {
                        east: 250.0,
                        north: 0.0,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                Parking {
                    name: "Unknown".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut state = State {
            demo: false,
            ..Default::default()
        };
        state.plan.arrival_stand = "Occupied".into();
        state.telemetry.traffic = Some(vec![TrafficTarget {
            on_ground: true,
            ..Default::default()
        }]);
        let names: Vec<_> = candidates(&airport, &state, 'C')
            .into_iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, ["C2", "Large"]);
        state.telemetry.traffic = None;
        assert!(candidates(&airport, &state, 'C').is_empty());
    }
}
