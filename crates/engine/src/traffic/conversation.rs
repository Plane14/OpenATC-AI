//! Optional pilot conversation; operational radio exchanges always take priority.
#[allow(unused_imports, clippy::wildcard_imports)]
use super::*;
use super::{
    BTreeMap, Duration, EngineState, Instant, IntoResponse, Json, PhaseCode, Pilot, Response,
    Shared, SpeechTag, Value, add_transmission, json, ready, say, transmit,
};

fn policy(state: &mut EngineState) {
    if state.traffic.policy.is_none() {
        state.traffic.policy = Some(super::policy::Chat::load(&state.config_dir));
    }
}

fn quiet(state: &EngineState) -> bool {
    let Some(policy) = &state.traffic.policy else {
        return false;
    };
    let phase = match state.session.phase {
        PhaseCode::Parked => "parked",
        PhaseCode::Cruise => "cruise",
        PhaseCode::TaxiIn => "taxi_in",
        _ => "critical",
    };
    ready(state)
        && state
            .session
            .clearance
            .as_ref()
            .is_none_or(|clearance| clearance.acknowledged)
        && !state.session.taxi_clearance.pending_readback
        && state.session.frequency_sequence == 0
        && !state.session.emergency_declared
        && state.session.telemetry.position_valid
        && state.session.taxi_clearance.crossing_runway.is_empty()
        && !state.session.taxi_clearance.traffic_hold
        && !state.session.taxi_clearance.parking_hold
        && policy.allowed_phases.iter().any(|allowed| allowed == phase)
        && !(state.session.telemetry.on_ground
            && state.session.telemetry.ground_speed_knots > policy.stationary_speed_knots)
}

fn available(state: &EngineState, id: u32) -> Option<&Pilot> {
    let policy = state.traffic.policy.as_ref()?;
    state.traffic.pilots.get(&id).filter(|p| {
        p.seen.elapsed() < Duration::from_secs(policy.observation_age_seconds)
            && !p.target.callsign.is_empty()
            && p.target.callsign != state.session.plan.callsign
            && p.target.com1_khz.is_some()
            && p.target.com1_khz == state.session.control_frequency()
            && openatc_core::ops::distance_nm(
                state.session.telemetry.latitude,
                state.session.telemetry.longitude,
                p.target.latitude,
                p.target.longitude,
            ) <= policy.range_nm
            && (p.target.on_ground
                && p.target
                    .ground_speed_knots
                    .is_some_and(|speed| speed <= policy.stationary_speed_knots)
                || !p.target.on_ground
                    && p.target.ground_speed_knots.is_some_and(|speed| speed > 0.0))
            && p.landing.as_ref().is_none_or(|landing| landing.vacated)
            && (p.target.on_ground
                || state
                    .airport
                    .iter()
                    .chain(state.arrival_airport.iter())
                    .all(|airport| {
                        openatc_core::ops::distance_nm(
                            p.target.latitude,
                            p.target.longitude,
                            airport.reference_latitude,
                            airport.reference_longitude,
                        ) > policy.critical_airport_radius_nm
                            || p.target.altitude_feet
                                >= airport.elevation_feet + policy.critical_height_feet
                    }))
    })
}

fn fallback(message: &str, humour: bool, facts: &BTreeMap<String, Value>) -> String {
    openatc_core::dialogue::matched_response("traffic_chat_reply_", message, humour, facts)
        .map(|key| say(&key, &[]))
        .unwrap_or_default()
}

fn safe_reply(reply: &str, maximum: usize) -> bool {
    let lower = reply.to_lowercase();
    !reply.is_empty()
        && reply.chars().count() <= maximum
        && !reply.contains(['{', '}', '<', '>'])
        && ![
            "cleared",
            "climb to",
            "descend to",
            "squawk",
            "contact tower",
            "taxi via",
            "ignore atc",
        ]
        .iter()
        .any(|phrase| lower.contains(phrase))
}

pub(super) async fn request(shared: Shared, body: Value) -> Response {
    let id = body["id"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);
    let message = body["message"].as_str().unwrap_or("").trim().to_owned();
    let (
        generation,
        frequency,
        callsign,
        voice,
        settings,
        client,
        context,
        baseline,
        configuration,
    ) = {
        let mut state = shared.write().await;
        policy(&mut state);
        let configuration = state.traffic.policy.as_ref().unwrap().clone();
        if message.is_empty() || message.len() > configuration.maximum_message_bytes {
            return crate::routes::bad_request(format!(
                "Enter a brief message of up to {} bytes.",
                configuration.maximum_message_bytes
            ));
        }
        if !state.settings.talk_to_traffic || !state.settings.informal_pilot_chat {
            return crate::routes::bad_request(openatc_core::dialogue::say(
                "notice_enable_pilot_conversations_and_informal_chat_first_8125fcb7",
                &[],
            ));
        }
        if !quiet(&state) || state.traffic.chat_pending {
            return crate::routes::bad_request(openatc_core::dialogue::say(
                "notice_wait_for_a_quiet_period_outside_critical_flight_phases_c286eca0",
                &[],
            ));
        }
        let Some(pilot) = available(&state, id) else {
            return crate::routes::bad_request(openatc_core::dialogue::say(
                "notice_that_pilot_is_unavailable_or_busy_flying_a_critical_phas_0180cb09",
                &[],
            ));
        };
        let landing = pilot.landing.as_ref().map(|landing| json!({"airport":landing.airport,"runway":landing.runway,"weather":landing.weather,"vacated":landing.vacated}));
        let context = json!({"pilot":pilot.target.callsign,"aircraft":pilot.target.aircraft,"onGround":pilot.target.on_ground,"recentLanding":landing,"history":crate::llm::history(pilot.conversation.iter().map(|(user, reply)| (user.as_str(), reply.as_str())), configuration.history_exchanges, configuration.maximum_message_bytes, configuration.maximum_reply_characters)});
        let callsign = pilot.target.callsign.clone();
        let voice = pilot.voice.clone();
        let phase = match state.session.phase {
            PhaseCode::Parked => "parked",
            PhaseCode::Cruise => "cruise",
            PhaseCode::TaxiIn => "taxi_in",
            _ => "critical",
        };
        let facts = BTreeMap::from([
            ("on_ground".into(), json!(pilot.target.on_ground)),
            ("phase".into(), json!(phase)),
            ("landing_known".into(), json!(pilot.landing.is_some())),
        ]);
        let baseline = fallback(&message, state.settings.pilot_chat_humour, &facts);
        state.traffic.chat_pending = true;
        (
            state.session_generation,
            state.session.control_frequency().unwrap_or(0),
            callsign,
            voice,
            state.settings.clone(),
            state.client.clone(),
            context,
            baseline,
            configuration,
        )
    };
    let reply = if settings.ai_enabled && !settings.ai_model.is_empty() {
        let prompt = {
            let state = shared.read().await;
            let instructions = state
                .prompts
                .guidance
                .get("pilot_context")
                .replace("{{facts}}", &context.to_string())
                .replace("{{humour}}", &settings.pilot_chat_humour.to_string());
            let examples = state
                .speech
                .select(
                    "pilot",
                    &["actor_conversation"],
                    &format!("{:?}", state.session.phase).to_lowercase(),
                    "icao",
                    4,
                )
                .into_iter()
                .flat_map(|entry| entry.say.iter().take(2).cloned())
                .collect::<Vec<_>>();
            crate::llm::prompt(
                &configuration.personality,
                state.prompts.guidance.get("examples"),
                &examples,
                &instructions,
            )
        };
        match tokio::time::timeout(
            Duration::from_secs(configuration.model_timeout_seconds),
            crate::llm::text(
                &client,
                &settings.ai_url,
                &settings.ai_model,
                0.6,
                &prompt,
                &message,
            ),
        )
        .await
        {
            Ok(Ok(reply)) if safe_reply(reply.trim(), configuration.maximum_reply_characters) => {
                reply.trim().to_owned()
            }
            _ => baseline,
        }
    } else {
        baseline
    };
    let mut state = shared.write().await;
    if state.session_generation != generation {
        return crate::routes::bad_request(openatc_core::dialogue::say(
            "notice_the_flight_changed_before_that_pilot_could_reply_14af1ae9",
            &[],
        ));
    }
    state.traffic.chat_pending = false;
    if !quiet(&state)
        || state.session.control_frequency() != Some(frequency)
        || !state.settings.talk_to_traffic
        || !state.settings.informal_pilot_chat
        || available(&state, id).is_none_or(|pilot| pilot.target.callsign != callsign)
    {
        return crate::routes::bad_request(openatc_core::dialogue::say(
            "notice_an_operational_exchange_or_changed_traffic_interrupted_t_c0859e89",
            &[],
        ));
    }
    let addressed = format!("{callsign}, {}, {message}", state.session.plan.callsign);
    let own = state.session.plan.callsign.clone();
    let tag = SpeechTag {
        voice: state.settings.pilot_voice.clone(),
        ..Default::default()
    };
    add_transmission(&mut state.session, &own, &addressed, &tag);
    transmit(&mut state, &callsign, &voice, &reply);
    if let Some(pilot) = state.traffic.pilots.get_mut(&id) {
        pilot.conversation.push_back((message, reply));
        while pilot.conversation.len() > configuration.history_exchanges {
            pilot.conversation.pop_front();
        }
    }
    state.traffic.last_chat = Some(Instant::now());
    Json(json!({"accepted":true})).into_response()
}

pub(super) fn initiate(state: &mut EngineState) -> bool {
    policy(state);
    let configuration = state.traffic.policy.as_ref().unwrap().clone();
    if !state.initiative.chat_allowed
        || !state.settings.pilots_call_me
        || !state.settings.talk_to_traffic
        || state.settings.pilot_chat_chance_percent == 0
        || state.traffic.chat_pending
        || !quiet(state)
        || state.traffic.last_radio.is_some_and(|at| {
            at.elapsed() < Duration::from_secs(configuration.operational_quiet_seconds)
        })
    {
        return false;
    }
    let Some(last) = state.traffic.last_chat else {
        state.traffic.last_chat = Some(Instant::now());
        return false;
    };
    if last.elapsed() < Duration::from_secs(configuration.initiation_interval_seconds) {
        return false;
    }
    let candidates: Vec<_> = state
        .traffic
        .pilots
        .keys()
        .copied()
        .filter(|id| available(state, *id).is_some())
        .collect();
    let phase = match state.session.phase {
        PhaseCode::Parked => "parked",
        PhaseCode::Cruise => "cruise",
        PhaseCode::TaxiIn => "taxi_in",
        _ => "critical",
    };
    // A current operational reason is required before rolling. Surface observations
    // justify asking for a report, never a claim about unobserved weather aloft.
    let opportunities: std::collections::BTreeSet<_> = candidates
        .iter()
        .flat_map(|id| {
            let pilot = &state.traffic.pilots[id];
            state
                .sim_weather
                .iter()
                .filter_map(move |(airport, (weather, observed))| {
                    let relevant = observed.elapsed().as_secs() <= 180
                        && weather["latitude"]
                            .as_f64()
                            .zip(weather["longitude"].as_f64())
                            .is_some_and(|(lat, lon)| {
                                openatc_core::ops::distance_nm(
                                    pilot.target.latitude,
                                    pilot.target.longitude,
                                    lat,
                                    lon,
                                ) <= configuration.range_nm
                            });
                    let wind = weather["windKnots"]
                        .as_f64()
                        .is_some_and(|v| v >= configuration.weather_wind_knots);
                    let visibility = weather["visibilityMeters"]
                        .as_f64()
                        .is_some_and(|v| v >= 0.0 && v <= configuration.weather_visibility_meters);
                    (relevant && (wind || visibility)).then(|| {
                        (
                            *id,
                            pilot.target.callsign.clone(),
                            airport.clone(),
                            u8::from(wind) + 2 * u8::from(visibility),
                        )
                    })
                })
        })
        .collect();
    // One roll for each pilot/weather-concern episode. A failed roll is remembered;
    // periodic polling does not eventually force the same unsolicited call.
    state
        .traffic
        .chat_attempts
        .retain(|key| opportunities.contains(key));
    let eligible: Vec<_> = opportunities
        .into_iter()
        .filter(|key| !state.traffic.chat_attempts.contains(key))
        .collect();
    let Some(opportunity) = eligible
        .iter()
        .find(|key| key.0 > state.traffic.last_chat_pilot)
        .or_else(|| eligible.first())
        .cloned()
    else {
        return false;
    };
    let id = opportunity.0;
    let pilot = &state.traffic.pilots[&id];
    let callsign = pilot.target.callsign.clone();
    let voice = pilot.voice.clone();
    let facts = BTreeMap::from([
        ("on_ground".into(), json!(pilot.target.on_ground)),
        ("phase".into(), json!(phase)),
        ("weather_concern".into(), json!(true)),
    ]);
    let Some(key) = openatc_core::dialogue::matched_response(
        "traffic_chat_initiate_",
        "",
        state.settings.pilot_chat_humour,
        &facts,
    ) else {
        return false;
    };
    state.traffic.last_chat = Some(Instant::now());
    state.traffic.chat_attempts.insert(opportunity);
    if !crate::initiative::roll_percent(state.settings.pilot_chat_chance_percent) {
        return false;
    }
    let text = say(
        &key,
        &[
            ("callsign", state.session.plan.callsign.clone()),
            ("traffic", callsign.clone()),
        ],
    );
    transmit(state, &callsign, &voice, &text);
    if let Some(pilot) = state.traffic.pilots.get_mut(&id) {
        pilot.conversation.push_back((String::new(), text));
        while pilot.conversation.len() > configuration.history_exchanges {
            pilot.conversation.pop_front();
        }
    }
    state.traffic.last_chat = Some(Instant::now());
    state.traffic.last_chat_pilot = id;
    state.traffic.last_radio = Some(Instant::now());
    true
}
