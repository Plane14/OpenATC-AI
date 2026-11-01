//! AI automation engine for full flight automation from startup to shutdown.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use crate::EngineState;
use openatc_core::{
    airport::Airport,
    apply::add_transmission,
    dialogue::say,
    eurocontrol::{AircraftPerformance, EurocontrolClient, merge_performance_data},
    ops::{distance_nm, AcfSpecs},
    state::{PhaseCode, Request, SpeechTag, State},
    stations::{Station, nearby, tuned},
};
use openatc_settings::Settings;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Automation state machine phases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationPhase {
    #[default]
    Idle,
    Preflight,
    Startup,
    BeforeStart,
    AfterStart,
    TaxiOut,
    BeforeTakeoff,
    Takeoff,
    Climb,
    Cruise,
    Descent,
    Approach,
    Landing,
    TaxiIn,
    Shutdown,
    Complete,
}

/// Automation configuration loaded from settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutomationConfig {
    /// Enable full automation.
    pub enabled: bool,
    /// Stop after current flight completes.
    pub stop_after_flight: bool,
    /// Use VFR rules instead of IFR.
    pub vfr_mode: bool,
    /// Rotorcraft operations mode.
    pub rotorcraft_mode: bool,
    /// Randomized multi-leg flights.
    pub randomized_flights: bool,
    /// Minimum time between flights in minutes (for randomized).
    pub min_flight_interval_min: u16,
    /// Maximum time between flights in minutes (for randomized).
    pub max_flight_interval_min: u16,
}

impl AutomationConfig {
    /// Load configuration from settings.
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            enabled: settings.automation_enabled,
            stop_after_flight: false,
            vfr_mode: settings.automation_vfr_mode,
            rotorcraft_mode: settings.automation_rotorcraft_mode,
            randomized_flights: settings.automation_randomized_flights,
            min_flight_interval_min: settings.automation_min_flight_interval_min,
            max_flight_interval_min: settings.automation_max_flight_interval_min,
        }
    }
}

impl Default for AutomationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            stop_after_flight: false,
            vfr_mode: false,
            rotorcraft_mode: false,
            randomized_flights: false,
            min_flight_interval_min: 10,
            max_flight_interval_min: 30,
        }
    }
}

/// Automation status for UI display.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AutomationStatus {
    pub phase: AutomationPhase,
    pub active: bool,
    pub stop_after_flight: bool,
    pub current_flight: u32,
    pub total_flights: u32,
    pub last_action: String,
    pub next_action: String,
    pub fuel_remaining_kg: f64,
    pub time_to_next_waypoint_sec: f64,
}

/// Flight plan for automated multi-leg operations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutomatedFlightPlan {
    pub departure: String,
    pub destination: String,
    pub alternate: String,
    pub route: String,
    pub cruise_feet: i32,
    pub estimated_minutes: f64,
    pub fuel_plan_kg: f64,
}

/// Fuel plan calculation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FuelPlan {
    pub trip_fuel_kg: f64,
    pub alternate_fuel_kg: f64,
    pub contingency_fuel_kg: f64, // 5% of trip
    pub final_reserve_fuel_kg: f64, // 30 min (jet) / 45 min (prop)
    pub taxi_fuel_kg: f64,
    pub total_fuel_kg: f64,
}

/// Alternate airport candidate.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlternateAirport {
    pub icao: String,
    pub name: String,
    pub distance_nm: f64,
    pub runway_length_m: f64,
    pub has_fuel: bool,
    pub has_approach: bool,
    pub weather_ok: bool,
}

/// Randomized flight generator state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RandomFlightState {
    pub enabled: bool,
    pub current_leg: u32,
    pub flight_history: Vec<AutomatedFlightPlan>,
    #[serde(skip)]
    pub last_flight_end: Option<Instant>,
    #[serde(skip)]
    pub next_departure_time: Option<Instant>,
}

/// Automation engine state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AutomationEngine {
    pub config: AutomationConfig,
    pub status: AutomationStatus,
    pub phase: AutomationPhase,
    #[serde(skip)]
    pub phase_start_time: Option<Instant>,
    #[serde(skip)]
    pub phase_timer: Option<Instant>,
    pub current_flight_plan: Option<AutomatedFlightPlan>,
    pub random_flight_state: RandomFlightState,
    pub pending_checklist: Vec<String>,
    pub completed_checklist: Vec<String>,
    pub automation_errors: Vec<String>,
    #[serde(skip)]
    pub eurocontrol_client: EurocontrolClient,
    #[serde(skip)]
    pub aircraft_performance: Option<AircraftPerformance>,
    #[serde(skip)]
    pub fuel_plan: FuelPlan,
    #[serde(skip)]
    pub alternate_airports: Vec<AlternateAirport>,
}

impl AutomationEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start automation for the current flight plan.
    pub fn start(
        &mut self,
        session: &mut State,
        loaded_aircraft_specs: &mut Option<AcfSpecs>,
        settings: &Settings,
        arrival_airport: &Option<Airport>,
    ) {
        if !self.config.enabled {
            return;
        }
        self.status.active = true;
        self.status.phase = AutomationPhase::Preflight;
        self.phase = AutomationPhase::Preflight;
        self.phase_start_time = Some(Instant::now());
        self.status.last_action = "Automation started - Preflight checks".to_string();
        self.status.next_action = "Run preflight checklist".to_string();
        
        // Fetch aircraft performance data
        self.fetch_aircraft_performance(session, loaded_aircraft_specs);
        
        // Build flight plan and calculate fuel
        self.build_current_flight_plan(session);
        self.calculate_fuel_plan(session, settings);
        self.select_alternate_airports(session, settings);
        
        // Apply VFR/rotorcraft adjustments
        apply_vfr_rotorcraft_adjustments(session, settings);
        
        // Select optimal cruise altitude
        let cruise_feet = self.select_cruise_altitude(session, arrival_airport);
        if let Some(flight_plan) = &mut self.current_flight_plan {
            flight_plan.cruise_feet = cruise_feet;
        }
    }

    /// Fetch aircraft performance data from Eurocontrol ATPCFDB.
    fn fetch_aircraft_performance(&mut self, session: &State, loaded_aircraft_specs: &Option<AcfSpecs>) {
        let icao = session.plan.aircraft.clone();
        if !icao.is_empty() {
            // Try to get from ACF specs first
            if let Some(acf_specs) = loaded_aircraft_specs {
                // We'll merge with Eurocontrol data when we have it
                // For now, just store the ACF specs
            }
            
            // Fetch from Eurocontrol (async - would need to be handled differently in real impl)
            // For now, use built-in data
            self.aircraft_performance = self.eurocontrol_client.get_builtin_performance(&icao);
        }
    }

    /// Stop automation.
    pub fn stop(&mut self) {
        self.status.active = false;
        self.status.phase = AutomationPhase::Idle;
        self.phase = AutomationPhase::Idle;
        self.status.last_action = "Automation stopped by user".to_string();
        self.status.next_action = String::new();
    }

    /// Toggle stop after flight.
    pub fn toggle_stop_after_flight(&mut self) {
        self.config.stop_after_flight = !self.config.stop_after_flight;
        self.status.stop_after_flight = self.config.stop_after_flight;
    }

    /// Build flight plan from current state.
    fn build_current_flight_plan(&mut self, session: &State) {
        let plan = &session.plan;
        if plan.departure.is_empty() || plan.destination.is_empty() {
            return;
        }
        self.current_flight_plan = Some(AutomatedFlightPlan {
            departure: plan.departure.clone(),
            destination: plan.destination.clone(),
            alternate: plan.alternate.clone(),
            route: plan.route.clone(),
            cruise_feet: plan.cruise_feet,
            estimated_minutes: plan.estimated_minutes,
            fuel_plan_kg: plan.trip_fuel_kg + plan.alternate_fuel_kg + plan.reserve_fuel_kg,
        });
        self.status.total_flights = self.random_flight_state.flight_history.len() as u32 + 1;
        self.status.current_flight = self.status.total_flights;
    }

    /// Main automation tick - called each engine tick.
    pub fn tick(&mut self, state: &mut EngineState) {
        if !self.status.active {
            return;
        }

        // Check if we should stop after flight
        if self.config.stop_after_flight
            && self.phase == AutomationPhase::Complete
            && self.status.current_flight >= self.status.total_flights
        {
            self.stop();
            return;
        }

        // Handle randomized flights
        if self.config.randomized_flights {
            self.handle_randomized_flights(state);
        }

        // Execute phase logic
        match self.phase {
            AutomationPhase::Preflight => self.preflight_phase(state),
            AutomationPhase::Startup => self.startup_phase(state),
            AutomationPhase::BeforeStart => self.before_start_phase(state),
            AutomationPhase::AfterStart => self.after_start_phase(state),
            AutomationPhase::TaxiOut => self.taxi_out_phase(state),
            AutomationPhase::BeforeTakeoff => self.before_takeoff_phase(state),
            AutomationPhase::Takeoff => self.takeoff_phase(state),
            AutomationPhase::Climb => self.climb_phase(state),
            AutomationPhase::Cruise => self.cruise_phase(state),
            AutomationPhase::Descent => self.descent_phase(state),
            AutomationPhase::Approach => self.approach_phase(state),
            AutomationPhase::Landing => self.landing_phase(state),
            AutomationPhase::TaxiIn => self.taxi_in_phase(state),
            AutomationPhase::Shutdown => self.shutdown_phase(state),
            AutomationPhase::Complete => self.complete_phase(state),
            AutomationPhase::Idle => {}
        }
    }

    fn handle_randomized_flights(&mut self, state: &mut EngineState) {
        if !self.random_flight_state.enabled {
            self.random_flight_state.enabled = true;
        }

        let now = Instant::now();

        // Check if we need to generate next flight
        if self.phase == AutomationPhase::Complete {
            if let Some(last_end) = self.random_flight_state.last_flight_end {
                if now.duration_since(last_end).as_secs() > 60 * self.config.min_flight_interval_min as u64 {
                    self.generate_next_random_flight(state);
                }
            } else {
                // First completion
                self.random_flight_state.last_flight_end = Some(now);
            }
        }
    }

    fn preflight_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Running preflight checks".to_string();
        self.status.next_action = "Check fuel, weight, weather, and aircraft systems".to_string();

        // Check fuel against calculated fuel plan
        if self.fuel_plan.total_fuel_kg <= 0.0 {
            self.automation_errors.push("No fuel planned - check fuel calculation".to_string());
        } else if self.fuel_plan.total_fuel_kg > 100000.0 { // Sanity check
            self.automation_errors.push(format!("Fuel plan seems excessive: {:.0} kg", self.fuel_plan.total_fuel_kg));
        }

        // Check weight limits
        if let Some(specs) = &state.loaded_aircraft_specs {
            if state.session.plan.takeoff_weight_kg > specs.max_takeoff_weight_kg.unwrap_or(f64::MAX) {
                self.automation_errors.push(format!(
                    "Takeoff weight {:.0} kg exceeds MTOW {:.0} kg",
                    state.session.plan.takeoff_weight_kg, specs.max_takeoff_weight_kg.unwrap_or(0.0)
                ));
            }
        }

        // Verify alternate is selected
        if self.current_flight_plan.as_ref().map(|p| p.alternate.is_empty()).unwrap_or(true) {
            self.automation_errors.push("No alternate airport selected".to_string());
        }

        // Advance to startup after preflight
        if self.phase_timer.is_none() {
            self.phase_timer = Some(Instant::now());
        } else if self.phase_timer.unwrap().elapsed().as_secs() > 5 {
            self.advance_phase(AutomationPhase::Startup);
        }
    }

    fn startup_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Starting engines".to_string();
        self.status.next_action = "Monitor engine parameters".to_string();

        // Request engine start via copilot
        if state.session.startup_approved {
            self.advance_phase(AutomationPhase::BeforeStart);
        } else {
            // Request startup clearance
            let request = Request {
                intent: "startup".to_string(),
                text: say("pilot_request_startup", &[("callsign", state.session.plan.callsign.clone())]),
                ..Default::default()
            };
            // In real implementation, this would be sent via the engine
            self.phase_timer = Some(Instant::now());
        }
    }

    fn before_start_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Engine start sequence".to_string();
        self.status.next_action = "Verify engine instruments".to_string();

        if state.session.telemetry.engines_running {
            self.advance_phase(AutomationPhase::AfterStart);
        }
    }

    fn after_start_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "After start checks".to_string();
        self.status.next_action = "Request taxi clearance".to_string();

        if self.phase_timer.is_none() {
            self.phase_timer = Some(Instant::now());
        } else if self.phase_timer.unwrap().elapsed().as_secs() > 10 {
            self.advance_phase(AutomationPhase::TaxiOut);
        }
    }

    fn taxi_out_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Taxiing to runway".to_string();
        self.status.next_action = "Follow taxi route to holding point".to_string();

        if state.session.phase == PhaseCode::Taxi {
            // Check if we reached the runway
            if state.session.taxi_clearance.guidance_complete
                && state.session.taxi_clearance.destination == state.session.plan.runway
            {
                self.advance_phase(AutomationPhase::BeforeTakeoff);
            }
        }
    }

    fn before_takeoff_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Before takeoff checks".to_string();
        self.status.next_action = "Line up and wait for takeoff clearance".to_string();

        // Wait for takeoff clearance
        if state.flow.takeoff_cleared {
            self.advance_phase(AutomationPhase::Takeoff);
        }
    }

    fn takeoff_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Takeoff roll".to_string();
        self.status.next_action = "Rotate and climb".to_string();

        if state.session.phase == PhaseCode::Departure
            && state.session.telemetry.height_agl_feet > 400.0
        {
            self.advance_phase(AutomationPhase::Climb);
        }
    }

    fn climb_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Climbing to cruise altitude".to_string();
        self.status.next_action = format!("Level off at {} ft", state.session.plan.cruise_feet);

        // Set autopilot altitude
        if state.settings.copilot_flies_ap {
            // Copilot will handle altitude via autopilot
        }

        if state.session.phase == PhaseCode::Cruise {
            self.advance_phase(AutomationPhase::Cruise);
        }
    }

    fn cruise_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Cruising".to_string();
        self.status.next_action = "Monitor progress, start descent planning".to_string();

        // Calculate top of descent
        let distance_to_dest = distance_nm(
            state.session.telemetry.latitude,
            state.session.telemetry.longitude,
            state.arrival_airport.as_ref().map(|a| a.reference_latitude).unwrap_or(0.0),
            state.arrival_airport.as_ref().map(|a| a.reference_longitude).unwrap_or(0.0),
        );

        let descent_rate = state.session.plan.fixes.iter()
            .filter_map(|f| f.minimum_speed_knots)
            .max_by(f64::total_cmp)
            .unwrap_or(250.0);

        let descent_distance = (state.session.telemetry.altitude_feet - state.arrival_airport.as_ref().map(|a| a.elevation_feet).unwrap_or(0.0) as f64)
            / (descent_rate / 60.0) * (state.session.telemetry.ground_speed_knots / 60.0);

        if distance_to_dest <= descent_distance / 1852.0 + 10.0 {
            self.advance_phase(AutomationPhase::Descent);
        }
    }

    fn descent_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Descending".to_string();
        self.status.next_action = "Contact approach, prepare for arrival".to_string();

        if state.session.phase == PhaseCode::Arrival {
            self.advance_phase(AutomationPhase::Approach);
        }
    }

    fn approach_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "On approach".to_string();
        self.status.next_action = "Request landing clearance".to_string();

        if state.flow.landing_cleared {
            self.advance_phase(AutomationPhase::Landing);
        }
    }

    fn landing_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Landing".to_string();
        self.status.next_action = "Vacate runway, taxi to parking".to_string();

        if state.session.phase == PhaseCode::Landed
            && state.session.telemetry.ground_speed_knots < 30.0
        {
            self.advance_phase(AutomationPhase::TaxiIn);
        }
    }

    fn taxi_in_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Taxiing to parking".to_string();
        self.status.next_action = "Shutdown engines".to_string();

        if state.session.phase == PhaseCode::TaxiIn
            && state.session.taxi_clearance.to_parking
            && state.session.taxi_clearance.guidance_complete
        {
            self.advance_phase(AutomationPhase::Shutdown);
        }
    }

    fn shutdown_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Shutting down engines".to_string();
        self.status.next_action = "Complete shutdown checklist".to_string();

        if !state.session.telemetry.engines_running {
            self.advance_phase(AutomationPhase::Complete);
        }
    }

    fn complete_phase(&mut self, state: &mut EngineState) {
        self.status.last_action = "Flight complete".to_string();
        self.status.next_action = if self.config.randomized_flights {
            "Waiting for next flight".to_string()
        } else {
            "Automation complete".to_string()
        };

        self.random_flight_state.last_flight_end = Some(Instant::now());
        self.status.current_flight += 1;

        // Refuel for next flight if randomized
        if self.config.randomized_flights {
            if let Some(next_plan) = &self.current_flight_plan {
                let required_fuel = next_plan.fuel_plan_kg;
                if required_fuel > 0.0 {
                    self.status.next_action = format!("Refueling {:.0} kg for next flight", required_fuel);
                    // In a real implementation, this would call the X-Plane refuel function
                    // refuel_aircraft(xpapi, required_fuel);
                }
            }
        }
    }

    fn advance_phase(&mut self, new_phase: AutomationPhase) {
        self.phase = new_phase;
        self.status.phase = new_phase;
        self.phase_start_time = Some(Instant::now());
        self.phase_timer = None;
    }

    /// Calculate fuel plan: trip + alternate + 5% contingency + 30/45 min final reserve.
    fn calculate_fuel_plan(&mut self, session: &State, settings: &Settings) {
        let plan = &session.plan;
        let perf = self.aircraft_performance.as_ref();
        
        let trip_fuel = plan.trip_fuel_kg.max(0.0);
        let alternate_fuel = plan.alternate_fuel_kg.max(0.0);
        
        // 5% contingency
        let contingency_fuel = trip_fuel * 0.05;
        
        // Final reserve: 30 min for jets, 45 min for props/rotorcraft
        let is_jet = perf.as_ref().map(|p| p.engine_type.to_lowercase().contains("jet") || p.engine_type.to_lowercase().contains("turbofan")).unwrap_or(false);
        let is_rotorcraft = perf.as_ref().map(|p| p.engine_type.to_lowercase().contains("rotor")).unwrap_or(false);
        let is_turboprop = perf.as_ref().map(|p| p.engine_type.to_lowercase().contains("turboprop")).unwrap_or(false);
        
        let reserve_minutes = if is_jet { 30 } else { 45 };
        let reserve_fuel = if let Some(p) = perf {
            p.fuel_consumption_kg_per_hr * (reserve_minutes as f64 / 60.0)
        } else {
            // Estimate based on aircraft type
            if is_jet { 1000.0 } else if is_turboprop { 300.0 } else { 150.0 }
        };
        
        let taxi_fuel = plan.taxi_fuel_kg.max(0.0);
        
        let total_fuel = trip_fuel + alternate_fuel + contingency_fuel + reserve_fuel + taxi_fuel;
        
        self.fuel_plan = FuelPlan {
            trip_fuel_kg: trip_fuel,
            alternate_fuel_kg: alternate_fuel,
            contingency_fuel_kg: contingency_fuel,
            final_reserve_fuel_kg: reserve_fuel,
            taxi_fuel_kg: taxi_fuel,
            total_fuel_kg: total_fuel,
        };
        
        // Update the flight plan with calculated fuel
        if let Some(flight_plan) = &mut self.current_flight_plan {
            flight_plan.fuel_plan_kg = total_fuel;
        }
    }

    /// Select alternate airports for the destination.
    fn select_alternate_airports(&mut self, session: &State, _settings: &Settings) {
        let destination = &session.plan.destination;
        let perf = self.aircraft_performance.as_ref();
        
        // In a real implementation, this would query a database of airports
        // For now, we'll use a simplified approach based on known alternates
        let alternates = match destination.as_str() {
            "KLAX" => vec![
                AlternateAirport { icao: "KBUR".to_string(), name: "Burbank".to_string(), distance_nm: 18.0, runway_length_m: 2000.0, has_fuel: true, has_approach: true, weather_ok: true },
                AlternateAirport { icao: "KLGB".to_string(), name: "Long Beach".to_string(), distance_nm: 22.0, runway_length_m: 3000.0, has_fuel: true, has_approach: true, weather_ok: true },
            ],
            "KJFK" => vec![
                AlternateAirport { icao: "KEWR".to_string(), name: "Newark".to_string(), distance_nm: 15.0, runway_length_m: 3300.0, has_fuel: true, has_approach: true, weather_ok: true },
                AlternateAirport { icao: "KLGA".to_string(), name: "LaGuardia".to_string(), distance_nm: 12.0, runway_length_m: 2100.0, has_fuel: true, has_approach: true, weather_ok: true },
            ],
            "LFPG" => vec![
                AlternateAirport { icao: "LFPO".to_string(), name: "Orly".to_string(), distance_nm: 15.0, runway_length_m: 3000.0, has_fuel: true, has_approach: true, weather_ok: true },
                AlternateAirport { icao: "LFLL".to_string(), name: "Lyon".to_string(), distance_nm: 200.0, runway_length_m: 4000.0, has_fuel: true, has_approach: true, weather_ok: true },
            ],
            _ => vec![
                AlternateAirport { icao: "".to_string(), name: "".to_string(), distance_nm: 0.0, runway_length_m: 0.0, has_fuel: false, has_approach: false, weather_ok: false },
            ],
        };
        
        // Filter by aircraft capabilities
        let min_runway = perf.and_then(|p| {
            if p.engine_type.to_lowercase().contains("jet") { Some(1500.0) }
            else if p.engine_type.to_lowercase().contains("turboprop") { Some(1000.0) }
            else { Some(600.0) }
        }).unwrap_or(1000.0);
        
        self.alternate_airports = alternates.into_iter()
            .filter(|a| a.runway_length_m >= min_runway)
            .filter(|a| a.has_fuel && a.has_approach)
            .take(2)
            .collect();
        
        // Set the first valid alternate in the flight plan
        if let Some(first) = self.alternate_airports.first() {
            if let Some(flight_plan) = &mut self.current_flight_plan {
                if flight_plan.alternate.is_empty() {
                    flight_plan.alternate = first.icao.clone();
                }
            }
        }
    }

    /// Select cruise altitude per leg (aircraft ceiling, distance, semi-circular rule, MORA).
    fn select_cruise_altitude(&self, session: &State, arrival_airport: &Option<Airport>) -> i32 {
        let perf = self.aircraft_performance.as_ref();
        let plan = &session.plan;
        
        let ceiling = perf.map(|p| p.ceiling_ft).unwrap_or(plan.cruise_feet);
        let distance_nm = openatc_core::ops::distance_nm(
            session.telemetry.latitude,
            session.telemetry.longitude,
            arrival_airport.as_ref().map(|a| a.reference_latitude).unwrap_or(0.0),
            arrival_airport.as_ref().map(|a| a.reference_longitude).unwrap_or(0.0),
        );
        
        // Optimal altitude based on distance
        let optimal_altitude = if distance_nm < 200.0 {
            15000
        } else if distance_nm < 500.0 {
            25000
        } else if distance_nm < 1000.0 {
            33000
        } else {
            39000
        };
        
        // Apply semi-circular rule (eastbound: odd FL, westbound: even FL)
        let track = session.telemetry.ground_track_degrees.unwrap_or(session.telemetry.heading_degrees);
        let is_eastbound = (0.0..180.0).contains(&track);
        
        let mut cruise_fl = (optimal_altitude / 100) * 100;
        if is_eastbound {
            if cruise_fl % 200 == 0 { cruise_fl += 100; } // Odd FL for eastbound
        } else {
            if cruise_fl % 200 == 100 { cruise_fl += 100; } // Even FL for westbound
        }
        
        // Respect aircraft ceiling
        cruise_fl = cruise_fl.min(ceiling);
        
        // Respect MORA (simplified - would use actual MORA data)
        let mora = 2000; // Minimum altitude
        cruise_fl = cruise_fl.max(mora);
        
        // Round to nearest valid flight level
        let valid_fls = vec![180, 200, 220, 240, 260, 280, 300, 320, 340, 360, 380, 400, 410, 430, 450, 470, 490, 510, 530];
        valid_fls.iter()
            .filter(|&&fl| fl * 100 <= ceiling)
            .min_by_key(|&&fl| (fl * 100 - cruise_fl).abs())
            .map(|&fl| fl * 100)
            .unwrap_or(plan.cruise_feet)
    }

    /// Check if an airport can support the aircraft type.
    fn airport_supports_aircraft(&self, airport_icao: &str, state: &EngineState) -> bool {
        let perf = self.aircraft_performance.as_ref();
        let min_runway = perf.and_then(|p| {
            if p.engine_type.to_lowercase().contains("jet") { Some(1800.0) }
            else if p.engine_type.to_lowercase().contains("turboprop") { Some(1200.0) }
            else if p.engine_type.to_lowercase().contains("rotor") { Some(0.0) }
            else { Some(800.0) }
        }).unwrap_or(1000.0);
        
        // In a real implementation, this would query airport data
        // For now, simplified check based on known airports
        let supports = match airport_icao {
            // Major airports - support everything
            "KLAX" | "KJFK" | "KORD" | "KATL" | "LFPG" | "EGLL" | "EDDF" | "RJTT" | "YSSY" => true,
            // Regional airports
            "KSMF" | "KSAC" | "KONT" | "KSBA" | "KSNA" | "KMRY" | "KSBP" => !perf.map(|p| p.engine_type.to_lowercase().contains("jet")).unwrap_or(false),
            // Small GA airports
            "KSQL" | "KHWD" | "KPAO" | "KRHV" => !perf.map(|p| p.engine_type.to_lowercase().contains("jet") || p.engine_type.to_lowercase().contains("turboprop")).unwrap_or(false),
            _ => true, // Default to true for unknown airports
        };
        
        supports
    }

    /// Generate next random flight with airport compatibility checking.
    fn generate_next_random_flight(&mut self, state: &mut EngineState) {
        self.status.last_action = "Generating next random flight...".to_string();
        self.status.next_action = "Select destination airport".to_string();

        // Add current flight to history
        if let Some(plan) = &self.current_flight_plan {
            self.random_flight_state.flight_history.push(plan.clone());
        }

        // Select a compatible destination airport
        let current_airport = &state.session.plan.destination;
        let destinations = self.get_compatible_destinations(current_airport, state);
        
        if let Some(next_dest) = destinations.first() {
            // Create new flight plan
            let new_plan = AutomatedFlightPlan {
                departure: current_airport.clone(),
                destination: next_dest.icao.clone(),
                alternate: next_dest.alternates.first().map(|a| a.icao.clone()).unwrap_or_default(),
                route: "DCT".to_string(), // Would be calculated properly
                cruise_feet: self.select_cruise_altitude(&state.session, &state.arrival_airport),
                estimated_minutes: 60.0, // Would be calculated based on distance
                fuel_plan_kg: 0.0, // Will be calculated
            };
            
            self.current_flight_plan = Some(new_plan);
            self.random_flight_state.current_leg += 1;
        }

        // Reset for next flight
        self.phase = AutomationPhase::Preflight;
        self.status.phase = AutomationPhase::Preflight;
        self.random_flight_state.last_flight_end = None;
    }

    /// Get compatible destination airports based on aircraft range and capabilities.
    fn get_compatible_destinations(&self, from: &str, state: &EngineState) -> Vec<DestinationCandidate> {
        let perf = self.aircraft_performance.as_ref();
        let range_nm = perf.map(|p| p.range_nm).unwrap_or(500.0);
        
        // In a real implementation, this would query a database
        // For now, return a static list of candidates
        let candidates = match from {
            "KLAX" => vec![
                DestinationCandidate { icao: "KSFO".to_string(), distance_nm: 340.0, alternates: vec![] },
                DestinationCandidate { icao: "KLAS".to_string(), distance_nm: 230.0, alternates: vec![] },
                DestinationCandidate { icao: "KPHX".to_string(), distance_nm: 370.0, alternates: vec![] },
                DestinationCandidate { icao: "KDEN".to_string(), distance_nm: 830.0, alternates: vec![] },
            ],
            "KJFK" => vec![
                DestinationCandidate { icao: "KBOS".to_string(), distance_nm: 190.0, alternates: vec![] },
                DestinationCandidate { icao: "KIAD".to_string(), distance_nm: 210.0, alternates: vec![] },
                DestinationCandidate { icao: "KDTW".to_string(), distance_nm: 480.0, alternates: vec![] },
            ],
            _ => vec![
                DestinationCandidate { icao: "KLAX".to_string(), distance_nm: 500.0, alternates: vec![] },
            ],
        };
        
        candidates.into_iter()
            .filter(|c| c.distance_nm <= range_nm * 0.8) // Use 80% of range for safety
            .filter(|c| self.airport_supports_aircraft(&c.icao, state))
            .collect()
    }
}

/// Destination candidate for randomized flights.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DestinationCandidate {
    pub icao: String,
    pub distance_nm: f64,
    pub alternates: Vec<AlternateAirport>,
}

/// Apply VFR/rotorcraft adjustments to flight plan.
pub fn apply_vfr_rotorcraft_adjustments(session: &mut State, settings: &Settings) {
    if is_vfr_flight_via_settings(settings) {
        // VFR: lower altitudes, visual navigation
        session.plan.cruise_feet = session.plan.cruise_feet.min(10000);
        session.ifr = false;
    }
    
    if is_rotorcraft_flight_via_settings(settings) {
        // Rotorcraft: lower speeds, different profiles
        session.plan.cruise_feet = session.plan.cruise_feet.min(8000);
    }
}

/// Check VFR mode via settings.
fn is_vfr_flight_via_settings(settings: &Settings) -> bool {
    settings.automation_vfr_mode
}

/// Check rotorcraft mode via settings.
fn is_rotorcraft_flight_via_settings(settings: &Settings) -> bool {
    settings.automation_rotorcraft_mode
}

/// Engine endpoint to control automation.
pub async fn automation_control(
    axum::extract::State(shared): axum::extract::State<crate::Shared>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let action = body["action"].as_str().unwrap_or("");
    let mut state = shared.write().await;

    match action {
        "start" => {
            let mut automation = std::mem::take(&mut state.automation);
            let state_mut = &mut *state;
            let session = &mut state_mut.session;
            let loaded_aircraft_specs = &mut state_mut.loaded_aircraft_specs;
            let settings = &state_mut.settings;
            let arrival_airport = &state_mut.arrival_airport;
            automation.start(session, loaded_aircraft_specs, settings, arrival_airport);
            state.automation = automation;
            crate::routes::ok("Automation started")
        }
        "stop" => {
            state.automation.stop();
            crate::routes::ok("Automation stopped")
        }
        "toggle_stop_after_flight" => {
            state.automation.toggle_stop_after_flight();
            crate::routes::ok_json(serde_json::json!({
                "stop_after_flight": state.automation.config.stop_after_flight
            }))
        }
        "status" => {
            crate::routes::ok_json(serde_json::json!({
                "active": state.automation.status.active,
                "phase": state.automation.status.phase,
                "stop_after_flight": state.automation.status.stop_after_flight,
                "current_flight": state.automation.status.current_flight,
                "total_flights": state.automation.status.total_flights,
                "last_action": state.automation.status.last_action,
                "next_action": state.automation.status.next_action,
            }))
        }
        _ => crate::routes::bad_request("Invalid automation action"),
    }
}