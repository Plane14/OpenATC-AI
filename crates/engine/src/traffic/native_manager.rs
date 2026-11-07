//! Native X-Plane AI aircraft management under OpenATC ATC control.
//!
//! This module tracks the simulator's native AI aircraft (read back through TCAS
//! telemetry) as participants in the OpenATC ATC system. It derives each aircraft's
//! operational phase from its own telemetry, issues simulated clearances and
//! readback expectations, and integrates with separation and wake-turbulence
//! checks. The simulator remains responsible for actually flying the aircraft;
//! clearances here are ATC state and transcript output, not flight-model commands.

use crate::traffic::separation;
use crate::EngineState;
use openatc_core::airport::{Airport, calculate_taxi_route};
use openatc_core::state::{
    ClearanceKind, NativeAircraft, NativeClearance, NativePhase, ReadbackExpectation,
    ReadbackItem, SpeechTag, Telemetry, TrafficTarget,
};
use openatc_core::traffic::profiles;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Time after which a readback is considered failed.
const READBACK_TIMEOUT: Duration = Duration::from_secs(45);

/// Minimum time between clearance updates for the same aircraft.
const CLEARANCE_COOLDOWN: Duration = Duration::from_secs(5);

/// Fallback runway name when no airport assignment is available.
const FALLBACK_RUNWAY: &str = "09";

/// Tracks a native X-Plane AI aircraft under ATC control.
#[derive(Clone, Debug)]
struct TrackedNativeAircraft {
    /// The native aircraft data.
    aircraft: NativeAircraft,
    /// When we last issued a clearance to this aircraft.
    last_clearance: Option<Instant>,
    /// Phase the current clearance was issued for; a new phase needs a new clearance.
    cleared_phase: Option<NativePhase>,
    /// Whether this aircraft departed from or arrived at the local airport.
    is_departure: bool,
    /// Whether the previous telemetry sample had the aircraft on the ground.
    was_on_ground: bool,
    /// Assigned taxi route (taxiway names).
    taxi_route: Option<Vec<String>>,
    /// Taxi clearance for detailed route.
    taxi_clearance: Option<openatc_core::state::TaxiClearance>,
    /// Departure runway for this aircraft.
    departure_runway: Option<String>,
    /// Arrival runway for this aircraft.
    arrival_runway: Option<String>,
}

impl TrackedNativeAircraft {
    fn new(target: &TrafficTarget) -> Self {
        let aircraft = NativeAircraft {
            id: target.id,
            callsign: target.callsign.clone(),
            aircraft: target.aircraft.clone(),
            latitude: target.latitude,
            longitude: target.longitude,
            altitude_feet: target.altitude_feet,
            track_degrees: target.track_degrees,
            ground_speed_knots: target.ground_speed_knots.unwrap_or(0.0),
            on_ground: target.on_ground,
            com1_khz: target.com1_khz,
            com2_khz: target.com2_khz,
            wake_category: target.wake_category,
            wingspan_meters: target.wingspan_meters,
            ..Default::default()
        };
        Self {
            aircraft,
            last_clearance: None,
            cleared_phase: None,
            // Assume aircraft first seen on the ground are departing and aircraft
            // first seen airborne are arriving; liftoff/touchdown transitions
            // correct the classification.
            is_departure: target.on_ground,
            was_on_ground: target.on_ground,
            taxi_route: None,
            taxi_clearance: None,
            departure_runway: None,
            arrival_runway: None,
        }
    }

    /// Refresh telemetry fields and advance the phase state machine.
    ///
    /// Phases are sticky: once an aircraft enters an operational phase (hold short,
    /// lineup, rollout, ...) raw telemetry cannot regress it. Only genuine
    /// transitions — liftoff, touchdown, stopping — move the aircraft forward.
    fn update_from_target(&mut self, target: &TrafficTarget) {
        let lifted_off = self.was_on_ground && !target.on_ground;
        let touched_down = !self.was_on_ground && target.on_ground;
        self.was_on_ground = target.on_ground;

        self.aircraft.latitude = target.latitude;
        self.aircraft.longitude = target.longitude;
        self.aircraft.altitude_feet = target.altitude_feet;
        self.aircraft.track_degrees = target.track_degrees;
        self.aircraft.ground_speed_knots = target.ground_speed_knots.unwrap_or(0.0);
        self.aircraft.on_ground = target.on_ground;
        self.aircraft.com1_khz = target.com1_khz;
        self.aircraft.com2_khz = target.com2_khz;
        self.aircraft.wake_category = target.wake_category;
        self.aircraft.wingspan_meters = target.wingspan_meters;

        if lifted_off {
            self.is_departure = true;
            self.transition(NativePhase::Takeoff);
        } else if touched_down {
            self.is_departure = false;
            self.transition(NativePhase::Rollout);
        } else {
            self.advance_phase();
        }
    }

    fn transition(&mut self, phase: NativePhase) {
        if self.aircraft.phase != phase {
            self.aircraft.phase = phase;
            // A phase change invalidates the previous clearance and readback.
            self.aircraft.clearance = None;
            self.aircraft.awaiting_readback = None;
            self.cleared_phase = None;
        }
    }

    /// Move the phase machine forward from raw telemetry when no explicit
    /// transition fired this tick.
    fn advance_phase(&mut self) {
        let a = &self.aircraft;
        let next = if a.on_ground {
            if self.is_departure {
                match a.phase {
                    // Hold-short and lineup waits persist until the aircraft rolls.
                    NativePhase::HoldShort | NativePhase::LineUp
                        if a.ground_speed_knots < 30.0 =>
                    {
                        a.phase
                    }
                    _ if a.ground_speed_knots < 1.0 => {
                        if a.phase == NativePhase::AtGateArrival {
                            NativePhase::AtGateArrival
                        } else {
                            NativePhase::AtGate
                        }
                    }
                    _ if a.ground_speed_knots < 30.0 => NativePhase::TaxiOut,
                    _ => NativePhase::Takeoff,
                }
            } else if a.ground_speed_knots < 1.0 {
                NativePhase::AtGateArrival
            } else {
                NativePhase::TaxiIn
            }
        } else {
            let phase = a.phase;
            match phase {
                // Airborne phases only move forward; never regress to climb.
                NativePhase::Takeoff | NativePhase::InitialClimb => {
                    if a.altitude_feet < 6_000.0 {
                        NativePhase::InitialClimb
                    } else {
                        NativePhase::Enroute
                    }
                }
                NativePhase::Enroute => {
                    if a.altitude_feet < 3_000.0 {
                        NativePhase::Approach
                    } else {
                        NativePhase::Enroute
                    }
                }
                NativePhase::Descent => {
                    if a.altitude_feet < 1_500.0 {
                        NativePhase::Final
                    } else {
                        NativePhase::Descent
                    }
                }
                NativePhase::Approach => {
                    if a.altitude_feet < 1_500.0 {
                        NativePhase::Final
                    } else {
                        NativePhase::Approach
                    }
                }
                NativePhase::Final | NativePhase::Landing => NativePhase::Final,
                // Unknown or surface phases while airborne: classify directly.
                _ => Self::classify_airborne(a),
            }
        };
        self.transition(next);
    }

    /// Classify an airborne aircraft with no prior operational phase.
    fn classify_airborne(a: &NativeAircraft) -> NativePhase {
        if a.altitude_feet < 1_200.0 {
            NativePhase::Final
        } else if a.altitude_feet < 3_000.0 {
            NativePhase::Approach
        } else if a.altitude_feet < 10_000.0 {
            NativePhase::InitialClimb
        } else {
            NativePhase::Enroute
        }
    }
}

/// Manages native X-Plane AI aircraft as ATC participants.
#[derive(Default)]
pub struct NativeAircraftManager {
    /// Tracked native aircraft, keyed by Mode S ID.
    aircraft: HashMap<u32, TrackedNativeAircraft>,
    /// Next sequence number for clearances.
    next_sequence: u32,
    /// Whether native AI participation is enabled.
    enabled: bool,
    /// Airport data for taxi routing and runway determination.
    airport_cache: Option<Arc<Airport>>,
    /// ICAO of the cached airport.
    airport_icao: Option<String>,
}

impl NativeAircraftManager {
    /// Create a new NativeAircraftManager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the airport data for taxi routing and runway determination.
    pub fn set_airport(&mut self, airport: Arc<Airport>) {
        self.airport_icao = Some(airport.icao.clone());
        self.airport_cache = Some(airport);
    }

    /// Clear the airport cache.
    pub fn clear_airport(&mut self) {
        self.airport_cache = None;
        self.airport_icao = None;
    }

    /// ICAO of the currently cached airport, for change detection by callers.
    #[allow(dead_code)]
    pub fn airport_icao(&self) -> Option<&str> {
        self.airport_icao.as_deref()
    }

    /// Process a telemetry update and manage native aircraft.
    pub fn tick(&mut self, state: &mut EngineState) {
        self.enabled = state.settings.manage_native_traffic;
        if !self.enabled || !state.settings.native_ai_participation {
            self.aircraft.clear();
            state.session.native_aircraft.clear();
            return;
        }

        // Clone once: the manager mutates state while iterating targets.
        let telemetry = state.session.telemetry.clone();
        let native_targets = telemetry.traffic.as_deref().unwrap_or(&[]).to_vec();

        // Update or create tracked aircraft.
        let mut seen_ids = std::collections::BTreeSet::new();
        for target in &native_targets {
            if target.id == 0 {
                continue; // Skip ownship / unidentified slots.
            }
            seen_ids.insert(target.id);

            let is_new = !self.aircraft.contains_key(&target.id);
            if is_new {
                self.aircraft
                    .insert(target.id, TrackedNativeAircraft::new(target));
                self.assign_runways(state, target.id);
            }
            if let Some(tracked) = self.aircraft.get_mut(&target.id) {
                tracked.update_from_target(target);
            }
            self.manage_atc_interaction(state, target.id);
        }

        // Remove aircraft no longer reported by the simulator.
        self.aircraft.retain(|id, _| seen_ids.contains(id));

        // Publish tracked aircraft for UI and ATC consumers.
        state.session.native_aircraft = self
            .aircraft
            .values()
            .map(|tracked| tracked.aircraft.clone())
            .collect();
    }

    /// Choose departure/arrival runways for a newly tracked aircraft using real
    /// airport assignment and simulated weather, not ownship heading/wingspan.
    fn assign_runways(&mut self, state: &EngineState, id: u32) {
        let Some(airport) = self.airport_cache.clone() else {
            return;
        };
        let Some(tracked) = self.aircraft.get(&id) else {
            return;
        };
        let is_departure = tracked.is_departure;
        let minimum_length = profiles::aircraft(&tracked.aircraft.aircraft)
            .map_or(1_200.0, |profile| profile.runway);

        let (wind_from, wind_knots) = state
            .sim_weather
            .get(&airport.icao)
            .map_or((0.0, 0.0), |(sample, _)| {
                (
                    sample["windDegrees"].as_f64().unwrap_or(0.0),
                    sample["windKnots"].as_f64().unwrap_or(0.0),
                )
            });

        let assignment = state.airports.assignment(
            &airport.icao,
            &state.session.plan.runway,
            &state.session.plan.arrival_runway,
            wind_from,
            wind_knots,
            minimum_length,
        );
        let Some(assignment) = assignment else {
            return;
        };
        if let Some(tracked) = self.aircraft.get_mut(&id) {
            if is_departure {
                tracked.departure_runway = Some(assignment.departure);
            } else {
                tracked.arrival_runway = Some(assignment.arrival);
            }
        }
    }

    /// Manage ATC interaction for a native aircraft based on its phase.
    fn manage_atc_interaction(&mut self, state: &mut EngineState, id: u32) {
        // Expire overdue readbacks and re-arm the phase for a fresh clearance.
        if let Some(tracked) = self.aircraft.get_mut(&id) {
            let expired = tracked
                .last_clearance
                .is_some_and(|issued| issued.elapsed() > READBACK_TIMEOUT)
                && tracked.aircraft.awaiting_readback.is_some();
            if expired {
                tracked.aircraft.awaiting_readback = None;
                tracked.aircraft.clearance = None;
                tracked.cleared_phase = None;
            }
        }

        let Some(phase) = self.aircraft.get(&id).map(|t| t.aircraft.phase) else {
            return;
        };
        if phase == NativePhase::Unknown {
            return;
        }

        // Issue a clearance when the aircraft enters a phase it has not been
        // cleared for yet, respecting the per-aircraft cooldown.
        let needs_clearance = self.aircraft.get(&id).is_some_and(|t| {
            t.cleared_phase != Some(phase)
                && t.last_clearance
                    .map_or(true, |last| last.elapsed() >= CLEARANCE_COOLDOWN)
        });
        if !needs_clearance {
            return;
        }

        match phase {
            NativePhase::AtGate => self.issue_taxi_clearance(state, id),
            NativePhase::TaxiOut => {
                self.check_hold_short(id);
                self.issue_taxi_clearance(state, id);
            }
            NativePhase::HoldShort | NativePhase::LineUp => {
                self.issue_lineup_clearance(state, id)
            }
            NativePhase::Takeoff => self.issue_takeoff_clearance(state, id),
            NativePhase::InitialClimb => self.issue_climb_clearance(state, id),
            NativePhase::Enroute | NativePhase::Descent => {
                self.issue_descent_clearance(state, id)
            }
            NativePhase::Approach => self.issue_approach_clearance(state, id),
            NativePhase::Final => self.issue_landing_clearance(state, id),
            NativePhase::Rollout => self.issue_rollout_clearance(state, id),
            NativePhase::TaxiIn => self.issue_taxi_in_clearance(state, id),
            NativePhase::Landing
            | NativePhase::AtGateArrival
            | NativePhase::Pushback
            | NativePhase::Unknown => {}
        }

        // Record the phase this clearance answers and arm the readback.
        if let Some(tracked) = self.aircraft.get_mut(&id) {
            if tracked.aircraft.clearance.is_some() {
                tracked.cleared_phase = Some(phase);
                if tracked.aircraft.awaiting_readback.is_none() {
                    let sequence = tracked
                        .aircraft
                        .clearance
                        .as_ref()
                        .map_or(0, |clearance| clearance.sequence);
                    tracked.aircraft.awaiting_readback = Some(ReadbackExpectation {
                        clearance_sequence: sequence,
                        expected_items: tracked
                            .aircraft
                            .clearance
                            .as_ref()
                            .map_or_else(Vec::new, Self::expected_readback_items),
                        deadline: 0.0,
                    });
                }
            }
        }
    }

    /// Determine expected readback items for a clearance.
    fn expected_readback_items(clearance: &NativeClearance) -> Vec<ReadbackItem> {
        let mut items = Vec::new();
        match clearance.kind {
            ClearanceKind::Taxi => {
                items.push(ReadbackItem::TaxiRoute);
                if clearance.hold_short_of.is_some() {
                    items.push(ReadbackItem::HoldShort);
                }
            }
            ClearanceKind::HoldShort => items.push(ReadbackItem::HoldShort),
            ClearanceKind::LineUp | ClearanceKind::Takeoff | ClearanceKind::Approach
            | ClearanceKind::Land | ClearanceKind::CrossRunway => {
                items.push(ReadbackItem::Runway);
            }
            ClearanceKind::Climb | ClearanceKind::Descend => {
                if clearance.altitude_feet.is_some() {
                    items.push(ReadbackItem::Altitude);
                }
            }
            ClearanceKind::Heading => items.push(ReadbackItem::Heading),
            ClearanceKind::Speed => items.push(ReadbackItem::Speed),
        }
        items
    }

    /// Build a telemetry view of a tracked aircraft for routing and wake checks.
    fn native_telemetry(&self, state: &EngineState, aircraft: &NativeAircraft) -> Telemetry {
        let mut telemetry = state.session.telemetry.clone();
        telemetry.latitude = aircraft.latitude;
        telemetry.longitude = aircraft.longitude;
        telemetry.altitude_feet = aircraft.altitude_feet;
        telemetry.heading_degrees = aircraft.track_degrees;
        telemetry.ground_speed_knots = aircraft.ground_speed_knots;
        telemetry.on_ground = aircraft.on_ground;
        telemetry.position_valid = true;
        telemetry
    }

    /// Aircraft profile for performance lookups, when the type is known.
    fn profile(&self, code: &str) -> Option<&'static profiles::Aircraft> {
        profiles::aircraft(code)
    }

    /// Runway assigned for departure, falling back to the first runway end.
    fn departure_runway(&self, id: u32) -> String {
        if let Some(tracked) = self.aircraft.get(&id) {
            if let Some(runway) = &tracked.departure_runway {
                return runway.clone();
            }
        }
        self.airport_cache
            .as_ref()
            .and_then(|airport| airport.runways.first())
            .map_or_else(|| FALLBACK_RUNWAY.to_string(), |runway| {
                runway.first_name.clone()
            })
    }

    /// Runway assigned for arrival, falling back to the departure runway.
    fn arrival_runway(&self, id: u32) -> String {
        self.aircraft
            .get(&id)
            .and_then(|tracked| {
                tracked
                    .arrival_runway
                    .clone()
                    .or_else(|| tracked.departure_runway.clone())
            })
            .unwrap_or_else(|| FALLBACK_RUNWAY.to_string())
    }

    /// Wake-turbulence departure delay for this aircraft on its assigned runway.
    fn departure_wake_delay(&self, state: &EngineState, id: u32) -> f64 {
        let (Some(airport), Some(tracked)) =
            (self.airport_cache.as_ref(), self.aircraft.get(&id))
        else {
            return 0.0;
        };
        let Some(runway) = tracked.departure_runway.clone() else {
            return 0.0;
        };
        crate::traffic::departure_wake_delay(
            state,
            tracked.aircraft.id,
            airport,
            &runway,
            &tracked.aircraft.aircraft,
            tracked.aircraft.latitude,
            tracked.aircraft.longitude,
        )
    }

    /// Arrival wake conflict for this aircraft on its assigned runway.
    fn arrival_wake_conflict(&self, state: &mut EngineState, id: u32) -> bool {
        let Some(airport) = self.airport_cache.clone() else {
            return false;
        };
        let Some(tracked) = self.aircraft.get(&id) else {
            return false;
        };
        let Some(runway) = tracked.arrival_runway.clone() else {
            return false;
        };
        let telemetry = self.native_telemetry(state, &tracked.aircraft);
        let targets = state.session.telemetry.traffic.clone().unwrap_or_default();
        state.traffic.wake.arrival_conflict(
            &telemetry,
            &targets,
            Some(airport.as_ref()),
            &runway,
            0.0,
        )
    }

    /// Issue taxi clearance toward the runway hold-short or to parking.
    fn issue_taxi_clearance(&mut self, state: &mut EngineState, id: u32) {
        if self.should_throttle_clearance(id) {
            return;
        }
        let sequence = self.next_sequence();
        let Some(airport) = self.airport_cache.clone() else {
            return;
        };
        let Some(tracked) = self.aircraft.get(&id) else {
            return;
        };

        // Hold if another aircraft is ahead on the ground track.
        let traffic_targets = state.traffic_targets();
        let own_length = self
            .profile(&tracked.aircraft.aircraft)
            .map_or(30.0, |profile| profile.length);
        if separation::closest_ahead(
            tracked.aircraft.latitude,
            tracked.aircraft.longitude,
            tracked.aircraft.track_degrees,
            own_length,
            tracked.aircraft.id,
            &traffic_targets,
            |_| Some(100.0),
        )
        .is_some()
        {
            return;
        }

        let is_departure = tracked.is_departure;
        let aircraft_code = tracked.aircraft.aircraft.clone();
        let aircraft = tracked.aircraft.clone();
        let departure_runway = tracked.departure_runway.clone();
        let telemetry = self.native_telemetry(state, &aircraft);
        let aircraft_size = self
            .profile(&aircraft_code)
            .map_or('C', profiles::Aircraft::parking_size);

        let (destination, to_parking) = if is_departure {
            (
                departure_runway
                    .clone()
                    .unwrap_or_else(|| FALLBACK_RUNWAY.to_string()),
                false,
            )
        } else {
            ("Gate 1".to_string(), true)
        };

        let Ok(taxi_clearance) =
            calculate_taxi_route(&airport, &telemetry, &destination, to_parking, aircraft_size)
        else {
            return;
        };

        let taxi_route: Vec<String> = taxi_clearance
            .via
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect();
        let hold_short_runway = taxi_clearance.hold_short_runway.clone();
        let instructions = taxi_clearance.instructions.clone();

        if let Some(tracked) = self.aircraft.get_mut(&id) {
            tracked.taxi_route = Some(taxi_route.clone());
            tracked.taxi_clearance = Some(taxi_clearance);
            tracked.aircraft.clearance = Some(NativeClearance {
                kind: ClearanceKind::Taxi,
                altitude_feet: None,
                heading: None,
                speed_knots: None,
                runway: departure_runway.clone(),
                taxi_route: if taxi_route.is_empty() {
                    None
                } else {
                    Some(taxi_route)
                },
                hold_short_of: if hold_short_runway.is_empty() {
                    None
                } else {
                    Some(hold_short_runway)
                },
                sequence,
            });
            tracked.last_clearance = Some(Instant::now());
        }
        self.add_atc_transmission(state, id, &instructions);
    }

    /// Detect an aircraft that has stopped at its hold-short point.
    fn check_hold_short(&mut self, id: u32) {
        let Some(tracked) = self.aircraft.get(&id) else {
            return;
        };
        let has_hold = tracked
            .taxi_clearance
            .as_ref()
            .is_some_and(|taxi| !taxi.hold_short_runway.is_empty());
        if !has_hold {
            return;
        }
        // Without a precise hold-line position, a stopped aircraft on a route
        // with a hold-short restriction is treated as holding short.
        if tracked.aircraft.ground_speed_knots < 1.0
            && tracked.aircraft.phase == NativePhase::TaxiOut
            && self.airport_cache.is_some()
        {
            if let Some(tracked) = self.aircraft.get_mut(&id) {
                tracked.transition(NativePhase::HoldShort);
            }
        }
    }

    /// Issue line-up-and-wait clearance once the runway is free.
    fn issue_lineup_clearance(&mut self, state: &mut EngineState, id: u32) {
        if self.should_throttle_clearance(id) || self.departure_wake_delay(state, id) > 0.0 {
            return;
        }
        let sequence = self.next_sequence();
        let runway = self.departure_runway(id);
        let Some(tracked) = self.aircraft.get_mut(&id) else {
            return;
        };
        tracked.aircraft.clearance = Some(NativeClearance {
            kind: ClearanceKind::LineUp,
            altitude_feet: None,
            heading: None,
            speed_knots: None,
            runway: Some(runway.clone()),
            taxi_route: None,
            hold_short_of: None,
            sequence,
        });
        tracked.last_clearance = Some(Instant::now());
        self.add_atc_transmission(
            state,
            id,
            &format!("Line up and wait runway {}", runway),
        );
        // Lineup wait leads to takeoff roll; keep phase until liftoff.
        if let Some(tracked) = self.aircraft.get_mut(&id) {
            tracked.transition(NativePhase::LineUp);
        }
    }

    /// Issue takeoff clearance once lined up, respecting wake turbulence.
    fn issue_takeoff_clearance(&mut self, state: &mut EngineState, id: u32) {
        if self.should_throttle_clearance(id) || self.departure_wake_delay(state, id) > 0.0 {
            return;
        }
        let sequence = self.next_sequence();
        let runway = self.departure_runway(id);
        let initial_alt = state.session.plan.initial_altitude_feet.max(3_000);
        let Some(tracked) = self.aircraft.get_mut(&id) else {
            return;
        };
        tracked.aircraft.clearance = Some(NativeClearance {
            kind: ClearanceKind::Takeoff,
            altitude_feet: Some(initial_alt),
            heading: None,
            speed_knots: None,
            runway: Some(runway.clone()),
            taxi_route: None,
            hold_short_of: None,
            sequence,
        });
        tracked.last_clearance = Some(Instant::now());
        self.add_atc_transmission(
            state,
            id,
            &format!("Cleared for takeoff runway {}, climb {}", runway, initial_alt),
        );
    }

    /// Issue climb clearance to the planned cruise altitude.
    fn issue_climb_clearance(&mut self, state: &mut EngineState, id: u32) {
        if self.should_throttle_clearance(id) {
            return;
        }
        let sequence = self.next_sequence();
        let cruise_alt = state
            .session
            .plan
            .cruise_feet
            .max(state.session.plan.initial_altitude_feet);
        let Some(tracked) = self.aircraft.get_mut(&id) else {
            return;
        };
        tracked.aircraft.clearance = Some(NativeClearance {
            kind: ClearanceKind::Climb,
            altitude_feet: Some(cruise_alt),
            heading: None,
            speed_knots: None,
            runway: None,
            taxi_route: None,
            hold_short_of: None,
            sequence,
        });
        tracked.last_clearance = Some(Instant::now());
        self.add_atc_transmission(state, id, &format!("Climb and maintain {}", cruise_alt));
    }

    /// Issue descent clearance toward the destination.
    fn issue_descent_clearance(&mut self, state: &mut EngineState, id: u32) {
        if self.should_throttle_clearance(id) {
            return;
        }
        let sequence = self.next_sequence();
        let approach_alt = 5_000;
        let Some(tracked) = self.aircraft.get_mut(&id) else {
            return;
        };
        tracked.aircraft.clearance = Some(NativeClearance {
            kind: ClearanceKind::Descend,
            altitude_feet: Some(approach_alt),
            heading: None,
            speed_knots: None,
            runway: None,
            taxi_route: None,
            hold_short_of: None,
            sequence,
        });
        tracked.last_clearance = Some(Instant::now());
        self.add_atc_transmission(state, id, &format!("Descend and maintain {}", approach_alt));
    }

    /// Issue approach clearance once wake spacing permits.
    fn issue_approach_clearance(&mut self, state: &mut EngineState, id: u32) {
        if self.should_throttle_clearance(id) || self.arrival_wake_conflict(state, id) {
            return;
        }
        let sequence = self.next_sequence();
        let runway = self.arrival_runway(id);
        let approach_type = if state.session.plan.approach.is_empty() {
            "ILS".to_string()
        } else {
            state.session.plan.approach.clone()
        };
        let Some(tracked) = self.aircraft.get_mut(&id) else {
            return;
        };
        tracked.aircraft.clearance = Some(NativeClearance {
            kind: ClearanceKind::Approach,
            altitude_feet: None,
            heading: None,
            speed_knots: None,
            runway: Some(runway.clone()),
            taxi_route: None,
            hold_short_of: None,
            sequence,
        });
        tracked.last_clearance = Some(Instant::now());
        self.add_atc_transmission(
            state,
            id,
            &format!("Cleared {} runway {} approach", approach_type, runway),
        );
    }

    /// Issue landing clearance once wake spacing permits.
    fn issue_landing_clearance(&mut self, state: &mut EngineState, id: u32) {
        if self.should_throttle_clearance(id) || self.arrival_wake_conflict(state, id) {
            return;
        }
        let sequence = self.next_sequence();
        let runway = self.arrival_runway(id);
        let Some(tracked) = self.aircraft.get_mut(&id) else {
            return;
        };
        tracked.aircraft.clearance = Some(NativeClearance {
            kind: ClearanceKind::Land,
            altitude_feet: None,
            heading: None,
            speed_knots: None,
            runway: Some(runway.clone()),
            taxi_route: None,
            hold_short_of: None,
            sequence,
        });
        tracked.last_clearance = Some(Instant::now());
        self.add_atc_transmission(state, id, &format!("Cleared to land runway {}", runway));
    }

    /// Issue rollout exit clearance after touchdown.
    fn issue_rollout_clearance(&mut self, state: &mut EngineState, id: u32) {
        if self.should_throttle_clearance(id) {
            return;
        }
        let sequence = self.next_sequence();
        let exit_taxi = self
            .aircraft
            .get(&id)
            .and_then(|tracked| tracked.taxi_clearance.as_ref())
            .and_then(|taxi| taxi.via.split(',').next().map(str::trim).map(str::to_string))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "the first available exit".to_string());
        let Some(tracked) = self.aircraft.get_mut(&id) else {
            return;
        };
        tracked.aircraft.clearance = Some(NativeClearance {
            kind: ClearanceKind::Taxi,
            altitude_feet: None,
            heading: None,
            speed_knots: None,
            runway: None,
            taxi_route: Some(vec![exit_taxi.clone()]),
            hold_short_of: None,
            sequence,
        });
        tracked.last_clearance = Some(Instant::now());
        self.add_atc_transmission(
            state,
            id,
            &format!("Exit runway via {}, contact ground {}", exit_taxi, exit_taxi),
        );
    }

    /// Issue taxi-in clearance toward parking after rollout.
    fn issue_taxi_in_clearance(&mut self, state: &mut EngineState, id: u32) {
        if self.should_throttle_clearance(id) {
            return;
        }
        let sequence = self.next_sequence();
        let Some(airport) = self.airport_cache.clone() else {
            return;
        };
        let Some(tracked) = self.aircraft.get(&id) else {
            return;
        };
        let aircraft_code = tracked.aircraft.aircraft.clone();
        let aircraft = tracked.aircraft.clone();
        let telemetry = self.native_telemetry(state, &aircraft);
        let aircraft_size = self
            .profile(&aircraft_code)
            .map_or('C', profiles::Aircraft::parking_size);

        let Ok(taxi_clearance) =
            calculate_taxi_route(&airport, &telemetry, "Gate 1", true, aircraft_size)
        else {
            return;
        };

        let taxi_route: Vec<String> = taxi_clearance
            .via
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect();
        let instructions = taxi_clearance.instructions.clone();

        if let Some(tracked) = self.aircraft.get_mut(&id) {
            tracked.taxi_route = Some(taxi_route.clone());
            tracked.taxi_clearance = Some(taxi_clearance);
            tracked.aircraft.clearance = Some(NativeClearance {
                kind: ClearanceKind::Taxi,
                altitude_feet: None,
                heading: None,
                speed_knots: None,
                runway: None,
                taxi_route: if taxi_route.is_empty() {
                    None
                } else {
                    Some(taxi_route)
                },
                hold_short_of: None,
                sequence,
            });
            tracked.last_clearance = Some(Instant::now());
        }
        self.add_atc_transmission(state, id, &instructions);
    }

    /// Check if we should throttle clearances for this aircraft.
    fn should_throttle_clearance(&self, id: u32) -> bool {
        self.aircraft.get(&id).is_some_and(|tracked| {
            tracked.last_clearance.is_some_and(|last| last.elapsed() < CLEARANCE_COOLDOWN)
        })
    }

    /// Get next sequence number.
    fn next_sequence(&mut self) -> u32 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        sequence.max(1)
    }

    /// Add an ATC transmission to the session transcript.
    fn add_atc_transmission(&self, state: &mut EngineState, id: u32, text: &str) {
        let Some(tracked) = self.aircraft.get(&id) else {
            return;
        };
        let tag = SpeechTag {
            frequency: tracked.aircraft.com1_khz,
            position: tracked.aircraft.callsign.clone(),
            delivery: "standard".to_string(),
            speed: 1.0,
            ..Default::default()
        };
        openatc_core::apply::add_transmission(&mut state.session, "ATC", text, &tag);
    }

    /// Get all tracked native aircraft.
    pub fn get_native_aircraft(&self) -> Vec<NativeAircraft> {
        self.aircraft
            .values()
            .map(|tracked| tracked.aircraft.clone())
            .collect()
    }

    /// Get count of tracked aircraft.
    pub fn len(&self) -> usize {
        self.aircraft.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.aircraft.is_empty()
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: u32, on_ground: bool, speed: f64, altitude: f64) -> TrafficTarget {
        TrafficTarget {
            id,
            callsign: format!("NAT{id}"),
            aircraft: "C172".to_string(),
            latitude: 37.0,
            longitude: -122.0,
            altitude_feet: altitude,
            track_degrees: 90.0,
            ground_speed_knots: Some(speed),
            on_ground,
            ..Default::default()
        }
    }

    #[test]
    fn ground_aircraft_taxies_out_then_lifts_off() {
        let mut tracked = TrackedNativeAircraft::new(&target(1, true, 0.0, 0.0));
        tracked.update_from_target(&target(1, true, 10.0, 0.0));
        assert_eq!(tracked.aircraft.phase, NativePhase::TaxiOut);
        tracked.update_from_target(&target(1, false, 80.0, 500.0));
        assert_eq!(tracked.aircraft.phase, NativePhase::Takeoff);
        assert!(tracked.is_departure);
        tracked.update_from_target(&target(1, false, 120.0, 2_000.0));
        assert_eq!(tracked.aircraft.phase, NativePhase::InitialClimb);
    }

    #[test]
    fn airborne_aircraft_lands_and_taxis_in() {
        let mut tracked = TrackedNativeAircraft::new(&target(2, false, 140.0, 4_000.0));
        tracked.update_from_target(&target(2, false, 130.0, 800.0));
        assert_eq!(tracked.aircraft.phase, NativePhase::Final);
        tracked.update_from_target(&target(2, true, 60.0, 0.0));
        assert_eq!(tracked.aircraft.phase, NativePhase::Rollout);
        assert!(!tracked.is_departure);
        tracked.update_from_target(&target(2, true, 10.0, 0.0));
        assert_eq!(tracked.aircraft.phase, NativePhase::TaxiIn);
        tracked.update_from_target(&target(2, true, 0.0, 0.0));
        assert_eq!(tracked.aircraft.phase, NativePhase::AtGateArrival);
    }

    #[test]
    fn hold_short_and_lineup_phases_do_not_regress() {
        let mut tracked = TrackedNativeAircraft::new(&target(3, true, 10.0, 0.0));
        tracked.update_from_target(&target(3, true, 10.0, 0.0));
        assert_eq!(tracked.aircraft.phase, NativePhase::TaxiOut);
        tracked.transition(NativePhase::HoldShort);
        tracked.update_from_target(&target(3, true, 0.0, 0.0));
        assert_eq!(tracked.aircraft.phase, NativePhase::HoldShort);
        tracked.transition(NativePhase::LineUp);
        tracked.update_from_target(&target(3, true, 0.0, 0.0));
        assert_eq!(tracked.aircraft.phase, NativePhase::LineUp);
    }

    #[test]
    fn phase_change_clears_outstanding_clearance() {
        let mut tracked = TrackedNativeAircraft::new(&target(4, true, 0.0, 0.0));
        tracked.aircraft.clearance = Some(NativeClearance {
            kind: ClearanceKind::Taxi,
            sequence: 1,
            ..Default::default()
        });
        tracked.aircraft.awaiting_readback = Some(ReadbackExpectation::default());
        tracked.transition(NativePhase::TaxiOut);
        assert!(tracked.aircraft.clearance.is_none());
        assert!(tracked.aircraft.awaiting_readback.is_none());
        assert_eq!(tracked.cleared_phase, None);
    }
}
