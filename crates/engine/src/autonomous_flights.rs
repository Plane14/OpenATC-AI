//! Autonomous randomized flights for the user aircraft.
//!
//! When enabled, after the user lands and parks, the system selects a compatible
//! destination airport, files a flight plan, manages the turnaround, and repeats
//! the cycle. Only airports that can support the user aircraft type are chosen.

use openatc_core::state::{FlightPlan, State};
use openatc_core::traffic::profiles::{Aircraft, Profiles};
use openatc_settings::Settings;
use rand::Rng;
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

/// Eurocontrol aircraft performance data from ATPCFDB.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct EurocontrolAircraft {
    pub icao: String,
    pub name: String,
    pub category: String,  // L, M, H, J (Light, Medium, Heavy, Super)
    pub wake_category: u8,
    pub wingspan_m: f64,
    pub length_m: f64,
    pub height_m: f64,
    pub max_takeoff_weight_kg: f64,
    pub max_landing_weight_kg: f64,
    pub max_zero_fuel_weight_kg: f64,
    pub max_fuel_capacity_kg: f64,
    pub max_speed_kts: f64,
    pub cruise_speed_kts: f64,
    pub stall_speed_kts: f64,
    pub service_ceiling_ft: f64,
    pub max_range_nm: f64,
    pub climb_rate_fpm: f64,
    pub descent_rate_fpm: f64,
    pub approach_speed_kts: f64,
    pub takeoff_distance_m: f64,
    pub landing_distance_m: f64,
    pub engine_type: String,
    pub engine_count: u8,
}

/// Cache for Eurocontrol performance data.
#[derive(Clone, Debug, Default)]
pub struct EurocontrolCache {
    pub aircraft_db: HashMap<String, EurocontrolAircraft>,
    pub last_updated: Option<Instant>,
    pub cache_duration: Duration,
}

impl EurocontrolCache {
    pub fn new() -> Self {
        Self {
            aircraft_db: HashMap::new(),
            last_updated: None,
            cache_duration: Duration::from_secs(24 * 3600), // 24 hours
        }
    }

    /// Check if cache is valid.
    pub fn is_valid(&self) -> bool {
        if let Some(last) = self.last_updated {
            last.elapsed() < self.cache_duration
        } else {
            false
        }
    }

    /// Fetch aircraft performance data from Eurocontrol ATPCFDB.
    /// This is a placeholder - the actual API endpoint would need to be implemented
    /// based on Eurocontrol's API specification.
    pub async fn fetch_performance(&mut self, icao: &str) -> Result<Option<EurocontrolAircraft>, String> {
        if self.is_valid() {
            if let Some(aircraft) = self.aircraft_db.get(icao) {
                return Ok(Some(aircraft.clone()));
            }
        }

        // In a real implementation, this would call the Eurocontrol API
        // For now, we return None to fall back to built-in profiles
        log::info!("Eurocontrol ATPCFDB fetch not yet implemented for {}", icao);
        Ok(None)
    }

    /// Update cache with new data.
    pub fn update_cache(&mut self, aircraft: EurocontrolAircraft) {
        self.aircraft_db.insert(aircraft.icao.clone(), aircraft);
        self.last_updated = Some(Instant::now());
    }
}

/// State for autonomous randomized flights.
#[derive(Clone, Debug, Default)]
pub struct AutonomousFlightsState {
    pub enabled: bool,
    pub min_turnaround_minutes: u16,
    pub max_turnaround_minutes: u16,
    pub next_flight_at: Option<Instant>,
    pub turnaround_started: bool,
    pub current_leg: u32,
    pub eurocontrol_cache: EurocontrolCache,
    pub use_eurocontrol: bool,
}

impl AutonomousFlightsState {
    pub fn new(settings: &Settings) -> Self {
        Self {
            enabled: settings.autonomous_random_flights,
            min_turnaround_minutes: settings.autonomous_min_turnaround_minutes,
            max_turnaround_minutes: settings.autonomous_max_turnaround_minutes,
            next_flight_at: None,
            turnaround_started: false,
            current_leg: 0,
            eurocontrol_cache: EurocontrolCache::new(),
            use_eurocontrol: settings.use_eurocontrol_performance,
        }
    }

    /// Update settings from the current configuration.
    pub fn update_settings(&mut self, settings: &Settings) {
        self.enabled = settings.autonomous_random_flights;
        self.min_turnaround_minutes = settings.autonomous_min_turnaround_minutes;
        self.max_turnaround_minutes = settings.autonomous_max_turnaround_minutes;
        self.use_eurocontrol = settings.use_eurocontrol_performance;
    }

    /// Check if it's time to start the next flight.
    pub fn should_start_next_flight(&self) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(next) = self.next_flight_at {
            Instant::now() >= next
        } else {
            false
        }
    }

    /// Start the turnaround timer after landing.
    pub fn start_turnaround(&mut self) {
        if !self.enabled {
            return;
        }
        let mut rng = rand::thread_rng();
        let minutes = rng.gen_range(self.min_turnaround_minutes..=self.max_turnaround_minutes);
        self.next_flight_at = Some(Instant::now() + Duration::from_secs(minutes as u64 * 60));
        self.turnaround_started = true;
    }

    /// Reset the state for a new flight.
    pub fn reset(&mut self) {
        self.next_flight_at = None;
        self.turnaround_started = false;
    }

    /// Find a compatible destination airport for the user aircraft.
    pub fn find_compatible_destination(
        &self,
        current_airport: &str,
        aircraft_icao: &str,
        profiles: &Profiles,
        root: &str,
    ) -> Option<String> {
        let aircraft = profiles.aircraft_for(aircraft_icao)?;
        
        // Load all airports and filter by compatibility
        let airports = openatc_core::airport::load_all_airports(root).ok()?;
        
        let mut rng = rand::thread_rng();
        let mut compatible: Vec<String> = airports
            .into_iter()
            .filter(|apt| apt.icao != current_airport)
            .filter(|apt| airport_supports_aircraft(apt, aircraft))
            .map(|apt| apt.icao)
            .collect();

        if compatible.is_empty() {
            return None;
        }

        compatible.shuffle(&mut rng);
        compatible.into_iter().next()
    }

    /// Get aircraft performance data, preferring Eurocontrol data if available.
    pub async fn get_aircraft_performance(&mut self, icao: &str) -> Aircraft {
        // Try Eurocontrol first
        if self.use_eurocontrol {
            if let Ok(Some(euro)) = self.eurocontrol_cache.fetch_performance(icao).await {
                // Convert Eurocontrol data to Aircraft profile
                return Aircraft {
                    icao: euro.icao.clone(),
                    name: euro.name,
                    category: match euro.wake_category {
                        0 => openatc_core::traffic::profiles::Category::Light,
                        1 => openatc_core::traffic::profiles::Category::Medium,
                        2 => openatc_core::traffic::profiles::Category::Heavy,
                        3 => openatc_core::traffic::profiles::Category::Super,
                        _ => openatc_core::traffic::profiles::Category::Medium,
                    },
                    wingspan: euro.wingspan_m,
                    length: euro.length_m,
                    height: euro.height_m,
                    mtow: euro.max_takeoff_weight_kg,
                    mlw: euro.max_landing_weight_kg,
                    mzfw: euro.max_zero_fuel_weight_kg,
                    fuel_capacity: euro.max_fuel_capacity_kg,
                    max_speed: euro.max_speed_kts,
                    cruise_speed: euro.cruise_speed_kts,
                    stall_speed: euro.stall_speed_kts,
                    ceiling: euro.service_ceiling_ft as i32,
                    range_nm: euro.max_range_nm,
                    climb_rate: euro.climb_rate_fpm,
                    descent_rate: euro.descent_rate_fpm,
                    approach_speed: euro.approach_speed_kts,
                    takeoff_distance: euro.takeoff_distance_m,
                    landing_distance: euro.landing_distance_m,
                    engine_type: euro.engine_type,
                    engine_count: euro.engine_count as i32,
                    ..Default::default()
                };
            }
        }

        // Fall back to built-in profiles
        Profiles::builtin()
            .ok()
            .and_then(|p| p.aircraft_for(icao).cloned())
            .unwrap_or_default()
    }
}

/// Check if an airport can support the given aircraft type.
fn airport_supports_aircraft(airport: &openatc_core::airport::Airport, aircraft: &Aircraft) -> bool {
    // Check if any runway is long enough for takeoff
    let min_runway_length = aircraft.takeoff_distance.max(aircraft.landing_distance);
    let has_sufficient_runway = airport.runways.iter().any(|rwy| {
        rwy.length_m >= min_runway_length || rwy.length_m >= 800.0 // At least 800m for GA
    });

    // Check if any parking spot can accommodate the wingspan
    let has_sufficient_parking = airport.parking.iter().any(|spot| {
        if let Some(size) = &spot.size {
            // Size categories: 0=GA, 1=Regional, 2=Narrow, 3=Wide, 4=Heavy
            match aircraft.category {
                openatc_core::traffic::profiles::Category::Light => *size >= 0,
                openatc_core::traffic::profiles::Category::Medium => *size >= 1,
                openatc_core::traffic::profiles::Category::Heavy => *size >= 3,
                openatc_core::traffic::profiles::Category::Super => *size >= 4,
            }
        } else {
            // No size info, assume it can handle the aircraft
            true
        }
    });

    has_sufficient_runway && has_sufficient_parking
}

/// Generate a new flight plan for the autonomous user aircraft.
pub fn generate_flight_plan(
    state: &State,
    destination: &str,
    aircraft: &Aircraft,
    root: &str,
) -> Result<FlightPlan, String> {
    let departure = state.plan.departure.clone();
    if departure.is_empty() {
        return Err("No departure airport in current plan".to_string());
    }

    let mut plan = FlightPlan::default();
    plan.departure = departure;
    plan.destination = destination.to_owned();
    plan.aircraft = aircraft.icao.clone();
    plan.callsign = state.plan.callsign.clone();
    plan.registration = state.plan.registration.clone();

    // Set cruise altitude based on aircraft ceiling
    let max_alt = aircraft.ceiling.min(41000);
    plan.cruise_feet = ((max_alt / 1000) * 1000).max(1000); // Round to nearest 1000ft
    plan.initial_altitude_feet = plan.cruise_feet.min(10000);

    // Enrich with navigation data
    openatc_core::navigation::enrich(&mut plan, root);

    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autonomous_flights_state_defaults() {
        let settings = Settings::default();
        let state = AutonomousFlightsState::new(&settings);
        assert!(!state.enabled);
        assert_eq!(state.min_turnaround_minutes, 30);
        assert_eq!(state.max_turnaround_minutes, 90);
    }
}