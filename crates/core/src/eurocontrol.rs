//! Eurocontrol ATPCFDB (Aircraft Type Performance Database) integration.
//! Provides aircraft performance data for flight planning and automation.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Base URL for Eurocontrol ATPCFDB.
const EUROCONTROL_BASE_URL: &str = "https://learningzone.eurocontrol.int/ilp/customs/ATCPFDB";

/// Cached performance data.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EurocontrolCache {
    pub aircraft_data: HashMap<String, AircraftPerformance>,
    pub last_updated: Option<DateTime<Utc>>,
    pub cache_duration: Duration,
}

/// Aircraft performance data from Eurocontrol ATPCFDB.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AircraftPerformance {
    pub icao_code: String,
    pub manufacturer: String,
    pub model: String,
    pub engine_type: String,
    pub engine_count: u8,
    pub mtow_kg: f64,
    pub mlw_kg: f64,
    pub mzfw_kg: f64,
    pub max_fuel_kg: f64,
    pub wingspan_m: f64,
    pub length_m: f64,
    pub height_m: f64,
    pub cruise_speed_kt: f64,
    pub cruise_mach: f64,
    pub ceiling_ft: i32,
    pub approach_speed_kt: f64,
    pub takeoff_distance_m: f64,
    pub landing_distance_m: f64,
    pub climb_rate_fpm: f64,
    pub descent_rate_fpm: f64,
    pub fuel_consumption_kg_per_hr: f64,
    pub range_nm: f64,
    pub wake_category: char,
    pub approach_category: char,
}

/// Eurocontrol ATPCFDB client.
pub struct EurocontrolClient {
    client: reqwest::Client,
    cache: EurocontrolCache,
}

impl EurocontrolClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            cache: EurocontrolCache {
                cache_duration: Duration::from_hours(24), // 24 hours
                ..Default::default()
            },
        }
    }

    /// Fetch aircraft performance data for a specific ICAO code.
    pub async fn fetch_aircraft_performance(&mut self, icao_code: &str) -> Result<AircraftPerformance, String> {
        let icao = icao_code.to_uppercase();

        // Check cache first
        if let Some(cached) = self.cache.aircraft_data.get(&icao) {
            if let Some(last) = self.cache.last_updated {
                let age = Utc::now() - last;
                let cache_duration = TimeDelta::from_std(self.cache.cache_duration).unwrap_or(TimeDelta::zero());
                if age < cache_duration {
                    return Ok(cached.clone());
                }
            }
        }

        // Try to fetch from Eurocontrol ATPCFDB
        let url = format!("{}/default.aspx?aircraft={}", EUROCONTROL_BASE_URL, icao);
        
        match self.fetch_and_parse(&url, &icao).await {
            Ok(perf) => {
                self.cache.aircraft_data.insert(icao.clone(), perf.clone());
                self.cache.last_updated = Some(Utc::now());
                Ok(perf)
            }
            Err(e) => {
                // Fallback to built-in data
                self.get_builtin_performance(&icao).ok_or(e)
            }
        }
    }

    /// Fetch and parse performance data from Eurocontrol web page.
    async fn fetch_and_parse(&self, url: &str, icao: &str) -> Result<AircraftPerformance, String> {
        let response = self.client.get(url).send().await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }

        let html = response.text().await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        self.parse_aircraft_page(&html, icao)
    }

    /// Parse the Eurocontrol aircraft performance page.
    fn parse_aircraft_page(&self, html: &str, icao: &str) -> Result<AircraftPerformance, String> {
        // The Eurocontrol ATPCFDB page contains tables with performance data
        // We need to extract the relevant tables
        let mut perf = AircraftPerformance {
            icao_code: icao.to_string(),
            ..Default::default()
        };

        // Parse HTML tables - this is a simplified parser
        // In production, you'd use a proper HTML parser like scraper or tl
        for line in html.lines() {
            let line = line.trim();
            
            // Look for table rows with performance data
            if line.contains("<td>") || line.contains("<th>") {
                if let Some((key, value)) = self.extract_table_cell(line) {
                    self.assign_performance_field(&mut perf, &key, &value);
                }
            }
        }

        // Validate we got meaningful data
        if perf.mtow_kg <= 0.0 {
            return Err("No performance data found".to_string());
        }

        Ok(perf)
    }

    /// Extract key-value from a table cell.
    fn extract_table_cell(&self, line: &str) -> Option<(String, String)> {
        // Simple extraction - in production use proper HTML parsing
        if line.contains("</td>") {
            let parts: Vec<&str> = line.split("</td>").collect();
            if parts.len() >= 2 {
                let key = parts[0].trim().trim_start_matches("<td>").trim_start_matches("<th>");
                let value = parts[1].trim().trim_start_matches("<td>").trim_start_matches("<th>");
                if !key.is_empty() && !value.is_empty() {
                    return Some((key.to_string(), value.to_string()));
                }
            }
        }
        None
    }

    /// Assign parsed value to performance field.
    fn assign_performance_field(&self, perf: &mut AircraftPerformance, key: &str, value: &str) {
        let key_lower = key.to_lowercase();
        let value_clean = value.trim();

        if let Ok(v) = value_clean.parse::<f64>() {
            if key_lower.contains("mtow") || key_lower.contains("max takeoff") {
                perf.mtow_kg = v;
            } else if key_lower.contains("mlw") || key_lower.contains("max landing") {
                perf.mlw_kg = v;
            } else if key_lower.contains("mzfw") || key_lower.contains("max zero fuel") {
                perf.mzfw_kg = v;
            } else if key_lower.contains("fuel") && key_lower.contains("max") {
                perf.max_fuel_kg = v;
            } else if key_lower.contains("wingspan") || key_lower.contains("wing span") {
                perf.wingspan_m = v;
            } else if key_lower.contains("length") {
                perf.length_m = v;
            } else if key_lower.contains("height") {
                perf.height_m = v;
            } else if key_lower.contains("cruise") && key_lower.contains("speed") {
                perf.cruise_speed_kt = v;
            } else if key_lower.contains("cruise") && key_lower.contains("mach") {
                perf.cruise_mach = v;
            } else if key_lower.contains("ceiling") || key_lower.contains("max altitude") {
                perf.ceiling_ft = v as i32;
            } else if key_lower.contains("approach") && key_lower.contains("speed") {
                perf.approach_speed_kt = v;
            } else if key_lower.contains("takeoff") && key_lower.contains("distance") {
                perf.takeoff_distance_m = v;
            } else if key_lower.contains("landing") && key_lower.contains("distance") {
                perf.landing_distance_m = v;
            } else if key_lower.contains("climb") && key_lower.contains("rate") {
                perf.climb_rate_fpm = v;
            } else if key_lower.contains("descent") && key_lower.contains("rate") {
                perf.descent_rate_fpm = v;
            } else if key_lower.contains("fuel") && key_lower.contains("consumption") {
                perf.fuel_consumption_kg_per_hr = v;
            } else if key_lower.contains("range") {
                perf.range_nm = v;
            }
        }
    }

    /// Get built-in performance data for common aircraft types.
    pub fn get_builtin_performance(&self, icao: &str) -> Option<AircraftPerformance> {
        let perf = match icao {
            "A320" | "A20N" | "A319" | "A321" => AircraftPerformance {
                icao_code: icao.to_string(),
                manufacturer: "Airbus".to_string(),
                model: "A320 Family".to_string(),
                engine_type: "Turbofan".to_string(),
                engine_count: 2,
                mtow_kg: 78000.0,
                mlw_kg: 66000.0,
                mzfw_kg: 62000.0,
                max_fuel_kg: 24000.0,
                wingspan_m: 35.8,
                length_m: 37.57,
                height_m: 11.76,
                cruise_speed_kt: 450.0,
                cruise_mach: 0.78,
                ceiling_ft: 39000,
                approach_speed_kt: 135.0,
                takeoff_distance_m: 2000.0,
                landing_distance_m: 1500.0,
                climb_rate_fpm: 2500.0,
                descent_rate_fpm: 2000.0,
                fuel_consumption_kg_per_hr: 2500.0,
                range_nm: 3300.0,
                wake_category: 'M',
                approach_category: 'C',
            },
            "B737" | "B738" | "B739" => AircraftPerformance {
                icao_code: icao.to_string(),
                manufacturer: "Boeing".to_string(),
                model: "737 NG/MAX".to_string(),
                engine_type: "Turbofan".to_string(),
                engine_count: 2,
                mtow_kg: 79000.0,
                mlw_kg: 66000.0,
                mzfw_kg: 62000.0,
                max_fuel_kg: 26000.0,
                wingspan_m: 35.8,
                length_m: 39.5,
                height_m: 12.5,
                cruise_speed_kt: 450.0,
                cruise_mach: 0.79,
                ceiling_ft: 41000,
                approach_speed_kt: 140.0,
                takeoff_distance_m: 2100.0,
                landing_distance_m: 1600.0,
                climb_rate_fpm: 2800.0,
                descent_rate_fpm: 2000.0,
                fuel_consumption_kg_per_hr: 2600.0,
                range_nm: 3500.0,
                wake_category: 'M',
                approach_category: 'C',
            },
            "C172" => AircraftPerformance {
                icao_code: icao.to_string(),
                manufacturer: "Cessna".to_string(),
                model: "172 Skyhawk".to_string(),
                engine_type: "Piston".to_string(),
                engine_count: 1,
                mtow_kg: 1157.0,
                mlw_kg: 1157.0,
                mzfw_kg: 1100.0,
                max_fuel_kg: 180.0,
                wingspan_m: 11.0,
                length_m: 8.3,
                height_m: 2.7,
                cruise_speed_kt: 122.0,
                cruise_mach: 0.0,
                ceiling_ft: 14000,
                approach_speed_kt: 65.0,
                takeoff_distance_m: 400.0,
                landing_distance_m: 350.0,
                climb_rate_fpm: 700.0,
                descent_rate_fpm: 500.0,
                fuel_consumption_kg_per_hr: 35.0,
                range_nm: 640.0,
                wake_category: 'L',
                approach_category: 'A',
            },
            _ => return None,
        };
        Some(perf)
    }

    /// Get performance data with fallback chain: cache -> Eurocontrol -> builtin.
    pub async fn get_performance(&mut self, icao_code: &str) -> AircraftPerformance {
        self.fetch_aircraft_performance(icao_code).await
            .unwrap_or_else(|_| AircraftPerformance {
                icao_code: icao_code.to_uppercase(),
                ..Default::default()
            })
    }
}

impl Default for EurocontrolClient {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for EurocontrolClient {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            cache: self.cache.clone(),
        }
    }
}

impl std::fmt::Debug for EurocontrolClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EurocontrolClient")
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

/// Merge Eurocontrol data with ACF specs for complete aircraft profile.
pub fn merge_performance_data(
    acf_specs: &crate::ops::AcfSpecs,
    eurocontrol: &AircraftPerformance,
) -> crate::ops::AcfSpecs {
    let mut merged = acf_specs.clone();
    
    // Use Eurocontrol data where ACF specs are missing
    if merged.max_takeoff_weight_kg.is_none() {
        merged.max_takeoff_weight_kg = Some(eurocontrol.mtow_kg);
    }
    if merged.max_landing_weight_kg.is_none() {
        merged.max_landing_weight_kg = Some(eurocontrol.mlw_kg);
    }
    if merged.max_fuel_kg.is_none() {
        merged.max_fuel_kg = Some(eurocontrol.max_fuel_kg);
    }
    if merged.wingspan_meters.is_none() {
        merged.wingspan_meters = Some(eurocontrol.wingspan_m);
    }
    if merged.length_meters.is_none() {
        merged.length_meters = Some(eurocontrol.length_m);
    }
    if merged.height_meters.is_none() {
        merged.height_meters = Some(eurocontrol.height_m);
    }
    if merged.ceiling_feet.is_none() {
        merged.ceiling_feet = Some(eurocontrol.ceiling_ft);
    }
    if merged.cruise_speed_knots.is_none() {
        merged.cruise_speed_knots = Some(eurocontrol.cruise_speed_kt);
    }
    if merged.vref_knots.is_none() {
        merged.vref_knots = Some(eurocontrol.approach_speed_kt);
    }
    if merged.max_climb_rate_fpm.is_none() {
        merged.max_climb_rate_fpm = Some(eurocontrol.climb_rate_fpm);
    }
    if merged.max_descent_rate_fpm.is_none() {
        merged.max_descent_rate_fpm = Some(eurocontrol.descent_rate_fpm);
    }
    if merged.wake_category.is_none() {
        merged.wake_category = Some(eurocontrol.wake_category);
    }
    if merged.approach_category.is_none() {
        merged.approach_category = Some(eurocontrol.approach_category);
    }
    
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_performance_a320() {
        let client = EurocontrolClient::new();
        let perf = client.get_builtin_performance("A320").unwrap();
        assert_eq!(perf.icao_code, "A320");
        assert_eq!(perf.wake_category, 'M');
        assert_eq!(perf.approach_category, 'C');
        assert!(perf.mtow_kg > 0.0);
    }

    #[test]
    fn test_builtin_performance_c172() {
        let client = EurocontrolClient::new();
        let perf = client.get_builtin_performance("C172").unwrap();
        assert_eq!(perf.icao_code, "C172");
        assert_eq!(perf.wake_category, 'L');
        assert_eq!(perf.approach_category, 'A');
        assert!(perf.mtow_kg > 0.0);
    }
}