//! Decode terminal crossings without discarding unsupported path terminators.
use super::{
    air::{self, Atmosphere},
    coordinates::Points,
    curves::Turn,
    motion::Waypoint,
    procedures::Leg,
    trajectory::Fix,
};
use crate::{
    airport::Airport,
    arrival::{bearing, destination, distance_nm},
};

/// Environmental and performance inputs already sampled outside the planning worker.
pub struct Context<'a> {
    /// Installed airport geometry.
    pub airport: &'a Airport,
    /// Region-qualified terminal coordinates.
    pub points: &'a Points,
    /// Airport pressure and temperature reference.
    pub atmosphere: Atmosphere,
    /// Local magnetic variation, east positive.
    pub variation: f64,
    /// Wind direction true, from which it blows.
    pub wind_from: f64,
    /// Wind speed in knots.
    pub wind_knots: f64,
    /// Planned indicated speed before published limits.
    pub indicated_knots: f64,
    /// Maximum distance permitted for a published heading intercept.
    pub maximum_intercept_nm: f64,
    /// Planned climb rate used for altitude-terminated legs.
    pub climb_fpm: f64,
    /// Geometric terrain/corridor floor supplied by the route planner.
    pub minimum_feet: f64,
    /// Geometric aircraft ceiling.
    pub maximum_feet: f64,
    /// Preferred geometric level.
    pub preferred_feet: f64,
}

fn limits(leg: &Leg, c: &Context<'_>) -> Result<(f64, f64), String> {
    let convert = |altitude: Option<i32>, standard: Option<bool>| -> Result<Option<f64>, String> {
        altitude
            .map(|value| {
                c.atmosphere.geometric(
                    f64::from(value),
                    standard.ok_or("Missing published pressure reference")?,
                )
            })
            .transpose()
    };
    let one = convert(leg.altitude_one, leg.altitude_one_is_flight_level)?;
    let two = convert(leg.altitude_two, leg.altitude_two_is_flight_level)?;
    let (low, high) = match leg.altitude_description.as_str() {
        "" => (one, one),
        "+" if one.is_some() => (one, None),
        "-" if one.is_some() => (None, one),
        "B" if one.is_some() && two.is_some() => (
            Some(one.unwrap().min(two.unwrap())),
            Some(one.unwrap().max(two.unwrap())),
        ),
        "H" | "J" if one.is_some() && one == two => (one, one),
        _ => {
            return Err(format!(
                "Unsupported vertical descriptor at {}",
                leg.identifier
            ));
        }
    };
    let low = low.unwrap_or(c.minimum_feet).max(c.minimum_feet);
    let high = high.unwrap_or(c.maximum_feet).min(c.maximum_feet);
    if low > high {
        return Err(format!(
            "Crossing conflicts with aircraft or terrain envelope at {}",
            leg.identifier
        ));
    }
    Ok((low, high))
}

fn intercept(
    from: [f64; 2],
    heading: f64,
    fix: [f64; 2],
    course: f64,
    maximum_nm: f64,
) -> Result<[f64; 2], String> {
    let scale = from[0].to_radians().cos();
    let delta = [
        crate::navigation::turn(from[1], fix[1]) * 60.0 * scale,
        (fix[0] - from[0]) * 60.0,
    ];
    let u = [heading.to_radians().sin(), heading.to_radians().cos()];
    let v = [course.to_radians().sin(), course.to_radians().cos()];
    let cross = |a: [f64; 2], b: [f64; 2]| a[0] * b[1] - a[1] * b[0];
    let divisor = cross(u, v);
    if divisor.abs() < 0.001 {
        return Err("Published intercept courses are parallel".into());
    }
    let along = cross(delta, v) / divisor;
    let remaining = cross(u, delta) / divisor;
    if !(0.001..=maximum_nm).contains(&along) || !(0.0..=maximum_nm).contains(&remaining) {
        return Err(
            "Published course intercept is behind the aircraft or outside its limit".into(),
        );
    }
    Ok(destination(from, heading, along))
}

/// Resolve fixes, published courses and heading intercepts without granting a clearance.
/// Corridor protection and reachable vertical scheduling remain separate checks.
pub fn crossings(start: Waypoint, legs: &[Leg], c: &Context<'_>) -> Result<Vec<Fix>, String> {
    if legs.is_empty()
        || legs.len() > 128
        || !c.variation.is_finite()
        || !(-180.0..=180.0).contains(&c.variation)
        || !(100.0..=10000.0).contains(&c.climb_fpm)
        || !(1.0..=100.0).contains(&c.maximum_intercept_nm)
        || c.minimum_feet > c.maximum_feet
    {
        return Err("Invalid terminal decoder inputs".into());
    }
    let mut from = [start.latitude, start.longitude];
    let mut altitude = start.altitude_feet;
    let mut result: Vec<Fix> = Vec::with_capacity(legs.len());
    for (index, leg) in legs.iter().enumerate() {
        if !matches!(
            leg.terminator.as_str(),
            "IF" | "TF" | "DF" | "CF" | "CA" | "VI"
        ) {
            return Err(format!(
                "{} uses unsupported {} geometry",
                leg.identifier, leg.terminator
            ));
        }
        let (minimum_feet, mut maximum_feet) = limits(leg, c)?;
        if leg.terminator == "CA" {
            maximum_feet = minimum_feet;
        }
        let preferred_feet = c.preferred_feet.clamp(minimum_feet, maximum_feet);
        let indicated = match leg.speed_knots {
            Some(speed) => match leg.speed_description.as_str() {
                "" | "-" => c.indicated_knots.min(speed),
                "+" => c.indicated_knots.max(speed),
                _ => {
                    return Err(format!(
                        "Unsupported speed descriptor at {}",
                        leg.identifier
                    ));
                }
            },
            None => c.indicated_knots,
        };
        let course = leg.course.map(|value| {
            (value
                + c.points
                    .course_variation(leg, &c.airport.icao)
                    .unwrap_or(c.variation))
            .rem_euclid(360.0)
        });
        let coordinate = if leg.terminator == "VI" {
            let heading = course.ok_or("Heading-intercept leg has no course")?;
            let next = legs
                .get(index + 1)
                .filter(|next| next.terminator == "CF")
                .ok_or("Heading intercept needs a following course-to-fix leg")?;
            let fix = Points::runway(next, c.airport)
                .or_else(|| c.points.resolve(next, &c.airport.icao))
                .ok_or("Intercept course has an unresolved fix")?
                .coordinate;
            let track = (next.course.ok_or("Intercept target has no course")?
                + c.points
                    .course_variation(next, &c.airport.icao)
                    .unwrap_or(c.variation))
            .rem_euclid(360.0);
            intercept(from, heading, fix, track, c.maximum_intercept_nm)?
        } else if leg.terminator == "CA" {
            let heading = course.ok_or("Altitude-terminated leg has no course")?;
            if minimum_feet <= altitude {
                continue;
            }
            let tas = c
                .atmosphere
                .true_knots(indicated, (altitude + minimum_feet) / 2.0)?;
            let (speed, _) = air::wind(tas, heading, c.wind_from, c.wind_knots)?;
            let minutes = (minimum_feet - altitude) / c.climb_fpm;
            destination(from, heading, speed * minutes / 60.0)
        } else {
            Points::runway(leg, c.airport)
                .or_else(|| c.points.resolve(leg, &c.airport.icao))
                .ok_or_else(|| format!("Unresolved terminal crossing {}", leg.identifier))?
                .coordinate
        };
        let distance = distance_nm(from, coordinate);
        let track = bearing(from, coordinate);
        let arrival_heading =
            if leg.terminator == "CF" || leg.terminator == "CA" || leg.terminator == "VI" {
                Some(course.ok_or("Course leg has no published course")?)
            } else if leg.fly_over() {
                Some(track)
            } else {
                None
            };
        let tas = c.atmosphere.true_knots(indicated, preferred_feet)?;
        let (ground_knots, _) = air::wind(tas, track, c.wind_from, c.wind_knots)?;
        let required_turn = match leg.turn.as_str() {
            "" => None,
            "L" => Some(Turn::Left),
            "R" => Some(Turn::Right),
            _ => return Err("Unknown published turn direction".into()),
        };
        if distance < 0.001 {
            if let Some(previous) = result.last_mut() {
                // A STAR/approach join can repeat a fix with a tighter crossing restriction.
                let low = previous.minimum_feet.max(minimum_feet);
                let high = previous.maximum_feet.min(maximum_feet);
                if low > high {
                    return Err(
                        "Coincident terminal crossings have conflicting altitude limits".into(),
                    );
                }
                if previous
                    .arrival_heading
                    .zip(arrival_heading)
                    .is_some_and(|(a, b)| crate::navigation::turn(a, b).abs() > 1.0)
                    || previous
                        .required_turn
                        .zip(required_turn)
                        .is_some_and(|(a, b)| a != b)
                {
                    return Err(
                        "Coincident terminal crossings have conflicting courses or turns".into(),
                    );
                }
                if leg.speed_description == "+" && ground_knots > previous.ground_knots {
                    return Err(
                        "Coincident minimum-speed crossing needs a joint speed solution".into(),
                    );
                }
                previous.minimum_feet = low;
                previous.maximum_feet = high;
                previous.preferred_feet = preferred_feet.clamp(low, high);
                previous.ground_knots = previous.ground_knots.min(ground_knots);
                previous.arrival_heading = previous.arrival_heading.or(arrival_heading);
                previous.required_turn = previous.required_turn.or(required_turn);
                altitude = previous.preferred_feet;
            } else if altitude < minimum_feet || altitude > maximum_feet {
                return Err("Initial fix restriction is not satisfied".into());
            }
            continue;
        }
        result.push(Fix {
            coordinate,
            arrival_heading,
            minimum_feet,
            maximum_feet,
            preferred_feet,
            ground_knots,
            required_turn,
        });
        from = coordinate;
        altitude = preferred_feet;
    }
    if result.is_empty() {
        return Err("Terminal route has no moving leg".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn leg(terminator: &str, description: &str, altitude: &str) -> Leg {
        let mut fields = vec![""; 38];
        fields[0] = "010";
        fields[1] = "1";
        fields[2] = "TEST1";
        fields[3] = "RW04";
        fields[11] = terminator;
        fields[20] = "0420";
        fields[22] = description;
        fields[23] = altitude;
        super::super::procedures::Catalogue::parse(&format!("SID:{};", fields.join(",")))
            .unwrap()
            .0
            .into_values()
            .next()
            .unwrap()
            .remove(0)
    }
    #[test]
    fn repeated_join_fix_intersects_limits_instead_of_dropping_them() {
        let airport = Airport {
            reference_latitude: 43.0,
            reference_longitude: 7.0,
            runways: vec![crate::airport::Runway {
                first_name: "04".into(),
                second_name: "22".into(),
                first: crate::state::Point {
                    north: 10000.0,
                    ..Default::default()
                },
                second: crate::state::Point {
                    north: 12000.0,
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        };
        let points = Points::default();
        let atmosphere = Atmosphere::surface(1013.25, 15.0, 0.0).unwrap();
        let context = Context {
            airport: &airport,
            points: &points,
            atmosphere,
            variation: 0.0,
            wind_from: 0.0,
            wind_knots: 0.0,
            indicated_knots: 150.0,
            maximum_intercept_nm: 40.0,
            climb_fpm: 1000.0,
            minimum_feet: 0.0,
            maximum_feet: 20000.0,
            preferred_feet: 10000.0,
        };
        let mut first = leg("TF", "+", "01000");
        first.identifier = "RW04".into();
        first.section = "P".into();
        first.subsection = "G".into();
        let mut second = first.clone();
        second.altitude_description = "-".into();
        second.altitude_one = Some(3000);
        let start = Waypoint {
            latitude: 43.0,
            longitude: 7.0,
            altitude_feet: 0.0,
        };
        let fixes = crossings(start, &[first.clone(), second.clone()], &context).unwrap();
        assert_eq!(fixes.len(), 1);
        assert_eq!(
            fixes[0].minimum_feet,
            atmosphere.geometric(1000.0, false).unwrap()
        );
        assert_eq!(
            fixes[0].maximum_feet,
            atmosphere.geometric(3000.0, false).unwrap()
        );
        assert!(fixes[0].preferred_feet <= fixes[0].maximum_feet);
        second.altitude_one = Some(500);
        assert!(crossings(start, &[first, second], &context).is_err());
    }
    #[test]
    fn mixed_pressure_bands_keep_their_physical_limits() {
        let airport = Airport::default();
        let points = Points::default();
        let atmosphere = Atmosphere::surface(1023.0, 25.0, 0.0).unwrap();
        let context = Context {
            airport: &airport,
            points: &points,
            atmosphere,
            variation: 2.0,
            wind_from: 44.0,
            wind_knots: 10.0,
            indicated_knots: 150.0,
            maximum_intercept_nm: 40.0,
            climb_fpm: 1000.0,
            minimum_feet: 0.0,
            maximum_feet: 30000.0,
            preferred_feet: 10000.0,
        };
        let mut crossing = leg("TF", "B", "FL120");
        crossing.altitude_two = Some(8000);
        crossing.altitude_two_is_flight_level = Some(false);
        let (low, high) = limits(&crossing, &context).unwrap();
        assert_eq!(low, atmosphere.geometric(8000.0, false).unwrap());
        assert_eq!(high, atmosphere.geometric(12000.0, true).unwrap());
        let start = Waypoint {
            latitude: 43.0,
            longitude: 7.0,
            altitude_feet: 1000.0,
        };
        let climb = crossings(start, &[leg("CA", "+", "03000")], &context).unwrap();
        assert_eq!(climb.len(), 1);
        assert_eq!(climb[0].arrival_heading, Some(44.0));
        assert_eq!(climb[0].minimum_feet, climb[0].maximum_feet);
        assert!(
            climb[0].ground_knots
                < atmosphere
                    .true_knots(150.0, climb[0].preferred_feet)
                    .unwrap()
        );
        assert!(
            crossings(start, &[leg("RF", "", "")], &context)
                .err()
                .unwrap()
                .contains("unsupported RF")
        );
    }
}

#[cfg(test)]
mod intercept_tests {
    #[test]
    fn intercept_joins_the_course_before_the_fix() {
        let origin = [44.0, 8.0];
        let fix =
            crate::arrival::destination(crate::arrival::destination(origin, 0.0, 5.0), 90.0, 5.0);
        let intersection = super::intercept(origin, 0.0, fix, 90.0, 20.0).unwrap();
        assert!((crate::arrival::distance_nm(origin, intersection) - 5.0).abs() < 0.02);
        assert!((crate::arrival::bearing(intersection, fix) - 90.0).abs() < 0.1);
        assert!(super::intercept(origin, 180.0, fix, 90.0, 20.0).is_err());
        assert!(super::intercept(origin, 90.0, fix, 90.0, 20.0).is_err());
    }
}

#[cfg(test)]
mod installed_tests {
    use super::*;
    #[test]
    #[ignore = "requires installed licensed navigation data"]
    fn nice_departure_decodes_the_published_intercept() {
        let root = std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap();
        let airport = crate::airport::load_airport_from_simulator(
            &root,
            "LFMN",
            &|path| std::path::Path::new(path).is_file(),
            &|path| std::fs::read_to_string(path).map_err(|error| error.to_string()),
        )
        .unwrap();
        let catalogue =
            super::super::procedures::Catalogue::load(std::path::Path::new(&root), "LFMN").unwrap();
        let mut points = Points::requested(&[("LFMN", &catalogue)]);
        points.load(std::path::Path::new(&root)).unwrap();
        let legs = catalogue
            .departure("BASI8A", "04R", "")
            .unwrap()
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        let runway = airport
            .runways
            .iter()
            .find(|r| r.first_name == "04R" || r.second_name == "04R")
            .unwrap();
        let point = if runway.first_name == "04R" {
            runway.first
        } else {
            runway.second
        };
        let start = Waypoint {
            latitude: airport.reference_latitude + point.north / 111320.0,
            longitude: airport.reference_longitude
                + point.east / (111320.0 * airport.reference_latitude.to_radians().cos()),
            altitude_feet: airport.elevation_feet + 35.0,
        };
        let context = Context {
            airport: &airport,
            points: &points,
            atmosphere: Atmosphere::surface(1023.0, 15.0, airport.elevation_feet).unwrap(),
            variation: 2.0,
            wind_from: 44.0,
            wind_knots: 5.0,
            indicated_knots: 180.0,
            maximum_intercept_nm: 40.0,
            climb_fpm: 2000.0,
            minimum_feet: 0.0,
            maximum_feet: 18000.0,
            preferred_feet: 5000.0,
        };
        let crossings = crossings(start, &legs, &context).unwrap();
        assert!(crossings.len() >= 5);
        assert!(
            crossings
                .iter()
                .any(|fix| fix.arrival_heading == Some(131.0))
        );
    }
}
