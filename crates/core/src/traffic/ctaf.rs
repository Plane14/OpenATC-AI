//! Advisory-only operations at fields without a Tower controller.
//!
//! A CTAF field is accepted only when no Tower service is published for the airport. The
//! pattern below therefore cannot issue or satisfy a controlled clearance; every leg is a
//! self-announced advisory. Towered fields keep their existing controlled-clearance path.
use crate::{
    airport::Airport,
    state::Point,
    stations::Station,
    traffic::motion::{Route, Segment, Stage, Waypoint},
};
use std::sync::Arc;

/// A published common traffic advisory frequency at an untowered field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// Airport identifier.
    pub airport: String,
    /// Advisory frequency in kHz.
    pub frequency_khz: i32,
}

fn tower_serves(airport: &str, stations: &[Station]) -> bool {
    stations.iter().any(|station| {
        station.airport == airport
            && (station.service == "Tower" || station.services.iter().any(|s| s == "Tower"))
    })
}

fn publishes_advisory(service: &str, name: &str) -> bool {
    let service = service.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    service.contains("ctaf")
        || service.contains("unicom")
        || name.contains("ctaf")
        || name.contains("unicom")
}

fn advisory_frequency(airport: &Airport, stations: &[Station]) -> Option<i32> {
    stations
        .iter()
        .filter(|station| station.airport == airport.icao)
        .filter(|station| publishes_advisory(&station.service, &station.name))
        .map(|station| station.khz)
        .chain(
            airport
                .frequencies
                .iter()
                .filter(|frequency| publishes_advisory(&frequency.service, &frequency.name))
                .map(|frequency| frequency.khz),
        )
        .find(|khz| (118_000..=136_975).contains(khz))
}

/// Airports that publish an advisory frequency and no Tower service.
///
/// A towered airport is never listed, so advisory traffic cannot be planned where a
/// controlled clearance is required.
#[must_use]
pub fn candidates(stations: &[Station]) -> Vec<String> {
    let mut airports = std::collections::BTreeSet::new();
    for station in stations {
        if !publishes_advisory(&station.service, &station.name)
            || tower_serves(&station.airport, stations)
            || station.airport.is_empty()
        {
            continue;
        }
        airports.insert(station.airport.clone());
    }
    airports.into_iter().collect()
}

/// Resolve the advisory frequency for one field.
///
/// This refuses a towered airport even when it also publishes a UNICOM frequency, so advisory
/// traffic can never appear where a controlled clearance is required.
pub fn field(airport: &Airport, stations: &[Station]) -> Result<Field, String> {
    if tower_serves(&airport.icao, stations) {
        return Err(format!(
            "{} is towered; controlled clearance is required",
            airport.icao
        ));
    }
    let frequency_khz = advisory_frequency(airport, stations)
        .ok_or_else(|| format!("{} publishes no CTAF or UNICOM frequency", airport.icao))?;
    Ok(Field {
        airport: airport.icao.clone(),
        frequency_khz,
    })
}

/// One self-announced leg of a visual pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct Advisory {
    /// Leg that produced the call.
    pub leg: &'static str,
    /// Spoken advisory, containing the callsign, field and leg.
    pub text: String,
}

/// The advisory made on one leg of a circuit.
#[derive(Clone, Debug, PartialEq)]
pub struct Leg {
    /// Route segment the aircraft is established on when the advisory is made.
    pub segment: usize,
    /// The advisory made on that leg.
    pub advisory: Advisory,
}

/// One circuit of a visual pattern: its geometry and the advisories made on it.
#[derive(Clone, Debug)]
pub struct Circuit {
    route: Arc<Route>,
    legs: Vec<Leg>,
}

impl Circuit {
    /// Geometry of this circuit.
    #[must_use]
    pub fn route(&self) -> Arc<Route> {
        self.route.clone()
    }
    /// Advisories made on this circuit, in circuit order.
    #[must_use]
    pub fn legs(&self) -> &[Leg] {
        &self.legs
    }
    /// The advisory made on one segment, when the circuit makes one there.
    #[must_use]
    pub fn advisory(&self, segment: usize) -> Option<&Advisory> {
        self.legs
            .iter()
            .find(|leg| leg.segment == segment)
            .map(|leg| &leg.advisory)
    }
}

/// Legs of a circuit and the segment each advisory is made on.
///
/// Every leg of the circuit carries its own call, so an aircraft announces the leg it is
/// established on.
const LEGS: [(&str, &str, usize); 4] = [
    ("downwind", "downwind", 0),
    ("base", "base", 1),
    ("final", "final", 2),
    ("landing", "landing", 3),
];

/// Climb and descent performance used to check the pattern geometry.
const CLIMB_FPM: f64 = 1_000.0;
const DESCENT_FPM: f64 = 1_000.0;

/// Distance from the runway centreline to the downwind leg.
const OFFSET_METERS: f64 = 1_500.0;
/// Distance from the threshold to the final approach fix.
const FINAL_METERS: f64 = 3_000.0;
/// Length of the straight climb ahead of the runway after a landing.
const UPWIND_METERS: f64 = 3_200.0;
/// Lead of the crosswind leg beyond the end of the climb.
const CROSSWIND_METERS: f64 = 1_500.0;
/// Height above the field of a visual pattern.
const PATTERN_FEET: f64 = 1_000.0;

/// Local geometry of a right-hand circuit, in metres along and right of the runway.
///
/// The downwind is flown on the right of the runway and against the landing direction, so
/// the aircraft turns base towards the runway and lands on the runway heading.
struct Geometry {
    reference_latitude: f64,
    reference_longitude: f64,
    /// Threshold of the runway the circuit uses, in the airport's local plane.
    origin: Point,
    /// Unit vector along the runway.
    along: (f64, f64),
    /// Unit vector to the right of the runway.
    right: (f64, f64),
    /// Runway length in metres.
    length: f64,
    /// Field elevation in feet.
    elevation: f64,
    /// Pattern altitude in feet.
    pattern_altitude: f64,
    /// Runway resource held during the landing roll.
    runway_resource: String,
}

impl Geometry {
    /// A point on the pattern, measured along and right of the runway.
    fn at(&self, along: f64, across: f64, altitude: f64) -> Waypoint {
        let east = self.origin.east + self.along.0 * along + self.right.0 * across;
        let north = self.origin.north + self.along.1 * along + self.right.1 * across;
        let latitude = self.reference_latitude + north / 111_320.0;
        let longitude = self.reference_longitude
            + east
                / (111_320.0
                    * (self.reference_latitude * std::f64::consts::PI / 180.0)
                        .cos()
                        .max(0.01));
        Waypoint {
            latitude,
            longitude,
            altitude_feet: altitude,
        }
    }
    /// The distance along and right of the runway of a point.
    fn local(&self, point: Waypoint) -> (f64, f64) {
        let north = (point.latitude - self.reference_latitude) * 111_320.0 - self.origin.north;
        let east = (point.longitude - self.reference_longitude)
            * 111_320.0
            * (self.reference_latitude * std::f64::consts::PI / 180.0)
                .cos()
                .max(0.01)
            - self.origin.east;
        (
            east * self.along.0 + north * self.along.1,
            east * self.right.0 + north * self.right.1,
        )
    }
    /// The point a given distance ahead of another point, on the same track.
    fn ahead(&self, point: Waypoint, distance: f64, altitude: f64) -> Waypoint {
        let (along, across) = self.local(point);
        self.at(along + distance, across, altitude)
    }
    /// End of the landing roll.
    fn rollout_end(&self) -> Waypoint {
        self.at(self.length.min(900.0), 0.0, self.elevation)
    }
    /// Top of the straight climb ahead of the runway.
    fn upwind_end(&self, from: &Waypoint) -> Waypoint {
        self.ahead(*from, UPWIND_METERS, self.pattern_altitude)
    }
    /// Start of the downwind leg.
    fn downwind_start(&self, from: &Waypoint) -> Waypoint {
        let (along, _) = self.local(self.upwind_end(from));
        self.at(along + CROSSWIND_METERS, OFFSET_METERS, self.pattern_altitude)
    }
    /// End of the downwind leg, past the threshold on the right of the runway.
    fn downwind_end(&self) -> Waypoint {
        self.at(-FINAL_METERS, OFFSET_METERS, self.pattern_altitude)
    }
    /// Start of the final approach, on the extended centreline.
    fn base_end(&self) -> Waypoint {
        self.at(-FINAL_METERS, 0.0, self.pattern_altitude)
    }
    /// Threshold of the runway.
    fn threshold(&self) -> Waypoint {
        self.at(0.0, 0.0, self.elevation)
    }
    /// The circuit itself: downwind, base, final and the landing roll.
    fn segments(&self, downwind_start: Waypoint) -> Vec<Segment> {
        vec![
            Segment {
                stage: Stage::Descent,
                points: vec![downwind_start, self.downwind_end()],
                knots: 90.0,
                resources: vec![],
                dwell_seconds: 0.0,
            },
            Segment {
                stage: Stage::Descent,
                points: vec![self.downwind_end(), self.base_end()],
                knots: 90.0,
                resources: vec![],
                dwell_seconds: 0.0,
            },
            Segment {
                stage: Stage::Approach,
                points: vec![self.base_end(), self.threshold()],
                knots: 85.0,
                resources: vec![],
                dwell_seconds: 0.0,
            },
            Segment {
                stage: Stage::Rollout,
                points: vec![self.threshold(), self.rollout_end()],
                knots: 70.0,
                resources: vec![self.runway_resource.clone()],
                dwell_seconds: 0.0,
            },
        ]
    }
}

/// A right-hand visual circuit for one aircraft at an untowered field.
pub struct Pattern {
    field: Field,
    callsign: String,
    runway: String,
    geometry: Geometry,
    circuit: Circuit,
    advisories: Vec<Advisory>,
}

impl Pattern {
    /// Build a right-hand circuit ending in a landing rollout on the named runway.
    ///
    /// The route is advisory traffic geometry only. It grants no movement and carries no
    /// controlled-clearance contract.
    pub fn new(
        airport: &Airport,
        runway_name: &str,
        callsign: &str,
        field: Field,
    ) -> Result<Self, String> {
        if field.airport != airport.icao {
            return Err("CTAF field belongs to another airport".into());
        }
        if callsign.trim().is_empty() {
            return Err("CTAF traffic needs a callsign".into());
        }
        let runway = airport
            .runways
            .iter()
            .find(|runway| runway.first_name == runway_name || runway.second_name == runway_name)
            .ok_or("CTAF pattern runway is unavailable")?;
        let (start, end) = if runway.first_name == runway_name {
            (runway.first, runway.second)
        } else {
            (runway.second, runway.first)
        };
        let (east, north) = (end.east - start.east, end.north - start.north);
        let length = east.hypot(north);
        if !length.is_finite() || length < 300.0 {
            return Err("CTAF pattern runway is too short for a circuit".into());
        }
        let (along_east, along_north) = (east / length, north / length);
        // Right normal in the local east/north plane.
        let (right_east, right_north) = (along_north, -along_east);
        let geometry = Geometry {
            reference_latitude: airport.reference_latitude,
            reference_longitude: airport.reference_longitude,
            origin: start,
            along: (along_east, along_north),
            right: (right_east, right_north),
            length,
            elevation: airport.elevation_feet,
            pattern_altitude: airport.elevation_feet + PATTERN_FEET,
            runway_resource: format!(
                "RUNWAY/{}/{}/{}",
                airport.icao, runway.first_name, runway.second_name
            ),
        };
        // The first circuit starts on the downwind, abeam the far end of the runway.
        let entry = geometry.at(length, OFFSET_METERS, geometry.pattern_altitude);
        let circuit = Self::compile(
            geometry.segments(entry),
            0,
            callsign.trim(),
            &field,
            runway_name,
        )?;
        let advisories = circuit
            .legs()
            .iter()
            .map(|leg| leg.advisory.clone())
            .collect::<Vec<_>>();
        Ok(Self {
            field,
            callsign: callsign.trim().to_owned(),
            runway: runway_name.to_owned(),
            geometry,
            circuit,
            advisories,
        })
    }

    /// Compile a circuit and the advisories made on it.
    fn compile(
        segments: Vec<Segment>,
        offset: usize,
        callsign: &str,
        field: &Field,
        runway_name: &str,
    ) -> Result<Circuit, String> {
        let route = Route::compile(segments, CLIMB_FPM, DESCENT_FPM)
            .map_err(|error| format!("CTAF pattern geometry: {error}"))?;
        let legs = LEGS
            .iter()
            .map(|(leg, phrase, segment)| Leg {
                segment: segment + offset,
                advisory: Advisory {
                    leg,
                    text: format!(
                        "Traffic, {callsign}, {}, {phrase}, runway {runway_name}",
                        field.airport
                    ),
                },
            })
            .collect::<Vec<_>>();
        Ok(Circuit {
            route: Arc::new(route),
            legs,
        })
    }

    /// Advisory frequency this pattern uses.
    #[must_use]
    pub fn field(&self) -> &Field {
        &self.field
    }

    /// Runway used by the circuit.
    #[must_use]
    pub fn runway(&self) -> &str {
        &self.runway
    }

    /// The runway resource the pattern holds during the landing roll.
    #[must_use]
    pub fn runway_resource(&self) -> &str {
        &self.geometry.runway_resource
    }

    /// Callsign making the advisories.
    #[must_use]
    pub fn callsign(&self) -> &str {
        &self.callsign
    }

    /// Geometry of the circuit. It carries no controlled clearance or takeoff grant.
    #[must_use]
    pub fn route(&self) -> Arc<Route> {
        self.circuit.route()
    }

    /// The first circuit, with the advisories made on it.
    #[must_use]
    pub fn circuit(&self) -> &Circuit {
        &self.circuit
    }

    /// One advisory per leg, in circuit order. Wording never contains a clearance.
    #[must_use]
    pub fn advisories(&self) -> &[Advisory] {
        &self.advisories
    }

    /// The circuit that follows the one just flown.
    ///
    /// The aircraft climbs from the end of the landing roll, turns onto the downwind and
    /// flies the same circuit again. The continuation is the same geometry every time, so a
    /// pattern can be repeated without a controller.
    pub fn next_circuit(&self) -> Result<Circuit, String> {
        self.circuit_from(self.geometry.rollout_end())
    }

    /// The circuit that follows a go-around from the position the aircraft abandoned.
    ///
    /// The aircraft climbs away from the runway and rejoins the downwind, so an occupied
    /// runway is never used.
    pub fn go_around_circuit(&self, from: Waypoint) -> Result<Circuit, String> {
        self.circuit_from(from)
    }

    /// Climb from a position back to the downwind, then fly the circuit.
    fn circuit_from(&self, entry: Waypoint) -> Result<Circuit, String> {
        let geometry = &self.geometry;
        let climb_end = geometry.upwind_end(&entry);
        let downwind_start = geometry.downwind_start(&entry);
        let mut segments = vec![
            Segment {
                stage: Stage::Climb,
                points: vec![entry, climb_end],
                knots: 70.0,
                resources: vec![],
                dwell_seconds: 0.0,
            },
            Segment {
                stage: Stage::Descent,
                points: vec![climb_end, downwind_start],
                knots: 90.0,
                resources: vec![],
                dwell_seconds: 0.0,
            },
        ];
        segments.extend(geometry.segments(downwind_start));
        Self::compile(
            segments,
            2,
            self.callsign.trim(),
            &self.field,
            self.runway.as_str(),
        )
    }

    /// Advisory when the runway is still occupied at the final call. The pilot goes around;
    /// no landing authority is created by this message.
    #[must_use]
    pub fn go_around(&self) -> Advisory {
        Advisory {
            leg: "final",
            text: format!(
                "Traffic, {}, {}, going around, runway {} occupied",
                self.callsign, self.field.airport, self.runway
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field_airport() -> Airport {
        let mut airport = crate::airport::demo_airport();
        airport.icao = "KCTF".into();
        airport.frequencies = vec![crate::airport::Frequency {
            service: "CTAF".into(),
            name: "CTAF".into(),
            khz: 122_900,
        }];
        airport
    }

    fn station(service: &str, khz: i32) -> Station {
        Station {
            airport: "KCTF".into(),
            service: service.into(),
            khz,
            ..Default::default()
        }
    }

    #[test]
    fn advisory_fields_refuse_any_towered_airport() {
        let airport = field_airport();
        let towered = [station("Tower", 118_300), station("Unicom", 122_800)];
        assert!(
            field(&airport, &towered).is_err(),
            "a towered field must never expose CTAF operations"
        );
        assert!(field(&airport, &[]).is_ok());
        let untowered = [station("Unicom", 122_800)];
        assert_eq!(field(&airport, &untowered).unwrap().frequency_khz, 122_800);
    }

    #[test]
    fn advisory_field_requires_a_published_frequency() {
        let mut airport = field_airport();
        airport.frequencies.clear();
        assert!(field(&airport, &[]).is_err());
        airport.frequencies.push(crate::airport::Frequency {
            service: "Unicom".into(),
            name: "Unicom".into(),
            khz: 122_700,
        });
        assert_eq!(field(&airport, &[]).unwrap().frequency_khz, 122_700);
    }

    #[test]
    fn only_untowered_fields_with_an_advisory_frequency_are_candidates() {
        let untowered = station("Unicom", 122_800);
        let towered = Station {
            airport: "KTOW".into(),
            service: "Tower".into(),
            khz: 118_300,
            ..Default::default()
        };
        let towered_advisory = Station {
            airport: "KTOW".into(),
            service: "CTAF".into(),
            khz: 122_900,
            ..Default::default()
        };
        let other = Station {
            airport: "KOTH".into(),
            service: "Ground".into(),
            khz: 121_900,
            ..Default::default()
        };
        let stations = [untowered.clone(), towered, towered_advisory, other];
        assert_eq!(candidates(&stations), vec!["KCTF".to_owned()]);
        assert!(candidates(&[]).is_empty());
    }

    #[test]
    fn a_pattern_announces_every_leg_and_never_grants_a_clearance() {
        let airport = field_airport();
        let field = field(&airport, &[]).unwrap();
        let runway = airport.runways[0].first_name.clone();
        let pattern = Pattern::new(&airport, &runway, "N12345", field).unwrap();
        assert_eq!(pattern.route().segment_count(), 4);
        assert_eq!(pattern.advisories().len(), 4);
        for advisory in pattern.advisories() {
            assert!(advisory.text.contains("N12345"));
            assert!(advisory.text.contains("KCTF"));
            assert!(advisory.text.contains(&runway));
            let lower = advisory.text.to_lowercase();
            assert!(!lower.contains("cleared"));
            assert!(!lower.contains("takeoff clearance"));
            assert!(!lower.contains("landing clearance"));
        }
        assert_eq!(pattern.advisories()[0].leg, "downwind");
        assert_eq!(pattern.advisories()[3].leg, "landing");
        let go_around = pattern.go_around();
        assert!(go_around.text.contains("going around"));
        assert!(!go_around.text.to_lowercase().contains("cleared"));
    }

    #[test]
    fn a_seeded_pattern_moves_on_its_first_advisory_leg_without_a_controller() {
        use crate::traffic::motion::World;
        let airport = field_airport();
        let field = field(&airport, &[]).unwrap();
        let runway = airport.runways[0].first_name.clone();
        let pattern = Pattern::new(&airport, &runway, "N12345", field).unwrap();
        let mut world = World::default();
        assert!(world.seed_airborne(0, 7, pattern.route(), 0));
        let start = world.snapshot()[0].clone();
        world.advance(0.0, false);
        world.advance(20.0, false);
        let moved = world.snapshot()[0].clone();
        assert_eq!(moved.stage, Stage::Descent);
        assert_ne!(moved.position, start.position);
        assert!(!moved.complete);
    }

    #[test]
    fn the_downwind_is_flown_against_the_landing_direction() {
        let airport = field_airport();
        let field = field(&airport, &[]).unwrap();
        let runway = airport.runways[0].first_name.clone();
        let pattern = Pattern::new(&airport, &runway, "N12345", field).unwrap();
        let route = pattern.route();
        let downwind = route.segment(0).unwrap();
        let final_approach = route.segment(2).unwrap();
        // The downwind ends past the threshold, on the right of the runway.
        let downwind_end = downwind.points[1];
        let landing_heading = crate::arrival::bearing(
            [final_approach.points[0].latitude, final_approach.points[0].longitude],
            [final_approach.points[1].latitude, final_approach.points[1].longitude],
        );
        let downwind_heading = crate::arrival::bearing(
            [downwind.points[0].latitude, downwind.points[0].longitude],
            [downwind_end.latitude, downwind_end.longitude],
        );
        let difference = (landing_heading - downwind_heading).abs();
        assert!(
            difference > 120.0 && difference < 240.0,
            "the downwind is flown against the landing direction, not with it"
        );
    }

    #[test]
    fn a_continuation_rejoins_the_downwind_from_the_end_of_the_landing_roll() {
        let airport = field_airport();
        let field = field(&airport, &[]).unwrap();
        let runway = airport.runways[0].first_name.clone();
        let pattern = Pattern::new(&airport, &runway, "N12345", field).unwrap();
        let first = pattern.circuit().clone();
        let next = pattern.next_circuit().unwrap();
        assert_eq!(next.route().segment_count(), 6);
        let last = first.route().end_position();
        let start = next.route().start_position();
        assert!(
            (last.latitude - start.latitude).abs() < 1e-6
                && (last.longitude - start.longitude).abs() < 1e-6
                && (last.altitude_feet - start.altitude_feet).abs() < 1.0,
            "the continuation starts where the landing roll ended"
        );
        // The continuation climbs away from the runway before it rejoins the downwind.
        let route = next.route();
        assert_eq!(route.segment(0).unwrap().stage, Stage::Climb);
        assert_eq!(route.segment(5).unwrap().stage, Stage::Rollout);
        // The same four advisories are made on the continuation, on the shifted segments.
        assert_eq!(next.legs().len(), 4);
        for (leg, original) in next.legs().iter().zip(first.legs()) {
            assert_eq!(leg.advisory.text, original.advisory.text);
            assert_eq!(leg.segment, original.segment + 2);
        }
        assert_eq!(next.advisory(2).unwrap().leg, "downwind");
        assert_eq!(next.advisory(5).unwrap().leg, "landing");
        // The continuation is the same geometry every time.
        let again = pattern.next_circuit().unwrap();
        assert_eq!(again.route().segment_count(), next.route().segment_count());
    }

    #[test]
    fn a_go_around_climbs_away_from_the_runway_and_never_uses_it() {
        let airport = field_airport();
        let field = field(&airport, &[]).unwrap();
        let runway = airport.runways[0].first_name.clone();
        let pattern = Pattern::new(&airport, &runway, "N12345", field).unwrap();
        let threshold = pattern.route().end_position();
        let circuit = pattern.go_around_circuit(threshold).unwrap();
        assert_eq!(circuit.route().segment_count(), 6);
        // The first leg climbs, so the aircraft leaves the runway before it rejoins.
        let route = circuit.route();
        let climb = route.segment(0).unwrap();
        assert_eq!(climb.stage, Stage::Climb);
        assert!(climb.points[1].altitude_feet > climb.points[0].altitude_feet);
        // The landing roll is still the last leg of the circuit.
        let rollout = route.segment(5).unwrap();
        assert_eq!(rollout.stage, Stage::Rollout);
    }
}
