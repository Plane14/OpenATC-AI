//! Join published terminal boundaries without treating a nominal connection as a clearance.
use super::{environment::Environment, routing::Terminal};
use openatc_core::{
    arrival::{bearing, destination, distance_nm},
    navigation::Mora,
    traffic::{
        profiles::Aircraft,
        trajectory::{Airspeed, Fix},
    },
};

pub(super) struct Request<'a> {
    pub terminal: &'a Terminal,
    pub departure: &'a mut [Fix],
    pub arrival: &'a [Fix],
    pub mora: &'a Mora,
    pub departure_weather: &'a Environment,
    pub arrival_weather: &'a Environment,
    pub aircraft: &'a Aircraft,
    pub spacing_nm: f64,
    pub budget: usize,
}

pub(super) fn crossings(input: Request<'_>) -> Result<Vec<Fix>, String> {
    if !input.departure_weather.fresh() || !input.arrival_weather.fresh() {
        return Err("Enroute planning needs fresh endpoint weather".into());
    }
    if !(5.0..=50.0).contains(&input.spacing_nm) || !(1..=128).contains(&input.budget) {
        return Err("Enroute crossing budget or spacing is invalid".into());
    }
    let start = input
        .departure
        .last()
        .ok_or("Enroute planning has no SID endpoint")?;
    let end = input
        .arrival
        .first()
        .ok_or("Enroute planning has no STAR entry")?;
    let from = input.terminal.enroute_from;
    let to = input.terminal.enroute_to;
    if distance_nm(start.coordinate, from) >= 0.001 || distance_nm(end.coordinate, to) >= 0.001 {
        return Err("Enroute connection does not match the resolved terminal boundaries".into());
    }
    let distance = distance_nm(from, to);
    if !distance.is_finite() || distance < 0.01 {
        return Err("Enroute connection has no supported flight distance".into());
    }
    let count = (distance / input.spacing_nm).ceil() as usize;
    if count > input.budget {
        return Err("Enroute connection exceeds the remaining flight crossing budget".into());
    }
    let floor = input
        .mora
        .corridor(from, to)
        .ok_or("Enroute connection has unknown grid MORA")?;
    if floor != input.terminal.enroute_minimum_feet {
        return Err("Enroute MORA changed since terminal selection".into());
    }
    let geometric =
        |weather: &Environment, feet, standard| weather.atmosphere.geometric(feet, standard);
    // Both airport observations bound the nominal connection. These are planning
    // references, not observations of the weather along the enroute path.
    let terrain_floor = geometric(input.departure_weather, f64::from(floor), false)?
        .max(geometric(input.arrival_weather, f64::from(floor), false)?);
    let maximum = geometric(input.departure_weather, input.aircraft.ceiling, true)?.min(geometric(
        input.arrival_weather,
        input.aircraft.ceiling,
        true,
    )?);
    if terrain_floor > maximum {
        return Err(
            "Enroute terrain floor conflicts with the aircraft or terminal level limits".into(),
        );
    }
    let course = bearing(from, to);
    let mut result: Vec<Fix> = Vec::with_capacity(count);
    let mut initial_floor = None;
    let mut previous = from;
    for index in 1..=count {
        let fraction = index as f64 / count as f64;
        let coordinate = if index == count {
            to
        } else {
            destination(from, course, distance * fraction)
        };
        let local_floor = input
            .mora
            .corridor(previous, coordinate)
            .ok_or("Enroute crossing has unknown grid MORA")?;
        let minimum = geometric(input.departure_weather, f64::from(local_floor), false)?.max(
            geometric(input.arrival_weather, f64::from(local_floor), false)?,
        );
        let preceding_maximum = result
            .last()
            .map_or(start.maximum_feet, |before| before.maximum_feet);
        if result.is_empty() {
            initial_floor = Some(minimum);
        }
        if minimum > preceding_maximum {
            return Err("Enroute terrain floor conflicts with the preceding crossing limit".into());
        }
        if let Some(before) = result.last_mut() {
            before.minimum_feet = before.minimum_feet.max(minimum);
            before.preferred_feet = before
                .preferred_feet
                .clamp(before.minimum_feet, before.maximum_feet);
        }
        if index == count {
            let mut entry = end.clone();
            entry.minimum_feet = entry.minimum_feet.max(minimum);
            entry.maximum_feet = entry.maximum_feet.min(maximum);
            if entry.minimum_feet > entry.maximum_feet {
                return Err("STAR entry conflicts with the enroute aircraft ceiling".into());
            }
            entry.preferred_feet = entry
                .preferred_feet
                .clamp(entry.minimum_feet, entry.maximum_feet);
            result.push(entry);
            break;
        }
        let weather = if fraction < 0.5 {
            input.departure_weather
        } else {
            input.arrival_weather
        };
        let indicated =
            input.aircraft.cruise_knots / weather.atmosphere.true_knots(100.0, maximum)? * 100.0;
        if !(input.aircraft.approach_knots..=350.0).contains(&indicated) {
            return Err(
                "Nominal cruise speed is outside the fleet's indicated-speed envelope".into(),
            );
        }
        let airspeed = Airspeed {
            indicated_knots: indicated,
            atmosphere: weather.atmosphere,
            wind_from: weather.wind_from,
            wind_knots: weather.wind_knots,
        };
        result.push(Fix {
            coordinate,
            arrival_heading: None,
            minimum_feet: minimum,
            maximum_feet: maximum,
            preferred_feet: maximum,
            ground_knots: airspeed.ground_knots(maximum, bearing(previous, coordinate))?,
            airspeed: Some(airspeed),
            required_turn: None,
        });
        previous = coordinate;
    }
    // The outgoing leg's floor applies at the SID endpoint as well. Commit this
    // constraint only after the entire nominal connection has been checked.
    let start = input.departure.last_mut().unwrap();
    start.minimum_feet = start.minimum_feet.max(initial_floor.unwrap());
    start.preferred_feet = start.preferred_feet.max(start.minimum_feet);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openatc_core::airport::Airport;
    use std::{collections::BTreeMap, time::Instant};

    #[test]
    fn terrain_is_scheduled_before_entry_without_raising_the_entire_sid_to_a_distant_grid() {
        let weather = |id: &str| {
            let airport = Airport {
                icao: id.into(),
                ..Default::default()
            };
            Environment::from_sample(
                &airport,
                &(
                    serde_json::json!({"airport": id, "latitude":0.0, "longitude":0.0,
                    "sampleAltitudeFeet":0.0, "pressureHpa":1013.25, "temperatureC":15.0,
                    "magneticVariationDegrees":0.0, "windDegrees":270.0, "windKnots":10.0}),
                    Instant::now(),
                ),
            )
            .unwrap()
        };
        let departure_weather = weather("ORIG");
        let arrival_weather = weather("DEST");
        let speed = Airspeed {
            indicated_knots: 90.0,
            atmosphere: arrival_weather.atmosphere,
            wind_from: 270.0,
            wind_knots: 10.0,
        };
        let fix = |coordinate, feet| Fix {
            coordinate,
            minimum_feet: feet,
            maximum_feet: feet,
            preferred_feet: feet,
            arrival_heading: Some(90.0),
            ground_knots: 100.0,
            airspeed: Some(speed),
            required_turn: None,
        };
        let terminal = Terminal {
            enroute_from: [0.2, 0.2],
            enroute_to: [0.2, 1.8],
            enroute_minimum_feet: 8000,
            departure_name: String::new(),
            departure_transition: String::new(),
            departure: vec![],
            arrival_name: String::new(),
            arrival_transition: String::new(),
            arrival: vec![],
            approach_name: String::new(),
            approach_transition: String::new(),
            approach: vec![],
            missed: vec![],
        };
        let mut departure = [fix(terminal.enroute_from, 2000.0)];
        let arrival = [fix(terminal.enroute_to, 9000.0)];
        let aircraft = openatc_core::traffic::profiles::aircraft("C172").unwrap();
        let grid = Mora(BTreeMap::from([((0, 0), 1000), ((0, 1), 8000)]));
        macro_rules! request {
            ($departure:expr, $mora:expr, $budget:expr) => {
                Request {
                    terminal: &terminal,
                    departure: $departure,
                    arrival: &arrival,
                    mora: $mora,
                    departure_weather: &departure_weather,
                    arrival_weather: &arrival_weather,
                    aircraft,
                    spacing_nm: 20.0,
                    budget: $budget,
                }
            };
        }
        let path = crossings(request!(&mut departure, &grid, 20)).unwrap();
        assert!(path.len() >= 4);
        assert_eq!(departure[0].minimum_feet, 2000.0);
        let high = arrival_weather.atmosphere.geometric(8000.0, false).unwrap();
        assert!(
            path.iter()
                .any(|fix| fix.coordinate[1] < 1.0 && fix.minimum_feet >= high)
        );
        let last = path.last().unwrap();
        assert_eq!(last.coordinate, arrival[0].coordinate);
        assert_eq!(last.maximum_feet, 9000.0);
        assert_eq!(last.arrival_heading, arrival[0].arrival_heading);
        assert_eq!(last.airspeed, arrival[0].airspeed);
        assert!(
            path.windows(2)
                .all(|pair| distance_nm(pair[0].coordinate, pair[1].coordinate) <= 20.0)
        );
        assert!(path.iter().all(|fix| fix.minimum_feet <= fix.maximum_feet));

        let initial = departure[0].clone();
        assert!(crossings(request!(&mut departure, &grid, 1)).is_err());
        let mut missing = grid.clone();
        missing.0.remove(&(0, 1));
        assert!(crossings(request!(&mut departure, &missing, 20)).is_err());
        let mut changed = grid.clone();
        changed.0.insert((0, 1), 8500);
        assert!(crossings(request!(&mut departure, &changed, 20)).is_err());
        assert_eq!(departure[0].minimum_feet, initial.minimum_feet);
        assert_eq!(departure[0].maximum_feet, initial.maximum_feet);
        let mut low = [fix(terminal.enroute_from, 500.0)];
        assert!(crossings(request!(&mut low, &grid, 20)).is_err());
        assert_eq!(low[0].minimum_feet, 500.0);
    }
}
