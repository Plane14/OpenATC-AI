//! Optional ADS-B feed integration backed by adsb.lol.

use crate::Shared;
use openatc_core::state::{Telemetry, TrafficTarget};
use openatc_settings::AdsbTrafficSource;
use serde::Deserialize;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Runtime {
    in_flight: bool,
    last_poll: Option<Instant>,
    updated_at: Option<Instant>,
    targets: Vec<TrafficTarget>,
}

#[derive(Clone, Deserialize)]
struct FeedResponse {
    #[serde(default)]
    ac: Vec<FeedAircraft>,
}

#[derive(Clone, Deserialize)]
struct FeedAircraft {
    hex: Option<String>,
    flight: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
    track: Option<f64>,
    gs: Option<f64>,
    #[serde(default)]
    seen: Option<f64>,
    #[serde(default)]
    t: Option<String>,
    #[serde(default, rename = "type")]
    aircraft_type: Option<String>,
    #[serde(default)]
    reg: Option<String>,
    #[serde(default)]
    alt_baro: Option<Altitude>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum Altitude {
    Number(f64),
    Integer(i64),
    Label(String),
}

pub async fn merge(shared: Shared, mut telemetry: Telemetry) -> Telemetry {
    let (source, poll_seconds, radius_nm, max_targets, max_seen, client) = {
        let state = shared.read().await;
        (
            state.settings.adsb_traffic_source,
            state.settings.adsb_poll_seconds,
            state.settings.adsb_radius_nm,
            state.settings.adsb_max_targets,
            state.settings.adsb_max_seen_seconds,
            state.client.clone(),
        )
    };
    if source != AdsbTrafficSource::AdsbLol || !telemetry.position_valid {
        return telemetry;
    }
    let now = Instant::now();
    let should_poll = {
        let mut state = shared.write().await;
        let runtime = &mut state.adsb;
        let due = runtime.last_poll.is_none_or(|at| {
            now.saturating_duration_since(at) >= Duration::from_secs(u64::from(poll_seconds))
        });
        if due && !runtime.in_flight {
            runtime.in_flight = true;
            runtime.last_poll = Some(now);
            true
        } else {
            false
        }
    };
    if should_poll {
        let fetch_shared = shared.clone();
        let url = format!(
            "https://api.adsb.lol/v2/point/{}/{}/{}",
            telemetry.latitude, telemetry.longitude, radius_nm
        );
        tokio::spawn(async move {
            let result = fetch(&client, &url, max_seen, max_targets).await;
            let mut state = fetch_shared.write().await;
            state.adsb.in_flight = false;
            if let Ok(targets) = result {
                state.adsb.updated_at = Some(Instant::now());
                state.adsb.targets = targets;
            }
        });
    }
    let adsb_targets = {
        let mut state = shared.write().await;
        let stale = state.adsb.updated_at.is_none_or(|at| {
            now.saturating_duration_since(at) > Duration::from_secs(u64::from(max_seen) * 3)
        });
        if stale {
            state.adsb.targets.clear();
            Vec::new()
        } else {
            state.adsb.targets.clone()
        }
    };
    if adsb_targets.is_empty() {
        return telemetry;
    }
    let mut merged = telemetry.traffic.take().unwrap_or_default();
    merged.reserve(adsb_targets.len());
    for target in adsb_targets {
        if merged.len() >= 126 {
            break;
        }
        if duplicate(&merged, &target) || likely_ownship(&telemetry, &target) {
            continue;
        }
        merged.push(target);
    }
    telemetry.traffic = Some(merged);
    telemetry
}

async fn fetch(
    client: &reqwest::Client,
    url: &str,
    max_seen_seconds: u8,
    max_targets: u8,
) -> Result<Vec<TrafficTarget>, String> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("adsb.lol status {}", response.status()));
    }
    let payload: FeedResponse = response.json().await.map_err(|error| error.to_string())?;
    let mut targets = Vec::new();
    for aircraft in payload.ac {
        if targets.len() >= usize::from(max_targets.min(126)) {
            break;
        }
        let Some(target) = to_target(&aircraft, max_seen_seconds) else {
            continue;
        };
        targets.push(target);
    }
    Ok(targets)
}

fn to_target(aircraft: &FeedAircraft, max_seen_seconds: u8) -> Option<TrafficTarget> {
    if aircraft
        .seen
        .is_some_and(|age| !age.is_finite() || age < 0.0 || age > f64::from(max_seen_seconds))
    {
        return None;
    }
    let latitude = aircraft.lat?;
    let longitude = aircraft.lon?;
    if !latitude.is_finite()
        || !longitude.is_finite()
        || latitude.abs() > 90.0
        || longitude.abs() > 180.0
    {
        return None;
    }
    let track = aircraft.track.unwrap_or(0.0).rem_euclid(360.0);
    let speed = aircraft
        .gs
        .filter(|value| value.is_finite() && *value >= 0.0);
    let (altitude_feet, on_ground) = match aircraft.alt_baro.as_ref() {
        Some(Altitude::Number(value)) if value.is_finite() => (*value, false),
        Some(Altitude::Integer(value)) => (*value as f64, false),
        Some(Altitude::Label(label)) if label.eq_ignore_ascii_case("ground") => (0.0, true),
        _ => (0.0, speed.is_some_and(|value| value < 35.0)),
    };
    let id = aircraft
        .hex
        .as_deref()
        .and_then(|hex| u32::from_str_radix(hex.trim(), 16).ok())
        .unwrap_or(0);
    let callsign = aircraft
        .flight
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .to_owned();
    let aircraft_type = aircraft
        .aircraft_type
        .as_deref()
        .or(aircraft.t.as_deref())
        .map(str::trim)
        .unwrap_or("")
        .to_owned();
    if id == 0 && callsign.is_empty() && aircraft_type.is_empty() {
        return None;
    }
    Some(TrafficTarget {
        id,
        callsign,
        aircraft: aircraft_type,
        name: aircraft.reg.clone().unwrap_or_default(),
        latitude,
        longitude,
        altitude_feet,
        track_degrees: track,
        ground_speed_knots: speed,
        on_ground,
        ..Default::default()
    })
}

fn duplicate(existing: &[TrafficTarget], candidate: &TrafficTarget) -> bool {
    existing.iter().any(|target| {
        (candidate.id != 0 && target.id == candidate.id)
            || (!candidate.callsign.is_empty()
                && !target.callsign.is_empty()
                && target.callsign.eq_ignore_ascii_case(&candidate.callsign))
    })
}

fn likely_ownship(telemetry: &Telemetry, target: &TrafficTarget) -> bool {
    let distance_nm = openatc_core::arrival::distance_nm(
        [telemetry.latitude, telemetry.longitude],
        [target.latitude, target.longitude],
    );
    distance_nm.is_finite()
        && distance_nm < 0.08
        && (telemetry.altitude_feet - target.altitude_feet).abs() < 500.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_rejects_stale_targets() {
        let aircraft = FeedAircraft {
            hex: Some("ABCDEF".into()),
            flight: Some("TEST123".into()),
            lat: Some(0.0),
            lon: Some(0.0),
            track: Some(180.0),
            gs: Some(250.0),
            seen: Some(35.0),
            t: Some("A320".into()),
            aircraft_type: None,
            reg: None,
            alt_baro: Some(Altitude::Integer(30000)),
        };
        assert!(to_target(&aircraft, 20).is_none());
    }
}
