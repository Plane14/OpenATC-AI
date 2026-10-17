//! Model-compatible airport pairs selected from installed controllers and airport scenery.
use super::*;

#[derive(Clone)]
pub(super) struct Pair {
    pub origin: Arc<Airport>,
    pub destination: Arc<Airport>,
    pub departure_runway: String,
    pub arrival_runway: String,
    compatible: [[bool; 4]; 2],
}

#[derive(Clone)]
pub(super) struct Plan {
    pub key: String,
    pub airports: BTreeMap<String, Arc<Airport>>,
    pairs: Vec<Pair>,
    warnings: Vec<String>,
}

fn published_pair(
    input: &Preparation,
    first: &Airport,
    second: &Airport,
    departure_runway: &str,
    arrival_runway: &str,
    mora: &openatc_core::navigation::Mora,
    budget: usize,
) -> Result<[Result<(), String>; 2], String> {
    use openatc_core::traffic::{coordinates::Points, procedures::Catalogue};
    let root = std::path::Path::new(&input.simulator_root);
    let departures = Catalogue::load(root, &first.icao)?;
    let arrivals = Catalogue::load(root, &second.icao)?;
    let mut points = Points::requested(&[(&first.icao, &departures), (&second.icao, &arrivals)]);
    points.load(root)?;
    Ok([
        (
            first,
            second,
            departure_runway,
            arrival_runway,
            &departures,
            &arrivals,
        ),
        (
            second,
            first,
            arrival_runway,
            departure_runway,
            &arrivals,
            &departures,
        ),
    ]
    .map(
        |(origin, destination, departure_runway, arrival_runway, departures, arrivals)| {
            routing::candidates(routing::Inputs {
                origin,
                destination,
                departure_runway,
                arrival_runway,
                departures,
                arrivals,
                points: &points,
                mora,
                budget,
            })
            .map(|_| ())
        },
    ))
}

fn distance(first: &Airport, second: &Airport) -> f64 {
    openatc_core::arrival::distance_nm(
        [first.reference_latitude, first.reference_longitude],
        [second.reference_latitude, second.reference_longitude],
    )
}

fn compatible(input: &Preparation, first: &Airport, second: &Airport) -> [bool; 4] {
    let (Some(origin), Some(destination)) = (port(first), port(second)) else {
        return [false; 4];
    };
    [
        Category::Airline,
        Category::Regional,
        Category::Cargo,
        Category::GeneralAviation,
    ]
    .map(|category| {
        input
            .profiles
            .select_where(
                &origin,
                &destination,
                distance(first, second),
                category,
                &input.models,
                input.epoch,
                |_, aircraft| {
                    [first, second].iter().all(|airport| {
                        airport
                            .parking
                            .iter()
                            .any(|stand| openatc_core::parking::moving_supports(stand, aircraft))
                    })
                },
            )
            .is_some()
    })
}

fn longest_runway(airport: &Airport) -> Option<String> {
    airport
        .runways
        .iter()
        .max_by(|a, b| {
            let length = |runway: &openatc_core::airport::Runway| {
                (runway.second.east - runway.first.east)
                    .hypot(runway.second.north - runway.first.north)
            };
            length(a).total_cmp(&length(b))
        })
        .map(|runway| runway.first_name.clone())
}

fn runway(airport: &Airport, preferred: &str) -> Option<String> {
    airport
        .runways
        .iter()
        .any(|runway| runway.first_name == preferred || runway.second_name == preferred)
        .then(|| preferred.to_owned())
        .or_else(|| longest_runway(airport))
}

/// Traffic discovery can start before the player has filed a route.
pub(super) fn endpoints(
    root: &str,
    plan: &openatc_core::state::FlightPlan,
    stations: &[openatc_core::stations::Station],
    own: [f64; 2],
    policy: &Operations,
) -> Result<(Airport, Airport, String, String), String> {
    if !own.iter().all(|coordinate| coordinate.is_finite())
        || own[0].abs() >= 85.0
        || own[1].abs() >= 180.0
    {
        return Err("Traffic airport discovery needs a valid position".into());
    }
    if plan.departure != plan.destination
        && let (Ok(origin), Ok(destination)) = (
            airport(root, &plan.departure),
            airport(root, &plan.destination),
        )
        && let (Some(departure), Some(arrival)) = (
            runway(&origin, &plan.runway),
            runway(&destination, &plan.arrival_runway),
        )
    {
        return Ok((origin, destination, departure, arrival));
    }
    let mut candidates = BTreeMap::<String, f64>::new();
    for station in stations.iter().filter(|station| {
        station.service == "Tower" || station.services.iter().any(|service| service == "Tower")
    }) {
        if !(3..=4).contains(&station.airport.len())
            || !station
                .airport
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
        {
            continue;
        }
        let distance =
            openatc_core::arrival::distance_nm(own, [station.latitude, station.longitude]);
        if distance.is_finite() && distance <= policy.network_radius_nm {
            candidates
                .entry(station.airport.clone())
                .and_modify(|current| *current = current.min(distance))
                .or_insert(distance);
        }
    }
    let mut candidates = candidates.into_iter().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.1
            .total_cmp(&right.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    let mut endpoints = candidates
        .into_iter()
        .take(policy.network_candidate_limit)
        .filter_map(|(id, _)| {
            let airport = airport(root, &id).ok()?;
            let runway = longest_runway(&airport)?;
            (!airport.parking.is_empty()).then_some((airport, runway))
        });
    let (origin, departure) = endpoints
        .next()
        .ok_or("No installed controlled traffic airport near the aircraft")?;
    let (destination, arrival) = endpoints
        .next()
        .ok_or("No second installed controlled traffic airport near the aircraft")?;
    Ok((origin, destination, departure, arrival))
}

/// Discovery requests real airport weather before any additional flight is scheduled.
pub(super) fn survey(input: &Preparation, key: String) -> Result<Plan, String> {
    let (_, policy, _) = policy::load(&input.config)?;
    let origin = Arc::new(input.origin.clone());
    let destination = Arc::new(input.destination.clone());
    let mut plan = Plan {
        key,
        airports: [origin.clone(), destination.clone()]
            .into_iter()
            .map(|airport| (airport.icao.clone(), airport))
            .collect(),
        warnings: Vec::new(),
        pairs: vec![Pair {
            origin: origin.clone(),
            destination: destination.clone(),
            departure_runway: input.runway.clone(),
            arrival_runway: input.arrival_runway.clone(),
            compatible: [
                compatible(input, &origin, &destination),
                compatible(input, &destination, &origin),
            ],
        }],
    };
    let own_distance = |airport: &Airport| {
        openatc_core::arrival::distance_nm(
            input.own,
            [airport.reference_latitude, airport.reference_longitude],
        )
    };
    let anchor = if own_distance(&origin) <= own_distance(&destination) {
        origin
    } else {
        destination
    };
    let mut candidates = BTreeMap::<String, (f64, f64)>::new();
    for station in input.stations.iter().filter(|station| {
        station.service == "Tower" || station.services.iter().any(|service| service == "Tower")
    }) {
        if plan.airports.contains_key(&station.airport)
            || !(3..=4).contains(&station.airport.len())
            || !station
                .airport
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
        {
            continue;
        }
        let distance =
            openatc_core::arrival::distance_nm(input.own, [station.latitude, station.longitude]);
        if !distance.is_finite() || !(5.0..=policy.network_radius_nm).contains(&distance) {
            continue;
        }
        let preference = input
            .profiles
            .hubs
            .get(&station.airport)
            .map_or(0u64, |hub| {
                hub.airlines.values().map(|weight| u64::from(*weight)).sum()
            });
        let score = distance / (1.0 + (preference as f64).ln_1p() * 0.1);
        candidates
            .entry(station.airport.clone())
            .or_insert((score, distance));
    }
    let mut candidates = candidates.into_iter().collect::<Vec<_>>();
    candidates.sort_by(|a, b| a.1.0.total_cmp(&b.1.0).then_with(|| a.0.cmp(&b.0)));
    let mora = openatc_core::navigation::Mora::load(&input.simulator_root);
    let mut one_way = Vec::new();
    for (id, _) in candidates.into_iter().take(policy.network_candidate_limit) {
        if plan.airports.len() >= policy.network_airport_limit {
            break;
        }
        let Ok(airport) = airport(&input.simulator_root, &id) else {
            continue;
        };
        let mut available = [
            compatible(input, &anchor, &airport),
            compatible(input, &airport, &anchor),
        ];
        if !available.iter().any(|direction| {
            direction
                .iter()
                .zip(input.counts)
                .any(|(eligible, count)| *eligible && count > 0)
        }) {
            continue;
        }
        let Some(runway) = longest_runway(&airport) else {
            continue;
        };
        let airport = Arc::new(airport);
        let anchor_runway = if anchor.icao == input.origin.icao {
            input.runway.clone()
        } else {
            input.arrival_runway.clone()
        };
        let routes = published_pair(
            input,
            &anchor,
            &airport,
            &anchor_runway,
            &runway,
            &mora,
            policy.terminal_candidate_limit,
        )
        .unwrap_or_else(|error| [Err(error.clone()), Err(error)]);
        for (direction, result) in routes.into_iter().enumerate() {
            if let Err(error) = result {
                available[direction] = [false; 4];
                if plan.warnings.len() < 8 {
                    let (from, to) = if direction == 0 {
                        (&anchor.icao, &id)
                    } else {
                        (&id, &anchor.icao)
                    };
                    plan.warnings
                        .push(format!("{from}–{to}: published route unavailable: {error}"));
                }
            }
        }
        if !available.iter().any(|direction| {
            direction
                .iter()
                .zip(input.counts)
                .any(|(eligible, count)| *eligible && count > 0)
        }) {
            continue;
        }
        let pair = Pair {
            origin: anchor.clone(),
            destination: airport.clone(),
            departure_runway: anchor_runway,
            arrival_runway: runway,
            compatible: available,
        };
        // Keep two-way coverage ahead of one-way alternatives within the same airport budget.
        if available.iter().all(|direction| {
            direction
                .iter()
                .zip(input.counts)
                .any(|(eligible, count)| *eligible && count > 0)
        }) {
            plan.pairs.push(pair);
            plan.airports.insert(id, airport);
        } else {
            one_way.push(pair);
        }
    }
    for pair in one_way {
        if plan.airports.len() >= policy.network_airport_limit {
            break;
        }
        plan.airports
            .insert(pair.destination.icao.clone(), pair.destination.clone());
        plan.pairs.push(pair);
    }
    Ok(plan)
}

fn allocate(pairs: &[Pair], counts: [u8; 4]) -> (Vec<[u8; 4]>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut allocations = vec![[0u8; 4]; pairs.len()];
    let mut remaining = 63u8;
    for category in 0..4 {
        let two_way = pairs
            .iter()
            .enumerate()
            .filter_map(|(index, pair)| {
                pair.compatible
                    .iter()
                    .all(|direction| direction[category])
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        let eligible = if two_way.is_empty() {
            pairs
                .iter()
                .enumerate()
                .filter_map(|(index, pair)| {
                    pair.compatible
                        .iter()
                        .any(|direction| direction[category])
                        .then_some(index)
                })
                .collect::<Vec<_>>()
        } else {
            two_way
        };
        let count = counts[category].min(remaining);
        remaining -= count;
        if count == 0 {
            continue;
        }
        if eligible.is_empty() {
            let category = [
                Category::Airline,
                Category::Regional,
                Category::Cargo,
                Category::GeneralAviation,
            ][category];
            warnings.push(format!(
                "{category:?}: no model-compatible installed airport pair"
            ));
            continue;
        }
        for slot in 0..usize::from(count) {
            allocations[eligible[slot % eligible.len()]][category] += 1;
        }
    }
    (allocations, warnings)
}

/// Split only between independently eligible directions; allocation already enforces the global limit.
fn directional_counts(pair: &Pair, counts: [u8; 4]) -> [[u8; 4]; 2] {
    let mut result = [[0; 4]; 2];
    for (category, count) in counts.into_iter().enumerate() {
        match (pair.compatible[0][category], pair.compatible[1][category]) {
            (true, true) => {
                result[0][category] = count.div_ceil(2);
                result[1][category] = count / 2;
            }
            (true, false) => result[0][category] = count,
            (false, true) => result[1][category] = count,
            (false, false) => {}
        }
    }
    result
}

fn ready_pairs(plan: &Plan, samples: &environment::Samples) -> (Vec<Pair>, BTreeSet<String>) {
    let missing = plan
        .airports
        .values()
        .filter(|airport| {
            samples
                .get(&airport.icao)
                .and_then(|sample| environment::Environment::from_sample(airport, sample).ok())
                .is_none()
        })
        .map(|airport| airport.icao.clone())
        .collect::<BTreeSet<_>>();
    let pairs = plan
        .pairs
        .iter()
        .cloned()
        .map(|mut pair| {
            if missing.contains(&pair.origin.icao) || missing.contains(&pair.destination.icao) {
                pair.compatible = [[false; 4]; 2];
            }
            pair
        })
        .collect();
    (pairs, missing)
}

pub(super) fn prepare(mut input: Preparation, plan: &Plan) -> Result<Scene, String> {
    input.refresh_weather()?;
    let (pairs, missing) = ready_pairs(plan, &input.weather);
    if !missing.is_empty()
        && pairs
            .iter()
            .all(|pair| pair.compatible.iter().flatten().all(|eligible| !eligible))
    {
        return Err(format!(
            "Waiting for simulator airport weather: {}",
            missing.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    let (allocations, mut warnings) = allocate(&pairs, input.counts);
    warnings.extend(missing.into_iter().map(|airport| {
        format!("{airport}: additional traffic route awaits fresh simulator weather")
    }));
    let mut scene: Option<Scene> = None;
    for (pair, counts) in pairs.iter().zip(allocations) {
        if counts.iter().all(|count| *count == 0) {
            continue;
        }
        for (direction, counts) in directional_counts(pair, counts).into_iter().enumerate() {
            if counts.iter().all(|count| *count == 0) {
                continue;
            }
            let mut request = input.clone();
            request.origin = pair.origin.as_ref().clone();
            request.destination = pair.destination.as_ref().clone();
            request.runway = pair.departure_runway.clone();
            request.arrival_runway = pair.arrival_runway.clone();
            if direction == 1 {
                std::mem::swap(&mut request.origin, &mut request.destination);
                std::mem::swap(&mut request.runway, &mut request.arrival_runway);
            }
            request.counts = counts;
            if let Some(previous) = &scene {
                previous.observe_prepared(&mut request);
            }
            let from = request.origin.icao.clone();
            let to = request.destination.icao.clone();
            match prepare_direction(request) {
                Ok(next) => {
                    scene = Some(match scene {
                        Some(previous) => previous.merge(next)?,
                        None => next,
                    })
                }
                Err(error) => warnings.push(format!("{from}–{to}: {error}")),
            }
        }
    }
    let mut scene = scene
        .ok_or_else(|| format!("No complete traffic airport pair: {}", warnings.join("; ")))?;
    scene.warnings.extend(plan.warnings.iter().cloned());
    scene.warnings.extend(warnings);
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_distant_weather_does_not_block_ready_local_pairs() {
        let airport = |id: &str, latitude| {
            Arc::new(Airport {
                icao: id.into(),
                reference_latitude: latitude,
                ..Default::default()
            })
        };
        let home = airport("HOME", 0.0);
        let near = airport("NEAR", 0.5);
        let far = airport("FAR1", 50.0);
        let pair = |destination| Pair {
            origin: home.clone(),
            destination,
            departure_runway: "09".into(),
            arrival_runway: "27".into(),
            compatible: [[true, false, false, false]; 2],
        };
        let plan = Plan {
            key: "test".into(),
            airports: [home.clone(), near.clone(), far.clone()]
                .into_iter()
                .map(|airport| (airport.icao.clone(), airport))
                .collect(),
            pairs: vec![pair(far), pair(near.clone())],
            warnings: Vec::new(),
        };
        let mut samples = [home, near].into_iter().map(|airport| (airport.icao.clone(), (serde_json::json!({
            "airport":airport.icao, "latitude":airport.reference_latitude, "longitude":0.0, "sampleAltitudeFeet":0.0,
            "pressureHpa":1013.25, "temperatureC":15.0, "magneticVariationDegrees":0.0, "windDegrees":0.0, "windKnots":0.0
        }), Instant::now()))).collect::<environment::Samples>();
        let (ready, missing) = ready_pairs(&plan, &samples);
        assert_eq!(missing, BTreeSet::from(["FAR1".into()]));
        let (allocation, _) = allocate(&ready, [8, 0, 0, 0]);
        assert_eq!(allocation[0], [0; 4]);
        assert_eq!(allocation[1], [8, 0, 0, 0]);
        samples.get_mut("HOME").unwrap().1 = Instant::now() - Duration::from_secs(46);
        let (ready, _) = ready_pairs(&plan, &samples);
        assert!(ready.iter().all(|pair| pair.compatible == [[false; 4]; 2]));
    }

    #[test]
    fn one_way_routes_keep_the_full_category_allocation_without_inventing_a_return() {
        let pair = Pair {
            origin: Arc::new(Airport::default()),
            destination: Arc::new(Airport::default()),
            departure_runway: "09".into(),
            arrival_runway: "27".into(),
            compatible: [[true, false, true, false], [false, true, true, false]],
        };
        let (allocations, warnings) = allocate(std::slice::from_ref(&pair), [5, 3, 7, 2]);
        assert_eq!(allocations, vec![[5, 3, 7, 0]]);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.starts_with("GeneralAviation:"))
        );
        assert_eq!(
            directional_counts(&pair, allocations[0]),
            [[5, 0, 4, 0], [0, 3, 3, 0]]
        );
        let (allocations, _) = allocate(std::slice::from_ref(&pair), [63; 4]);
        assert_eq!(
            directional_counts(&pair, allocations[0])
                .iter()
                .flatten()
                .map(|n| usize::from(*n))
                .sum::<usize>(),
            63
        );
    }

    #[test]
    fn one_way_fallback_does_not_displace_two_way_category_coverage() {
        let mut pair = Pair {
            origin: Arc::new(Airport::default()),
            destination: Arc::new(Airport::default()),
            departure_runway: "09".into(),
            arrival_runway: "27".into(),
            compatible: [[true, false, false, false]; 2],
        };
        let two_way = pair.clone();
        pair.compatible = [[true, true, false, false], [false; 4]];
        let (allocations, _) = allocate(&[two_way.clone(), pair.clone()], [8, 3, 0, 0]);
        assert_eq!(allocations, vec![[8, 0, 0, 0], [0, 3, 0, 0]]);
        let mut expired = two_way;
        expired.compatible = [[false; 4]; 2];
        let (allocations, _) = allocate(&[expired, pair], [8, 3, 0, 0]);
        assert_eq!(allocations, vec![[0; 4], [8, 3, 0, 0]]);
    }

    #[test]
    fn allocation_keeps_one_global_limit_and_respects_fleet_eligibility() {
        let airport = Arc::new(Airport::default());
        let pair = |compatible| Pair {
            origin: airport.clone(),
            destination: airport.clone(),
            departure_runway: "09".into(),
            arrival_runway: "27".into(),
            compatible: [compatible; 2],
        };
        let pairs = [
            pair([true, false, false, true]),
            pair([true, true, false, false]),
            pair([false, true, false, true]),
        ];
        for counts in [[8, 4, 3, 2], [63, 63, 63, 63], [0, 0, 0, 0]] {
            let (allocated, warnings) = allocate(&pairs, counts);
            assert!(
                allocated
                    .iter()
                    .flatten()
                    .map(|count| usize::from(*count))
                    .sum::<usize>()
                    <= 63
            );
            for (pair, allocation) in pairs.iter().zip(&allocated) {
                for (category, count) in allocation.iter().enumerate() {
                    assert!(
                        pair.compatible.iter().any(|direction| direction[category]) || *count == 0
                    );
                }
            }
            if counts == [8, 4, 3, 2] {
                for (category, expected) in [8, 4, 0, 2].into_iter().enumerate() {
                    assert_eq!(
                        allocated
                            .iter()
                            .map(|allocation| usize::from(allocation[category]))
                            .sum::<usize>(),
                        expected
                    );
                }
                assert!(warnings.iter().any(|warning| warning.starts_with("Cargo:")));
            }
        }
    }
}
