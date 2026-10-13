//! Surface route monitoring, runway crossing continuation and holding-point handoffs.
use super::Shared;
use super::routes::bad_request;
use axum::{
    Json,
    extract::State as AxumState,
    response::{IntoResponse, Response},
};
use openatc_core::{
    apply::add_transmission,
    ops::update_flight_phase,
    state::{Request, SurfaceStage, Telemetry},
};
use serde_json::json;

pub async fn post_telemetry(
    AxumState(state): AxumState<Shared>,
    Json(telemetry): Json<Telemetry>,
) -> Response {
    let shared = state;
    let (settings, pilot_report, automatic_intent, generation, route_sequence, channel, before) = {
        let mut state = shared.write().await;
        if let Err(error) = update_flight_phase(&mut state.session, &telemetry) {
            return bad_request(&error);
        }
        if state
            .pending_copilot_reply
            .as_ref()
            .is_some_and(|p| p.frequency != telemetry.com1_khz)
        {
            state.pending_copilot_reply = None;
        }
        state.session.demo = false;
        super::cabin::tick(&mut state);
        super::missions::clock(&mut state);
        super::traffic::observe(&mut state);
        if super::flight::urgent(&mut state) || super::traffic::advisories::urgent(&mut state) {
            return Json(json!({"accepted":true})).into_response();
        }
        let radio_busy = telemetry.radio_busy
            || state.pending_copilot_reply.is_some()
            || telemetry.radio_sequence_seen.is_some_and(|seen| {
                state.session.transcript.iter().any(|entry| {
                    entry.sequence > seen && entry.speaker == "ATC" && !entry.background
                })
            });
        if super::discipline::tick(&mut state, !radio_busy) || radio_busy {
            return Json(json!({"accepted":true})).into_response();
        }
        let sequence = state.session.next_sequence;
        super::flight::tick(&mut state);
        if state.session.next_sequence != sequence {
            return Json(json!({"accepted":true})).into_response();
        }
        super::cabin::follow_up(&mut state);
        if state.session.next_sequence != sequence {
            return Json(json!({"accepted":true})).into_response();
        }
        super::parking::tick(&mut state);
        if state.session.next_sequence != sequence || state.session.taxi_clearance.parking_hold {
            return Json(json!({"accepted":true})).into_response();
        }
        super::traffic::tick(&mut state);
        if state.session.next_sequence != sequence || state.session.taxi_clearance.traffic_hold {
            return Json(json!({"accepted":true})).into_response();
        }
        super::initiative::tick(&mut state);
        if state.session.next_sequence != sequence {
            return Json(json!({"accepted":true})).into_response();
        }
        if state
            .session
            .clearance
            .as_ref()
            .is_none_or(|c| c.acknowledged)
            && !state.session.taxi_clearance.pending_readback
        {
            super::missions::tick(&mut state);
            if state.session.next_sequence != sequence {
                return Json(json!({"accepted":true})).into_response();
            }
        }
        if state.session.taxi_clearance.approved
            && !state.session.taxi_clearance.crossing_runway.is_empty()
            && state.airport.as_ref().is_some_and(|a| {
                openatc_core::airport::crossing_vacated(
                    a,
                    &state.session.taxi_clearance,
                    &telemetry,
                )
            })
        {
            state.session.taxi_clearance.guidance_complete = true;
        }
        if state.session.taxi_clearance.approved
            && state.airport.as_ref().is_some_and(|a| {
                openatc_core::airport::taxi_hold_reached(
                    a,
                    &state.session.taxi_clearance,
                    &telemetry,
                )
            })
        {
            state.session.taxi_clearance.guidance_complete = true;
        }
        let crossing = state.session.taxi_clearance.clone();
        if crossing.stage() == SurfaceStage::CrossingComplete
            && !state.settings.copilot_replies
            && telemetry.on_ground
            && !telemetry.paused
            && telemetry.radio_power
            && let Some(airport) = state.airport.as_ref()
        {
            let nearby = openatc_core::stations::nearby(&state.stations, &telemetry);
            if let Some(station) = openatc_core::stations::tuned(&nearby, telemetry.com1_khz)
                && station.airport == crossing.airport
                && openatc_core::stations::station_serves(station, "cross_runway")
                && let Ok(mut route) = openatc_core::airport::calculate_taxi_route(
                    airport,
                    &telemetry,
                    &crossing.destination,
                    crossing.to_parking,
                    openatc_core::parking::aircraft_size(&state.session).unwrap_or('F'),
                )
            {
                route.pending_readback = true;
                route.sequence = state.session.next_sequence;
                let controller_key = super::flight::station_key(station);
                let station_name = station.name.clone();
                let settings = state.settings.clone();
                let mut tag = super::routes::assign_airspace_controller(
                    &mut state,
                    &settings,
                    &controller_key,
                    "standard",
                );
                tag.position = station_name;
                let text = route.instructions.clone();
                state.session.taxi_clearance = route;
                add_transmission(&mut state.session, "ATC", &text, &tag);
                return Json(json!({"accepted":true})).into_response();
            }
        }
        let route = &state.session.taxi_clearance;
        let eligible = matches!(
            state.session.phase,
            openatc_core::state::PhaseCode::Taxi | openatc_core::state::PhaseCode::TaxiIn
        ) && route.stage() == SurfaceStage::AtHold
            && (!route.to_parking || !route.hold_short_runway.is_empty())
            && telemetry.on_ground
            && !telemetry.paused
            && telemetry.radio_power;
        if !eligible {
            return Json(json!({"accepted":true})).into_response();
        }
        if state
            .airport
            .as_ref()
            .is_none_or(|a| !openatc_core::airport::taxi_hold_reached(a, route, &telemetry))
        {
            return Json(json!({"accepted":true})).into_response();
        }
        let nearby = openatc_core::stations::nearby(&state.stations, &telemetry);
        let Some(station) = openatc_core::stations::tuned(&nearby, telemetry.com1_khz) else {
            return Json(json!({"accepted":true})).into_response();
        };
        if station.airport != route.airport
            || station.service == "ATIS"
            || station.service == "Unicom"
        {
            return Json(json!({"accepted":true})).into_response();
        }
        let retry = route.waiting_for_traffic
            && (route.hold_short_runway != state.session.plan.runway
                || state.session.departure_wake_seconds <= 0.0)
            && state.airport.as_ref().is_some_and(|a| {
                openatc_core::holding::blocked(
                    a,
                    &state.session.taxi_clearance.hold_short_runway,
                    &telemetry,
                )
                .is_none()
            });
        if route.holding_channel == telemetry.com1_khz && !retry {
            return Json(json!({"accepted":true})).into_response();
        }
        let needs_further = route.to_parking
            || route.backtrack_required
            || (!route.hold_short_runway.is_empty()
                && route.hold_short_runway != state.session.plan.runway);
        let backtrack = route.backtrack_required;
        let tower = openatc_core::stations::station_serves(station, "ready");
        if !tower && !needs_further {
            // A frequency transfer needs no new runway clearance or scenery reload.
            if let Some(next) = nearby
                .iter()
                .find(|candidate| {
                    candidate.airport == route.airport
                        && candidate.receivable
                        && openatc_core::stations::station_serves(candidate, "ready")
                })
                .cloned()
            {
                let current = station.clone();
                let tag = super::flight::tag(&mut state, &current);
                let frequency = format!("{:.3}", f64::from(next.khz) / 1000.0);
                let slots = [
                    ("callsign", state.session.plan.callsign.clone()),
                    ("station", next.name),
                    ("frequency", frequency),
                ];
                let text = openatc_core::dialogue::say("flight_handoff", &slots);
                state.session.recommended_frequency_khz = next.khz;
                state.session.frequency_sequence = state.session.next_sequence;
                add_transmission(&mut state.session, "ATC", &text, &tag);
                if let Some(entry) = state.session.transcript.last_mut() {
                    entry.pilot_reply =
                        openatc_core::dialogue::say("pilot_frequency_readback", &slots);
                }
                state.session.taxi_clearance.holding_channel = telemetry.com1_khz;
            }
            return Json(json!({"accepted":true})).into_response();
        }
        let controller_key = super::flight::station_key(station);
        let station_name = station.name.clone();
        let settings = state.settings.clone();
        let mut tag = super::routes::assign_airspace_controller(
            &mut state,
            &settings,
            &controller_key,
            "standard",
        );
        tag.position = station_name;
        state.session.taxi_clearance.holding_channel = telemetry.com1_khz;
        if tower && !needs_further && !retry {
            return Json(json!({"accepted":true})).into_response();
        }
        if backtrack && tower {
            let text = openatc_core::dialogue::say(
                "holding_report_ready",
                &[(
                    "runway",
                    state.session.taxi_clearance.hold_short_runway.clone(),
                )],
            );
            add_transmission(&mut state.session, "ATC", &text, &tag);
            return Json(json!({"accepted":true})).into_response();
        }
        (
            state.settings.clone(),
            openatc_core::dialogue::say(
                "pilot_holding_ready",
                &[("callsign", state.session.plan.callsign.clone())],
            ),
            if needs_further && !state.session.taxi_clearance.backtrack_required {
                "cross_runway"
            } else {
                "ready"
            },
            state.session_generation,
            state.session.taxi_clearance.sequence,
            telemetry.com1_khz,
            state.session.next_sequence,
        )
    };
    // Normal station-role and runway checks still govern this automatic request.
    let response = super::routes::atc_request(
        &shared,
        &settings,
        Request {
            controller_initiated: true,
            intent: automatic_intent.into(),
            text: if automatic_intent == "cross_runway" {
                String::new()
            } else {
                pilot_report
            },
            ..Default::default()
        },
    )
    .await;
    let mut state = shared.write().await;
    if state.session_generation == generation
        && state.session.taxi_clearance.sequence == route_sequence
        && state.session.taxi_clearance.holding_channel == channel
        && (response.status().is_client_error()
            || response.status().is_server_error()
            || state.session.next_sequence == before)
    {
        // A failed request must not consume the holding-point notification.
        state.session.taxi_clearance.holding_channel = 0;
    }
    Json(json!({"accepted":true})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use openatc_core::{
        airport::Airport,
        state::{PhaseCode, Point, TaxiClearance},
        stations::Station,
    };

    fn fixture() -> crate::EngineState {
        let mut state = crate::test_state();
        state.session.phase = PhaseCode::Taxi;
        state.session.plan.departure = "TEST".into();
        state.session.plan.runway = "09".into();
        state.session.plan.callsign = "C-GTLT".into();
        state.airport = Some(Airport {
            icao: "TEST".into(),
            ..Default::default()
        });
        state.session.taxi_clearance = TaxiClearance {
            airport: "TEST".into(),
            approved: true,
            hold_short_runway: "09".into(),
            points: vec![Point::default()],
            sequence: 42,
            ..Default::default()
        };
        state.stations = vec![
            Station {
                airport: "TEST".into(),
                name: "Test Ground".into(),
                service: "Ground".into(),
                khz: 121400,
                ..Default::default()
            },
            Station {
                airport: "TEST".into(),
                name: "Test Tower".into(),
                service: "Tower".into(),
                khz: 118700,
                ..Default::default()
            },
        ];
        state
    }

    fn telemetry() -> Telemetry {
        Telemetry {
            position_valid: true,
            on_ground: true,
            radio_power: true,
            com1_khz: 121400,
            ground_speed_knots: 3.0,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn handoff_waits_for_readback_and_radio_then_transfers_once_while_moving() {
        let mut state = fixture();
        state.session.taxi_clearance.pending_readback = true;
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(state));
        let t = telemetry();
        post_telemetry(AxumState(shared.clone()), Json(t.clone())).await;
        assert!(shared.read().await.session.transcript.is_empty());
        shared.write().await.session.taxi_clearance.pending_readback = false;
        post_telemetry(
            AxumState(shared.clone()),
            Json(Telemetry {
                radio_busy: true,
                ..t.clone()
            }),
        )
        .await;
        assert_eq!(
            shared.read().await.session.taxi_clearance.holding_channel,
            0
        );
        post_telemetry(AxumState(shared.clone()), Json(t.clone())).await;
        {
            let state = shared.read().await;
            assert_eq!(state.session.recommended_frequency_khz, 118700);
            assert_eq!(state.session.transcript.len(), 1);
            let entry = &state.session.transcript[0];
            assert!(entry.text.contains("Test Tower") && entry.text.contains("118.700"));
            assert!(!entry.pilot_reply.is_empty());
            assert_eq!(state.session.frequency_sequence, entry.sequence);
        }
        post_telemetry(AxumState(shared.clone()), Json(t.clone())).await;
        post_telemetry(
            AxumState(shared.clone()),
            Json(Telemetry {
                com1_khz: 118700,
                ..t
            }),
        )
        .await;
        assert_eq!(
            shared.read().await.session.transcript.len(),
            1,
            "Tower must wait for the pilot's ready call"
        );
    }

    #[tokio::test]
    async fn missing_tower_can_be_discovered_on_the_next_update() {
        let mut state = fixture();
        let tower = state.stations.pop().unwrap();
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(state));
        post_telemetry(AxumState(shared.clone()), Json(telemetry())).await;
        assert_eq!(
            shared.read().await.session.taxi_clearance.holding_channel,
            0
        );
        shared.write().await.stations.push(tower);
        post_telemetry(AxumState(shared.clone()), Json(telemetry())).await;
        assert_eq!(
            shared.read().await.session.recommended_frequency_khz,
            118700
        );
    }

    #[tokio::test]
    #[ignore = "requires installed X-Plane scenery"]
    async fn klax_reported_hold_transfers_from_ground_to_published_tower() {
        let root = std::env::var("OPENATC_SIM_ROOT").unwrap();
        let airport = openatc_core::airport::load_airport_from_simulator(
            &root,
            "KLAX",
            &|p| std::path::Path::new(p).exists(),
            &|p| std::fs::read_to_string(p).map_err(|e| e.to_string()),
        )
        .unwrap();
        let t = Telemetry {
            latitude: 33.938318,
            longitude: -118.383084,
            ground_speed_knots: 0.19,
            ..telemetry()
        };
        let mut state = fixture();
        state.session.plan.departure = "KLAX".into();
        state.session.plan.runway = "25L".into();
        state.session.taxi_clearance = openatc_core::airport::calculate_taxi_route(
            &airport,
            &Telemetry {
                latitude: 33.9389,
                ..t.clone()
            },
            "25L",
            false,
            'C',
        )
        .unwrap();
        assert_eq!(state.session.taxi_clearance.hold_short_runway, "25L");
        state.session.taxi_clearance.approved = true;
        state.stations = openatc_core::stations::from_airport(&airport);
        let tower = state
            .stations
            .iter()
            .find(|s| s.service == "Tower")
            .unwrap()
            .clone();
        let expected = tower.khz;
        state.airport = Some(airport);
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(state));
        post_telemetry(AxumState(shared.clone()), Json(t)).await;
        let state = shared.read().await;
        assert_eq!(state.session.recommended_frequency_khz, expected);
        assert!(
            state
                .session
                .transcript
                .last()
                .unwrap()
                .text
                .contains(&tower.name)
        );
    }
}
