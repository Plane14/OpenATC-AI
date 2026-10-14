//! Select compatible terminal inputs before building physical flight paths.
use super::*;
use openatc_core::traffic::{
    coordinates::Points,
    procedures::{Catalogue, Leg},
};

#[derive(Clone)]
pub(super) struct Terminal {
    pub enroute_from: [f64; 2],
    pub enroute_to: [f64; 2],
    pub enroute_minimum_feet: i32,
    pub departure_name: String,
    pub departure_transition: String,
    pub departure: Vec<Leg>,
    pub arrival_name: String,
    pub arrival_transition: String,
    pub arrival: Vec<Leg>,
    pub approach_name: String,
    pub approach_transition: String,
    pub approach: Vec<Leg>,
    pub missed: Vec<Leg>,
}

fn supported(legs: &[Leg], missed: bool) -> bool {
    !legs.is_empty()
        && legs.len() <= 128
        && legs.iter().all(|leg| {
            matches!(
                leg.terminator.as_str(),
                "IF" | "TF" | "DF" | "CF" | "CA" | "VI"
            ) || (missed && leg.terminator == "HM")
        })
}
fn coordinate(leg: &Leg, airport: &Airport, points: &Points) -> Option<[f64; 2]> {
    Points::runway(leg, airport)
        .or_else(|| points.resolve(leg, &airport.icao))
        .map(|point| point.coordinate)
}
fn assemble(parts: Vec<&[Leg]>) -> Vec<Leg> {
    // Retain repeated fixes and their restrictions; geometry resolves coincident crossings later.
    parts.into_iter().flatten().cloned().collect()
}
fn variants(catalogue: &Catalogue, kind: &str, name: &str) -> Vec<String> {
    let transitions: BTreeSet<_> = catalogue
        .0
        .keys()
        .filter(|key| {
            key.kind == kind
                && key.name == name
                && !key.transition.is_empty()
                && !key.transition.starts_with("RW")
                && key.transition != "ALL"
        })
        .map(|key| key.transition.clone())
        .collect();
    if transitions.is_empty() {
        vec![String::new()]
    } else {
        transitions.into_iter().collect()
    }
}

pub(super) struct Inputs<'a> {
    pub origin: &'a Airport,
    pub destination: &'a Airport,
    pub departure_runway: &'a str,
    pub arrival_runway: &'a str,
    pub departures: &'a Catalogue,
    pub arrivals: &'a Catalogue,
    pub points: &'a Points,
    pub mora: &'a openatc_core::navigation::Mora,
    pub budget: usize,
}

/// Rank runway-compatible procedures with an explicit STAR-to-approach join.
/// These candidates are not movement permissions or a terrain-checked enroute route.
pub(super) fn candidates(input: Inputs<'_>) -> Result<Vec<Terminal>, String> {
    if !(1..=32).contains(&input.budget)
        || input.departure_runway.is_empty()
        || input.arrival_runway.is_empty()
    {
        return Err("Terminal selection needs both runways and a bounded candidate budget".into());
    }
    let departure_names: BTreeSet<_> = input
        .departures
        .0
        .keys()
        .filter(|key| key.kind == "SID")
        .map(|key| key.name.clone())
        .collect();
    let arrival_names: BTreeSet<_> = input
        .arrivals
        .0
        .keys()
        .filter(|key| key.kind == "STAR")
        .map(|key| key.name.clone())
        .collect();
    let approach_names: BTreeSet<_> = input
        .arrivals
        .0
        .keys()
        .filter(|key| key.kind == "APPCH")
        .map(|key| key.name.clone())
        .collect();
    let destination = [
        input.destination.reference_latitude,
        input.destination.reference_longitude,
    ];
    let origin = [
        input.origin.reference_latitude,
        input.origin.reference_longitude,
    ];
    let mut departures = Vec::new();
    for name in departure_names {
        for transition in variants(input.departures, "SID", &name) {
            let Ok(parts) = input
                .departures
                .departure(&name, input.departure_runway, &transition)
            else {
                continue;
            };
            let legs = assemble(parts);
            if !supported(&legs, false) {
                continue;
            }
            let Some(exit) = legs
                .last()
                .and_then(|leg| coordinate(leg, input.origin, input.points))
            else {
                continue;
            };
            departures.push((
                openatc_core::arrival::distance_nm(exit, destination),
                name.clone(),
                transition,
                legs,
                exit,
            ));
            departures.sort_by(|a, b| {
                a.0.total_cmp(&b.0)
                    .then_with(|| a.1.cmp(&b.1))
                    .then_with(|| a.2.cmp(&b.2))
            });
            departures.truncate(input.budget);
        }
    }
    departures.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    departures.truncate(input.budget);
    let mut arrivals = Vec::new();
    for name in arrival_names {
        for transition in variants(input.arrivals, "STAR", &name) {
            let Ok(parts) = input
                .arrivals
                .arrival(&name, input.arrival_runway, &transition)
            else {
                continue;
            };
            let legs = assemble(parts);
            if !supported(&legs, false) {
                continue;
            }
            let Some(entry) = legs
                .first()
                .and_then(|leg| coordinate(leg, input.destination, input.points))
            else {
                continue;
            };
            let Some(exit) = legs
                .last()
                .and_then(|leg| coordinate(leg, input.destination, input.points))
            else {
                continue;
            };
            for approach_name in &approach_names {
                let transitions: BTreeSet<_> = input
                    .arrivals
                    .0
                    .keys()
                    .filter(|key| {
                        key.kind == "APPCH" && &key.name == approach_name && key.route_type == "A"
                    })
                    .map(|key| key.transition.clone())
                    .chain(std::iter::once(String::new()))
                    .collect();
                for approach_transition in transitions {
                    let Ok(approach) = input.arrivals.approach(approach_name, &approach_transition)
                    else {
                        continue;
                    };
                    if approach
                        .landing
                        .last()
                        .is_none_or(|leg| leg.identifier != format!("RW{}", input.arrival_runway))
                    {
                        continue;
                    }
                    let assembled = approach
                        .transition
                        .iter()
                        .chain(approach.landing)
                        .cloned()
                        .collect::<Vec<_>>();
                    if !supported(&assembled, false) || !supported(approach.missed, true) {
                        continue;
                    }
                    let Some(join) = assembled
                        .first()
                        .and_then(|leg| coordinate(leg, input.destination, input.points))
                    else {
                        continue;
                    };
                    if openatc_core::arrival::distance_nm(exit, join) > 0.01 {
                        continue;
                    }
                    arrivals.push((
                        openatc_core::arrival::distance_nm(origin, entry),
                        name.clone(),
                        transition.clone(),
                        legs.clone(),
                        entry,
                        approach_name.clone(),
                        approach_transition,
                        assembled,
                        approach.missed.to_vec(),
                    ));
                    arrivals.sort_by(|a, b| {
                        a.0.total_cmp(&b.0)
                            .then_with(|| a.1.cmp(&b.1))
                            .then_with(|| a.5.cmp(&b.5))
                            .then_with(|| a.6.cmp(&b.6))
                    });
                    arrivals.truncate(input.budget);
                }
            }
        }
    }
    arrivals.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.5.cmp(&b.5))
            .then_with(|| a.6.cmp(&b.6))
    });
    arrivals.truncate(input.budget);
    let mut result = Vec::new();
    for departure in &departures {
        for arrival in &arrivals {
            let Some(floor) = input.mora.corridor(departure.4, arrival.4) else {
                continue;
            };
            result.push((
                openatc_core::arrival::distance_nm(departure.4, arrival.4),
                Terminal {
                    enroute_from: departure.4,
                    enroute_to: arrival.4,
                    enroute_minimum_feet: floor,
                    departure_name: departure.1.clone(),
                    departure_transition: departure.2.clone(),
                    departure: departure.3.clone(),
                    arrival_name: arrival.1.clone(),
                    arrival_transition: arrival.2.clone(),
                    arrival: arrival.3.clone(),
                    approach_name: arrival.5.clone(),
                    approach_transition: arrival.6.clone(),
                    approach: arrival.7.clone(),
                    missed: arrival.8.clone(),
                },
            ));
            result.sort_by(|a, b| {
                a.0.total_cmp(&b.0)
                    .then_with(|| a.1.departure_name.cmp(&b.1.departure_name))
                    .then_with(|| a.1.arrival_name.cmp(&b.1.arrival_name))
                    .then_with(|| a.1.approach_name.cmp(&b.1.approach_name))
            });
            result.truncate(input.budget);
        }
    }
    result.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| a.1.departure_name.cmp(&b.1.departure_name))
            .then_with(|| a.1.arrival_name.cmp(&b.1.arrival_name))
            .then_with(|| a.1.approach_name.cmp(&b.1.approach_name))
    });
    result.truncate(input.budget);
    if result.is_empty() {
        return Err("No resolved runway-compatible SID/STAR/approach with a published recovery and connected approach entry".into());
    }
    Ok(result.into_iter().map(|(_, route)| route).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed licensed navigation data"]
    fn installed_nice_to_milan_terminal_candidates_keep_restrictions_and_recovery() {
        let root = std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap();
        let origin = airport(&root, "LFMN").unwrap();
        let destination = airport(&root, "LIMC").unwrap();
        let departures = Catalogue::load(std::path::Path::new(&root), "LFMN").unwrap();
        let arrivals = Catalogue::load(std::path::Path::new(&root), "LIMC").unwrap();
        let mut points = Points::requested(&[("LFMN", &departures), ("LIMC", &arrivals)]);
        points.load(std::path::Path::new(&root)).unwrap();
        let result = candidates(Inputs {
            origin: &origin,
            destination: &destination,
            departure_runway: "04R",
            arrival_runway: "35R",
            departures: &departures,
            arrivals: &arrivals,
            points: &points,
            mora: &openatc_core::navigation::Mora::load(&root),
            budget: 16,
        })
        .unwrap();
        assert!(result.len() <= 16);
        assert!(
            candidates(Inputs {
                origin: &origin,
                destination: &destination,
                departure_runway: "04R",
                arrival_runway: "35R",
                departures: &departures,
                arrivals: &arrivals,
                points: &points,
                mora: &Default::default(),
                budget: 16
            })
            .is_err()
        );
        for route in result {
            assert!(route.enroute_minimum_feet > 0);
            assert!(openatc_core::arrival::distance_nm(route.enroute_from, route.enroute_to) > 0.0);
            assert_eq!(route.approach.last().unwrap().identifier, "RW35R");
            assert!(
                route
                    .missed
                    .last()
                    .is_some_and(|leg| leg.terminator == "HM")
            );
            let star = route.arrival.last().unwrap();
            let approach = route.approach.first().unwrap();
            assert_eq!(
                coordinate(star, &destination, &points),
                coordinate(approach, &destination, &points)
            );
            assert!(route.arrival.iter().any(|leg| leg.altitude_one.is_some()));
        }
    }
}
