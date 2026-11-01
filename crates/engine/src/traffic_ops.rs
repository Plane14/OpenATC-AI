//! Persistent ambient surface departures.
//!
//! This layer is deliberately independent of the full-flight scene publisher. It provides the
//! opening airport with taxi-out traffic that already has a departure route and can take off
//! without waiting for a SID/STAR/arrival pair.

use openatc_core::airport::Airport;
use openatc_core::state::{GeneratedTraffic, Point, TrafficDebug, TrafficTarget};
use openatc_core::traffic::csl::{Catalogue, Model};
use openatc_core::traffic::motion::Waypoint;
use openatc_core::traffic::profiles;
use openatc_core::traffic::profiles::Aircraft as Profile;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TAXI_METERS_PER_SECOND: f64 = 5.0;
const TAKEOFF_METERS_PER_SECOND: f64 = 60.0;
const CLIMB_METERS_PER_SECOND: f64 = 80.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AmbientKind {
    Departure,
    Arrival,
}

struct AmbientAircraft {
    id: u32,
    callsign: String,
    runway: String,
    model: Arc<Model>,
    kind: AmbientKind,
    points: Vec<Waypoint>,
    cumulative: Vec<f64>,
    ground_start: f64,
    ground_end: f64,
    taxi_end: f64,
    distance: f64,
    cleared: bool,
    hold_since: Option<Instant>,
    requested: bool,
    speed: f64,
    blocked: bool,
    last: Instant,
    frequency_khz: Option<i32>,
}

/// Owns the ambient airport traffic. Not part of `generated::Runtime`.
#[derive(Default)]
pub struct TrafficManager {
    aircraft: Vec<AmbientAircraft>,
    models: Option<Arc<Catalogue>>,
    last_spawn: Option<Instant>,
    next_id: u32,
    attempts: u32,
    rejected_route: u32,
    rejected_pavement: u32,
    rejected_model: u32,
    last_error: String,
    announced: std::collections::BTreeSet<(u32, &'static str)>,
}

impl TrafficManager {
    /// Whether the manager still needs the CSL catalogue before it can spawn.
    #[must_use]
    pub fn needs_models(&self) -> bool {
        self.models.is_none()
    }

    /// Install the scanned CSL catalogue.
    pub fn set_models(&mut self, catalogue: Arc<Catalogue>) {
        self.models = Some(catalogue);
    }

    /// Number of ambient aircraft currently alive.
    #[must_use]
    pub fn len(&self) -> usize {
        self.aircraft.len()
    }

    /// Whether the ambient layer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.aircraft.is_empty()
    }

    /// Advance ambient traffic and publish frames. Called once per telemetry update.
    pub fn tick(&mut self, state: &mut crate::EngineState) {
        let Some(models) = self.models.clone() else {
            return;
        };
        let Some(airport) = state.airport.clone() else {
            return;
        };
        let target = usize::from(
            state
                .settings
                .traffic_taxi_max
                .max(state.settings.traffic_taxi_min),
        )
        .clamp(2, 6);
        let now = Instant::now();
        let missing = target.saturating_sub(self.aircraft.len());
        if missing > 0
            && self
                .last_spawn
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(4))
        {
            self.spawn(&airport, &models, state, missing);
            self.last_spawn = Some(now);
        }

        let dt = self.aircraft.first().map_or(0.016, |aircraft| {
            now.duration_since(aircraft.last)
                .as_secs_f64()
                .clamp(0.0, 0.2)
        });
        for index in 0..self.aircraft.len() {
            let (kind, on_ground, distance) = {
                let aircraft = &self.aircraft[index];
                let on_ground = match aircraft.kind {
                    AmbientKind::Departure => aircraft.distance < aircraft.ground_end,
                    AmbientKind::Arrival => aircraft.distance >= aircraft.ground_start,
                };
                (aircraft.kind, on_ground, aircraft.distance)
            };
            // Simple physical follow: do not drive into an aircraft ahead on the ground.
            let follow_blocked = on_ground
                && self
                    .aircraft
                    .iter()
                    .enumerate()
                    .any(|(other_index, other)| {
                        other_index != index
                            && other.kind == kind
                            && other.distance > distance
                            && other.distance - distance < 80.0
                    });
            let runway_occupied = self
                .aircraft
                .iter()
                .enumerate()
                .any(|(other_index, other)| other_index != index && other.on_runway());
            let runway_blocked = runway_occupied && self.aircraft[index].approaching_runway(5.0);
            // Give way to traffic on the right at a converging taxiway. The higher id yields
            // when both aircraft see each other, which breaks the mutual-yield case.
            let right_blocked = on_ground
                && self
                    .aircraft
                    .iter()
                    .enumerate()
                    .any(|(other_index, other)| {
                        if other_index == index
                            || !other.on_runway() && other.distance >= other.taxi_end
                        {
                            return false;
                        }
                        let (own_lat, own_lon, own_heading) = position_heading_at(
                            &self.aircraft[index].points,
                            &self.aircraft[index].cumulative,
                            distance,
                        );
                        let (other_lat, other_lon, _) =
                            position_heading_at(&other.points, &other.cumulative, other.distance);
                        let separation = openatc_core::arrival::distance_nm(
                            [own_lat, own_lon],
                            [other_lat, other_lon],
                        ) * 1852.0;
                        if separation > 100.0 {
                            return false;
                        }
                        let bearing = openatc_core::arrival::bearing(
                            [own_lat, own_lon],
                            [other_lat, other_lon],
                        );
                        let relative = (bearing - own_heading).rem_euclid(360.0);
                        let yields = relative > 10.0 && relative < 170.0;
                        yields && self.aircraft[index].id > other.id
                    });
            let hold_for_clearance = self.aircraft[index].kind == AmbientKind::Departure
                && distance >= self.aircraft[index].taxi_end - 15.0
                && !self.aircraft[index].cleared;
            if hold_for_clearance && self.aircraft[index].hold_since.is_none() {
                self.aircraft[index].hold_since = Some(now);
                self.aircraft[index].requested = true;
            }
            if hold_for_clearance
                && self.aircraft[index]
                    .hold_since
                    .is_some_and(|at| at.elapsed() >= Duration::from_secs(8))
            {
                self.aircraft[index].cleared = true;
            }
            let blocked = follow_blocked || runway_blocked || right_blocked || hold_for_clearance;
            let aircraft = &mut self.aircraft[index];
            aircraft.last = now;
            aircraft.blocked = blocked;
            aircraft.speed = if blocked {
                0.0
            } else if aircraft.kind == AmbientKind::Departure
                && aircraft.distance >= aircraft.taxi_end
                && aircraft.distance < aircraft.ground_end
            {
                // Takeoff roll: accelerate to rotation speed down the runway.
                let roll = aircraft.ground_end - aircraft.taxi_end;
                let fraction = if roll > 1.0 {
                    ((aircraft.distance - aircraft.taxi_end) / roll).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                TAXI_METERS_PER_SECOND
                    + fraction * (TAKEOFF_METERS_PER_SECOND - TAXI_METERS_PER_SECOND)
            } else if aircraft.kind == AmbientKind::Departure
                && aircraft.distance < aircraft.taxi_end
            {
                TAXI_METERS_PER_SECOND
            } else if aircraft.kind == AmbientKind::Departure {
                CLIMB_METERS_PER_SECOND
            } else if aircraft.kind == AmbientKind::Arrival
                && aircraft.distance < aircraft.ground_start
            {
                CLIMB_METERS_PER_SECOND
            } else if aircraft.kind == AmbientKind::Arrival
                && aircraft.distance < aircraft.ground_start + 1_000.0
            {
                TAKEOFF_METERS_PER_SECOND
            } else {
                TAXI_METERS_PER_SECOND
            };
            aircraft.distance += aircraft.speed * dt;
        }
        self.aircraft.retain(|aircraft| {
            aircraft.distance < aircraft.cumulative.last().copied().unwrap_or(0.0)
        });

        // Speak only when the player is tuned to the aircraft's frequency.
        let lead_holding = self
            .aircraft
            .iter()
            .filter(|aircraft| {
                aircraft.kind == AmbientKind::Departure
                    && !aircraft.cleared
                    && aircraft.distance >= aircraft.taxi_end - 15.0
            })
            .map(|aircraft| aircraft.id)
            .min();
        let mut calls: Vec<(openatc_core::state::Radio, String, String, String)> = Vec::new();
        for aircraft in &mut self.aircraft {
            let Some(frequency) = aircraft.frequency_khz else {
                continue;
            };
            let Some(radio) = state.session.receiving_radio(frequency) else {
                continue;
            };
            let phase = match aircraft.kind {
                AmbientKind::Departure if aircraft.distance < aircraft.taxi_end - 15.0 => continue,
                AmbientKind::Departure if aircraft.cleared => "takeoff",
                AmbientKind::Departure if Some(aircraft.id) == lead_holding => "contact",
                AmbientKind::Departure => "lineup",
                AmbientKind::Arrival if aircraft.distance >= aircraft.ground_start => "landing",
                AmbientKind::Arrival => continue,
            };
            if !self.announced.insert((aircraft.id, phase)) {
                continue;
            }
            let text = match phase {
                "contact" => format!("{}, contact Tower.", aircraft.callsign),
                "lineup" => format!(
                    "{}, runway {}, line up and wait.",
                    aircraft.callsign, aircraft.runway
                ),
                "takeoff" => format!(
                    "{}, runway {}, cleared for takeoff.",
                    aircraft.callsign, aircraft.runway
                ),
                _ => format!(
                    "{}, runway {}, cleared to land.",
                    aircraft.callsign, aircraft.runway
                ),
            };
            let pilot_reply = match phase {
                "contact" => format!("{}, with you.", aircraft.callsign),
                "lineup" => format!(
                    "Line up and wait, runway {}, {}.",
                    aircraft.runway, aircraft.callsign
                ),
                "takeoff" => format!(
                    "Cleared for takeoff, runway {}, {}.",
                    aircraft.runway, aircraft.callsign
                ),
                _ => format!(
                    "Cleared to land, runway {}, {}.",
                    aircraft.runway, aircraft.callsign
                ),
            };
            calls.push((radio, text, pilot_reply, aircraft.callsign.clone()));
        }
        for (radio, text, pilot_reply, callsign) in calls {
            openatc_core::apply::add_transmission(
                &mut state.session,
                "ATC",
                &text,
                &openatc_core::state::SpeechTag {
                    radio: Some(radio),
                    position: "Tower".into(),
                    voice: "tower".into(),
                    ..Default::default()
                },
            );
            if let Some(last) = state.session.transcript.last_mut() {
                last.background = true;
                last.pilot_reply.clone_from(&pilot_reply);
            }
            openatc_core::apply::add_transmission(
                &mut state.session,
                &callsign,
                &pilot_reply,
                &openatc_core::state::SpeechTag {
                    radio: Some(radio),
                    position: "Traffic".into(),
                    ..Default::default()
                },
            );
            if let Some(last) = state.session.transcript.last_mut() {
                last.background = true;
            }
        }

        let frames: Vec<GeneratedTraffic> =
            self.aircraft.iter().filter_map(ambient_frame).collect();
        let debug: Vec<TrafficDebug> = self
            .aircraft
            .iter()
            .map(|aircraft| TrafficDebug {
                id: aircraft.id,
                callsign: aircraft.callsign.clone(),
                stage: match aircraft.kind {
                    AmbientKind::Departure if aircraft.distance < aircraft.ground_end => {
                        "TaxiOut".to_owned()
                    }
                    AmbientKind::Departure => "Takeoff".to_owned(),
                    AmbientKind::Arrival if aircraft.distance < aircraft.ground_start => {
                        "Approach".to_owned()
                    }
                    AmbientKind::Arrival => "TaxiIn".to_owned(),
                },
                reason: if aircraft.blocked {
                    "holding: traffic or runway".to_owned()
                } else {
                    match aircraft.kind {
                        AmbientKind::Departure => "ambient departure".to_owned(),
                        AmbientKind::Arrival => "ambient arrival".to_owned(),
                    }
                },
                blocker: None,
                awaiting_clearance: false,
                hold: if aircraft.kind == AmbientKind::Departure {
                    let stop = (aircraft.taxi_end - 15.0).max(0.0);
                    Some([
                        position_at(&aircraft.points, &aircraft.cumulative, stop).0,
                        position_at(&aircraft.points, &aircraft.cumulative, stop).1,
                    ])
                } else {
                    None
                },
            })
            .collect();
        // Replace the ambient portion only; keep frames produced by other layers.
        let ambient_ids: std::collections::BTreeSet<u32> =
            self.aircraft.iter().map(|aircraft| aircraft.id).collect();
        state
            .session
            .generated_traffic
            .retain(|frame| !ambient_ids.contains(&frame.target.id));
        state.session.generated_traffic.extend(frames);
        state
            .session
            .generated_traffic_debug
            .retain(|debug| !ambient_ids.contains(&debug.id));
        state.session.generated_traffic_debug.extend(debug);
        if state.session.generated_traffic_debug.len() > 126 {
            state.session.generated_traffic_debug.truncate(126);
        }
        state
            .session
            .generated_traffic
            .sort_by_key(|frame| frame.target.id);
    }

    /// Read-only route diagnostics for the ambient layer.
    #[must_use]
    pub fn debug(&self, airport: &Airport) -> serde_json::Value {
        let aircraft: Vec<serde_json::Value> = self
            .aircraft
            .iter()
            .map(|aircraft| {
                let points = aircraft
                    .points
                    .iter()
                    .map(|point| {
                        let local = openatc_core::airport::airport_point(
                            airport,
                            point.latitude,
                            point.longitude,
                        );
                        openatc_core::airport::on_taxiway(airport, &local, 5.0)
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "id": aircraft.id,
                    "callsign": aircraft.callsign,
                    "kind": match aircraft.kind {
                        AmbientKind::Departure => "departure",
                        AmbientKind::Arrival => "arrival",
                    },
                    "distance": aircraft.distance,
                    "points": aircraft.points.len(),
                    "onTaxiway": points.iter().filter(|on| **on).count(),
                    "first": aircraft.points.first(),
                    "last": aircraft.points.last(),
                })
            })
            .collect();
        serde_json::json!({
            "aircraft": aircraft,
            "models": self.models.is_some(),
            "attempts": self.attempts,
            "rejectedRoute": self.rejected_route,
            "rejectedPavement": self.rejected_pavement,
            "rejectedModel": self.rejected_model,
            "lastError": self.last_error,
        })
    }

    fn spawn(
        &mut self,
        airport: &Airport,
        models: &Catalogue,
        state: &crate::EngineState,
        target: usize,
    ) {
        let runway = if state.session.plan.runway.is_empty() {
            airport
                .runways
                .first()
                .map(|runway| runway.first_name.clone())
                .unwrap_or_default()
        } else {
            state.session.plan.runway.clone()
        };
        if runway.is_empty() {
            return;
        }
        let frequency = state
            .stations
            .iter()
            .find(|station| {
                station.airport == airport.icao
                    && (station.service == "Tower"
                        || station.services.iter().any(|service| service == "Tower"))
            })
            .map(|station| station.khz);
        if models.models.is_empty() {
            return;
        }
        let mut used_nodes = std::collections::BTreeSet::new();
        for slot in 0..target {
            self.attempts = self.attempts.saturating_add(1);
            let profile = profiles::aircraft("A20N")
                .cloned()
                .unwrap_or_else(default_profile);
            let size = profile.parking_size();
            // Start on the taxiway network, not on a stand or apron connector.
            let Some(node) = airport.nodes.iter().find(|node| {
                !used_nodes.contains(&node.id)
                    && !openatc_core::airport::within_runway(airport, &node.point)
                    && openatc_core::airport::on_taxiway(airport, &node.point, 8.0)
            }) else {
                continue;
            };
            let latitude = airport.reference_latitude + node.point.north / 111_320.0;
            let longitude = airport.reference_longitude
                + node.point.east
                    / (111_320.0 * (airport.reference_latitude.to_radians().cos()).max(0.01));
            let Some(surface_points) = plan_taxi(airport, node.id, &runway, size) else {
                self.rejected_route = self.rejected_route.saturating_add(1);
                self.last_error = format!("no taxi graph route from node {}", node.id);
                continue;
            };
            let kind = if target >= 3 && slot == target - 1 {
                AmbientKind::Arrival
            } else {
                AmbientKind::Departure
            };
            let stand_waypoint = Waypoint {
                latitude,
                longitude,
                altitude_feet: airport.elevation_feet,
            };
            let path = match kind {
                AmbientKind::Departure => {
                    departure_path(airport, &surface_points, &runway, stand_waypoint)
                }
                AmbientKind::Arrival => {
                    arrival_path(airport, &surface_points, &runway, stand_waypoint)
                }
            };
            let Some((points, ground_start, ground_end, taxi_end)) = path else {
                self.rejected_route = self.rejected_route.saturating_add(1);
                self.last_error = format!("path build failed for node {}", node.id);
                continue;
            };
            let Some(model) = models
                .choose("A20N", "", "", self.next_id as u64)
                .or_else(|| models.models.first())
                .cloned()
            else {
                continue;
            };
            let callsign = callsign(self.next_id);
            let cumulative = cumulative_distances(&points);
            let total = cumulative.last().copied().unwrap_or(0.0);
            if total < 100.0 {
                continue;
            }
            let distance = match kind {
                AmbientKind::Arrival => 0.0,
                AmbientKind::Departure => {
                    let fraction = if target <= 1 {
                        0.25
                    } else {
                        (slot as f64 + 1.0) / (target as f64 + 1.0)
                    };
                    (total * fraction).min(taxi_end * 0.8)
                }
            };
            let candidate = position_at(&points, &cumulative, distance);
            if self.aircraft.iter().any(|other| {
                openatc_core::arrival::distance_nm(
                    [candidate.0, candidate.1],
                    [
                        position_at(&other.points, &other.cumulative, other.distance).0,
                        position_at(&other.points, &other.cumulative, other.distance).1,
                    ],
                ) * 1852.0
                    < 120.0
            }) {
                self.rejected_route = self.rejected_route.saturating_add(1);
                self.last_error = "spawn point too close to another aircraft".to_owned();
                continue;
            }
            self.aircraft.push(AmbientAircraft {
                id: 0x00e0_0000 + self.next_id,
                callsign,
                runway: runway.clone(),
                model: Arc::new(model),
                kind,
                points,
                cumulative,
                ground_start,
                ground_end,
                taxi_end,
                distance,
                cleared: false,
                hold_since: None,
                requested: false,
                speed: TAXI_METERS_PER_SECOND,
                blocked: false,
                last: Instant::now(),
                frequency_khz: frequency,
            });
            self.next_id = self.next_id.wrapping_add(1);
            used_nodes.insert(node.id);
        }
    }
}

impl AmbientAircraft {
    fn on_runway(&self) -> bool {
        match self.kind {
            AmbientKind::Departure => {
                self.distance >= self.ground_end - 120.0 && self.distance <= self.ground_end + 600.0
            }
            AmbientKind::Arrival => {
                self.distance >= self.ground_start - 500.0
                    && self.distance <= self.ground_start + 600.0
            }
        }
    }

    fn approaching_runway(&self, seconds: f64) -> bool {
        let next = self.distance + self.speed.max(TAXI_METERS_PER_SECOND) * seconds;
        match self.kind {
            AmbientKind::Departure => {
                self.distance < self.ground_end - 120.0 && next >= self.ground_end - 120.0
            }
            AmbientKind::Arrival => {
                self.distance < self.ground_start - 500.0 && next >= self.ground_start - 500.0
            }
        }
    }
}

fn hold_line_distance(airport: &Airport, point: Point) -> f64 {
    let mut best = f64::INFINITY;
    for line in &airport.taxi_hold_lines {
        let a = line[0];
        let b = line[1];
        let dx = b.east - a.east;
        let dn = b.north - a.north;
        let length = dx.hypot(dn);
        if length < 0.5 {
            continue;
        }
        let along = (((point.east - a.east) * dx + (point.north - a.north) * dn) / length)
            .clamp(0.0, length);
        let projected = Point {
            east: a.east + dx / length * along,
            north: a.north + dn / length * along,
            height: point.height,
        };
        best = best.min((point.east - projected.east).hypot(point.north - projected.north));
    }
    best
}

fn plan_taxi(airport: &Airport, start: i64, runway: &str, size: char) -> Option<Vec<Point>> {
    use std::cmp::Reverse;
    use std::collections::{BTreeMap, BinaryHeap};
    let runway_data = airport
        .runways
        .iter()
        .find(|candidate| candidate.first_name == runway || candidate.second_name == runway)?;
    let threshold = if runway_data.first_name == runway {
        runway_data.first
    } else {
        runway_data.second
    };
    let nodes: BTreeMap<i64, Point> = airport
        .nodes
        .iter()
        .map(|node| (node.id, node.point))
        .collect();
    let target = airport
        .nodes
        .iter()
        .filter(|node| {
            if openatc_core::airport::within_runway(airport, &node.point)
                || !openatc_core::airport::on_taxiway(airport, &node.point, 12.0)
            {
                return false;
            }
            let latitude = airport.reference_latitude + node.point.north / 111_320.0;
            let longitude = airport.reference_longitude
                + node.point.east
                    / (111_320.0 * (airport.reference_latitude.to_radians().cos()).max(0.01));
            openatc_core::traffic::runway_position(airport, runway, latitude, longitude)
                .is_none_or(|(_, across, _, _)| across.abs() > 50.0)
        })
        .min_by(|left, right| {
            let key = |node: &openatc_core::airport::TaxiNode| {
                let hold = hold_line_distance(airport, node.point) <= 40.0;
                let distance =
                    (node.point.east - threshold.east).hypot(node.point.north - threshold.north);
                (!hold, distance)
            };
            key(left)
                .partial_cmp(&key(right))
                .unwrap_or(std::cmp::Ordering::Equal)
        })?
        .id;
    let mut adjacency: BTreeMap<i64, Vec<(i64, f64)>> = BTreeMap::new();
    for edge in &airport.edges {
        if edge.runway || edge.size < size {
            continue;
        }
        let (Some(first), Some(second)) = (nodes.get(&edge.first), nodes.get(&edge.second)) else {
            continue;
        };
        let cost = (second.east - first.east).hypot(second.north - first.north);
        adjacency
            .entry(edge.first)
            .or_default()
            .push((edge.second, cost));
        if !edge.one_way {
            adjacency
                .entry(edge.second)
                .or_default()
                .push((edge.first, cost));
        }
    }
    let mut costs = BTreeMap::from([(start, 0.0)]);
    let mut previous = BTreeMap::new();
    let mut heap = BinaryHeap::from([(Reverse(0u64), start)]);
    while let Some((Reverse(bits), node)) = heap.pop() {
        let cost = f64::from_bits(bits);
        if node == target {
            break;
        }
        if cost > costs.get(&node).copied().unwrap_or(f64::INFINITY) {
            continue;
        }
        for (next, step) in adjacency.get(&node).into_iter().flatten() {
            let next_cost = cost + step;
            if next_cost < costs.get(next).copied().unwrap_or(f64::INFINITY) {
                costs.insert(*next, next_cost);
                previous.insert(*next, node);
                heap.push((Reverse(next_cost.to_bits()), *next));
            }
        }
    }
    if !costs.contains_key(&target) {
        return None;
    }
    let mut path = vec![target];
    let mut node = target;
    while let Some(previous_node) = previous.get(&node) {
        path.push(*previous_node);
        node = *previous_node;
    }
    path.reverse();
    if path.len() < 2 || path.first() != Some(&start) {
        return None;
    }
    Some(
        path.into_iter()
            .filter_map(|id| nodes.get(&id).copied())
            .collect(),
    )
}

fn default_profile() -> Profile {
    Profile {
        spacing_speed: None,
        model_aliases: Vec::new(),
        handling: None,
        fixed_gear: false,
        name: "A320".into(),
        wingspan: 36.0,
        length: 38.0,
        wake: 'M',
        cwt: None,
        approach_category: 'C',
        runway: 1800.0,
        range_nm: 3000.0,
        cruise_knots: 450.0,
        ceiling: 39000.0,
        approach_knots: 140.0,
        climb_fpm: 2500.0,
        descent_fpm: 2000.0,
        propeller: false,
        rnav: true,
    }
}

fn callsign(seed: u32) -> String {
    const PREFIXES: [&str; 5] = ["EZY", "AFR", "HOP", "BAW", "DLH"];
    let prefix = PREFIXES[(u64::from(seed) % PREFIXES.len() as u64) as usize];
    format!("{prefix}{}", 100 + seed % 900)
}

fn surface_waypoints(airport: &Airport, points: &[Point]) -> Vec<Waypoint> {
    points
        .iter()
        .map(|point| Waypoint {
            latitude: airport.reference_latitude + point.north / 111_320.0,
            longitude: airport.reference_longitude
                + point.east
                    / (111_320.0 * (airport.reference_latitude.to_radians().cos()).max(0.01)),
            altitude_feet: airport.elevation_feet,
        })
        .collect()
}

fn runway_ends(
    airport: &Airport,
    runway: &str,
) -> Option<(openatc_core::airport::Runway, Waypoint, Waypoint)> {
    let runway_data = airport
        .runways
        .iter()
        .find(|candidate| candidate.first_name == runway || candidate.second_name == runway)?
        .clone();
    let (threshold_point, far_point) = if runway_data.first_name == runway {
        (runway_data.first, runway_data.second)
    } else {
        (runway_data.second, runway_data.first)
    };
    let waypoint = |point: Point, altitude: f64| Waypoint {
        latitude: airport.reference_latitude + point.north / 111_320.0,
        longitude: airport.reference_longitude
            + point.east / (111_320.0 * (airport.reference_latitude.to_radians().cos()).max(0.01)),
        altitude_feet: altitude,
    };
    Some((
        runway_data,
        waypoint(threshold_point, airport.elevation_feet),
        waypoint(far_point, airport.elevation_feet + 2_000.0),
    ))
}

fn entry_waypoint(airport: &Airport, runway: &str, point: Point) -> Option<Waypoint> {
    let runway_data = airport
        .runways
        .iter()
        .find(|candidate| candidate.first_name == runway || candidate.second_name == runway)?;
    let (start, end) = if runway_data.first_name == runway {
        (runway_data.first, runway_data.second)
    } else {
        (runway_data.second, runway_data.first)
    };
    let dx = end.east - start.east;
    let dn = end.north - start.north;
    let length = dx.hypot(dn);
    if length < 1.0 {
        return None;
    }
    let along = (((point.east - start.east) * dx + (point.north - start.north) * dn) / length)
        .clamp(0.0, length);
    let entry = Point {
        east: start.east + dx / length * along,
        north: start.north + dn / length * along,
        height: airport.elevation_feet,
    };
    Some(Waypoint {
        latitude: airport.reference_latitude + entry.north / 111_320.0,
        longitude: airport.reference_longitude
            + entry.east / (111_320.0 * (airport.reference_latitude.to_radians().cos()).max(0.01)),
        altitude_feet: airport.elevation_feet,
    })
}

fn departure_path(
    airport: &Airport,
    points: &[Point],
    runway: &str,
    stand: Waypoint,
) -> Option<(Vec<Waypoint>, f64, f64, f64)> {
    let (_, threshold, far) = runway_ends(airport, runway)?;
    let mut path = surface_waypoints(airport, points);
    if path.is_empty() {
        return None;
    }
    let _ = stand;
    if let Some(entry) = points
        .last()
        .and_then(|point| entry_waypoint(airport, runway, *point))
    {
        path.push(entry);
    }
    let taxi_end = cumulative_distances(&path).last().copied().unwrap_or(0.0);
    let heading = openatc_core::arrival::bearing(
        [threshold.latitude, threshold.longitude],
        [far.latitude, far.longitude],
    );
    let offset = |nm: f64, altitude: f64| {
        let radians = heading.to_radians();
        let latitude = threshold.latitude + radians.cos() * nm * 1852.0 / 111_320.0;
        let longitude = threshold.longitude
            + radians.sin() * nm * 1852.0 / (111_320.0 * (latitude.to_radians().cos()).max(0.01));
        Waypoint {
            latitude,
            longitude,
            altitude_feet: airport.elevation_feet + altitude,
        }
    };
    let rotation = offset(0.5, 0.0);
    let climb = offset(5.0, 1_500.0);
    path.push(threshold);
    path.push(rotation);
    path.push(climb);
    let cumulative = cumulative_distances(&path);
    let ground_end = cumulative
        .get(path.len().saturating_sub(2))
        .copied()
        .unwrap_or(0.0);
    Some((path, 0.0, ground_end, taxi_end))
}

fn arrival_path(
    airport: &Airport,
    points: &[Point],
    runway: &str,
    stand: Waypoint,
) -> Option<(Vec<Waypoint>, f64, f64, f64)> {
    let (_, threshold, far) = runway_ends(airport, runway)?;
    let bearing = openatc_core::arrival::bearing(
        [threshold.latitude, threshold.longitude],
        [far.latitude, far.longitude],
    );
    let offset = |nm: f64, altitude: f64| {
        let radians = bearing.to_radians();
        let latitude = threshold.latitude - radians.cos() * nm * 1852.0 / 111_320.0;
        let longitude = threshold.longitude
            - radians.sin() * nm * 1852.0 / (111_320.0 * (latitude.to_radians().cos()).max(0.01));
        Waypoint {
            latitude,
            longitude,
            altitude_feet: airport.elevation_feet + altitude,
        }
    };
    let mut path = vec![offset(5.0, 1_590.0), offset(3.0, 954.0), offset(1.0, 318.0)];
    path.push(threshold);
    if let Some(entry) = points
        .last()
        .and_then(|point| entry_waypoint(airport, runway, *point))
    {
        path.push(entry);
    }
    let surface = surface_waypoints(airport, points);
    path.extend(surface.into_iter().rev().skip(1));
    path.push(stand);
    let cumulative = cumulative_distances(&path);
    let ground_start = cumulative.get(3).copied().unwrap_or(0.0);
    let ground_end = cumulative.last().copied().unwrap_or(0.0);
    Some((path, ground_start, ground_end, ground_start))
}

fn cumulative_distances(points: &[Waypoint]) -> Vec<f64> {
    let mut result = Vec::with_capacity(points.len());
    let mut total = 0.0;
    result.push(0.0);
    for pair in points.windows(2) {
        total += openatc_core::arrival::distance_nm(
            [pair[0].latitude, pair[0].longitude],
            [pair[1].latitude, pair[1].longitude],
        ) * 1852.0;
        result.push(total);
    }
    result
}

fn position_heading_at(points: &[Waypoint], cumulative: &[f64], distance: f64) -> (f64, f64, f64) {
    let (latitude, longitude) = position_at(points, cumulative, distance);
    let index = cumulative
        .iter()
        .position(|value| *value >= distance)
        .unwrap_or(points.len().saturating_sub(1))
        .max(1);
    let start = points.get(index.saturating_sub(1)).copied();
    let end = points.get(index).copied();
    let heading = start.zip(end).map_or(0.0, |(start, end)| {
        openatc_core::arrival::bearing(
            [start.latitude, start.longitude],
            [end.latitude, end.longitude],
        )
    });
    (latitude, longitude, heading)
}

fn position_at(points: &[Waypoint], cumulative: &[f64], distance: f64) -> (f64, f64) {
    let index = cumulative
        .iter()
        .position(|value| *value >= distance)
        .unwrap_or(points.len().saturating_sub(1))
        .max(1);
    let start = points.get(index - 1).copied().unwrap_or(Waypoint {
        latitude: 0.0,
        longitude: 0.0,
        altitude_feet: 0.0,
    });
    let end = points.get(index).copied().unwrap_or(start);
    let start_distance = cumulative.get(index - 1).copied().unwrap_or(0.0);
    let end_distance = cumulative.get(index).copied().unwrap_or(start_distance);
    let fraction = if end_distance > start_distance {
        ((distance - start_distance) / (end_distance - start_distance)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (
        start.latitude + (end.latitude - start.latitude) * fraction,
        start.longitude + (end.longitude - start.longitude) * fraction,
    )
}

fn ambient_frame(aircraft: &AmbientAircraft) -> Option<GeneratedTraffic> {
    let index = aircraft
        .cumulative
        .iter()
        .position(|distance| *distance >= aircraft.distance)
        .unwrap_or(aircraft.points.len().saturating_sub(1));
    let index = index.max(1);
    let start = aircraft.points.get(index - 1)?;
    let end = aircraft.points.get(index)?;
    let start_distance = aircraft.cumulative[index - 1];
    let end_distance = aircraft.cumulative[index];
    let fraction = if end_distance > start_distance {
        ((aircraft.distance - start_distance) / (end_distance - start_distance)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let latitude = start.latitude + (end.latitude - start.latitude) * fraction;
    let longitude = start.longitude + (end.longitude - start.longitude) * fraction;
    let altitude = start.altitude_feet + (end.altitude_feet - start.altitude_feet) * fraction;
    let track = openatc_core::arrival::bearing(
        [start.latitude, start.longitude],
        [end.latitude, end.longitude],
    );
    let on_ground = match aircraft.kind {
        AmbientKind::Departure => aircraft.distance < aircraft.ground_end,
        AmbientKind::Arrival => aircraft.distance >= aircraft.ground_start,
    };
    Some(GeneratedTraffic {
        vertical_speed_fpm: if on_ground {
            0.0
        } else if aircraft.kind == AmbientKind::Departure {
            1_800.0
        } else {
            -700.0
        },
        body_heading_degrees: Some(track),
        target: TrafficTarget {
            id: aircraft.id,
            callsign: aircraft.callsign.clone(),
            aircraft: "A20N".into(),
            latitude,
            longitude,
            altitude_feet: altitude,
            track_degrees: track,
            on_ground,
            ground_speed_knots: Some(aircraft.speed * 1.943_844),
            wingspan_meters: Some(36.0),
            com1_khz: aircraft.frequency_khz,
            ..Default::default()
        },
        model: aircraft.model.clone(),
        animation: Default::default(),
    })
}
