//! Station discovery and simulator-authoritative airport weather/ATIS.
use crate::Shared;
use axum::{
    Json,
    extract::State as AxumState,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

/// Wording history is separate from flight state, so a new flight keeps phrase rotation.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PhraseHistory {
    pub choices: std::collections::BTreeMap<String, usize>,
    pub templates: std::collections::BTreeMap<String, String>,
    pub last: std::collections::BTreeMap<String, String>,
}
impl PhraseHistory {
    pub fn load(dir: &std::path::Path) -> Self {
        std::fs::read(dir.join("phrase-history.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, dir: &std::path::Path) {
        if let Ok(bytes) = serde_json::to_vec(self) {
            let temporary = dir.join("phrase-history.json.tmp");
            if let Err(error) = std::fs::write(&temporary, bytes)
                .and_then(|()| std::fs::rename(temporary, dir.join("phrase-history.json")))
            {
                eprintln!("Could not save phrase history: {error}");
            }
        }
    }
}

fn error(text: &str) -> Response {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(json!({"error":text})),
    )
        .into_response()
}
/// Requests must work even when the pilot has never opened the station browser.
pub async fn ensure_index(shared: &Shared) -> Result<(), String> {
    let scan = shared.read().await.station_index_lock.clone();
    let _scan = scan.lock().await;
    let (root, old) = {
        let s = shared.read().await;
        (s.settings.simulator_root.clone(), s.station_root.clone())
    };
    if root.is_empty() {
        return Err("Set the X-Plane folder".into());
    }
    if root != old {
        let copy = root.clone();
        let workers = shared.read().await.workers.clone();
        let result = workers
            .run(move || {
                let stations = openatc_core::stations::load_index(&copy)?;
                let airports = openatc_core::stations::map_airports(&copy)?;
                Ok::<_, String>((stations, airports))
            })
            .await;
        let index = match result {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err("Station indexing failed".into()),
        };
        let mut s = shared.write().await;
        if s.settings.simulator_root == root {
            s.map_root = root.clone();
            s.station_root = root;
            s.stations = index.0;
            s.map_airports = index.1;
            s.sim_weather.clear();
            s.atis_information.clear();
        }
    }
    Ok(())
}

/// Index scenery outside the state lock and return shared reception results.
pub async fn nearby(AxumState(shared): AxumState<Shared>, Json(_): Json<Value>) -> Response {
    if shared.read().await.settings.simulator_root.is_empty() {
        return Json(
            json!({"stations":[], "occupiedAirport":null, "notice":"Set the X-Plane folder"}),
        )
        .into_response();
    }
    if let Err(message) = ensure_index(&shared).await {
        return error(&message);
    }
    let s = shared.read().await;
    let airport = openatc_core::stations::occupied_airport(&s.map_airports, &s.session.telemetry)
        .map(|a| a.id.as_str());
    Json(json!({"stations":openatc_core::stations::nearby(&s.stations,&s.session.telemetry),
        "occupiedAirport": airport,
        "notice": if s.stations.is_empty() { "No radio stations found in the installed scenery" } else { "" }}))
        .into_response()
}
/// Store actual simulator samples, without accepting invented/stale observations.
pub async fn weather(AxumState(shared): AxumState<Shared>, Json(body): Json<Value>) -> Response {
    let Some(airport) = body.get("airport").and_then(Value::as_str) else {
        return error("Airport required");
    };
    if ![
        "temperatureC",
        "dewpointC",
        "pressureHpa",
        "windDegrees",
        "windKnots",
        "visibilityMeters",
    ]
    .iter()
    .all(|k| {
        body.get(k)
            .and_then(Value::as_f64)
            .is_some_and(f64::is_finite)
    }) {
        return error("Incomplete simulator weather");
    }
    if !(800.0..1100.0).contains(&body["pressureHpa"].as_f64().unwrap_or(0.)) {
        return error("Invalid simulator pressure");
    }
    let mut s = shared.write().await;
    if s.station_root != s.settings.simulator_root {
        return error("Station catalogue not ready");
    }
    let Some(station) = s.stations.iter().find(|v| v.airport == airport) else {
        return error("Unknown weather airport");
    };
    if !["latitude", "longitude", "sampleAltitudeFeet"]
        .iter()
        .all(|k| {
            body.get(k)
                .and_then(Value::as_f64)
                .is_some_and(f64::is_finite)
        })
        || (body["latitude"].as_f64().unwrap_or(999.) - station.latitude).abs() > 0.001
        || (body["longitude"].as_f64().unwrap_or(999.) - station.longitude).abs() > 0.001
        || (body["sampleAltitudeFeet"].as_f64().unwrap_or(-9999.) - station.elevation_feet).abs()
            > 50.
        || !(0.0..=400.0).contains(&body["windKnots"].as_f64().unwrap_or(-1.))
        || !(0.0..=360.0).contains(&body["windDegrees"].as_f64().unwrap_or(-1.))
        || !(0.0..=1_000_000.0).contains(&body["visibilityMeters"].as_f64().unwrap_or(-1.))
    {
        return error("Weather must be a valid airport surface sample");
    }
    let signature = format!(
        "{:.0}/{:.0}/{:.0}/{:.0}/{:.0}/{:.0}/{}",
        body["windDegrees"].as_f64().unwrap_or(0.),
        body["windKnots"].as_f64().unwrap_or(0.),
        body["pressureHpa"].as_f64().unwrap_or(0.),
        body["visibilityMeters"].as_f64().unwrap_or(0.) / 100.,
        body["temperatureC"].as_f64().unwrap_or(0.),
        body["dewpointC"].as_f64().unwrap_or(0.),
        body["clouds"].as_str().unwrap_or("")
    );
    let information = s
        .atis_information
        .entry(airport.to_owned())
        .or_insert((signature.clone(), 0));
    if information.0 != signature {
        information.0 = signature;
        information.1 = (information.1 + 1) % 26;
    }

    s.sim_weather
        .insert(airport.to_owned(), (body, std::time::Instant::now()));
    Json(json!({"accepted":true})).into_response()
}
/// Deterministic simulated ATIS. Never use online METAR or aircraft-local weather.
pub async fn atis(AxumState(shared): AxumState<Shared>, Json(_): Json<Value>) -> Response {
    let s = shared.read().await;
    if s.station_root != s.settings.simulator_root {
        return error("Station catalogue not ready");
    }
    if !s.session.telemetry.radio_power {
        return error("Radio power off");
    }
    let nearby = openatc_core::stations::nearby(&s.stations, &s.session.telemetry);
    let Some(station) = openatc_core::stations::tuned(&nearby, s.session.telemetry.com1_khz) else {
        return error("No station receiving");
    };
    if station.service != "ATIS" {
        return error("Not tuned to ATIS");
    }
    let Some((weather, when)) = s
        .sim_weather
        .get(&station.airport)
        .filter(|(_, t)| t.elapsed().as_secs() < 45)
    else {
        return error("Simulator airport weather unavailable");
    };
    let number = |key: &str| weather[key].as_f64().unwrap_or(0.);
    let clouds = weather.get("clouds").and_then(Value::as_str).map_or_else(
        || openatc_core::dialogue::say("weather_cloud_report_unavailable", &[]),
        str::to_owned,
    );
    let letter = s.atis_information.get(&station.airport).map_or(0, |v| v.1);
    let code = openatc_core::dialogue::say(&format!("atis_information_{letter:02}"), &[]);
    let runway = if station.airport == s.session.plan.departure {
        &s.session.plan.runway
    } else if station.airport == s.session.plan.destination {
        &s.session.plan.arrival_runway
    } else {
        ""
    };
    let runway_text = if runway.is_empty() {
        openatc_core::dialogue::say("runway_information_unavailable", &[])
    } else {
        openatc_core::dialogue::say(
            "planned_runway_confirm_with_controller",
            &[("runway", (runway).to_string())],
        )
    };
    let text = openatc_core::dialogue::say(
        "automatic_terminal_information_information_simulator_observation_wind",
        &[
            ("airport", (station.airport_name).clone()),
            ("code", code.clone()),
            ("wind_direction", format!("{:.0}", number("windDegrees"))),
            ("wind_speed", format!("{:.0}", number("windKnots"))),
            ("visibility", format!("{:.0}", number("visibilityMeters"))),
            ("clouds", (clouds).clone()),
            ("temperature", format!("{:.0}", number("temperatureC"))),
            ("dewpoint", format!("{:.0}", number("dewpointC"))),
            ("pressure", format!("{:.0}", number("pressureHpa"))),
            ("runway_text", (runway_text).clone()),
        ],
    );
    let _ = when;
    Json(json!({"text":text,"airport":station.airport,"information":code,"source":"simulator","frequency":station.khz})).into_response()
}

/// Render the appropriate speech event after the structured controller decision.
pub fn apply_wording(
    state: &mut crate::EngineState,
    intent: &str,
    station: &openatc_core::stations::Station,
    result: &mut openatc_core::apply::RequestOutcome,
) {
    if !result.accepted {
        return;
    }
    let id = match intent {
        "clearance" => "delivery.ifr_route",
        "start" => "ground.startup_only",
        "pushback" => "ground.pushback_basic",
        "start_pushback" => "ground.start_and_pushback",
        _ => return,
    };
    let Some(entry) = state.speech.get(id) else {
        return;
    };
    let service = if intent == "clearance" {
        "Clearance"
    } else {
        "Ground"
    };
    let phase = format!("{:?}", state.session.phase).to_lowercase();
    if entry.role != "atc"
        || (!entry.service.is_empty() && !entry.service.iter().any(|v| v == service))
        || !entry.phase.contains(&phase)
    {
        return;
    }
    let Some(clearance) = state.session.clearance.as_ref() else {
        return;
    };
    let slots = openatc_core::catalog::Slots {
        callsign: Some(state.session.plan.callsign.clone()),
        dest: Some(
            state
                .stations
                .iter()
                .find(|s| s.airport == state.session.plan.destination && !s.airport_name.is_empty())
                .map_or_else(
                    || {
                        state
                            .map_airports
                            .iter()
                            .find(|a| a.id == state.session.plan.destination)
                            .map_or_else(
                                || state.session.plan.destination.clone(),
                                |a| a.name.clone(),
                            )
                    },
                    |s| s.airport_name.clone(),
                ),
        ),
        runway: Some(clearance.runway.clone()),
        altitude_feet: Some(clearance.altitude_feet),
        squawk: Some(clearance.squawk.clone()),
        via: Some(
            if state.session.plan.sid.is_empty()
                || clearance.route.split_whitespace().next()
                    == Some(state.session.plan.sid.as_str())
            {
                clearance.route.clone()
            } else {
                format!("{} then {}", state.session.plan.sid, clearance.route)
            },
        ),
        ..Default::default()
    };
    let next = state.phrase_history.choices.get(id).copied().unwrap_or(0);
    let previous = state.phrase_history.templates.get(id);
    let Some((pick, line)) = (0..entry.say.len())
        .map(|offset| {
            let pick = (next + offset) % entry.say.len();
            (pick, &entry.say[pick])
        })
        .find(|(_, line)| previous != Some(*line))
        .or_else(|| entry.say.first().map(|line| (0, line)))
    else {
        return;
    };
    match openatc_core::speech::render_checked(line, &slots) {
        Ok(text) => {
            state.phrase_history.choices.insert(id.to_owned(), pick + 1);
            state
                .phrase_history
                .templates
                .insert(id.to_owned(), line.clone());
            eprintln!(
                "ATC wording: template={id} alternative={} station={} text={text}",
                pick + 1,
                station.name
            );
            result.message.clone_from(&text);
            if let Some(last) = state
                .session
                .transcript
                .last_mut()
                .filter(|e| e.speaker == "ATC")
            {
                last.text = text;
                last.position.clone_from(&station.name);
            }
        }
        Err(error) => eprintln!("ATC wording template {id} could not be rendered: {error}"),
    }
}
/// Conservative paraphrase check: every non-filler token stays in the same order.
/// Numbers, instructions, negation, callsign and route cannot change or be added.
pub fn safe_variety(original: &str, candidate: &str) -> bool {
    fn tokens(text: &str) -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric() && c != '-')
            .filter(|t| !t.is_empty())
            .map(str::to_ascii_lowercase)
            .collect()
    }
    if candidate.trim().is_empty() || candidate.len() > original.len() + 80 {
        return false;
    }
    let original = tokens(original);
    let mut expected = original.iter().peekable();
    for token in tokens(candidate) {
        if expected.peek().is_some_and(|next| **next == token) {
            expected.next();
        } else if !["please", "the", "your", "is"].contains(&token.as_str()) {
            return false;
        }
    }
    expected.next().is_none()
}

/// The debug map reads the same controller catalogue used for handoffs.
pub async fn debug_map(AxumState(shared): AxumState<Shared>, Json(body): Json<Value>) -> Response {
    let (root, indexed_root, latitude, longitude) = {
        let s = shared.read().await;
        if !s.settings.dev_mode {
            return error("Map is available in developer mode.");
        }
        (
            s.settings.simulator_root.clone(),
            s.map_root.clone(),
            s.session.telemetry.latitude,
            s.session.telemetry.longitude,
        )
    };
    if root.is_empty() {
        return error("Set the X-Plane folder to load the map.");
    }
    if root != indexed_root {
        let path = root.clone();
        let workers = shared.read().await.workers.clone();
        let airports = match workers
            .run(move || openatc_core::stations::map_airports(&path))
            .await
        {
            Ok(Ok(v)) => v,
            _ => return error("Could not index airport markers."),
        };
        let mut s = shared.write().await;
        if s.settings.simulator_root != root {
            return error("Simulator folder changed; refresh the map.");
        }
        s.map_airports = airports;
        s.map_root = root.clone();
    }
    let s = shared.read().await;
    let lat = body
        .get("latitude")
        .and_then(Value::as_f64)
        .unwrap_or(latitude);
    let lon = body
        .get("longitude")
        .and_then(Value::as_f64)
        .unwrap_or(longitude);
    let range = body
        .get("rangeNm")
        .and_then(Value::as_f64)
        .unwrap_or(80.)
        .clamp(2., 1500.);
    if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90. || lon.abs() > 180. {
        return error("Invalid map position.");
    }
    let mut t = s.session.telemetry.clone();
    t.latitude = lat;
    t.longitude = lon;
    let distance = |a, b| openatc_core::ops::distance_nm(lat, lon, a, b);
    let stations: Vec<_> = s
        .stations
        .iter()
        .filter(|station| {
            distance(station.latitude, station.longitude) <= range * 1.5
                || station
                    .coverage
                    .iter()
                    .any(|volume| volume.radio_distance_nm(&t) <= range * 1.5)
        })
        .collect();
    let airports: Vec<_> = s
        .map_airports
        .iter()
        .filter(|a| distance(a.latitude, a.longitude) <= range * 1.5)
        .collect();
    Json(json!({"stations":stations,"airports":airports,
        "notice":if s.station_root != root { "Waiting for the radio station index." } else { "Installed airport data and published ATC coverage. Green outlines contain the aircraft on the tuned frequency." }})).into_response()
}

#[cfg(test)]
mod tests {
    use super::safe_variety;
    #[test]
    fn phrase_history_survives_reload() {
        let dir = std::env::temp_dir().join(format!(
            "openatc-phrase-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut history = super::PhraseHistory::default();
        history.choices.insert("delivery.ifr_route".into(), 3);
        history.last.insert(
            "station|clearance".into(),
            "Recorded clearance wording".into(),
        );
        history.save(&dir);
        let restored = super::PhraseHistory::load(&dir);
        assert_eq!(restored.choices["delivery.ifr_route"], 3);
        assert_eq!(
            restored.last["station|clearance"],
            "Recorded clearance wording"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn wording_must_preserve_all_operational_tokens() {
        assert!(!safe_variety("THE, taxi via YOUR.", "Taxi via."));
        let text = "VH-BIL, taxi via Alpha. Hold short runway 27.";
        assert!(safe_variety(
            text,
            "VH-BIL, please taxi via Alpha. Hold short runway 27."
        ));
        for changed in [
            "VH-BIL, taxi via Alpha. Hold short runway 09.",
            "VH-BIL, taxi via Bravo. Hold short runway 27.",
            "VH-BIL, taxi via Alpha. Cross runway 27.",
            "VH-BIL, taxi via Alpha. Do not hold short runway 27.",
        ] {
            assert!(!safe_variety(text, changed));
        }
    }
}
