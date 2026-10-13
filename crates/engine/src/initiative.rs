//! Optional en-route offers. The pilot still requests and reads back any amendment.
use crate::{EngineState, Shared};
use axum::{
    Json,
    extract::State as AxumState,
    response::{IntoResponse, Response},
};
use openatc_core::{
    apply::add_transmission,
    dialogue::say,
    state::{AtcOffer, PhaseCode, Request, SpeechTag},
    stations,
};
use serde_json::{Value, json};
use std::time::Instant;

#[derive(Default)]
pub struct Monitor {
    last: Option<Instant>,
    quiet_seconds: f64,
    offered: Option<Instant>,
    previous_fix: String,
}

fn candidate(state: &EngineState, index: usize) -> bool {
    let fixes = &state.session.plan.fixes;
    let next = state.flow.monitor.next_fix.max(1);
    let Some(fix) = fixes
        .get(index)
        .filter(|_| index > next && next < fixes.len())
    else {
        return false;
    };
    if fixes[next..=index].iter().any(|f| {
        f.minimum_altitude_feet.is_some()
            || f.maximum_altitude_feet.is_some()
            || f.maximum_speed_knots.is_some()
    }) {
        return false;
    }
    let t = &state.session.telemetry;
    let own = [t.latitude, t.longitude];
    let target = [fix.latitude, fix.longitude];
    let direct = openatc_core::arrival::distance_nm(own, target);
    let Some(floor) = state.mora.corridor(own, target) else {
        return false;
    };
    if t.altitude_feet < f64::from(floor) || !(5.0..=100.0).contains(&direct) {
        return false;
    }
    let mut prior = own;
    let original: f64 = fixes[next..=index]
        .iter()
        .map(|f| {
            let at = [f.latitude, f.longitude];
            let distance = openatc_core::arrival::distance_nm(prior, at);
            prior = at;
            distance
        })
        .sum();
    let Some(arrival) = state.arrival_airport.as_ref() else {
        return false;
    };
    let remaining = direct
        + fixes[index..]
            .windows(2)
            .map(|w| {
                openatc_core::arrival::distance_nm(
                    [w[0].latitude, w[0].longitude],
                    [w[1].latitude, w[1].longitude],
                )
            })
            .sum::<f64>()
        + fixes.last().map_or(0.0, |f| {
            openatc_core::arrival::distance_nm(
                [f.latitude, f.longitude],
                [arrival.reference_latitude, arrival.reference_longitude],
            )
        });
    original - direct >= 2.0
        && t.altitude_feet
            <= arrival.elevation_feet
                + ((remaining - 10.0).max(0.0) * openatc_core::arrival::gradient(3.0)).max(2000.0)
}

pub fn tick(state: &mut EngineState) {
    let elapsed = state
        .initiative
        .last
        .replace(Instant::now())
        .map_or(0.0, |last| last.elapsed().as_secs_f64().min(2.0));
    if !state.settings.atc_shortcut_offers
        || state.session.emergency_declared
        || state.session.phase != PhaseCode::Cruise
    {
        state.session.atc_offer = None;
        state.initiative.quiet_seconds = 0.0;
        return;
    }
    if state
        .session
        .atc_offer
        .as_ref()
        .is_some_and(|o| o.frequency_khz != state.session.telemetry.com1_khz)
        || state
            .initiative
            .offered
            .is_some_and(|at| at.elapsed().as_secs() > 90)
    {
        state.session.atc_offer = None;
    }
    if !crate::traffic::ready(state)
        || state.session.telemetry.on_ground
        || state.session.telemetry.vertical_speed_fpm.abs() > 200.0
        || state
            .session
            .clearance
            .as_ref()
            .is_none_or(|c| !c.acknowledged)
        || state.flow.monitor.heading.is_some()
        || state.flow.monitor.deviation.is_some()
        || state.session.atc_offer.is_some()
    {
        return;
    }
    state.initiative.quiet_seconds += elapsed;
    if state.initiative.quiet_seconds < f64::from(state.settings.atc_offer_interval_minutes) * 60.0
    {
        return;
    }
    state.initiative.quiet_seconds = 0.0;
    let stations = stations::nearby(&state.stations, &state.session.telemetry);
    let Some(station) = stations::tuned(&stations, state.session.telemetry.com1_khz)
        .filter(|s| ["Center", "Departure", "Approach"].contains(&s.service.as_str()))
    else {
        return;
    };
    let next = state.flow.monitor.next_fix.max(1);
    let target = (next + 1..state.session.plan.fixes.len().min(next + 4)).find(|&i| {
        state.session.plan.fixes[i].identifier != state.initiative.previous_fix
            && candidate(state, i)
    });
    let Some(target) = target else {
        return;
    };
    let waypoint = state.session.plan.fixes[target].identifier.clone();
    state.initiative.previous_fix = waypoint.clone();
    state.initiative.offered = Some(Instant::now());
    let offer = AtcOffer {
        waypoint,
        sequence: state.session.next_sequence,
        frequency_khz: station.khz,
    };
    let voice = crate::flight::tag(state, station);
    let text = say(
        "atc_offer_direct",
        &[
            ("callsign", state.session.plan.callsign.clone()),
            ("waypoint", offer.waypoint.clone()),
        ],
    );
    add_transmission(&mut state.session, "ATC", &text, &voice);
    state.session.atc_offer = Some(offer);
}

pub async fn answer(AxumState(shared): AxumState<Shared>, Json(body): Json<Value>) -> Response {
    let mut s = shared.write().await;
    let Some(offer) = s
        .session
        .atc_offer
        .clone()
        .filter(|o| body["sequence"].as_u64() == Some(u64::from(o.sequence)))
    else {
        return crate::routes::bad_request("That controller offer is no longer available.");
    };
    let accept = body["accept"].as_bool();
    if accept.is_none()
        || !crate::traffic::ready(&s)
        || !s.settings.atc_shortcut_offers
        || s.session.telemetry.com1_khz != offer.frequency_khz
        || s.session.phase != PhaseCode::Cruise
        || s.initiative
            .offered
            .is_none_or(|t| t.elapsed().as_secs() > 90)
    {
        return crate::routes::bad_request(
            "Wait for a clear radio and a current offer from this controller.",
        );
    }
    let stations = stations::nearby(&s.stations, &s.session.telemetry);
    let Some(station) = stations::tuned(&stations, offer.frequency_khz) else {
        return crate::routes::bad_request("Controller unavailable.");
    };
    let call = s.session.plan.callsign.clone();
    if accept == Some(false) {
        s.session.atc_offer = None;
        let text = say("pilot_decline_shortcut", &[("callsign", call.clone())]);
        add_transmission(&mut s.session, &call, &text, &SpeechTag::default());
        let voice = crate::flight::tag(&mut s, station);
        add_transmission(
            &mut s.session,
            "ATC",
            &say("atc_shortcut_declined", &[("callsign", call)]),
            &voice,
        );
        return Json(json!({"accepted":true})).into_response();
    }
    let Some(index) = s
        .session
        .plan
        .fixes
        .iter()
        .position(|f| f.identifier == offer.waypoint)
    else {
        return crate::routes::bad_request("The route has changed.");
    };
    if !candidate(&s, index) {
        s.session.atc_offer = None;
        return crate::routes::bad_request("The shortcut no longer meets route or descent limits.");
    }
    let text = say(
        "pilot_accept_shortcut",
        &[
            ("station", station.name.clone()),
            ("callsign", call),
            ("waypoint", offer.waypoint.clone()),
        ],
    );
    s.session.atc_offer = None;
    drop(s);
    crate::routes::post_request(
        AxumState(shared),
        Json(json!(Request {
            intent: "direct".into(),
            waypoint: offer.waypoint,
            text,
            role: "atc".into(),
            ..Default::default()
        })),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn optional_shortcut_needs_terrain_restrictions_and_pilot_acceptance() {
        let mut s = crate::test_state();
        s.settings.atc_shortcut_offers = true;
        s.session.phase = PhaseCode::Cruise;
        s.session.telemetry = openatc_core::state::Telemetry {
            latitude: 0.1,
            longitude: 0.1,
            altitude_feet: 5000.0,
            on_ground: false,
            position_valid: true,
            com1_khz: 123100,
            ..Default::default()
        };
        s.session.clearance = Some(openatc_core::state::Clearance {
            acknowledged: true,
            ..Default::default()
        });
        s.session.plan.fixes = [
            (0.0, 0.0, "START"),
            (0.3, 0.1, "BEND"),
            (0.1, 0.7, "SHORT"),
            (0.1, 3.0, "END"),
        ]
        .into_iter()
        .map(
            |(latitude, longitude, identifier)| openatc_core::state::RouteFix {
                latitude,
                longitude,
                identifier: identifier.into(),
                ..Default::default()
            },
        )
        .collect();
        s.arrival_airport = Some(openatc_core::airport::Airport {
            reference_latitude: 0.1,
            reference_longitude: 3.1,
            ..Default::default()
        });
        s.stations = vec![stations::Station {
            service: "Center".into(),
            name: "Test Center".into(),
            khz: 123100,
            latitude: 0.1,
            longitude: 0.1,
            ..Default::default()
        }];
        assert!(!candidate(&s, 2));
        s.mora.0.insert((0, 0), 1000);
        assert!(candidate(&s, 2));
        s.session.plan.fixes[1].minimum_altitude_feet = Some(4000);
        assert!(!candidate(&s, 2));
        s.session.plan.fixes[1].minimum_altitude_feet = None;
        let prior = s.session.clearance.clone();
        s.initiative.quiet_seconds = 600.0;
        tick(&mut s);
        let offer = s.session.atc_offer.clone().expect("offer");
        assert_eq!(s.session.clearance, prior);
        assert!(s.session.transcript.last().unwrap().pilot_reply.is_empty());
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(s));
        assert!(
            answer(
                AxumState(shared.clone()),
                Json(json!({"sequence":offer.sequence,"accept":false}))
            )
            .await
            .status()
            .is_success()
        );
        assert!(shared.read().await.session.atc_offer.is_none());
        assert!(
            !answer(
                AxumState(shared.clone()),
                Json(json!({"sequence":offer.sequence,"accept":true}))
            )
            .await
            .status()
            .is_success()
        );
    }
}
