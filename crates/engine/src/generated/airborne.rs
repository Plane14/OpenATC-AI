//! Solve terminal and en-route crossings together before creating movement limits.
use openatc_core::traffic::{
    motion::{Segment, Stage, Waypoint},
    trajectory::{self, Envelope, Fix},
};

/// Aircraft-specific published inputs. They are not a flyable trajectory or a clearance.
#[derive(Clone)]
pub(super) struct Decoded {
    pub candidate: usize,
    pub departure: Vec<Fix>,
    pub arrival: Vec<Fix>,
    pub approach: Vec<Fix>,
}

pub(super) struct Request<'a> {
    pub candidate: usize,
    pub terminal: &'a super::routing::Terminal,
    pub rotation: Waypoint,
    pub origin: &'a openatc_core::airport::Airport,
    pub destination: &'a openatc_core::airport::Airport,
    pub points: &'a openatc_core::traffic::coordinates::Points,
    pub departure_weather: &'a super::environment::Environment,
    pub arrival_weather: &'a super::environment::Environment,
    pub aircraft: &'a openatc_core::traffic::profiles::Aircraft,
    pub handling: &'a openatc_core::traffic::animation::Handling,
    pub intercept_limit_nm: f64,
    pub departure_floor_feet: f64,
    pub arrival_floor_feet: f64,
}

/// Decode the selected fleet's actual SID, STAR and approach on the preparation worker.
pub(super) fn resolve(input: Request<'_>) -> Result<Decoded, String> {
    use openatc_core::traffic::resolved::{self, Context};
    if !input.departure_weather.fresh() || !input.arrival_weather.fresh() {
        return Err("Terminal planning needs fresh weather at both airports".into());
    }
    if ![input.departure_floor_feet, input.arrival_floor_feet]
        .iter()
        .all(|height| height.is_finite() && (-3000.0..=20000.0).contains(height))
    {
        return Err("Terminal planning has an invalid aerodrome vertical bound".into());
    }
    if !input.aircraft.rnav {
        return Err("Generated terminal planning needs supported navigation equipment".into());
    }
    let departure_ceiling = input
        .departure_weather
        .atmosphere
        .geometric(input.aircraft.ceiling, true)?;
    let arrival_ceiling = input
        .arrival_weather
        .atmosphere
        .geometric(input.aircraft.ceiling, true)?;
    let floor = f64::from(input.terminal.enroute_minimum_feet);
    if floor <= 0.0
        || input.departure_weather.atmosphere.geometric(floor, false)? > departure_ceiling
        || input.arrival_weather.atmosphere.geometric(floor, false)? > arrival_ceiling
    {
        return Err("Terminal connection exceeds this aircraft's ceiling".into());
    }
    let context =
        |airport, weather: &super::environment::Environment, floor, ceiling, speed| Context {
            airport,
            points: input.points,
            atmosphere: weather.atmosphere,
            variation: weather.variation,
            wind_from: weather.wind_from,
            wind_knots: weather.wind_knots,
            indicated_knots: speed,
            maximum_intercept_nm: input.intercept_limit_nm,
            climb_fpm: input.aircraft.climb_fpm,
            minimum_feet: floor,
            maximum_feet: ceiling,
            preferred_feet: ceiling,
        };
    let departure = resolved::boundary_crossings(
        input.rotation,
        &input.terminal.departure,
        &context(
            input.origin,
            input.departure_weather,
            input.departure_floor_feet,
            departure_ceiling,
            input
                .handling
                .rotation_knots
                .max(input.aircraft.approach_knots),
        ),
    )?;
    let end = departure.last().ok_or("Resolved departure is empty")?;
    if openatc_core::arrival::distance_nm(end.coordinate, input.terminal.enroute_from) >= 0.001 {
        return Err("Resolved SID does not reach its selected enroute connection".into());
    }
    let arrival_context = context(
        input.destination,
        input.arrival_weather,
        input.arrival_floor_feet,
        arrival_ceiling,
        input.aircraft.approach_knots,
    );
    let arrival = resolved::boundary_crossings(
        Waypoint {
            latitude: input.terminal.enroute_to[0],
            longitude: input.terminal.enroute_to[1],
            altitude_feet: arrival_ceiling,
        },
        &input.terminal.arrival,
        &arrival_context,
    )?;
    let entry = arrival.last().ok_or("Resolved arrival is empty")?;
    let approach = resolved::boundary_crossings(
        Waypoint {
            latitude: entry.coordinate[0],
            longitude: entry.coordinate[1],
            altitude_feet: entry.preferred_feet,
        },
        &input.terminal.approach,
        &arrival_context,
    )?;
    let first = approach.first().ok_or("Resolved approach is empty")?;
    if openatc_core::arrival::distance_nm(entry.coordinate, first.coordinate) >= 0.001 {
        return Err("Resolved STAR and approach no longer share their published entry".into());
    }
    if departure.len() + arrival.len() + approach.len() + 1 > 128 {
        return Err(
            "Aircraft terminal crossings exceed the combined flight planning budget".into(),
        );
    }
    if !input.departure_weather.fresh() || !input.arrival_weather.fresh() {
        return Err("Terminal weather expired during preparation".into());
    }
    Ok(Decoded {
        candidate: input.candidate,
        departure,
        arrival,
        approach,
    })
}

/// Aircraft-specific production planning context; candidate attempts have no world side effects.
pub(super) struct Planner<'a> {
    pub rotation: Waypoint,
    pub origin: &'a openatc_core::airport::Airport,
    pub destination: &'a openatc_core::airport::Airport,
    pub points: &'a openatc_core::traffic::coordinates::Points,
    pub departure_weather: &'a super::environment::Environment,
    pub arrival_weather: &'a super::environment::Environment,
    pub aircraft: &'a openatc_core::traffic::profiles::Aircraft,
    pub handling: &'a openatc_core::traffic::animation::Handling,
    pub policy: &'a super::Operations,
    pub departure_surface: &'a super::runway::Surface,
    pub arrival_surface: &'a super::runway::Surface,
    pub observed: &'a [openatc_core::state::TrafficTarget],
    pub reserved: &'a std::collections::BTreeSet<String>,
}
impl Planner<'_> {
    /// Select only after the published crossing connects through landing and parking.
    /// A decodable approach alone is insufficient to consume a flight's candidate slot.
    pub fn arrival(
        &self,
        candidates: &[super::routing::Terminal],
    ) -> Result<(Decoded, super::landing::Arrival), String> {
        let departure_floor_feet = self.origin.elevation_feet.min(
            self.departure_weather.atmosphere.geometric(
                self.departure_surface.height(
                    self.departure_surface
                        .point(self.departure_surface.threshold),
                ),
                false,
            )?,
        );
        let arrival_floor_feet = self.destination.elevation_feet.min(
            self.arrival_weather.atmosphere.geometric(
                self.arrival_surface
                    .height(self.arrival_surface.point(self.arrival_surface.threshold)),
                false,
            )?,
        );
        let mut reason = "No terminal candidate is available".to_owned();
        let mut attempted = 0;
        for (candidate, terminal) in candidates
            .iter()
            .take(self.policy.terminal_candidate_limit)
            .enumerate()
        {
            attempted += 1;
            let attempt = (|| {
                let decoded = resolve(Request {
                    candidate,
                    terminal,
                    rotation: self.rotation,
                    origin: self.origin,
                    destination: self.destination,
                    points: self.points,
                    departure_weather: self.departure_weather,
                    arrival_weather: self.arrival_weather,
                    aircraft: self.aircraft,
                    handling: self.handling,
                    intercept_limit_nm: self.policy.terminal_intercept_limit_nm,
                    departure_floor_feet,
                    arrival_floor_feet,
                })?;
                let crossing = decoded
                    .approach
                    .last()
                    .ok_or("Approach has no threshold crossing")?;
                let angle = terminal
                    .approach
                    .last()
                    .and_then(|leg| leg.vertical_angle_degrees)
                    .ok_or("Approach has no published final descent angle")?;
                let arrival = super::landing::prepare(super::landing::Request {
                    airport: self.destination,
                    surface: self.arrival_surface,
                    crossing: Waypoint {
                        latitude: crossing.coordinate[0],
                        longitude: crossing.coordinate[1],
                        altitude_feet: crossing.preferred_feet,
                    },
                    angle_degrees: angle,
                    airspeed: crossing.airspeed.ok_or("Approach airspeed is unresolved")?,
                    aircraft: self.aircraft,
                    handling: self.handling,
                    policy: self.policy,
                    observed: self.observed,
                    reserved: self.reserved,
                })?;
                if !self.departure_weather.fresh() || !self.arrival_weather.fresh() {
                    return Err("Terminal weather expired before arrival selection".to_owned());
                }
                Ok((decoded, arrival))
            })();
            match attempt {
                Ok(prepared) => return Ok(prepared),
                Err(error) => {
                    reason = format!(
                        "{}/{}/{}: {error}",
                        terminal.departure_name, terminal.arrival_name, terminal.approach_name
                    )
                }
            }
        }
        Err(format!(
            "No complete arrival among {attempted} terminal candidates: {reason}"
        ))
    }
}

pub(super) struct Section {
    pub stage: Stage,
    pub crossings: Vec<Fix>,
}

/// Published levels in later sections constrain the earlier climb and cruise too.
/// Callers must supply verified corridor protection, not a permissive placeholder.
pub(super) fn compile(
    rotation: Waypoint,
    departure_heading: f64,
    runway_heading: f64,
    sections: Vec<Section>,
    envelope: &Envelope,
    protected: impl Fn(Waypoint, Waypoint) -> bool,
) -> Result<Vec<Segment>, String> {
    if sections.len() != 4
        || sections.iter().map(|section| section.stage).ne([
            Stage::Climb,
            Stage::Cruise,
            Stage::Descent,
            Stage::Approach,
        ])
        || sections.iter().any(|section| section.crossings.is_empty())
    {
        return Err(
            "Airborne plan needs ordered departure, en-route, arrival and approach sections".into(),
        );
    }
    let count: usize = sections.iter().map(|section| section.crossings.len()).sum();
    if count > 128 {
        return Err("Combined airborne crossings exceed the planning budget".into());
    }
    let mut stages = Vec::with_capacity(count);
    let mut crossings: Vec<Fix> = Vec::with_capacity(count);
    for section in sections {
        for crossing in section.crossings {
            if let Some(previous) = crossings.last_mut()
                && openatc_core::arrival::distance_nm(previous.coordinate, crossing.coordinate)
                    < 0.001
            {
                merge(previous, &crossing)?;
            } else {
                stages.push(section.stage);
                crossings.push(crossing);
            }
        }
    }
    let legs = trajectory::compile(
        rotation,
        departure_heading,
        &crossings,
        runway_heading,
        envelope,
        protected,
    )?;
    let segments = legs
        .into_iter()
        .zip(stages)
        .map(|(leg, stage)| Segment {
            stage,
            points: leg.points,
            knots: leg.ground_knots,
            resources: Vec::new(),
            dwell_seconds: 0.0,
        })
        .collect::<Vec<_>>();
    // The complete movement route is compiled only after a real rollout and taxi-in are attached.
    Ok(segments)
}

fn merge(previous: &mut Fix, next: &Fix) -> Result<(), String> {
    let minimum = previous.minimum_feet.max(next.minimum_feet);
    let maximum = previous.maximum_feet.min(next.maximum_feet);
    if minimum > maximum {
        return Err("Coincident flight sections have conflicting crossing levels".into());
    }
    if previous
        .arrival_heading
        .zip(next.arrival_heading)
        .is_some_and(|(a, b)| openatc_core::navigation::turn(a, b).abs() > 1.0)
        || previous
            .required_turn
            .zip(next.required_turn)
            .is_some_and(|(a, b)| a != b)
    {
        return Err("Coincident flight sections have conflicting courses or turns".into());
    }
    // Do not discard a minimum-speed restriction by selecting the slower of two joins.
    // A speed change at the same point needs an upstream, aircraft-specific deceleration plan.
    match (previous.airspeed, next.airspeed) {
        (Some(a), Some(b)) if a == b => {}
        (None, None) if (previous.ground_knots - next.ground_knots).abs() <= 1e-6 => {}
        _ => {
            return Err(
                "Coincident flight sections have incompatible airspeed or weather inputs".into(),
            );
        }
    }
    previous.minimum_feet = minimum;
    previous.maximum_feet = maximum;
    previous.preferred_feet = next.preferred_feet.clamp(minimum, maximum);
    previous.arrival_heading = previous.arrival_heading.or(next.arrival_heading);
    previous.required_turn = previous.required_turn.or(next.required_turn);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fleet_terminal_inputs_keep_restrictions_and_lower_runway_thresholds() {
        use openatc_core::{
            airport::{Airport, Runway},
            state::Point,
            traffic::{coordinates::Points, procedures::Catalogue},
        };
        let leg = |kind: &str, runway: &str, altitude: &str| {
            let mut fields = vec![""; 38];
            fields[0] = "010";
            fields[1] = "1";
            fields[2] = "TEST";
            fields[11] = kind;
            fields[20] = "0000";
            fields[22] = " ";
            fields[23] = altitude;
            let mut leg = Catalogue::parse(&format!("SID:{};", fields.join(",")))
                .unwrap()
                .0
                .into_values()
                .next()
                .unwrap()
                .remove(0);
            leg.identifier = format!("RW{runway}");
            leg.section = "P".into();
            leg.subsection = "G".into();
            leg
        };
        let origin = Airport {
            icao: "ORIG".into(),
            runways: vec![Runway {
                first_name: "09".into(),
                second_name: "27".into(),
                width: 45.0,
                first: Point {
                    north: 1000.0,
                    ..Default::default()
                },
                second: Point {
                    north: 1500.0,
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        };
        let destination = Airport {
            icao: "DEST".into(),
            reference_latitude: 1.0,
            elevation_feet: 100.0,
            ..origin.clone()
        };
        let weather = |airport: &Airport| {
            super::super::environment::Environment::from_sample(airport,&(
            serde_json::json!({"airport":airport.icao,"latitude":airport.reference_latitude,"longitude":airport.reference_longitude,
                "sampleAltitudeFeet":airport.elevation_feet,"pressureHpa":1013.25,"temperatureC":15.0,
                "magneticVariationDegrees":0.0,"windDegrees":0.0,"windKnots":0.0}),std::time::Instant::now())).unwrap()
        };
        let departure_weather = weather(&origin);
        let arrival_weather = weather(&destination);
        let departure = leg("TF", "09", "01000");
        let arrival = leg("IF", "09", "04000");
        let final_fix = leg("TF", "27", "00050");
        let points = Points::default();
        let terminal = super::super::routing::Terminal {
            enroute_from: Points::runway(&departure, &origin).unwrap().coordinate,
            enroute_to: Points::runway(&arrival, &destination).unwrap().coordinate,
            enroute_minimum_feet: 1000,
            departure_name: "TEST".into(),
            departure_transition: String::new(),
            departure: vec![departure],
            arrival_name: "TEST".into(),
            arrival_transition: String::new(),
            arrival: vec![arrival.clone()],
            approach_name: "TEST".into(),
            approach_transition: String::new(),
            approach: vec![arrival, final_fix],
            missed: vec![],
        };
        let aircraft = openatc_core::traffic::profiles::aircraft("C172")
            .unwrap()
            .clone();
        let handling = &openatc_core::traffic::animation::builtin().unwrap()
            [&openatc_core::traffic::animation::Kind::Piston];
        let request = |aircraft, terminal, floor| Request {
            candidate: 0,
            terminal,
            rotation: Waypoint {
                latitude: 0.0,
                longitude: 0.0,
                altitude_feet: 0.0,
            },
            origin: &origin,
            destination: &destination,
            points: &points,
            departure_weather: &departure_weather,
            arrival_weather: &arrival_weather,
            aircraft,
            handling,
            intercept_limit_nm: 20.0,
            departure_floor_feet: 0.0,
            arrival_floor_feet: floor,
        };
        assert!(
            resolve(request(&aircraft, &terminal, destination.elevation_feet)).is_err(),
            "Airport maximum elevation is above this runway's approach crossing"
        );
        let decoded = resolve(request(&aircraft, &terminal, 0.0)).unwrap();
        let expected = arrival_weather.atmosphere.geometric(4000.0, false).unwrap();
        assert_eq!(decoded.arrival.last().unwrap().maximum_feet, expected);
        assert_eq!(decoded.approach.first().unwrap().maximum_feet, expected);
        assert_eq!(
            decoded
                .approach
                .last()
                .unwrap()
                .airspeed
                .unwrap()
                .indicated_knots,
            aircraft.approach_knots
        );
        let mut too_low = aircraft.clone();
        too_low.ceiling = 500.0;
        assert!(
            resolve(request(&too_low, &terminal, 0.0))
                .err()
                .unwrap()
                .contains("ceiling")
        );
        let mut unsupported = terminal.clone();
        unsupported.departure[0].terminator = "RF".into();
        assert!(resolve(request(&aircraft, &unsupported, 0.0)).is_err());
    }

    fn inputs() -> (Waypoint, Vec<Section>, Envelope) {
        let start = Waypoint {
            latitude: 0.0,
            longitude: 0.0,
            altitude_feet: 0.0,
        };
        let sections = [
            (Stage::Climb, 0.1, 0.0, 6000.0, 6000.0),
            (Stage::Cruise, 0.2, 0.0, 6000.0, 6000.0),
            (Stage::Descent, 0.3, 0.0, 1000.0, 1000.0),
            (Stage::Approach, 0.4, 0.0, 0.0, 0.0),
        ]
        .into_iter()
        .map(
            |(stage, latitude, minimum_feet, maximum_feet, preferred_feet)| Section {
                stage,
                crossings: vec![Fix {
                    airspeed: None,
                    coordinate: [latitude, 0.0],
                    arrival_heading: Some(0.0),
                    minimum_feet,
                    maximum_feet,
                    preferred_feet,
                    ground_knots: 120.0,
                    required_turn: None,
                }],
            },
        )
        .collect();
        (
            start,
            sections,
            Envelope {
                bank_degrees: 20.0,
                turn_rate_degrees_sec: 3.0,
                climb_fpm: 2000.0,
                descent_fpm: 1000.0,
                spacing_meters: 250.0,
                maximum_detour_meters: 100.0,
            },
        )
    }

    #[test]
    fn arrival_ceiling_shapes_the_preceding_enroute_section() {
        let (start, sections, envelope) = inputs();
        let segments = compile(start, 0.0, 0.0, sections, &envelope, |a, b| {
            a.longitude.abs() < 0.001 && b.longitude.abs() < 0.001
        })
        .unwrap();
        assert_eq!(segments.len(), 4);
        let touchdown = *segments.last().unwrap().points.last().unwrap();
        let vacated = Waypoint {
            latitude: touchdown.latitude + 0.01,
            ..touchdown
        };
        let parked = Waypoint {
            longitude: touchdown.longitude + 0.01,
            ..vacated
        };
        let mut complete = segments.clone();
        complete.extend([
            Segment {
                stage: Stage::Rollout,
                points: vec![touchdown, vacated],
                knots: 40.0,
                resources: vec!["RUNWAY/TEST/01/19".into()],
                dwell_seconds: 0.0,
            },
            Segment {
                stage: Stage::TaxiIn,
                points: vec![vacated, parked],
                knots: 5.0,
                resources: vec![],
                dwell_seconds: 0.0,
            },
            Segment {
                stage: Stage::Turnaround,
                points: vec![parked],
                knots: 0.0,
                resources: vec![],
                dwell_seconds: 120.0,
            },
        ]);
        openatc_core::traffic::motion::Route::compile(
            complete,
            envelope.climb_fpm,
            envelope.descent_fpm,
        )
        .unwrap();

        let cruise_end = segments[1].points.last().unwrap().altitude_feet;
        assert!(
            cruise_end < 6000.0,
            "Cruise retained an unreachable later ceiling"
        );
        assert!(segments[2].points.last().unwrap().altitude_feet <= 1000.0);
        assert_eq!(segments[3].points.last().unwrap().altitude_feet, 0.0);
        assert!(
            segments
                .windows(2)
                .all(|pair| pair[0].points.last() == pair[1].points.first())
        );
    }

    #[test]
    fn a_repeated_approach_entry_limits_the_incoming_star_leg() {
        let (start, mut sections, envelope) = inputs();
        let mut initial = sections[2].crossings[0].clone();
        initial.maximum_feet = 500.0;
        initial.preferred_feet = 500.0;
        sections[3].crossings.insert(0, initial);
        let segments = compile(start, 0.0, 0.0, sections, &envelope, |_, _| true).unwrap();
        assert_eq!(segments.len(), 4);
        assert_eq!(segments[2].points.last().unwrap().altitude_feet, 500.0);
        let (start, mut sections, envelope) = inputs();
        let mut initial = sections[2].crossings[0].clone();
        initial.ground_knots = 80.0;
        sections[3].crossings.insert(0, initial);
        assert!(compile(start, 0.0, 0.0, sections, &envelope, |_, _| true).is_err());
    }

    #[test]
    fn compatible_ias_survives_a_lower_star_approach_join() {
        use openatc_core::traffic::{air::Atmosphere, trajectory::Airspeed};
        let (start, mut sections, envelope) = inputs();
        let speed = Airspeed {
            indicated_knots: 120.0,
            atmosphere: Atmosphere::surface(1013.25, 15.0, 0.0).unwrap(),
            wind_from: 0.0,
            wind_knots: 10.0,
        };
        sections[2].crossings[0].airspeed = Some(speed);
        sections[2].crossings[0].ground_knots = speed.ground_knots(1000.0, 0.0).unwrap();
        let mut entry = sections[2].crossings[0].clone();
        entry.maximum_feet = 500.0;
        entry.preferred_feet = 500.0;
        entry.ground_knots = speed.ground_knots(500.0, 0.0).unwrap();
        sections[3].crossings.insert(0, entry);
        let path = compile(start, 0.0, 0.0, sections, &envelope, |a, b| {
            a.longitude.abs() < 0.001 && b.longitude.abs() < 0.001
        })
        .unwrap();
        assert_eq!(path[2].points.last().unwrap().altitude_feet, 500.0);
        assert!((path[2].knots - speed.ground_knots(500.0, 0.0).unwrap()).abs() < 0.01);
    }
    #[test]
    fn unprotected_or_incomplete_flights_do_not_create_movement() {
        let (start, sections, envelope) = inputs();
        assert!(
            compile(start, 0.0, 0.0, sections, &envelope, |_, b| b.latitude
                < 0.25)
            .is_err()
        );
        let (start, mut sections, envelope) = inputs();
        sections.pop();
        assert!(compile(start, 0.0, 0.0, sections, &envelope, |_, _| true).is_err());
    }
}
