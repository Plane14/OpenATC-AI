//! Nominal published missed approach from the MAP to its stated holding fix.
use openatc_core::{
    arrival::distance_nm,
    traffic::{
        holding,
        motion::{Route, Segment, Stage, Waypoint},
        resolved::{self, Context},
        trajectory::{self, Envelope},
    },
};

#[derive(Clone)]
pub(super) struct Draft {
    pub map: Waypoint,
    pub holding_fix: String,
    pub holding_feet: f64,
    path: trajectory::Draft,
    hold: holding::Draft,
    climb_fpm: f64,
    descent_fpm: f64,
}
impl Draft {
    /// This checks both the missed path and the whole hold before returning a route.
    pub fn protect(self, protected: impl Fn(Waypoint, Waypoint) -> bool) -> Result<Route, String> {
        let mut segments = self
            .path
            .protect(&protected)?
            .into_iter()
            .map(|leg| Segment {
                stage: Stage::MissedApproach,
                points: leg.points,
                knots: leg.ground_knots,
                resources: Vec::new(),
                dwell_seconds: 0.0,
            })
            .collect::<Vec<_>>();
        segments.push(self.hold.protect(protected)?);
        Route::compile(segments, self.climb_fpm, self.descent_fpm)
    }
}

/// This seed starts at the published MAP, not at an arbitrary early or late go-around position.
pub(super) fn prepare(
    planner: &super::airborne::Planner<'_>,
    terminal: &super::routing::Terminal,
    map: &trajectory::Fix,
    minimum_feet: f64,
) -> Result<Draft, String> {
    let map_leg = terminal
        .approach
        .last()
        .filter(|leg| leg.missed_point())
        .ok_or("Approach has no explicit published missed-approach point")?;
    let position = openatc_core::traffic::coordinates::Points::runway(map_leg, planner.destination)
        .or_else(|| planner.points.resolve(map_leg, &planner.destination.icao))
        .ok_or("Published missed-approach point is unresolved")?;
    if distance_nm(map.coordinate, position.coordinate) >= 0.001 {
        return Err("Nominal missed approach must start at its published MAP".into());
    }
    let (published, prefix) = terminal
        .missed
        .split_last()
        .ok_or("Missed approach is missing")?;
    if published.terminator != "HM"
        || prefix.is_empty()
        || prefix.iter().any(|leg| leg.terminator == "HM")
        || published.altitude_one.is_none()
    {
        return Err(
            "Missed approach needs a complete path and an explicit terminal hold level".into(),
        );
    }
    let weather = planner.arrival_weather;
    if !weather.fresh() {
        return Err("Missed approach weather is stale".into());
    }
    let start = Waypoint {
        latitude: map.coordinate[0],
        longitude: map.coordinate[1],
        altitude_feet: map.preferred_feet,
    };
    let map_speed = map.airspeed.ok_or("Missed approach has no MAP airspeed")?;
    if map_speed.atmosphere != weather.atmosphere
        || map_speed.wind_from != weather.wind_from
        || map_speed.wind_knots != weather.wind_knots
    {
        return Err("Missed approach weather does not match its MAP inputs".into());
    }
    let indicated = published
        .speed_knots
        .map_or(map_speed.indicated_knots, |limit| {
            map_speed.indicated_knots.min(limit)
        });
    if indicated < planner.aircraft.approach_knots {
        return Err("Published hold speed is below the fleet approach envelope".into());
    }
    let context = Context {
        airport: planner.destination,
        points: planner.points,
        atmosphere: weather.atmosphere,
        variation: weather.variation,
        wind_from: weather.wind_from,
        wind_knots: weather.wind_knots,
        indicated_knots: indicated,
        maximum_intercept_nm: planner.policy.terminal_intercept_limit_nm,
        climb_fpm: planner.aircraft.climb_fpm,
        minimum_feet,
        maximum_feet: weather
            .atmosphere
            .geometric(planner.aircraft.ceiling, true)?,
        preferred_feet: start.altitude_feet,
    };
    let (low, high) = resolved::altitude_bounds(published, &context)?;
    let level = low.max(start.altitude_feet);
    if level > high {
        return Err("MAP altitude conflicts with the published holding level".into());
    }
    let standard = published
        .altitude_one_is_flight_level
        .ok_or("Holding pressure reference is unresolved")?;
    if published.altitude_two.is_some() && published.altitude_two_is_flight_level != Some(standard)
    {
        return Err("Mixed holding pressure references need an explicit clearance solution".into());
    }
    let mut position = planner
        .points
        .resolve(published, &planner.destination.icao)
        .ok_or("Missed approach holding fix is unresolved")?;
    position.magnetic_variation_degrees = Some(
        planner
            .points
            .course_variation(published, &planner.destination.icao)
            .unwrap_or(weather.variation),
    );
    let course = (published.course.ok_or("Missed holding course is missing")?
        + position.magnetic_variation_degrees.unwrap())
    .rem_euclid(360.0);
    let airspeed = trajectory::Airspeed {
        indicated_knots: indicated,
        ..map_speed
    };
    let pattern = holding::Pattern::published(
        published,
        position,
        &holding::Clearance {
            level_feet: weather.atmosphere.reported(level, standard)?.round() as i32,
            geometric_feet: level,
            indicated_knots: indicated,
            ground_knots: airspeed.ground_knots(level, course)?,
            bank_degrees: planner.handling.maximum_bank_degrees,
            turn_rate_degrees_sec: planner.policy.holding_turn_rate_degrees_sec,
        },
    )?;
    let mut fixes = resolved::boundary_crossings(start, prefix, &context)?;
    let last = fixes
        .last_mut()
        .ok_or("Missed approach has no holding entry")?;
    if distance_nm(last.coordinate, position.coordinate) >= 0.001 {
        return Err("Missed approach does not reach its named holding fix".into());
    }
    if last
        .arrival_heading
        .is_some_and(|heading| openatc_core::navigation::turn(heading, course).abs() > 1.0)
        || last.airspeed != Some(airspeed)
        || level < last.minimum_feet
        || level > last.maximum_feet
    {
        return Err("Missed approach requires a separate holding entry or speed transition".into());
    }
    // The HM level applies to the incoming leg. Its turn direction belongs to
    // the racetrack and must not replace a preceding missed-approach turn.
    last.minimum_feet = level;
    last.maximum_feet = level;
    last.preferred_feet = level;
    last.arrival_heading = Some(course);
    last.ground_knots = pattern.ground_knots;
    let path = trajectory::draft(
        start,
        planner.arrival_surface.heading(),
        &fixes,
        course,
        &Envelope {
            bank_degrees: planner.handling.maximum_bank_degrees,
            turn_rate_degrees_sec: planner.policy.airborne_turn_rate_degrees_sec,
            climb_fpm: planner.aircraft.climb_fpm,
            descent_fpm: planner.aircraft.descent_fpm,
            spacing_meters: planner.policy.airborne_sample_meters,
            maximum_detour_meters: planner.policy.airborne_detour_meters,
        },
    )?;
    let hold = pattern.draft()?;
    if !weather.fresh() {
        return Err("Missed approach weather expired during preparation".into());
    }
    Ok(Draft {
        map: start,
        holding_fix: published.identifier.clone(),
        holding_feet: level,
        path,
        hold,
        climb_fpm: planner.aircraft.climb_fpm,
        descent_fpm: planner.aircraft.descent_fpm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use openatc_core::{
        airport::{Airport, Runway},
        arrival::Threshold,
        state::Point,
        traffic::{
            animation, coordinates::Points, procedures::Catalogue, profiles, trajectory::Fix,
        },
    };
    use std::{
        collections::BTreeSet,
        time::{Instant, SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn published_missed_seed_requires_its_map_fix_level_and_protected_hold() {
        let record = |sequence: &str, id: &str, kind: &str, altitude: &str, description: &str| {
            let mut fields = vec![""; 38];
            fields[0] = sequence;
            fields[1] = "I";
            fields[2] = "TEST";
            fields[4] = id;
            fields[5] = "ZZ";
            fields[6] = "P";
            fields[7] = if id.starts_with("RW") { "G" } else { "C" };
            fields[8] = description;
            fields[11] = kind;
            fields[20] = "0000";
            fields[22] = "+";
            fields[23] = altitude;
            format!("APPCH:{};", fields.join(","))
        };
        let text = [
            record("010", "RW36", "CF", "00050", "G  M"),
            record("020", "RW36", "FA", "01000", "G M "),
            record("030", "HOLD", "TF", "02000", "E   "),
            record("040", "HOLD", "HM", "02000", "VE H"),
        ]
        .join("\n");
        let mut catalogue = Catalogue::parse(&text).unwrap();
        let legs = catalogue.0.values_mut().next().unwrap();
        let hold = legs.last_mut().unwrap();
        hold.turn = "R".into();
        hold.distance_time = "T010".into();
        hold.speed_knots = Some(100.0);
        hold.speed_description = "-".into();
        let airport = Airport {
            icao: "TEST".into(),
            runways: vec![Runway {
                first_name: "36".into(),
                second_name: "18".into(),
                width: 45.0,
                second: Point {
                    north: 3000.0,
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut points = Points::requested(&[("TEST", &catalogue)]);
        let root = std::env::temp_dir().join(format!(
            "openatc-missed-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let custom = root.join("Custom Data");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(
            custom.join("earth_fix.dat"),
            "I\n1200 Version - data cycle 2610\n0.1 0.0 HOLD TEST ZZ\n99\n",
        )
        .unwrap();
        std::fs::write(
            custom.join("earth_nav.dat"),
            "I\n1200 Version - data cycle 2610\n99\n",
        )
        .unwrap();
        points.load(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        let legs = catalogue.0.values().next().unwrap();
        let terminal = super::super::routing::Terminal {
            enroute_from: [0.0, 0.0],
            enroute_to: [0.0, 0.0],
            enroute_minimum_feet: 1000,
            departure_name: String::new(),
            departure_transition: String::new(),
            departure: vec![],
            arrival_name: String::new(),
            arrival_transition: String::new(),
            arrival: vec![],
            approach_name: "TEST".into(),
            approach_transition: String::new(),
            approach: vec![legs[0].clone()],
            missed: legs[1..].to_vec(),
        };
        let weather=super::super::environment::Environment::from_sample(&airport,&(
            serde_json::json!({"airport":"TEST","latitude":0.0,"longitude":0.0,"sampleAltitudeFeet":0.0,
                "pressureHpa":1013.25,"temperatureC":15.0,"magneticVariationDegrees":0.0,"windDegrees":0.0,"windKnots":0.0}),Instant::now(),
        )).unwrap();
        let heights = [
            Threshold {
                runway: "36".into(),
                elevation_feet: 0.0,
            },
            Threshold {
                runway: "18".into(),
                elevation_feet: 0.0,
            },
        ];
        let surface = super::super::runway::Surface::new(&airport, "36", &heights, 2.0).unwrap();
        let policy =
            super::super::parse_operations(include_str!("../../../../traffic/operations.toml"))
                .unwrap();
        let handling = animation::builtin().unwrap();
        let mora = Default::default();
        let reserved = BTreeSet::new();
        let planner = super::super::airborne::Planner {
            rotation: Waypoint {
                latitude: 0.0,
                longitude: 0.0,
                altitude_feet: 50.0,
            },
            origin: &airport,
            destination: &airport,
            points: &points,
            mora: &mora,
            departure_weather: &weather,
            arrival_weather: &weather,
            aircraft: profiles::aircraft("C172").unwrap(),
            handling: &handling[&animation::Kind::Piston],
            policy: &policy,
            departure_surface: &surface,
            arrival_surface: &surface,
            observed: &[],
            reserved: &reserved,
        };
        let speed = trajectory::Airspeed {
            indicated_knots: planner.aircraft.approach_knots,
            atmosphere: weather.atmosphere,
            wind_from: 0.0,
            wind_knots: 0.0,
        };
        let map = Fix {
            coordinate: [0.0, 0.0],
            arrival_heading: Some(0.0),
            minimum_feet: 50.0,
            maximum_feet: 50.0,
            preferred_feet: 50.0,
            ground_knots: speed.ground_knots(50.0, 0.0).unwrap(),
            airspeed: Some(speed),
            required_turn: None,
        };
        let draft = prepare(&planner, &terminal, &map, 0.0).unwrap();
        assert_eq!(draft.holding_fix, "HOLD");
        assert!(draft.clone().protect(|_, _| false).is_err());
        // Geometry-only inspection; no terrain or obstacle protection is asserted.
        let route = draft.clone().protect(|_, _| true).unwrap();
        assert_eq!(route.segment(0).unwrap().stage, Stage::MissedApproach);
        assert_eq!(route.segment(0).unwrap().points[0], draft.map);
        let hold = (0..512)
            .map_while(|index| route.segment(index))
            .last()
            .unwrap();
        assert_eq!(hold.stage, Stage::Holding);
        assert_eq!(
            hold.points[0].altitude_feet,
            weather.atmosphere.geometric(2000.0, false).unwrap()
        );
        let edge = [hold.points[0], hold.points[1]];
        assert!(draft.protect(|a, b| [a, b] != edge).is_err());
        let mut early = map.clone();
        early.coordinate[0] -= 0.01;
        assert!(prepare(&planner, &terminal, &early, 0.0).is_err());
        let mut bad = terminal.clone();
        bad.missed[0].terminator = "RF".into();
        assert!(prepare(&planner, &bad, &map, 0.0).is_err());
        bad = terminal.clone();
        bad.missed.last_mut().unwrap().region = "XX".into();
        assert!(prepare(&planner, &bad, &map, 0.0).is_err());
        bad = terminal.clone();
        bad.missed.last_mut().unwrap().speed_knots = Some(planner.aircraft.approach_knots - 1.0);
        assert!(prepare(&planner, &bad, &map, 0.0).is_err());
    }
}
