//! Bounded worker preparation of model-compatible traffic. No simulator calls live here.
use crate::{EngineState, Shared};
use openatc_core::{
    airport::Airport,
    state::{GeneratedTraffic, Point, TrafficTarget},
    traffic::{
        animation::{self, Animation},
        csl::Catalogue,
        profiles::{Category, Port, Profiles},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant},
};
mod airborne;
mod departure;
mod display;
mod enroute;
mod environment;
mod flight;
mod landing;
mod policy;
mod radio;
mod routing;
mod runway;
pub use radio::tick as radio_tick;

#[derive(Default, Clone)]
pub struct Runtime {
    key: String,
    preparation_job: Option<u64>,
    retry: Option<Instant>,
    frames: Vec<GeneratedTraffic>,
    visible: BTreeSet<u32>,
    world: openatc_core::traffic::motion::World,
    pub(crate) airport: Option<Arc<Airport>>,
    pub(crate) destination: Option<Arc<Airport>>,
    departure_procedures: Option<Arc<openatc_core::traffic::procedures::Catalogue>>,
    arrival_procedures: Option<Arc<openatc_core::traffic::procedures::Catalogue>>,
    terminal_points: Option<Arc<openatc_core::traffic::coordinates::Points>>,
    published_holds: Option<Arc<openatc_core::traffic::holding::published::Catalogue>>,
    nominal_departures: BTreeMap<u32, Arc<departure::Surface>>,
    decoded_terminals: BTreeMap<u32, Arc<airborne::Decoded>>,
    nominal_arrivals: BTreeMap<u32, Arc<landing::Arrival>>,
    spoken_callsigns: BTreeMap<String, String>,
    movement: BTreeMap<u32, animation::Handling>,
    previous_time: Option<f64>,
    epoch: Option<u64>,
    pub status: String,
    policy: Option<Operations>,
    fixed_gear: BTreeSet<u32>,
    simulation_job: Option<u64>,
    channels: BTreeMap<String, animation::Channel>,
    orders: BTreeMap<u32, Vec<(usize, openatc_core::state::TaxiClearance)>>,
    approved_orders: BTreeSet<(u32, usize)>,
    exchange: Option<radio::Exchange>,
    reported_crossings: BTreeSet<(u32, usize)>,
    warnings: Vec<String>,
    reported_holds: BTreeSet<(u32, usize)>,
    tower_notified: BTreeSet<u32>,
    tower_ready: BTreeSet<u32>,
    flights: BTreeMap<u32, Arc<flight::Prepared>>,
    terminal_routes: Vec<routing::Terminal>,
    environments: BTreeMap<String, environment::Environment>,
    active_flights: BTreeSet<u32>,
    handoff_targets: BTreeMap<u32, usize>,
    pending_checkins: BTreeSet<(u32, usize)>,
}

impl Runtime {
    /// Expand only whole flight identities; TCAS and target addressing keep their short codes.
    pub(crate) fn spoken_text(&self, text: &str) -> String {
        let mut output = String::with_capacity(text.len());
        for part in text.split_inclusive(|c: char| !c.is_ascii_alphanumeric()) {
            let word = part.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
            output.push_str(self.spoken_callsigns.get(word).map_or(word, String::as_str));
            output.push_str(&part[word.len()..]);
        }
        output
    }
    fn crossing_blocked(&self, id: u32, index: usize, targets: &[TrafficTarget]) -> bool {
        let segment = self
            .flights
            .get(&id)
            .filter(|_| self.active_flights.contains(&id))
            .and_then(|flight| flight.route.segment(index))
            .or_else(|| self.world.current_segment(id));
        let Some(segment) = segment else {
            return true;
        };
        if segment.stage != openatc_core::traffic::motion::Stage::Crossing {
            return false;
        }
        let airport = self
            .orders
            .get(&id)
            .and_then(|orders| orders.iter().find(|(segment, _)| *segment == index))
            .and_then(|(_, order)| self.surface_airport(&order.airport))
            .or_else(|| {
                self.flights
                    .get(&id)
                    .and_then(|flight| flight.clearances.get(&index))
                    .and_then(|clearance| self.surface_airport(&clearance.station.airport))
            });
        let (Some(airport), Some(policy)) = (airport, self.policy.as_ref()) else {
            return true;
        };
        let mut runways = airport
            .runways
            .iter()
            .filter(|runway| {
                segment.resources.contains(&format!(
                    "RUNWAY/{}/{}/{}",
                    airport.icao, runway.first_name, runway.second_name
                ))
            })
            .peekable();
        if runways.peek().is_none() {
            return true;
        }
        let seconds = segment
            .points
            .windows(2)
            .map(|pair| {
                openatc_core::arrival::distance_nm(
                    [pair[0].latitude, pair[0].longitude],
                    [pair[1].latitude, pair[1].longitude],
                ) * 1852.0
            })
            .sum::<f64>()
            / (segment.knots.max(1.0) * 0.514444)
            + policy.runway_clearance_buffer_seconds;
        runways.any(|runway| {
            openatc_core::holding::blocked_during(
                airport,
                &runway.first_name,
                targets.iter().filter(|target| target.id != id),
                seconds,
            )
            .is_some()
        })
    }
    fn surface_airport(&self, id: &str) -> Option<&Airport> {
        self.airport
            .iter()
            .chain(self.destination.iter())
            .map(Arc::as_ref)
            .find(|airport| id.is_empty() || airport.icao == id)
    }
}

fn next_job() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
fn finish_job(owner: &mut Option<u64>, job: u64) -> bool {
    if *owner != Some(job) {
        return false;
    }
    *owner = None;
    true
}

fn port(airport: &Airport) -> Option<Port> {
    Some(Port {
        icao: airport.icao.clone(),
        runway: airport
            .runways
            .iter()
            .map(|runway| {
                (runway.second.east - runway.first.east)
                    .hypot(runway.second.north - runway.first.north)
            })
            .reduce(f64::max)?,
        parking_size: airport
            .parking
            .iter()
            .filter(|stand| openatc_core::parking::fixed_wing_stand(stand))
            .filter_map(openatc_core::parking::stand_size)
            .max()?,
    })
}
fn airport(root: &str, id: &str) -> Result<Airport, String> {
    openatc_core::airport::load_airport_from_simulator(
        root,
        id,
        &|path| std::path::Path::new(path).is_file(),
        &|path| std::fs::read_to_string(path).map_err(|error| error.to_string()),
    )
}
fn position(airport: &Airport, point: Point) -> [f64; 2] {
    [
        airport.reference_latitude + point.north / 111320.0,
        airport.reference_longitude
            + point.east / (111320.0 * airport.reference_latitude.to_radians().cos()),
    ]
}
struct Preparation {
    simulator_root: String,
    origin: Airport,
    destination: Airport,
    profiles: Arc<Profiles>,
    models: Arc<Catalogue>,
    counts: [u8; 4],
    own: [f64; 2],
    observed: Vec<TrafficTarget>,
    epoch: u64,
    runway: String,
    arrival_runway: String,
    config: std::path::PathBuf,
    weather: BTreeMap<String, (serde_json::Value, Instant)>,
}
struct Scene {
    frames: Vec<GeneratedTraffic>,
    world: openatc_core::traffic::motion::World,
    movement: BTreeMap<u32, animation::Handling>,
    airport: Airport,
    destination: Airport,
    departure_procedures: Option<Arc<openatc_core::traffic::procedures::Catalogue>>,
    arrival_procedures: Option<Arc<openatc_core::traffic::procedures::Catalogue>>,
    terminal_points: Option<Arc<openatc_core::traffic::coordinates::Points>>,
    published_holds: Option<Arc<openatc_core::traffic::holding::published::Catalogue>>,
    nominal_departures: BTreeMap<u32, Arc<departure::Surface>>,
    decoded_terminals: BTreeMap<u32, Arc<airborne::Decoded>>,
    nominal_arrivals: BTreeMap<u32, Arc<landing::Arrival>>,
    spoken_callsigns: BTreeMap<String, String>,
    terminal_routes: Vec<routing::Terminal>,
    environments: BTreeMap<String, environment::Environment>,
    policy: Operations,
    fixed_gear: BTreeSet<u32>,
    channels: BTreeMap<String, animation::Channel>,
    orders: BTreeMap<u32, Vec<(usize, openatc_core::state::TaxiClearance)>>,
    warnings: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Operations {
    holding_turn_rate_degrees_sec: f64,
    version: u32,
    preparation_seconds_min: f64,
    preparation_seconds_max: f64,
    surface_sample_meters: f64,
    surface_segment_meters: f64,
    conflict_cell_meters: f64,
    stand_clearance_meters: f64,
    maximum_surface_segments: usize,
    radio_characters_per_second: f64,
    radio_gap_seconds: f64,
    runway_clearance_buffer_seconds: f64,
    pushback_knots: f64,
    pushback_radius_scale: f64,
    pushback_max_meters: f64,
    flight_clearance_lead_seconds: f64,
    terminal_candidate_limit: usize,
    terminal_intercept_limit_nm: f64,
    enroute_segment_nm: f64,
    airborne_sample_meters: f64,
    airborne_detour_meters: f64,
    airborne_turn_rate_degrees_sec: f64,
    runway_slope_limit_percent: f64,
    visible_aircraft_limit: usize,
    display_enter_nm: f64,
    display_leave_nm: f64,
    display_replacement_margin_nm: f64,
    surface_monitor_radius_nm: f64,
    gate_connector_meters: f64,
    gate_connector_detour_meters: f64,
}
fn parse_operations(text: &str) -> Result<Operations, String> {
    if text.len() > 65536 {
        return Err("Generated operations policy exceeds 64 KiB".into());
    }
    let mut value: toml::Value = toml::from_str(text).map_err(|error| error.to_string())?;
    let defaults: toml::Value = toml::from_str(include_str!("../../../traffic/operations.toml"))
        .map_err(|error| error.to_string())?;
    let table = value
        .as_table_mut()
        .ok_or("Operations policy must be a table")?;
    for (key, value) in defaults.as_table().unwrap() {
        table.entry(key.clone()).or_insert_with(|| value.clone());
    }
    let policy: Operations = value
        .try_into()
        .map_err(|error: toml::de::Error| error.to_string())?;
    if !(1..=63).contains(&policy.visible_aircraft_limit)
        || !(5.0..=100.0).contains(&policy.display_enter_nm)
        || !(policy.display_enter_nm..=150.0).contains(&policy.display_leave_nm)
        || !(0.0..=10.0).contains(&policy.display_replacement_margin_nm)
        || !(1..=32).contains(&policy.terminal_candidate_limit)
        || !(1.0..=100.0).contains(&policy.terminal_intercept_limit_nm)
        || !(5.0..=50.0).contains(&policy.enroute_segment_nm)
        || !(10.0..=250.0).contains(&policy.airborne_sample_meters)
        || !(0.0..=10000.0).contains(&policy.airborne_detour_meters)
        || !(0.5..=3.0).contains(&policy.airborne_turn_rate_degrees_sec)
        || !(0.1..=10.0).contains(&policy.runway_slope_limit_percent)
        || !(1.0..=50.0).contains(&policy.surface_monitor_radius_nm)
        || !(1.0..=350.0).contains(&policy.gate_connector_meters)
        || !(0.0..=500.0).contains(&policy.gate_connector_detour_meters)
        || policy.version != 1
        || !(30.0..=300.0).contains(&policy.flight_clearance_lead_seconds)
        || !(0.5..=3.0).contains(&policy.holding_turn_rate_degrees_sec)
        || !(0.5..=5.0).contains(&policy.pushback_knots)
        || !(0.25..=1.0).contains(&policy.pushback_radius_scale)
        || !(20.0..=500.0).contains(&policy.pushback_max_meters)
        || !(5.0..=600.0).contains(&policy.preparation_seconds_min)
        || !(policy.preparation_seconds_min..=900.0).contains(&policy.preparation_seconds_max)
        || !(1.0..=10.0).contains(&policy.surface_sample_meters)
        || !(10.0..=50.0).contains(&policy.surface_segment_meters)
        || !(25.0..=100.0).contains(&policy.conflict_cell_meters)
        || !(30.0..=150.0).contains(&policy.stand_clearance_meters)
        || !(32..=512).contains(&policy.maximum_surface_segments)
        || !(8.0..=25.0).contains(&policy.radio_characters_per_second)
        || !(0.5..=10.0).contains(&policy.radio_gap_seconds)
        || !(10.0..=90.0).contains(&policy.runway_clearance_buffer_seconds)
    {
        return Err("Generated operations policy has invalid timing or distances".into());
    }
    Ok(policy)
}
fn cells(airport: &Airport, point: Point, radius: f64, size: f64) -> BTreeSet<String> {
    let mut cells = BTreeSet::new();
    let first_x = ((point.east - radius) / size).floor() as i32;
    let last_x = ((point.east + radius) / size).floor() as i32;
    let first_y = ((point.north - radius) / size).floor() as i32;
    let last_y = ((point.north + radius) / size).floor() as i32;
    for x in first_x..=last_x {
        for y in first_y..=last_y {
            cells.insert(format!("GROUND/{}/{x}/{y}", airport.icao));
        }
    }
    cells
}
struct GroundPlan {
    route: openatc_core::traffic::motion::Route,
    orders: Vec<(usize, openatc_core::state::TaxiClearance)>,
}
fn ground_route(
    airport: &Airport,
    stand: Point,
    heading: f64,
    gate: bool,
    aircraft: &openatc_core::traffic::profiles::Aircraft,
    handling: &animation::Handling,
    runway: &str,
    seed: u64,
    policy: &Operations,
    to_parking: bool,
    observed: &[TrafficTarget],
) -> Result<GroundPlan, String> {
    use openatc_core::traffic::motion::{Route, Segment, Stage, Waypoint};
    let start = position(airport, stand);
    let mut telemetry = openatc_core::state::Telemetry {
        latitude: start[0],
        longitude: start[1],
        position_valid: true,
        on_ground: true,
        heading_degrees: heading,
        traffic: Some(observed.to_vec()),
        ..Default::default()
    };
    let mut path = openatc_core::airport::calculate_taxi_route(
        airport,
        &telemetry,
        runway,
        to_parking,
        aircraft.parking_size(),
    )?;
    if path.points.is_empty() {
        return Err("No surface route".into());
    }
    let waypoint = |point: Point| {
        let coordinate = position(airport, point);
        Waypoint {
            latitude: coordinate[0],
            longitude: coordinate[1],
            altitude_feet: airport.elevation_feet + point.height / 0.3048,
        }
    };
    let mut segments = vec![Segment {
        stage: Stage::Parked,
        points: vec![waypoint(stand)],
        knots: 0.0,
        resources: cells(
            airport,
            stand,
            aircraft.wingspan.max(aircraft.length) / 2.0,
            policy.conflict_cell_meters,
        )
        .into_iter()
        .collect(),
        dwell_seconds: policy.preparation_seconds_min
            + (seed % 1000) as f64 / 1000.0
                * (policy.preparation_seconds_max - policy.preparation_seconds_min),
    }];
    if to_parking {
        segments.clear();
    }
    let taxi_stage = if to_parking {
        Stage::TaxiIn
    } else {
        Stage::TaxiOut
    };
    let mut from = stand;
    if gate && !to_parking {
        let first = path
            .points
            .iter()
            .position(|point| (point.east - stand.east).hypot(point.north - stand.north) > 1.0);
        if let Some(index) = first {
            let entry = path.points[index];
            let nose = [heading.to_radians().sin(), heading.to_radians().cos()];
            if (entry.east - stand.east) * nose[0] + (entry.north - stand.north) * nose[1] < -1.0 {
                let next = path
                    .points
                    .iter()
                    .skip(index + 1)
                    .find(|point| (point.east - entry.east).hypot(point.north - entry.north) > 1.0)
                    .ok_or("Pushback needs a known onward taxi direction")?;
                let outgoing = (next.east - entry.east)
                    .atan2(next.north - entry.north)
                    .to_degrees()
                    .rem_euclid(360.0);
                let radius = (handling.taxi_turn_radius_meters * policy.pushback_radius_scale)
                    .max(
                        handling.wheelbase_meters
                            / handling.maximum_steering_degrees.to_radians().tan(),
                    )
                    .max(1.0);
                let endpoint = Point {
                    east: entry.east - radius * outgoing.to_radians().sin(),
                    north: entry.north - radius * outgoing.to_radians().cos(),
                    height: entry.height,
                };
                let curve = openatc_core::traffic::curves::connect(
                    openatc_core::traffic::curves::Pose {
                        point: stand,
                        heading: (heading + 180.0).rem_euclid(360.0),
                    },
                    openatc_core::traffic::curves::Pose {
                        point: endpoint,
                        heading: (outgoing + 180.0).rem_euclid(360.0),
                    },
                    &openatc_core::traffic::curves::Limits {
                        radius,
                        spacing: policy.surface_sample_meters,
                        length: policy.pushback_max_meters,
                        initial_turn: None,
                    },
                    |first, last| {
                        [first, last].iter().all(|sample| {
                            openatc_core::airport::on_pavement(airport, sample.point)
                                && !openatc_core::airport::within_runway(airport, &sample.point)
                        })
                    },
                )?;
                let mut protected = BTreeSet::new();
                for sample in &curve {
                    protected.extend(cells(
                        airport,
                        sample.point,
                        aircraft.wingspan.max(aircraft.length) / 2.0,
                        policy.conflict_cell_meters,
                    ));
                }
                segments.push(Segment {
                    stage: Stage::Pushback,
                    points: curve.iter().map(|sample| waypoint(sample.point)).collect(),
                    knots: policy.pushback_knots,
                    resources: protected.into_iter().collect(),
                    dwell_seconds: 0.0,
                });
                from = endpoint;
                let coordinate = position(airport, endpoint);
                telemetry.latitude = coordinate[0];
                telemetry.longitude = coordinate[1];
                telemetry.heading_degrees = outgoing;
                path = openatc_core::airport::calculate_taxi_route(
                    airport,
                    &telemetry,
                    runway,
                    false,
                    aircraft.parking_size(),
                )?;
            }
        }
    }
    let mut stage = taxi_stage;
    let mut crossing_resource = None;
    let mut crossed = BTreeSet::new();
    let mut orders = Vec::new();
    for _ in 0..16 {
        if to_parking && stage != Stage::Crossing && path.hold_short_runway.is_empty() {
            let target = airport
                .parking
                .iter()
                .find(|stand| stand.name == runway)
                .ok_or("Arrival stand disappeared")?;
            let last = *path
                .points
                .last()
                .ok_or("Arrival has no mapped taxi endpoint")?;
            if (last.east - target.point.east).hypot(last.north - target.point.north) > 0.01 {
                let previous = path.points.iter().rev().nth(1).copied().unwrap_or(from);
                let heading = (last.east - previous.east)
                    .atan2(last.north - previous.north)
                    .to_degrees()
                    .rem_euclid(360.0);
                let distance =
                    (last.east - target.point.east).hypot(last.north - target.point.north);
                if distance > policy.gate_connector_meters {
                    return Err("Arrival stand is too far from its mapped taxi route".into());
                }
                let curve = openatc_core::traffic::curves::connect(
                    openatc_core::traffic::curves::Pose {
                        point: last,
                        heading,
                    },
                    openatc_core::traffic::curves::Pose {
                        point: target.point,
                        heading: target.heading,
                    },
                    &openatc_core::traffic::curves::Limits {
                        radius: handling.taxi_turn_radius_meters,
                        spacing: policy.surface_sample_meters,
                        length: distance + policy.gate_connector_detour_meters,
                        initial_turn: None,
                    },
                    |a, b| {
                        [a, b].iter().all(|sample| {
                            openatc_core::airport::on_pavement(airport, sample.point)
                                && !openatc_core::airport::within_runway(airport, &sample.point)
                        })
                    },
                )?;
                path.points
                    .extend(curve.into_iter().skip(1).map(|sample| sample.point));
            }
        }
        let start_segment = segments.len();
        orders.push((start_segment, path.clone()));
        let raw: Vec<_> = std::iter::once(from)
            .chain(path.points.iter().copied())
            .collect();
        let stationary_hold = raw
            .iter()
            .all(|point| (point.east - from.east).hypot(point.north - from.north) < 0.01);
        if stationary_hold {
            if path.hold_short_runway.is_empty() || stage == Stage::Crossing {
                return Err("Surface clearance has no verified onward movement".into());
            }
            segments.push(Segment {
                stage,
                points: vec![waypoint(from)],
                knots: 0.0,
                resources: cells(
                    airport,
                    from,
                    aircraft.wingspan.max(aircraft.length) / 2.0,
                    policy.conflict_cell_meters,
                )
                .into_iter()
                .collect(),
                dwell_seconds: 0.0,
            });
        }
        let rounded = if stationary_hold {
            Vec::new()
        } else {
            openatc_core::traffic::surface::rounded(
                &raw,
                handling.taxi_turn_radius_meters,
                handling.wheelbase_meters / handling.maximum_steering_degrees.to_radians().tan(),
                policy.surface_sample_meters,
                |point| {
                    openatc_core::airport::on_pavement(airport, point)
                        && (stage == Stage::Crossing
                            || !openatc_core::airport::within_runway(airport, &point))
                },
            )?
        };
        let mut turning = false;
        for sample in &rounded {
            let to = &sample.point;
            let distance = (to.east - from.east).hypot(to.north - from.north);
            if distance < 0.01 {
                continue;
            }
            let samples = (distance / policy.surface_sample_meters).ceil() as usize;
            if samples > 4096
                || (0..=samples).any(|index| {
                    let amount = index as f64 / samples as f64;
                    !openatc_core::airport::on_pavement(
                        airport,
                        Point {
                            east: from.east + (to.east - from.east) * amount,
                            north: from.north + (to.north - from.north) * amount,
                            height: 0.0,
                        },
                    )
                })
            {
                return Err("Unverified surface connector".into());
            }
            let steps = (distance / policy.surface_segment_meters).ceil() as usize;
            let previous = from;
            for index in 1..=steps {
                if segments.len() >= 4096 {
                    return Err("Surface route exceeds segment budget".into());
                }
                let amount = index as f64 / steps as f64;
                let next = Point {
                    east: previous.east + (to.east - previous.east) * amount,
                    north: previous.north + (to.north - previous.north) * amount,
                    height: 0.0,
                };
                let radius = aircraft.wingspan.max(aircraft.length) / 2.0;
                let resources = cells(airport, from, radius, policy.conflict_cell_meters)
                    .union(&cells(airport, next, radius, policy.conflict_cell_meters))
                    .cloned()
                    .collect();
                segments.push(Segment {
                    stage,
                    points: vec![waypoint(from), waypoint(next)],
                    knots: if turning || sample.turning {
                        handling.taxi_turn_knots
                    } else {
                        handling.taxi_knots
                    },
                    resources,
                    dwell_seconds: 0.0,
                });
                from = next;
            }
            turning = sample.turning;
        }
        if stage == Stage::TaxiOut {
            let parts: Vec<_> = segments.drain(start_segment..).collect();
            let mut merged: Vec<Segment> = Vec::new();
            let mut length = 0.0;
            for part in parts {
                let distance = openatc_core::arrival::distance_nm(
                    [part.points[0].latitude, part.points[0].longitude],
                    [
                        part.points.last().unwrap().latitude,
                        part.points.last().unwrap().longitude,
                    ],
                ) * 1852.0;
                if let Some(previous) = merged.last_mut()
                    && previous.knots == part.knots
                    && length + distance <= policy.surface_segment_meters
                {
                    let resources: BTreeSet<_> = previous
                        .resources
                        .iter()
                        .chain(&part.resources)
                        .cloned()
                        .collect();
                    if resources.len() <= 32 {
                        previous.points.extend(part.points.into_iter().skip(1));
                        previous.resources = resources.into_iter().collect();
                        length += distance;
                        continue;
                    }
                }
                length = distance;
                merged.push(part);
            }
            segments.extend(merged);
            if segments.len() > policy.maximum_surface_segments {
                return Err("Surface route exceeds segment budget".into());
            }
        }
        if stage == Stage::Crossing {
            let parts: Vec<_> = segments.drain(start_segment..).collect();
            let mut resources = BTreeSet::new();
            let mut points = Vec::new();
            for part in parts {
                if points.is_empty() {
                    points.push(part.points[0]);
                }
                points.extend(part.points.into_iter().skip(1));
                resources.extend(part.resources);
            }
            resources.insert(
                crossing_resource
                    .take()
                    .ok_or("Crossing has no runway resource")?,
            );
            segments.push(Segment {
                stage: Stage::Crossing,
                points,
                knots: handling.taxi_turn_knots,
                resources: resources.into_iter().collect(),
                dwell_seconds: 0.0,
            });
        } else if to_parking && path.hold_short_runway.is_empty() {
            segments.push(Segment {
                stage: Stage::Turnaround,
                points: vec![waypoint(from)],
                knots: 0.0,
                resources: cells(
                    airport,
                    from,
                    aircraft.wingspan.max(aircraft.length) / 2.0,
                    policy.conflict_cell_meters,
                )
                .into_iter()
                .collect(),
                dwell_seconds: policy.preparation_seconds_min,
            });
            return Ok(GroundPlan {
                route: Route::compile(segments, aircraft.climb_fpm, aircraft.descent_fpm)?,
                orders,
            });
        } else if !to_parking && path.hold_short_runway == runway {
            return Ok(GroundPlan {
                route: Route::compile(segments, aircraft.climb_fpm, aircraft.descent_fpm)?,
                orders,
            });
        }
        let coordinate = position(airport, from);
        telemetry.latitude = coordinate[0];
        telemetry.longitude = coordinate[1];
        if stage == Stage::Crossing {
            stage = taxi_stage;
            path = openatc_core::airport::calculate_taxi_route(
                airport,
                &telemetry,
                runway,
                to_parking,
                aircraft.parking_size(),
            )?;
        } else {
            let crossing = airport
                .runways
                .iter()
                .find(|r| {
                    r.first_name == path.hold_short_runway
                        || r.second_name == path.hold_short_runway
                })
                .ok_or("No protected crossing runway")?;
            let resource = format!(
                "RUNWAY/{}/{}/{}",
                airport.icao, crossing.first_name, crossing.second_name
            );
            if !crossed.insert(resource.clone()) {
                return Err("Surface route repeats a runway crossing".into());
            }
            crossing_resource = Some(resource);
            path = openatc_core::airport::calculate_crossing_route(
                airport,
                &telemetry,
                &path.hold_short_runway,
                aircraft.parking_size(),
            )?;
            stage = Stage::Crossing;
        }
    }
    Err("Surface route exceeds crossing budget".into())
}
fn prepare_scene(mut input: Preparation) -> Result<Scene, String> {
    let (Some(origin), Some(destination)) = (port(&input.origin), port(&input.destination)) else {
        return Err(
            "Generated traffic needs published runway and stand capacities at both airports".into(),
        );
    };
    let mut environments = BTreeMap::new();
    for (airport, runway) in [
        (&input.origin, &mut input.runway),
        (&input.destination, &mut input.arrival_runway),
    ] {
        if let Some(sample) = input.weather.get(&airport.icao)
            && let Ok(weather) = environment::Environment::from_sample(airport, sample)
        {
            *runway = crate::radio::atis_runway(
                &airport.runways,
                runway,
                weather.wind_from,
                weather.wind_knots,
            );
            environments.insert(airport.icao.clone(), weather);
        }
    }
    let distance = openatc_core::arrival::distance_nm(
        [
            input.origin.reference_latitude,
            input.origin.reference_longitude,
        ],
        [
            input.destination.reference_latitude,
            input.destination.reference_longitude,
        ],
    );
    let mut used = BTreeSet::new();
    let mut result: Vec<GeneratedTraffic> = Vec::new();
    let classes = [
        Category::Airline,
        Category::Regional,
        Category::Cargo,
        Category::GeneralAviation,
    ];
    let mut seed = input.epoch.wrapping_add(1);
    for byte in origin.icao.bytes() {
        seed = seed.wrapping_mul(16777619) ^ u64::from(byte);
    }
    let (handling, policy, channels) = policy::load(&input.config)?;
    let mut world = openatc_core::traffic::motion::World::default();
    let mut movement = BTreeMap::new();
    let mut fixed_gear = BTreeSet::new();
    let mut orders = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut terminal = |id: &str| {
        if input.simulator_root.is_empty() {
            return None;
        }
        match openatc_core::traffic::procedures::Catalogue::load(
            std::path::Path::new(&input.simulator_root),
            id,
        ) {
            Ok(catalogue) => Some(Arc::new(catalogue)),
            Err(error) => {
                warnings.push(format!("{id}: {error}"));
                None
            }
        }
    };
    let departure_procedures = terminal(&input.origin.icao);
    let arrival_procedures = terminal(&input.destination.icao);
    let catalogues = departure_procedures
        .as_ref()
        .map(|catalogue| (input.origin.icao.as_str(), catalogue.as_ref()))
        .into_iter()
        .chain(
            arrival_procedures
                .as_ref()
                .map(|catalogue| (input.destination.icao.as_str(), catalogue.as_ref())),
        )
        .collect::<Vec<_>>();
    let terminal_points = if catalogues.is_empty() {
        None
    } else {
        let mut points = openatc_core::traffic::coordinates::Points::requested(&catalogues);
        match points.load(std::path::Path::new(&input.simulator_root)) {
            Ok(()) => Some(Arc::new(points)),
            Err(error) => {
                warnings.push(error);
                None
            }
        }
    };
    let mut shared_models = BTreeMap::new();
    let published_holds = terminal_points
        .as_ref()
        .and_then(|points| points.cycle.as_deref())
        .and_then(|cycle| {
            match openatc_core::traffic::holding::published::Catalogue::load(
                std::path::Path::new(&input.simulator_root),
                &catalogues,
                cycle,
            ) {
                Ok(holds) => Some(Arc::new(holds)),
                Err(error) => {
                    warnings.push(error);
                    None
                }
            }
        });
    let mora = openatc_core::navigation::Mora::load(&input.simulator_root);
    let terminal_routes = match (&departure_procedures, &arrival_procedures, &terminal_points) {
        (Some(departures), Some(arrivals), Some(points)) => {
            match routing::candidates(routing::Inputs {
                origin: &input.origin,
                destination: &input.destination,
                departure_runway: &input.runway,
                arrival_runway: &input.arrival_runway,
                departures,
                arrivals,
                points,
                mora: &mora,
                budget: policy.terminal_candidate_limit,
            }) {
                Ok(routes) => routes,
                Err(error) => {
                    warnings.push(error);
                    Vec::new()
                }
            }
        }
        _ => Vec::new(),
    };
    let mut terminal_surfaces = BTreeMap::new();
    for (airport, name) in [
        (&input.origin, &input.runway),
        (&input.destination, &input.arrival_runway),
    ] {
        let surface = (|| {
            let data = openatc_core::arrival::ArrivalAirport::load(
                &input.simulator_root,
                airport.clone(),
            )?;
            runway::Surface::new(
                airport,
                name,
                &data.thresholds,
                policy.runway_slope_limit_percent,
            )
        })();
        match surface {
            Ok(surface) => {
                terminal_surfaces.insert(airport.icao.clone(), surface);
            }
            Err(error) => warnings.push(format!("{} runway {name}: {error}", airport.icao)),
        }
    }
    let mut nominal_departures = BTreeMap::new();
    let mut decoded_terminals = BTreeMap::new();
    let mut nominal_arrivals = BTreeMap::new();
    let mut spoken_callsigns = BTreeMap::new();
    let mut reserved_arrival_stands = BTreeSet::new();
    for (category, count) in classes.into_iter().zip(input.counts) {
        for _ in 0..count.min(63) {
            if result.len() >= 63 {
                break;
            }
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let Some(choice) = input.profiles.select(
                &origin,
                &destination,
                distance,
                category,
                &input.models,
                seed,
            ) else {
                warnings.push(format!(
                    "{category:?}: no compatible fleet/model for {} to {}",
                    origin.icao, destination.icao
                ));
                continue;
            };
            let Some(aircraft) = input.profiles.aircraft_for(&choice.aircraft) else {
                continue;
            };
            let Some(kind) = aircraft.handling else {
                continue;
            };
            let callsign = if choice.operator.is_empty() {
                let Some(registration) =
                    input.profiles.airlines[&choice.airline].registration(&input.origin.icao, seed)
                else {
                    warnings.push(format!(
                        "{}: no registration pattern for {}",
                        choice.airline, input.origin.icao
                    ));
                    continue;
                };
                registration
            } else {
                format!("{}{}", choice.operator, 100 + seed % 8900)
            };
            if result.iter().any(|frame| frame.target.callsign == callsign) {
                continue;
            }
            if !handling.contains_key(&kind) {
                continue;
            }
            let Some(model) = input
                .models
                .models
                .iter()
                .find(|model| model.id == choice.model)
            else {
                continue;
            };
            let stands =
                input
                    .origin
                    .parking
                    .iter()
                    .enumerate()
                    .filter(|(index, stand)| {
                        if used.contains(index)
                            || !openatc_core::parking::supports_aircraft(stand, aircraft)
                        {
                            return false;
                        }
                        let point = position(&input.origin, stand.point);
                        let safe = (aircraft.wingspan + aircraft.length)
                            .max(policy.stand_clearance_meters)
                            / 1852.0;
                        openatc_core::arrival::distance_nm(point, input.own) > safe
                            && input.observed.iter().filter(|target| target.on_ground).all(
                                |target| {
                                    openatc_core::arrival::distance_nm(
                                        point,
                                        [target.latitude, target.longitude],
                                    ) > safe
                                },
                            )
                            && result.iter().all(|frame| {
                                openatc_core::arrival::distance_nm(
                                    point,
                                    [frame.target.latitude, frame.target.longitude],
                                ) > safe
                            })
                    })
                    .take(32)
                    .collect::<Vec<_>>();
            if stands.is_empty() {
                warnings.push(format!(
                    "{}: no free equipment-compatible stand for {}",
                    input.origin.icao, choice.aircraft
                ));
                continue;
            }
            for (index, stand) in stands {
                used.insert(index);
                let point = position(&input.origin, stand.point);
                let Some(id) = (1..=126).map(|slot| 0xf00000 + slot).find(|id| {
                    !input.observed.iter().any(|target| target.id == *id)
                        && !result.iter().any(|frame| frame.target.id == *id)
                }) else {
                    return Err("No unused generated traffic identity".into());
                };
                if aircraft.fixed_gear {
                    fixed_gear.insert(id);
                }
                let planned = ground_route(
                    &input.origin,
                    stand.point,
                    stand.heading,
                    stand.kind == "gate",
                    aircraft,
                    &handling[&kind],
                    &input.runway,
                    seed,
                    &policy,
                    false,
                    &input.observed,
                );
                if let Ok(route) = planned {
                    let hold = route.route.end_position();
                    if world.insert(world.generation(), id, Arc::new(route.route)) {
                        orders.insert(id, route.orders);
                        world.ground_rates(
                            id,
                            handling[&kind].acceleration_knots_sec,
                            handling[&kind].braking_knots_sec,
                        );
                        movement.insert(id, handling[&kind].clone());
                        let departure_surface = (|| {
                            let environment = environments
                                .get(&input.origin.icao)
                                .ok_or("Departure surface needs fresh airport weather")?;
                            let surface = terminal_surfaces
                                .get(&input.origin.icao)
                                .ok_or("Departure runway surface is unavailable")?;
                            departure::plan(
                                &input.origin,
                                &input.runway,
                                hold,
                                aircraft,
                                &handling[&kind],
                                environment,
                                surface,
                                &policy,
                            )
                        })();
                        match departure_surface {
                            Ok(surface) => {
                                let decoded = (|| {
                                    let points = terminal_points
                                        .as_ref()
                                        .ok_or("Terminal coordinates are unavailable".to_owned())?;
                                    let departure_weather = environments
                                        .get(&input.origin.icao)
                                        .ok_or("Departure weather is unavailable".to_owned())?;
                                    let arrival_weather = environments
                                        .get(&input.destination.icao)
                                        .ok_or("Arrival weather is unavailable".to_owned())?;
                                    let departure_surface =
                                        terminal_surfaces.get(&input.origin.icao).ok_or(
                                            "Departure runway surface is unavailable".to_owned(),
                                        )?;
                                    let arrival_surface =
                                        terminal_surfaces.get(&input.destination.icao).ok_or(
                                            "Arrival runway surface is unavailable".to_owned(),
                                        )?;
                                    airborne::Planner {
                                        rotation: surface.rotation,
                                        origin: &input.origin,
                                        destination: &input.destination,
                                        points,
                                        mora: &mora,
                                        departure_weather,
                                        arrival_weather,
                                        aircraft,
                                        handling: &handling[&kind],
                                        policy: &policy,
                                        departure_surface,
                                        arrival_surface,
                                        observed: &input.observed,
                                        reserved: &reserved_arrival_stands,
                                    }
                                    .arrival(&terminal_routes)
                                })();
                                match decoded {
                                    Ok((decoded, arrival)) => {
                                        reserved_arrival_stands
                                            .insert(arrival.parking.stand.clone());
                                        nominal_arrivals.insert(id, Arc::new(arrival));
                                        decoded_terminals.insert(id, Arc::new(decoded));
                                    }
                                    Err(error) => warnings.push(format!(
                                        "{} / {}: terminal inputs: {error}",
                                        input.origin.icao, stand.name
                                    )),
                                }
                                nominal_departures.insert(id, Arc::new(surface));
                            }
                            Err(error) => warnings.push(format!(
                                "{} / {}: departure surface: {error}",
                                input.origin.icao, stand.name
                            )),
                        }
                    } else {
                        warnings.push(format!(
                            "{} / {}: stand protection overlaps another generated aircraft",
                            input.origin.icao, stand.name
                        ));
                        fixed_gear.remove(&id);
                        continue;
                    }
                } else if let Err(error) = planned {
                    warnings.push(format!(
                        "{} / {} / {}: {error}",
                        input.origin.icao, stand.name, choice.aircraft
                    ));
                    fixed_gear.remove(&id);
                    continue;
                }
                if !choice.operator.is_empty()
                    && !choice.spoken_operator.trim().is_empty()
                    && let Some(number) = callsign.strip_prefix(&choice.operator)
                {
                    spoken_callsigns.insert(
                        callsign.clone(),
                        format!("{} {number}", choice.spoken_operator),
                    );
                }
                result.push(GeneratedTraffic {
                    vertical_speed_fpm: 0.0,
                    body_heading_degrees: Some(stand.heading),
                    target: TrafficTarget {
                        id,
                        callsign,
                        name: input.profiles.airlines[&choice.airline].name.clone(),
                        aircraft: choice.aircraft,
                        latitude: point[0],
                        longitude: point[1],
                        altitude_feet: input.origin.elevation_feet,
                        track_degrees: stand.heading,
                        on_ground: true,
                        ground_speed_knots: Some(0.0),
                        wingspan_meters: Some(aircraft.wingspan),
                        ..Default::default()
                    },
                    model: shared_models
                        .entry(model.id.clone())
                        .or_insert_with(|| Arc::new(model.clone()))
                        .clone(),
                    animation: Animation::default(),
                });
                break;
            }
        }
    }
    if result.is_empty() && input.counts.iter().any(|count| *count > 0) {
        return Err(warnings
            .into_iter()
            .next()
            .unwrap_or_else(|| "No eligible generated aircraft".into()));
    }
    if !decoded_terminals.is_empty() && environments.values().any(|weather| !weather.fresh()) {
        decoded_terminals.clear();
        nominal_arrivals.clear();
        warnings.push("Terminal weather expired before scene preparation completed".into());
    }
    for airport in [&input.origin, &input.destination] {
        if !environments.contains_key(&airport.icao) {
            warnings.push(format!(
                "{} needs fresh airport weather before an airborne flight can be planned",
                airport.icao
            ));
        }
    }
    Ok(Scene {
        frames: result,
        world,
        movement,
        airport: input.origin,
        destination: input.destination,
        departure_procedures,
        arrival_procedures,
        terminal_points,
        published_holds,
        nominal_departures,
        decoded_terminals,
        nominal_arrivals,
        spoken_callsigns,
        terminal_routes,
        environments,
        policy,
        fixed_gear,
        channels,
        orders,
        warnings,
    })
}

fn scene_key(state: &EngineState) -> String {
    format!(
        "{}|{}|{}|{}|{}|{:?}|{}",
        state.settings.simulator_root,
        state.session.plan.departure,
        state.session.plan.destination,
        state.session.plan.runway,
        state.session.plan.arrival_runway,
        [
            state.settings.airline_traffic,
            state.settings.regional_traffic,
            state.settings.cargo_traffic,
            state.settings.ga_traffic
        ],
        state.session_generation
    )
}

pub async fn prepare(shared: Shared) {
    let (
        job,
        key,
        epoch,
        settings,
        plan,
        profiles,
        models,
        config,
        workers,
        own,
        observed,
        weather,
    ) = {
        let mut state = shared.write().await;
        if !state.settings.generated_traffic || !state.session.telemetry.position_valid {
            return;
        }
        let key = scene_key(&state);
        if state.generated.epoch == Some(state.session_generation)
            && state.generated.key != key
            && state.generated.world.snapshot().iter().any(|pose| {
                !pose.complete
                    && !matches!(
                        pose.stage,
                        openatc_core::traffic::motion::Stage::Parked
                            | openatc_core::traffic::motion::Stage::Turnaround
                    )
            })
        {
            state.generated.status =
                "Traffic changes are queued while existing aircraft finish their clearances".into();
            return;
        }
        if state.generated.preparation_job.is_some()
            || state.generated.simulation_job.is_some()
            || state.generated.key == key
            || state
                .generated
                .retry
                .is_some_and(|time| time.elapsed() < Duration::from_secs(10))
        {
            return;
        }
        let job = next_job();
        state.generated.preparation_job = Some(job);
        state.generated.retry = Some(Instant::now());
        (
            job,
            key,
            state.session_generation,
            state.settings.clone(),
            state.session.plan.clone(),
            state.traffic_profiles.clone(),
            state.model_installer.clone(),
            state.config_dir.clone(),
            state.workers.clone(),
            [
                state.session.telemetry.latitude,
                state.session.telemetry.longitude,
            ],
            state.session.telemetry.traffic.clone().unwrap_or_default(),
            [
                &state.session.plan.departure,
                &state.session.plan.destination,
            ]
            .into_iter()
            .filter_map(|id| {
                state
                    .sim_weather
                    .get(id)
                    .map(|sample| (id.clone(), sample.clone()))
            })
            .collect(),
        )
    };
    let result = async {
        let profiles = profiles.get(&config, &workers, false).await?;
        let models = models.models(&settings.simulator_root, &workers).await?;
        let counts = [
            settings.airline_traffic,
            settings.regional_traffic,
            settings.cargo_traffic,
            settings.ga_traffic,
        ];
        workers
            .run(move || {
                prepare_scene(Preparation {
                    simulator_root: settings.simulator_root.clone(),
                    origin: airport(&settings.simulator_root, &plan.departure)?,
                    destination: airport(&settings.simulator_root, &plan.destination)?,
                    profiles,
                    models,
                    counts,
                    own,
                    observed,
                    epoch,
                    runway: plan.runway.clone(),
                    arrival_runway: plan.arrival_runway.clone(),
                    config,
                    weather,
                })
            })
            .await?
    }
    .await;
    let mut state = shared.write().await;
    if !finish_job(&mut state.generated.preparation_job, job) {
        return;
    }
    if state.session_generation != epoch
        || !state.settings.generated_traffic
        || scene_key(&state) != key
    {
        return;
    }
    match result {
        Ok(scene) => {
            state.generated.status = format!(
                "{} generated surface aircraft prepared; {} terminal candidates, {} aircraft-specific terminal inputs, {} joined arrivals; full airborne planning is still under development",
                scene.frames.len(),
                scene.terminal_routes.len(),
                scene.decoded_terminals.len(),
                scene.nominal_arrivals.len()
            );
            state.generated.terminal_routes = scene.terminal_routes;
            state.generated.environments = scene.environments;
            state.generated.key = key;
            state.generated.frames = scene.frames;
            state.generated.visible.clear();
            state.generated.world = scene.world;
            state.generated.movement = scene.movement;
            state.generated.airport = Some(Arc::new(scene.airport));
            state.generated.destination = Some(Arc::new(scene.destination));
            state.generated.departure_procedures = scene.departure_procedures;
            state.generated.arrival_procedures = scene.arrival_procedures;
            state.generated.terminal_points = scene.terminal_points;
            state.generated.published_holds = scene.published_holds;
            state.generated.nominal_departures = scene.nominal_departures;
            state.generated.decoded_terminals = scene.decoded_terminals;
            state.generated.nominal_arrivals = scene.nominal_arrivals;
            state.generated.spoken_callsigns = scene.spoken_callsigns;
            state.generated.policy = Some(scene.policy);
            state.generated.fixed_gear = scene.fixed_gear;
            state.generated.channels = scene.channels;
            state.generated.orders = scene.orders;
            state.generated.warnings = scene.warnings;
            state.generated.previous_time = None;
            state.generated.epoch = Some(epoch);
            state.generated.simulation_job = None;
        }
        Err(error) => state.generated.status = error,
    }
}

fn external_surface_cells(
    runtime: &Runtime,
    telemetry: &openatc_core::state::Telemetry,
) -> BTreeSet<String> {
    let Some(policy) = &runtime.policy else {
        return BTreeSet::new();
    };
    let mut blocked = BTreeSet::new();
    let mut add = |target: &TrafficTarget| {
        for airport in runtime.airport.iter().chain(runtime.destination.iter()) {
            if openatc_core::arrival::distance_nm(
                [airport.reference_latitude, airport.reference_longitude],
                [target.latitude, target.longitude],
            ) > policy.surface_monitor_radius_nm
            {
                continue;
            }
            blocked.extend(cells(
                airport,
                openatc_core::airport::airport_point(airport, target.latitude, target.longitude),
                target.wingspan_meters.unwrap_or(80.0) / 2.0,
                policy.conflict_cell_meters,
            ));
            // Runway phases reserve the runway itself, not only its taxi conflict cells.
            for runway in &airport.runways {
                if openatc_core::holding::blocked_targets(
                    airport,
                    &runway.first_name,
                    std::iter::once(target),
                )
                .is_some()
                {
                    blocked.insert(format!(
                        "RUNWAY/{}/{}/{}",
                        airport.icao, runway.first_name, runway.second_name
                    ));
                }
            }
        }
    };
    if telemetry.on_ground {
        add(&TrafficTarget {
            id: 0,
            aircraft: telemetry.aircraft_icao.clone(),
            latitude: telemetry.latitude,
            longitude: telemetry.longitude,
            altitude_feet: telemetry.altitude_feet,
            track_degrees: telemetry.heading_degrees,
            on_ground: true,
            wingspan_meters: telemetry.wingspan_meters,
            ..Default::default()
        });
    }
    if let Some(observed) = &telemetry.traffic {
        for target in observed
            .iter()
            .filter(|target| target.on_ground && !runtime.movement.contains_key(&target.id))
        {
            add(target);
        }
    }
    blocked
}

fn observed_targets(
    runtime: &Runtime,
    telemetry: &openatc_core::state::Telemetry,
) -> Vec<TrafficTarget> {
    let ids: BTreeSet<_> = runtime.frames.iter().map(|frame| frame.target.id).collect();
    let mut targets: Vec<_> = telemetry
        .traffic
        .as_ref()
        .into_iter()
        .flatten()
        .take(63)
        .filter(|target| !ids.contains(&target.id))
        .cloned()
        .collect();
    targets.extend(runtime.frames.iter().map(|frame| frame.target.clone()));
    targets.push(TrafficTarget {
        id: 0,
        aircraft: telemetry.aircraft_icao.clone(),
        latitude: telemetry.latitude,
        longitude: telemetry.longitude,
        altitude_feet: telemetry.altitude_feet,
        track_degrees: telemetry.heading_degrees,
        on_ground: telemetry.on_ground,
        ground_speed_knots: Some(telemetry.ground_speed_knots),
        wingspan_meters: telemetry.wingspan_meters,
        ..Default::default()
    });
    targets
}

fn advance(runtime: &mut Runtime, telemetry: &openatc_core::state::Telemetry) {
    if let (Some(_airport), Some(seconds), Some(_policy)) = (
        &runtime.airport,
        telemetry.simulation_seconds,
        &runtime.policy,
    ) {
        let blocked = external_surface_cells(runtime, telemetry);
        runtime
            .world
            .recheck_surface(&blocked, telemetry.traffic.is_some());
        if telemetry.traffic.is_some() {
            let targets = observed_targets(runtime, telemetry);
            let generation = runtime.world.generation();
            for pose in runtime.world.snapshot() {
                if pose.recovering {
                    continue;
                }
                if pose.awaiting_clearance {
                    // Complete flights carry their own clearances after legacy ground orders are removed.
                    if runtime.active_flights.contains(&pose.id)
                        && pose.stage != openatc_core::traffic::motion::Stage::Turnaround
                        && !runtime.approved_orders.contains(&(pose.id, pose.segment))
                    {
                        continue;
                    }
                    if !matches!(
                        pose.stage,
                        openatc_core::traffic::motion::Stage::Parked
                            | openatc_core::traffic::motion::Stage::Turnaround
                            | openatc_core::traffic::motion::Stage::TaxiOut
                            | openatc_core::traffic::motion::Stage::TaxiIn
                            | openatc_core::traffic::motion::Stage::Crossing
                    ) && !runtime.approved_orders.contains(&(pose.id, pose.segment))
                    {
                        continue;
                    }
                    if runtime.orders.get(&pose.id).is_some_and(|orders| {
                        orders.iter().any(|(segment, _)| *segment == pose.segment)
                    }) && !runtime.approved_orders.contains(&(pose.id, pose.segment))
                    {
                        continue;
                    }
                    if runtime.crossing_blocked(pose.id, pose.segment, &targets) {
                        continue;
                    }
                    runtime
                        .world
                        .authorise(generation, pose.id, pose.segment, &blocked);
                }
                if !runtime.active_flights.contains(&pose.id) {
                    runtime
                        .world
                        .reserve_next_taxi(generation, pose.id, &blocked);
                }
                if runtime.active_flights.contains(&pose.id)
                    && runtime
                        .approved_orders
                        .contains(&(pose.id, pose.segment + 1))
                {
                    runtime.world.authorise_continuation(
                        generation,
                        pose.id,
                        pose.segment + 1,
                        &blocked,
                    );
                }
            }
        }
        runtime.world.advance(seconds, telemetry.paused);
        let elapsed = runtime
            .previous_time
            .replace(seconds)
            .map_or(0.0, |previous| {
                let elapsed = seconds - previous;
                if (0.0..=30.0).contains(&elapsed) {
                    elapsed
                } else {
                    0.0
                }
            });
        for pose in runtime.world.snapshot() {
            let Some(frame) = runtime
                .frames
                .iter_mut()
                .find(|frame| frame.target.id == pose.id)
            else {
                continue;
            };
            frame.target.latitude = pose.position.latitude;
            frame.target.longitude = pose.position.longitude;
            frame.vertical_speed_fpm = if elapsed > 0.0 {
                (pose.position.altitude_feet - frame.target.altitude_feet) * 60.0 / elapsed
            } else {
                0.0
            };
            frame.target.altitude_feet = pose.position.altitude_feet;
            let body = (pose.heading
                + if pose.stage == openatc_core::traffic::motion::Stage::Pushback {
                    180.0
                } else {
                    0.0
                })
            .rem_euclid(360.0);
            let turn = if pose.knots > 0.0 {
                openatc_core::navigation::turn(
                    frame
                        .body_heading_degrees
                        .unwrap_or(frame.target.track_degrees),
                    body,
                )
            } else {
                0.0
            };
            if pose.knots > 0.0 {
                frame.target.track_degrees = pose.heading;
                frame.body_heading_degrees = Some(body);
            }
            frame.target.ground_speed_knots = Some(pose.knots);
            frame.target.on_ground = pose.stage.on_ground();
            let Some(handling) = runtime.movement.get(&pose.id) else {
                continue;
            };
            let steps = elapsed.ceil().clamp(1.0, 30.0) as usize;
            for _ in 0..steps {
                frame.animation.advance(
                    animation::Input {
                        stage: pose.stage,
                        knots: pose.knots,
                        turn_degrees_sec: if elapsed > 0.0 { turn / elapsed } else { 0.0 },
                        vertical_fpm: frame.vertical_speed_fpm,
                        engines_running: !matches!(
                            pose.stage,
                            openatc_core::traffic::motion::Stage::Parked
                                | openatc_core::traffic::motion::Stage::Turnaround
                        ) && (pose.stage
                            != openatc_core::traffic::motion::Stage::Pushback
                            || runtime.approved_orders.contains(&(pose.id, pose.segment))),
                        fixed_gear: runtime.fixed_gear.contains(&pose.id),
                        landing_lights: false,
                    },
                    handling,
                    if telemetry.paused {
                        0.0
                    } else {
                        elapsed / steps as f64
                    },
                );
            }
        }
    }
}

/// Publish the most recent complete worker sample without waiting for simulation.
pub fn step(state: &mut EngineState) {
    if !state.settings.generated_traffic {
        state.generated = Runtime::default();
    }
    if !state.settings.generated_traffic || state.generated.epoch != Some(state.session_generation)
    {
        state.session.generated_traffic.clear();
        state.session.generated_animation_channels.clear();
        state.session.generated_traffic_diagnostics.clear();
        return;
    }
    let visible = state
        .generated
        .policy
        .as_ref()
        .map(|policy| {
            display::select(
                &state.generated.frames,
                &state.session.telemetry,
                &state.generated.visible,
                policy,
            )
        })
        .unwrap_or_default();
    state.generated.visible = visible.iter().map(|frame| frame.target.id).collect();
    state.session.generated_traffic = visible;
    state
        .session
        .generated_animation_channels
        .clone_from(&state.generated.channels);
    state
        .session
        .generated_traffic_diagnostics
        .clone_from(&state.generated.warnings);
    if let Some(observed) = &mut state.session.telemetry.traffic {
        let ids: BTreeSet<_> = state
            .generated
            .frames
            .iter()
            .map(|frame| frame.target.id)
            .collect();
        observed.retain(|target| !ids.contains(&target.id));
        observed.extend(
            state
                .generated
                .frames
                .iter()
                .map(|frame| frame.target.clone()),
        );
    }
}

/// At most one simulation job is outstanding; later telemetry replaces the next input.
pub async fn simulate(shared: Shared) {
    let (job, mut runtime, telemetry, workers, key, epoch) = {
        let mut state = shared.write().await;
        if !state.settings.generated_traffic
            || state.generated.simulation_job.is_some()
            || state.generated.preparation_job.is_some()
            || state.generated.epoch != Some(state.session_generation)
        {
            return;
        }
        let job = next_job();
        state.generated.simulation_job = Some(job);
        (
            job,
            state.generated.clone(),
            state.session.telemetry.clone(),
            state.workers.clone(),
            state.generated.key.clone(),
            state.session_generation,
        )
    };
    let result = workers
        .run(move || {
            advance(&mut runtime, &telemetry);
            runtime
        })
        .await;
    let mut state = shared.write().await;
    if !finish_job(&mut state.generated.simulation_job, job) {
        return;
    }
    if state.session_generation != epoch || state.generated.key != key {
        return;
    }
    if !state.settings.generated_traffic {
        return;
    }
    match result {
        Ok(runtime) => {
            state.generated.world = runtime.world;
            state.generated.frames = runtime.frames;
            state.generated.previous_time = runtime.previous_time;
            step(&mut state);
        }
        Err(error) => state.generated.status = error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arrival_surface_monitoring_uses_both_airports() {
        let mut runtime = Runtime::default();
        runtime.airport = Some(Arc::new(Airport {
            icao: "ORIG".into(),
            reference_latitude: 40.0,
            reference_longitude: 0.0,
            ..Default::default()
        }));
        runtime.destination = Some(Arc::new(Airport {
            icao: "DEST".into(),
            reference_latitude: 45.0,
            reference_longitude: 8.0,
            ..Default::default()
        }));
        runtime.policy =
            Some(parse_operations(include_str!("../../../traffic/operations.toml")).unwrap());
        assert_eq!(runtime.surface_airport("DEST").unwrap().icao, "DEST");
        assert_eq!(runtime.surface_airport("ORIG").unwrap().icao, "ORIG");
        assert!(runtime.surface_airport("UNKNOWN").is_none());
        let telemetry = openatc_core::state::Telemetry {
            latitude: 45.0,
            longitude: 8.0,
            on_ground: true,
            traffic: Some(vec![]),
            ..Default::default()
        };
        let blocked = external_surface_cells(&runtime, &telemetry);
        let expected = cells(
            runtime.surface_airport("DEST").unwrap(),
            Point::default(),
            40.0,
            50.0,
        );
        assert!(!expected.is_empty());
        assert!(expected.is_subset(&blocked));
        let origin = cells(
            runtime.surface_airport("ORIG").unwrap(),
            Point::default(),
            40.0,
            50.0,
        );
        assert!(origin.is_disjoint(&blocked));
    }
    #[test]
    fn pushback_waits_for_the_completed_radio_exchange() {
        use openatc_core::traffic::motion::{Route, Segment, Stage, Waypoint};
        let mut state = crate::test_state();
        state.settings.generated_traffic = true;
        state.generated.epoch = Some(state.session_generation);
        state.settings.hear_traffic_radio = true;
        state.generated.policy =
            Some(parse_operations(include_str!("../../../traffic/operations.toml")).unwrap());
        state.generated.airport = Some(Arc::new(Airport {
            icao: "TEST".into(),
            ..Default::default()
        }));
        state.stations.push(openatc_core::stations::Station {
            airport: "TEST".into(),
            name: "Test Ground".into(),
            service: "Ground".into(),
            khz: 121700,
            ..Default::default()
        });
        state.session.telemetry.com1_khz = 121700;
        state.session.telemetry.com2_khz = Some(121700);
        state.session.control_radio = openatc_core::state::Radio::Com2;
        state.session.telemetry.radio_power = true;
        state.generated.frames.push(GeneratedTraffic {
            vertical_speed_fpm: 0.0,
            body_heading_degrees: None,
            target: TrafficTarget {
                id: 7,
                callsign: "TEST7".into(),
                ..Default::default()
            },
            model: Arc::new(openatc_core::traffic::csl::Model {
                id: "TEST".into(),
                objects: vec![],
                vertical_offset: None,
                matches: vec![],
            }),
            animation: Animation::default(),
        });
        let point = Waypoint {
            latitude: 43.0,
            longitude: 7.0,
            altitude_feet: 10.0,
        };
        assert!(
            state.generated.world.insert(
                0,
                7,
                Route::compile(
                    vec![Segment {
                        stage: Stage::Pushback,
                        points: vec![
                            point,
                            Waypoint {
                                latitude: 43.0001,
                                ..point
                            }
                        ],
                        knots: 3.0,
                        resources: vec!["PUSHBACK".into()],
                        dwell_seconds: 0.0,
                    }],
                    100.0,
                    100.0
                )
                .unwrap()
                .into()
            )
        );
        for _ in 0..3 {
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            radio_tick(&mut state);
            assert!(!state.generated.approved_orders.contains(&(7, 0)));
            assert_eq!(state.generated.world.snapshot()[0].knots, 0.0);
        }
        state.generated.exchange.as_mut().unwrap().due = Instant::now();
        radio_tick(&mut state);
        assert!(state.generated.approved_orders.contains(&(7, 0)));
        assert_eq!(state.session.transcript.len(), 3);
        assert!(
            state
                .session
                .transcript
                .iter()
                .all(|entry| entry.radio == Some(openatc_core::state::Radio::Com2))
        );
        assert!(state.session.transcript[0].text.contains("pushback"));
        assert!(state.session.transcript[2].text.contains("ready to taxi"));
    }
    #[tokio::test]
    async fn reset_while_worker_is_queued_does_not_block_or_replace_the_new_scene() {
        let mut engine = crate::test_state();
        engine.session.telemetry.position_valid = true;
        engine.settings.generated_traffic = true;
        let frame = |id| GeneratedTraffic {
            vertical_speed_fpm: 0.0,
            body_heading_degrees: None,
            target: TrafficTarget {
                id,
                ..Default::default()
            },
            model: Arc::new(openatc_core::traffic::csl::Model {
                id: "TEST".into(),
                objects: vec![],
                vertical_offset: None,
                matches: vec![],
            }),
            animation: Animation::default(),
        };
        engine.generated.epoch = Some(engine.session_generation);
        engine.generated.key = "old".into();
        engine.generated.frames = vec![frame(1)];
        let workers = engine.workers.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let blocker = tokio::spawn(async move {
            workers
                .run(move || {
                    let _ = started.send(());
                    let _ = wait.recv();
                })
                .await
                .unwrap();
        });
        ready.await.unwrap();
        let shared = Arc::new(tokio::sync::RwLock::new(engine));
        let old = tokio::spawn(simulate(shared.clone()));
        tokio::time::timeout(Duration::from_secs(2), async {
            while shared.read().await.generated.simulation_job.is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let old_job = shared.read().await.generated.simulation_job;
        {
            let mut engine = shared.write().await;
            engine.session_generation += 1;
            engine.generated = Runtime::default();
            engine.generated.policy =
                Some(parse_operations(include_str!("../../../traffic/operations.toml")).unwrap());
            engine.generated.epoch = Some(engine.session_generation);
            engine.generated.key = "new".into();
            engine.generated.frames = vec![frame(2)];
        }
        let new = tokio::spawn(simulate(shared.clone()));
        tokio::time::timeout(Duration::from_secs(2), async {
            while shared.read().await.generated.simulation_job.is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_ne!(old_job, shared.read().await.generated.simulation_job);
        release.send(()).unwrap();
        blocker.await.unwrap();
        old.await.unwrap();
        new.await.unwrap();
        let engine = shared.read().await;
        assert!(engine.generated.simulation_job.is_none());
        assert_eq!(engine.generated.frames[0].target.id, 2);
        assert_eq!(engine.session.generated_traffic[0].target.id, 2);
    }
    #[test]
    #[ignore = "requires installed scenery and Bluebell"]
    fn installed_nice_scene_has_verified_departure_routes() {
        let root = std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap();
        let profiles = Arc::new(Profiles::builtin().unwrap());
        let scene = prepare_scene(Preparation {
            simulator_root: root.clone(),
            origin: airport(&root, "LFMN").unwrap(),
            destination: airport(&root, "LIMC").unwrap(),
            profiles: profiles.clone(),
            models: Arc::new(
                Catalogue::scan(
                    &std::path::Path::new(&root).join("Resources/plugins/OpenATC/Resources/CSL"),
                )
                .unwrap(),
            ),
            counts: [8, 4, 2, 5],
            own: [43.655764, 7.202480],
            observed: vec![],
            epoch: 1,
            runway: "04R".into(),
            arrival_runway: "35R".into(),
            config: std::path::PathBuf::new(),
            weather: ["LFMN","LIMC"].into_iter().map(|id| {
                let airport=airport(&root,id).unwrap();
                (id.into(),(serde_json::json!({"airport":id,"latitude":airport.reference_latitude,"longitude":airport.reference_longitude,
                    "sampleAltitudeFeet":airport.elevation_feet,"pressureHpa":1013.25,"temperatureC":15.0,
                    "magneticVariationDegrees":0.0,"windDegrees":0.0,"windKnots":0.0}),Instant::now()))
            }).collect(),
        })
        .unwrap();
        assert!(!scene.frames.is_empty(), "No compatible installed aircraft");
        assert!(
            !scene.terminal_routes.is_empty(),
            "No production terminal candidates: {:?}",
            scene.warnings
        );
        assert!(
            !scene.movement.is_empty(),
            "No verified ground routes: {:?}",
            scene.warnings
        );
        assert!(
            !scene.nominal_departures.is_empty(),
            "No nominal protected departure surfaces: {:?}",
            scene.warnings
        );
        let entry = &scene
            .arrival_procedures
            .as_ref()
            .unwrap()
            .approach("I35RZ", "INLER")
            .unwrap()
            .transition[0];
        assert!(
            scene
                .published_holds
                .as_ref()
                .unwrap()
                .at(entry, "LIMC")
                .iter()
                .any(|leg| leg.altitude_limits().unwrap() == (Some(4000), None)
                    && leg.speed_knots == Some(190.0))
        );
        assert!(
            !scene.decoded_terminals.is_empty(),
            "No aircraft-specific production terminal inputs: {:?}",
            scene.warnings
        );
        assert!(
            scene.decoded_terminals.keys().any(|id| profiles
                .aircraft_for(
                    &scene
                        .frames
                        .iter()
                        .find(|frame| frame.target.id == *id)
                        .unwrap()
                        .target
                        .aircraft
                )
                .unwrap()
                .propeller),
            "No aircraft-specific turboprop terminal inputs"
        );
        assert!(
            !scene.nominal_arrivals.is_empty(),
            "No threshold-to-parking arrivals: {:?}",
            scene.warnings
        );
        for propeller in [false, true] {
            assert!(
                scene.nominal_arrivals.keys().any(|id| profiles
                    .aircraft_for(
                        &scene
                            .frames
                            .iter()
                            .find(|frame| frame.target.id == *id)
                            .unwrap()
                            .target
                            .aircraft
                    )
                    .unwrap()
                    .propeller
                    == propeller),
                "No joined arrival for propeller={propeller}: {:?}",
                scene.warnings
            );
        }
        assert!(!scene.spoken_callsigns.is_empty());
        for (short, spoken) in &scene.spoken_callsigns {
            assert!(
                scene
                    .frames
                    .iter()
                    .any(|frame| frame.target.callsign == *short)
            );
            let number = &short[3..];
            assert!(number.bytes().all(|digit| digit.is_ascii_digit()));
            assert!(spoken.ends_with(&format!(" {number}")));
            assert!(
                profiles
                    .airlines
                    .values()
                    .any(|airline| short.starts_with(&airline.operator)
                        && spoken == &format!("{} {number}", airline.callsign))
            );
        }
        let mut stands = BTreeSet::new();
        for (id, arrival) in &scene.nominal_arrivals {
            assert!(
                stands.insert(arrival.parking.stand.clone()),
                "Two nominal arrivals selected the same gate"
            );
            let crossing = scene.decoded_terminals[id].approach.last().unwrap();
            assert_eq!(
                arrival.flare.points[0].altitude_feet,
                crossing.preferred_feet
            );
            assert_eq!(
                arrival.flare.points.last(),
                arrival.parking.exit.rollout[0].points.first()
            );
            assert_eq!(
                arrival.parking.exit.rollout.last().unwrap().points.last(),
                arrival
                    .parking
                    .surface
                    .route
                    .segment(0)
                    .unwrap()
                    .points
                    .first()
            );
        }
        for (id, decoded) in &scene.decoded_terminals {
            let frame = scene
                .frames
                .iter()
                .find(|frame| frame.target.id == *id)
                .unwrap();
            let aircraft = profiles.aircraft_for(&frame.target.aircraft).unwrap();
            let terminal = &scene.terminal_routes[decoded.candidate];
            assert_eq!(
                decoded.departure[0].airspeed.unwrap().atmosphere,
                scene.environments["LFMN"].atmosphere
            );
            assert_eq!(
                decoded.approach[0].airspeed.unwrap().atmosphere,
                scene.environments["LIMC"].atmosphere
            );
            assert!(f64::from(terminal.enroute_minimum_feet) <= aircraft.ceiling);
            assert_eq!(
                decoded.departure.last().unwrap().coordinate,
                terminal.enroute_from
            );
            assert!(decoded.nominal.as_ref().unwrap().leg_count() > 0);
            assert!(
                decoded
                    .nominal
                    .as_ref()
                    .unwrap()
                    .clone()
                    .protect(|_, _| false)
                    .is_err()
            );
            assert!(!decoded.enroute.is_empty());
            assert_eq!(
                decoded.enroute.last().unwrap().coordinate,
                terminal.enroute_to
            );
            assert_eq!(
                decoded.enroute.last().unwrap().airspeed,
                decoded.arrival.first().unwrap().airspeed
            );
            assert_eq!(
                decoded.arrival.last().unwrap().coordinate,
                decoded.approach.first().unwrap().coordinate
            );
            assert!(
                decoded
                    .departure
                    .iter()
                    .chain(&decoded.enroute)
                    .chain(&decoded.arrival)
                    .chain(&decoded.approach)
                    .all(|fix| fix.airspeed.is_some() && fix.minimum_feet <= fix.maximum_feet)
            );
        }
        let threshold_data =
            openatc_core::arrival::ArrivalAirport::load(&root, scene.airport.clone()).unwrap();
        let runway_surface =
            runway::Surface::new(&scene.airport, "04R", &threshold_data.thresholds, 2.0).unwrap();
        for (id, surface) in &scene.nominal_departures {
            assert!(scene.movement.contains_key(id));
            assert_eq!(
                surface.segments[0].stage,
                openatc_core::traffic::motion::Stage::LineUp
            );
            assert_eq!(
                surface.segments[1].stage,
                openatc_core::traffic::motion::Stage::Takeoff
            );
            assert!(surface.rotation.altitude_feet < 100.0);
            let rotation_point = openatc_core::airport::airport_point(
                &scene.airport,
                surface.rotation.latitude,
                surface.rotation.longitude,
            );
            assert!(
                (surface.rotation.altitude_feet - runway_surface.height(rotation_point)).abs()
                    < 1e-8
            );
            assert_eq!(
                surface.segments[0].points.last(),
                surface.segments[1].points.first()
            );
            assert_eq!(
                surface.segments[0].points[0].altitude_feet,
                scene.airport.elevation_feet
            );
            let knots = scene.environments["LFMN"]
                .runway_speed(
                    &scene.airport,
                    "04R",
                    scene.movement[id].rotation_knots,
                    surface.rotation.altitude_feet,
                )
                .unwrap();
            assert!((surface.segments[1].knots - knots).abs() < 0.001);
        }
        // Exercise a visibly sloping nominal runway without changing the licensed scenery.
        let (id, original) = scene.nominal_departures.first_key_value().unwrap();
        let frame = scene
            .frames
            .iter()
            .find(|frame| frame.target.id == *id)
            .unwrap();
        let aircraft = profiles.aircraft_for(&frame.target.aircraft).unwrap();
        let runway = scene
            .airport
            .runways
            .iter()
            .find(|r| r.first_name == "04R" || r.second_name == "04R")
            .unwrap();
        let heights = vec![
            openatc_core::arrival::Threshold {
                runway: runway.first_name.clone(),
                elevation_feet: 0.0,
            },
            openatc_core::arrival::Threshold {
                runway: runway.second_name.clone(),
                elevation_feet: 50.0,
            },
        ];
        let slope = runway::Surface::new(&scene.airport, "04R", &heights, 2.0).unwrap();
        let policy = parse_operations(include_str!("../../../traffic/operations.toml")).unwrap();
        let planned = departure::plan(
            &scene.airport,
            "04R",
            original.segments[0].points[0],
            aircraft,
            &scene.movement[id],
            &scene.environments["LFMN"],
            &slope,
            &policy,
        )
        .unwrap();
        assert_eq!(
            planned.segments[0].points[0],
            original.segments[0].points[0]
        );
        assert_eq!(
            planned.segments[0].points.last(),
            planned.segments[1].points.first()
        );
        let roll = &planned.segments[1].points;
        assert!((roll[0].altitude_feet - roll[1].altitude_feet).abs() > 1.0);
        for point in roll {
            let local = openatc_core::airport::airport_point(
                &scene.airport,
                point.latitude,
                point.longitude,
            );
            assert!((point.altitude_feet - slope.height(local)).abs() < 1e-8);
        }
        let (&arrival_id, _) = scene.nominal_arrivals.first_key_value().unwrap();
        let selected = &scene.terminal_routes[scene.decoded_terminals[&arrival_id].candidate];
        let mut unusable = selected.clone();
        unusable.approach.last_mut().unwrap().vertical_angle_degrees = None;
        let candidates = vec![unusable, selected.clone()];
        let arrival_data =
            openatc_core::arrival::ArrivalAirport::load(&root, scene.destination.clone()).unwrap();
        let arrival_surface =
            runway::Surface::new(&scene.destination, "35R", &arrival_data.thresholds, 2.0).unwrap();
        let frame = scene
            .frames
            .iter()
            .find(|frame| frame.target.id == arrival_id)
            .unwrap();
        let unreserved = BTreeSet::new();
        let mora = openatc_core::navigation::Mora::load(&root);
        let planner = airborne::Planner {
            rotation: scene.nominal_departures[&arrival_id].rotation,
            origin: &scene.airport,
            destination: &scene.destination,
            points: scene.terminal_points.as_ref().unwrap(),
            mora: &mora,
            departure_weather: &scene.environments["LFMN"],
            arrival_weather: &scene.environments["LIMC"],
            aircraft: profiles.aircraft_for(&frame.target.aircraft).unwrap(),
            handling: &scene.movement[&arrival_id],
            policy: &policy,
            departure_surface: &runway_surface,
            arrival_surface: &arrival_surface,
            observed: &[],
            reserved: &unreserved,
        };
        let (retried, arrival) = planner.arrival(&candidates).unwrap();
        assert_eq!(
            retried.candidate, 1,
            "A decodable but unusable first landing must not exhaust alternatives"
        );
        assert_eq!(
            arrival.flare.points.last(),
            arrival.parking.exit.rollout[0].points.first()
        );
        let mut limited = policy.clone();
        limited.terminal_candidate_limit = 1;
        let bounded = airborne::Planner {
            policy: &limited,
            ..planner
        };
        assert!(
            bounded
                .arrival(&candidates)
                .err()
                .unwrap()
                .contains("among 1 terminal candidates")
        );
        let occupied: BTreeSet<_> = scene
            .destination
            .parking
            .iter()
            .map(|stand| stand.name.clone())
            .collect();
        let blocked = airborne::Planner {
            reserved: &occupied,
            ..planner
        };
        assert!(
            blocked.arrival(&candidates).is_err(),
            "Retry cannot bypass occupied parking"
        );
        assert!(
            scene
                .frames
                .iter()
                .any(|frame| frame.target.aircraft == "AT72"),
            "No turboprop selected"
        );
        assert!(
            scene
                .frames
                .iter()
                .any(|frame| matches!(frame.target.aircraft.as_str(), "A319" | "A320" | "A321")),
            "No airliner selected"
        );
        let tower_frequency = scene
            .airport
            .frequencies
            .iter()
            .find(|frequency| frequency.service == "Tower")
            .unwrap()
            .khz;
        let mut state = crate::test_state();
        state.settings.generated_traffic = true;
        state.settings.hear_traffic_radio = true;
        state.settings.monitor_runway_permissions = false;
        state.settings.monitor_transponder = false;
        state.generated.key = scene_key(&state);
        state.generated.epoch = Some(state.session_generation);
        state.generated.frames = scene.frames;
        state.generated.world = scene.world;
        state.generated.airport = Some(Arc::new(scene.airport));
        state.generated.movement = scene.movement;
        state.generated.fixed_gear = scene.fixed_gear;
        state.generated.orders = scene.orders;
        state.generated.policy = Some(scene.policy);
        state.session.telemetry = openatc_core::state::Telemetry {
            position_valid: true,
            latitude: 43.655764,
            longitude: 7.202480,
            on_ground: true,
            traffic: Some(vec![]),
            com1_khz: 121705,
            radio_power: true,
            ..Default::default()
        };
        let mut crossing_readbacks = BTreeSet::new();
        let mut crossings_seen = BTreeSet::new();
        for tick in 0..2400 {
            state.session.telemetry.simulation_seconds = Some(f64::from(tick) * 0.5);
            if let Some(exchange) = &mut state.generated.exchange {
                exchange.due = Instant::now();
            }
            radio_tick(&mut state);
            if state
                .generated
                .frames
                .iter()
                .any(|frame| frame.target.com1_khz == Some(tower_frequency))
            {
                state.session.telemetry.com1_khz = tower_frequency;
            }
            for (id, segment) in &state.generated.approved_orders {
                if state.generated.orders.get(id).is_some_and(|orders| {
                    orders
                        .iter()
                        .any(|(index, order)| index == segment && !order.crossing_runway.is_empty())
                }) {
                    crossing_readbacks.insert((*id, *segment));
                }
            }
            let telemetry = state.session.telemetry.clone();
            advance(&mut state.generated, &telemetry);
            for pose in state.generated.world.snapshot() {
                if pose.stage == openatc_core::traffic::motion::Stage::Crossing && pose.knots > 0.0
                {
                    assert!(
                        crossing_readbacks.contains(&(pose.id, pose.segment)),
                        "Crossing moved before readback"
                    );
                    crossings_seen.insert(pose.id);
                }
            }
        }
        assert!(
            !crossings_seen.is_empty(),
            "No generated aircraft reached a crossing: poses={:?}, approved={:?}, warnings={:?}",
            state.generated.world.snapshot(),
            state.generated.approved_orders,
            scene.warnings
        );
        assert!(
            !state.generated.reported_crossings.is_empty(),
            "No runway-vacated report"
        );
        assert!(
            !state.generated.tower_notified.is_empty(),
            "No departure Tower handoff"
        );
        assert!(
            !state.generated.tower_ready.is_empty(),
            "No generated ready call"
        );
        assert!(
            state
                .session
                .transcript
                .iter()
                .any(|entry| entry.speaker == "ATC"
                    && entry.position.contains("Tower")
                    && entry.text.to_lowercase().contains("hold short")),
            "No Tower holding instruction"
        );
        assert!(
            state
                .generated
                .world
                .snapshot()
                .iter()
                .any(|pose| pose.complete),
            "No aircraft reached its departure hold"
        );
        assert!(
            state
                .session
                .transcript
                .iter()
                .filter(|entry| entry.speaker == "ATC")
                .all(|entry| entry.background)
        );
        assert!(
            state.generated.orders.values().any(|orders| orders
                .iter()
                .any(|(_, order)| !order.crossing_runway.is_empty())),
            "No crossing route was planned: {:?}",
            scene.warnings
        );
    }
    #[test]
    fn scene_uses_compatible_models_and_keeps_the_user_stand_clear() {
        let mut origin = openatc_core::airport::demo_airport();
        let corner = |east, north| Point {
            east,
            north,
            height: 0.0,
        };
        origin.pavement_triangles = vec![
            [
                corner(-1300.0, 30.0),
                corner(1300.0, 30.0),
                corner(1300.0, 700.0),
            ],
            [
                corner(-1300.0, 30.0),
                corner(1300.0, 700.0),
                corner(-1300.0, 700.0),
            ],
        ];
        for stand in &mut origin.parking {
            stand.equipment = "props".into();
        }
        let mut destination = origin.clone();
        destination.icao = "TEST".into();
        destination.reference_latitude += 1.0;
        let user_stand = position(&origin, origin.parking[0].point);
        let profiles = Arc::new(Profiles::builtin().unwrap());
        let models = Arc::new(Catalogue {
            models: vec![openatc_core::traffic::csl::Model {
                id: "TEST/C172".into(),
                objects: vec!["/models/C172.obj".into()],
                vertical_offset: Some(0.0),
                matches: vec![openatc_core::traffic::csl::Match {
                    aircraft: "C172".into(),
                    operator: String::new(),
                    livery: String::new(),
                }],
            }],
            warnings: vec![],
        });
        let scene = prepare_scene(Preparation {
            simulator_root: String::new(),
            origin,
            destination,
            profiles,
            models,
            counts: [0, 0, 0, 5],
            own: user_stand,
            observed: vec![],
            epoch: 1,
            runway: "09".into(),
            arrival_runway: "09".into(),
            config: std::path::PathBuf::new(),
            weather: BTreeMap::new(),
        })
        .unwrap();
        assert_eq!(scene.frames.len(), scene.movement.len());
        assert_eq!(scene.frames.len(), scene.world.snapshot().len());
        let frames = scene.frames;
        assert!(!frames.is_empty());
        for frame in frames {
            assert_eq!(frame.target.aircraft, "C172");
            assert_eq!(frame.model.id, "TEST/C172");
            assert!(
                openatc_core::arrival::distance_nm(
                    user_stand,
                    [frame.target.latitude, frame.target.longitude]
                ) > 60.0 / 1852.0
            );
            assert_eq!(frame.target.ground_speed_knots, Some(0.0));
        }
    }
    #[test]
    #[ignore = "requires installed scenery and Bluebell"]
    fn installed_cannes_piston_scene_uses_propeller_stands() {
        let root = std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap();
        let origin = airport(&root, "LFMD").unwrap();
        let runway = origin
            .runways
            .iter()
            .max_by(|a, b| {
                (a.second.east - a.first.east)
                    .hypot(a.second.north - a.first.north)
                    .total_cmp(
                        &(b.second.east - b.first.east).hypot(b.second.north - b.first.north),
                    )
            })
            .unwrap()
            .first_name
            .clone();
        let scene = prepare_scene(Preparation {
            simulator_root: root.clone(),
            own: [origin.reference_latitude, origin.reference_longitude],
            origin,
            destination: airport(&root, "LFMN").unwrap(),
            profiles: Arc::new(Profiles::builtin().unwrap()),
            models: Arc::new(
                Catalogue::scan(
                    &std::path::Path::new(&root).join("Resources/plugins/OpenATC/Resources/CSL"),
                )
                .unwrap(),
            ),
            counts: [0, 0, 0, 4],
            observed: vec![],
            epoch: 1,
            runway,
            arrival_runway: "04R".into(),
            config: std::path::PathBuf::new(),
            weather: BTreeMap::new(),
        })
        .unwrap();
        assert!(!scene.frames.is_empty());
        assert!(
            scene
                .frames
                .iter()
                .any(|frame| frame.target.aircraft == "C172")
        );
        assert_eq!(scene.frames.len(), scene.movement.len());
        assert!(scene.frames.iter().all(|frame| {
            let point = openatc_core::airport::airport_point(
                &scene.airport,
                frame.target.latitude,
                frame.target.longitude,
            );
            scene.airport.parking.iter().any(|stand| {
                stand
                    .equipment
                    .split('|')
                    .any(|equipment| equipment == "props")
                    && (stand.point.east - point.east).hypot(stand.point.north - point.north) < 1.0
            })
        }));
    }
    #[test]
    fn reset_never_republishes_an_old_generated_scene() {
        let mut state = crate::test_state();
        state.settings.generated_traffic = true;
        state.generated.epoch = Some(1);
        state.session_generation = 2;
        step(&mut state);
        assert!(state.session.generated_traffic.is_empty());
    }
}
