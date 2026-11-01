//! ADSB.lol traffic source integration.
//! Fetches real-time ADS-B traffic data from adsb.lol API.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// ADSB.lol aircraft state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AdsbAircraft {
    pub hex: String,
    pub flight: Option<String>,
    pub r#type: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub alt_baro: Option<i32>,
    pub alt_geom: Option<i32>,
    pub gs: Option<f64>,
    pub track: Option<f64>,
    pub baro_rate: Option<f64>,
    pub squawk: Option<String>,
    pub category: Option<String>,
    pub nav_qnh: Option<f64>,
    pub nav_altitude_mcp: Option<i32>,
    pub nav_heading: Option<f64>,
    pub emergency: Option<String>,
    pub seen: f64,
    pub seen_pos: f64,
    pub rssi: Option<f64>,
}

/// ADSB.lol API response.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AdsbResponse {
    pub now: f64,
    pub messages: u64,
    pub aircraft: Vec<AdsbAircraft>,
}

/// ADSB.lol client configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdsbClientConfig {
    /// Base URL for ADSB.lol API.
    pub base_url: String,
    /// API key (if required).
    pub api_key: Option<String>,
    /// Radius in nautical miles for traffic query.
    pub radius_nm: f64,
    /// Latitude for query center.
    pub center_lat: f64,
    /// Longitude for query center.
    pub center_lon: f64,
    /// Minimum altitude filter (feet).
    pub min_altitude_ft: Option<i32>,
    /// Maximum altitude filter (feet).
    pub max_altitude_ft: Option<i32>,
    /// Update interval in seconds.
    pub update_interval_sec: u64,
}

impl Default for AdsbClientConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.adsb.lol".to_string(),
            api_key: None,
            radius_nm: 50.0,
            center_lat: 0.0,
            center_lon: 0.0,
            min_altitude_ft: None,
            max_altitude_ft: None,
            update_interval_sec: 30,
        }
    }
}

/// ADSB.lol traffic client.
pub struct AdsbClient {
    client: reqwest::Client,
    config: AdsbClientConfig,
    cache: Option<AdsbResponse>,
    last_update: Option<Instant>,
}

impl AdsbClient {
    pub fn new(config: AdsbClientConfig) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            config,
            cache: None,
            last_update: None,
        }
    }

    /// Update the query center position.
    pub fn set_center(&mut self, lat: f64, lon: f64) {
        self.config.center_lat = lat;
        self.config.center_lon = lon;
    }

    /// Fetch traffic data from ADSB.lol.
    pub async fn fetch_traffic(&mut self) -> Result<Vec<AdsbAircraft>, String> {
        let now = Instant::now();
        
        // Check cache
        if let Some(last) = self.last_update {
            if now.duration_since(last).as_secs() < self.config.update_interval_sec {
                if let Some(cached) = &self.cache {
                    return Ok(cached.aircraft.clone());
                }
            }
        }

        // Build query URL
        let url = format!(
            "{}/v2/lat/{}/lon/{}/dist/{}",
            self.config.base_url,
            self.config.center_lat,
            self.config.center_lon,
            self.config.radius_nm
        );

        let mut request = self.client.get(&url);
        if let Some(key) = &self.config.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request.send().await
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }

        let adsb_response: AdsbResponse = response.json().await
            .map_err(|e| format!("Failed to parse JSON: {}", e))?;

        // Filter by altitude if configured
        let filtered: Vec<AdsbAircraft> = adsb_response.aircraft.into_iter()
            .filter(|a| {
                if let Some(min) = self.config.min_altitude_ft {
                    if a.alt_baro.unwrap_or(0) < min || a.alt_geom.unwrap_or(0) < min {
                        return false;
                    }
                }
                if let Some(max) = self.config.max_altitude_ft {
                    if a.alt_baro.unwrap_or(i32::MAX) > max || a.alt_geom.unwrap_or(i32::MAX) > max {
                        return false;
                    }
                }
                true
            })
            .collect();

        self.cache = Some(AdsbResponse {
            aircraft: filtered.clone(),
            ..adsb_response
        });
        self.last_update = Some(now);

        Ok(filtered)
    }

    /// Convert ADS-B aircraft to OpenATC traffic format.
    pub fn convert_to_traffic(&self, aircraft: &AdsbAircraft) -> Option<AdsbTraffic> {
        let callsign = aircraft.flight.as_ref()?.trim();
        if callsign.is_empty() {
            return None;
        }

        let icao_type = aircraft.r#type.as_ref()?.trim();
        if icao_type.is_empty() {
            return None;
        }

        let lat = aircraft.lat;
        let lon = aircraft.lon;
        let altitude = aircraft.alt_baro.or(aircraft.alt_geom).unwrap_or(0);
        let ground_speed = aircraft.gs.unwrap_or(0.0);
        let track = aircraft.track.unwrap_or(0.0);
        let vertical_speed = aircraft.baro_rate.unwrap_or(0.0);

        Some(AdsbTraffic {
            callsign: callsign.to_string(),
            icao_type: icao_type.to_string(),
            hex: aircraft.hex.clone(),
            lat,
            lon,
            altitude_ft: altitude,
            ground_speed_kt: ground_speed,
            track_deg: track,
            vertical_speed_fpm: vertical_speed,
            squawk: aircraft.squawk.clone().unwrap_or_default(),
            emergency: aircraft.emergency.clone().unwrap_or_default(),
            last_update: Utc::now(),
        })
    }
}

/// Converted ADS-B traffic for OpenATC.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdsbTraffic {
    pub callsign: String,
    pub icao_type: String,
    pub hex: String,
    pub lat: f64,
    pub lon: f64,
    pub altitude_ft: i32,
    pub ground_speed_kt: f64,
    pub track_deg: f64,
    pub vertical_speed_fpm: f64,
    pub squawk: String,
    pub emergency: String,
    pub last_update: DateTime<Utc>,
}

/// ADSB.lol traffic source for generated traffic.
pub struct AdsbTrafficSource {
    client: AdsbClient,
    enabled: bool,
}

impl AdsbTrafficSource {
    pub fn new(config: AdsbClientConfig) -> Self {
        Self {
            client: AdsbClient::new(config),
            enabled: false,
        }
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_center(&mut self, lat: f64, lon: f64) {
        self.client.set_center(lat, lon);
    }

    /// Fetch and convert traffic for the current position.
    pub async fn fetch_traffic(&mut self) -> Result<Vec<AdsbTraffic>, String> {
        if !self.enabled {
            return Ok(Vec::new());
        }

        let aircraft = self.client.fetch_traffic().await?;
        let traffic: Vec<AdsbTraffic> = aircraft
            .iter()
            .filter_map(|a| self.client.convert_to_traffic(a))
            .collect();

        Ok(traffic)
    }
}

impl Default for AdsbTrafficSource {
    fn default() -> Self {
        Self::new(AdsbClientConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adsb_client_config_default() {
        let config = AdsbClientConfig::default();
        assert_eq!(config.base_url, "https://api.adsb.lol");
        assert_eq!(config.radius_nm, 50.0);
        assert_eq!(config.update_interval_sec, 30);
    }

    #[test]
    fn test_adsb_aircraft_conversion() {
        let aircraft = AdsbAircraft {
            hex: "a1b2c3".to_string(),
            flight: Some("UAL123".to_string()),
            r#type: Some("B738".to_string()),
            lat: 37.6189,
            lon: -122.375,
            alt_baro: Some(35000),
            gs: Some(450.0),
            track: Some(270.0),
            baro_rate: Some(0.0),
            squawk: Some("1234".to_string()),
            emergency: Some("none".to_string()),
            ..Default::default()
        };

        let client = AdsbClient::new(AdsbClientConfig::default());
        let traffic = client.convert_to_traffic(&aircraft);

        assert!(traffic.is_some());
        let t = traffic.unwrap();
        assert_eq!(t.callsign, "UAL123");
        assert_eq!(t.icao_type, "B738");
        assert_eq!(t.altitude_ft, 35000);
    }
}