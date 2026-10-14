//! Crew requests become bounded aircraft-profile actions, confirmed by the plugin.
use crate::{Shared, routes::snapshot};
use axum::{
    Json,
    extract::State as AxumState,
    response::{IntoResponse, Response},
};
use openatc_core::{
    apply::add_transmission,
    crew::{Action, CrewProfile},
    dialogue::say,
    state::{Request, SpeechTag},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

pub struct Batch {
    outcomes: Vec<String>,
    request: String,
    services: Vec<openatc_core::crew::request::Service>,
    started: std::time::Instant,
}

pub struct CrewSession {
    pub aircraft: String,
    pub profile: CrewProfile,
    pub values: BTreeMap<String, f64>,
    pub checklist: Option<(String, usize, bool)>,
    pub sequence: u32,
    pub pushback: Option<openatc_core::crew::PushbackRequest>,
    pub observed_at: Option<std::time::Instant>,
    pub batch: Option<Batch>,
    pub service_requests: BTreeMap<String, u32>,
}
impl Default for CrewSession {
    fn default() -> Self {
        Self {
            aircraft: String::new(),
            profile: CrewProfile::default(),
            values: BTreeMap::new(),
            checklist: None,
            sequence: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |t| t.as_millis() as u32),
            pushback: None,
            observed_at: None,
            batch: None,
            service_requests: BTreeMap::new(),
        }
    }
}
#[derive(Deserialize, Serialize)]
pub struct Observation {
    pub aircraft: String,
    pub profile: Option<CrewProfile>,
    pub values: BTreeMap<String, f64>,
    #[serde(default)]
    pub available: Vec<String>,
}
pub async fn observe(
    AxumState(shared): AxumState<Shared>,
    Json(input): Json<Observation>,
) -> Response {
    let mut s = shared.write().await;
    let profile = input.profile.unwrap_or_else(|| s.crew.profile.clone());
    if s.crew.aircraft != input.aircraft || s.crew.profile != profile {
        s.request_memory.cancel_pending();
        s.crew.checklist = None;
        s.crew.batch = None;
        s.crew.service_requests.clear();
        s.session.crew_actions.clear();
    }
    s.crew.aircraft = input.aircraft;
    s.session.crew_controls = profile
        .controls
        .iter()
        .map(|(id, c)| openatc_core::crew::Capability {
            id: id.clone(),
            label: c.label.clone(),
            role: c.role.clone(),
            available: input.available.contains(id),
            value: input.values.get(id).copied(),
        })
        .collect();
    s.crew.profile = profile;
    s.crew.values = input.values;
    s.crew.observed_at = Some(std::time::Instant::now());
    if s.crew
        .batch
        .as_ref()
        .is_some_and(|b| b.started.elapsed().as_secs() >= 90)
    {
        let actions = std::mem::take(&mut s.session.crew_actions);
        if let Some(mut batch) = s.crew.batch.take() {
            for action in &actions {
                s.request_memory.confirm(action.sequence, false);
                let item = say(&format!("crew_control_{}", action.control), &[]);
                batch
                    .outcomes
                    .push(say("crew_completed_unconfirmed", &[("item", item)]));
            }
            if let Some(action) = actions.first() {
                let text = batch_reply(&mut s, batch);
                speak(&mut s, &action.role, &text);
            }
        }
    }
    Json(json!({"accepted":true})).into_response()
}
pub(crate) fn speak(s: &mut crate::EngineState, role: &str, text: &str) {
    let voice = match role {
        "cabin" => &s.settings.attendant_voice,
        "ground" => &s.settings.ground_voice,
        _ => &s.settings.copilot_voice,
    }
    .clone();
    add_transmission(
        &mut s.session,
        match role {
            "cabin" => "ATTENDANT",
            "ground" => "GROUND",
            _ => "COPILOT",
        },
        text,
        &SpeechTag {
            voice,
            speed: 1.0,
            delivery: "standard".into(),
            ..Default::default()
        },
    );
}
fn reply(
    s: &mut crate::EngineState,
    role: &str,
    id: &str,
    slots: &[(&str, String)],
    ok: bool,
) -> Response {
    let text = say(id, slots);
    speak(s, role, &text);
    Json(json!({"result":{"accepted":ok,"message":text},"state":snapshot(&s.session,&s.settings)}))
        .into_response()
}
fn queue(s: &mut crate::EngineState, role: &str, id: &str, value: f64, response: String) -> bool {
    let t = &s.session.telemetry;
    let Some(c) = s.crew.profile.controls.get(id) else {
        return false;
    };
    if !openatc_core::crew::validate(&s.crew.profile, role, id, value)
        || !t.position_valid
        || t.paused
        || (c.ground_only && (!t.on_ground || t.ground_speed_knots > 0.5))
        || (c.airborne_only && t.on_ground)
        || (id == "gear" && value == c.off && t.on_ground)
        || s.crew
            .observed_at
            .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(3))
        || s.session.crew_actions.len() >= 8
    {
        return false;
    }
    s.crew.sequence = s.crew.sequence.wrapping_add(1).max(1);
    s.session.crew_actions.push(Action {
        sequence: s.crew.sequence,
        aircraft: s.crew.aircraft.clone(),
        control: id.into(),
        value,
        role: role.into(),
        response,
    });
    true
}
fn next_item(s: &mut crate::EngineState) -> Response {
    let Some((name, index, perform)) = s.crew.checklist.clone() else {
        return reply(s, "copilot", "crew_checklist_none", &[], false);
    };
    let Some(item) = s
        .crew
        .profile
        .checklists
        .get(&name)
        .and_then(|items| items.get(index))
        .cloned()
    else {
        s.crew.checklist = None;
        return reply(
            s,
            "copilot",
            "crew_checklist_complete",
            &[("checklist", name.replace('_', " "))],
            true,
        );
    };
    if perform {
        if !queue(
            s,
            "copilot",
            &item.control,
            item.value,
            say(&item.response, &[]),
        ) {
            return reply(
                s,
                "copilot",
                "crew_action_unavailable",
                &[("item", say(&item.challenge, &[]))],
                false,
            );
        }
        Json(json!({"result":{"accepted":true,"pending":true},"state":snapshot(&s.session,&s.settings)})).into_response()
    } else {
        reply(
            s,
            "copilot",
            "crew_checklist_challenge",
            &[("item", say(&item.challenge, &[]))],
            true,
        )
    }
}
#[derive(Deserialize)]
pub struct Ack {
    pub sequence: u32,
    pub aircraft: String,
    pub success: bool,
    pub detail: String,
}
pub async fn acknowledge(AxumState(shared): AxumState<Shared>, Json(ack): Json<Ack>) -> Response {
    let mut s = shared.write().await;
    let Some(action) = s
        .session
        .crew_actions
        .first()
        .filter(|a| a.sequence == ack.sequence && a.aircraft == ack.aircraft)
        .cloned()
    else {
        return Json(json!({"accepted":false})).into_response();
    };
    s.session.crew_actions.remove(0);
    s.request_memory.confirm(action.sequence, ack.success);
    let label = s
        .crew
        .profile
        .controls
        .get(&action.control)
        .map_or(action.control.clone(), |_| {
            say(&format!("crew_control_{}", action.control), &[])
        });
    if ack.success {
        s.crew.values.insert(action.control.clone(), action.value);
        let template = if action.control == "pushback_start" {
            "crew_pushback_start_sent"
        } else if s
            .crew
            .profile
            .controls
            .get(&action.control)
            .is_some_and(|c| c.momentary)
        {
            "crew_button_pressed"
        } else if action.role == "cabin" && action.control == "slides" {
            if action.value > 0.0 {
                "crew_slides_armed_checked"
            } else {
                "crew_slides_disarmed_checked"
            }
        } else {
            "crew_action_confirmed"
        };
        let text = if s.crew.batch.is_some() {
            let key = format!(
                "crew_completed_{}_{}",
                action.control,
                if action.value == 0.0 { "off" } else { "on" }
            );
            if ["chocks", "external_power", "high_pressure_air"].contains(&action.control.as_str())
            {
                say(&key, &[])
            } else if action.control == "slides" || action.control == "pushback_start" {
                say(template, &[]).trim_end_matches('.').to_owned()
            } else {
                say(
                    "crew_completed_control",
                    &[("item", label), ("state", action.response)],
                )
            }
        } else {
            say(template, &[("item", label), ("state", action.response)])
        };
        if let Some(batch) = s.crew.batch.as_mut() {
            batch.outcomes.push(text);
        } else {
            speak(&mut s, &action.role, &text);
        }
        if let Some((_, index, true)) = s.crew.checklist.as_mut() {
            *index += 1;
            return next_item(&mut s);
        }
    } else {
        eprintln!("Crew action {} failed: {}", action.control, ack.detail);
        if let Some(batch) = s.crew.batch.as_mut() {
            batch
                .outcomes
                .push(say("crew_completed_unconfirmed", &[("item", label)]));
        } else {
            s.session.crew_actions.clear();
            s.crew.checklist = None;
            let text = say("crew_action_failed", &[("item", label)]);
            speak(&mut s, &action.role, &text);
        }
    }
    if s.session.crew_actions.is_empty()
        && let Some(batch) = s.crew.batch.take()
    {
        let request = batch.request.clone();
        let protected = batch.outcomes.clone();
        let generation = s.session_generation;
        let text = batch_reply(&mut s, batch);
        drop(s);
        let text = narrate(&shared, &action.role, &request, &text, &protected).await;
        s = shared.write().await;
        if s.session_generation == generation && s.crew.aircraft == ack.aircraft {
            speak(&mut s, &action.role, &text);
        }
    }
    Json(json!({"accepted":true,"state":snapshot(&s.session,&s.settings)})).into_response()
}

pub(crate) async fn narrate(
    shared: &Shared,
    role: &str,
    request: &str,
    fallback: &str,
    protected: &[String],
) -> String {
    let s = shared.read().await;
    if !s.settings.ai_enabled || s.settings.ai_model.is_empty() {
        return fallback.to_owned();
    }
    let personality = match role {
        "cabin" => &s.settings.attendant_personality,
        "ground" => &s.settings.ground_personality,
        _ => &s.settings.copilot_personality,
    };
    let role_prompt = match role {
        "cabin" => &s.prompts.cabin,
        "ground" => &s.prompts.ground,
        _ => &s.prompts.copilot_chat,
    };
    let role_prompt = crate::support::fill_prompt(role_prompt, &s.session.plan.callsign);
    let voice_role = if role == "cabin" { "attendant" } else { role };
    let examples = s
        .speech
        .select(
            voice_role,
            &["actor_conversation"],
            &format!("{:?}", s.session.phase).to_lowercase(),
            "icao",
            4,
        )
        .into_iter()
        .flat_map(|example| example.say.iter().take(2).cloned().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let recent = crate::context::conversation(&s, role, 4, 768);
    let controls: Vec<_> = s
        .session
        .crew_controls
        .iter()
        .filter(|control| control.role == role)
        .take(12)
        .map(|control| json!({"id":control.id,"available":control.available,"value":s.crew.values.get(&control.id)}))
        .collect();
    let context=json!({"phase":s.session.phase,"onGround":s.session.telemetry.on_ground,"groundSpeedKnots":s.session.telemetry.ground_speed_knots,
        "departure":s.session.plan.departure,"destination":s.session.plan.destination,"cabin":s.cabin,"controls":controls,
        "missionIncident":s.session.mission.incident,"recentConversationNewestFirst":recent,"serviceCounts":s.crew.service_requests,
        "humourAllowed":s.settings.llm_phrase_variety && !s.cabin.preparing_for_turbulence && s.session.taxi_clearance.crossing_runway.is_empty() && !matches!(s.session.phase,openatc_core::state::PhaseCode::Departure|openatc_core::state::PhaseCode::Approach|openatc_core::state::PhaseCode::Landed) && s.session.mission.incident.is_empty()}).to_string();
    let system = crate::llm::prompt(
        &[
            role_prompt.as_str(),
            personality.as_str(),
            &s.prompts.crew_response,
        ]
        .join("\n"),
        s.prompts.guidance.get("examples"),
        &examples,
        &context,
    );
    let client = s.client.clone();
    let settings = s.settings.clone();
    let language_path = s.config_dir.join("cabin/language.toml");
    drop(s);
    let language = openatc_core::crew::request::Grammar::load(&language_path).ok();
    let user = json!({"currentRequest":request,"confirmedResult":fallback}).to_string();
    let response = crate::llm::narration(
        &client,
        &settings.ai_url,
        &settings.ai_model,
        if settings.llm_phrase_variety {
            0.6
        } else {
            0.2
        },
        &system,
        &user,
    )
    .await;
    let Ok(reply) = response else {
        return fallback.to_owned();
    };
    if valid_narration(&reply, fallback, protected)
        && language
            .as_ref()
            .is_some_and(|language| language.narration_allowed(&reply))
    {
        reply.trim().to_owned()
    } else {
        fallback.to_owned()
    }
}

fn valid_narration(reply: &str, fallback: &str, protected: &[String]) -> bool {
    let lower = reply.to_lowercase();
    let failed = fallback.to_lowercase();
    if lower.matches("captain").count() > 1
        || reply.contains("?.")
        || reply.contains("!.")
        || reply.trim().is_empty()
        || reply.len() > 1200
        || [
            "simulat",
            "plugin",
            "language model",
            "artificial intelligence",
        ]
        .iter()
        .any(|word| lower.contains(word))
        || ((failed.contains("unconfirmed")
            || failed.contains("not confirm")
            || failed.contains("could not")
            || failed.contains("couldn’t")
            || failed.contains("can’t confirm"))
            && ![
                "not",
                "can't",
                "can’t",
                "cannot",
                "couldn",
                "unable",
                "haven’t",
                "haven't",
            ]
            .iter()
            .any(|word| lower.contains(word)))
    {
        return false;
    }
    let outcomes = protected.join(" ").to_lowercase();
    if [
        "heading", "altitude", "chocks", "power", "lights", "gear", "flaps", "speed",
    ]
    .iter()
    .any(|term| outcomes.contains(term) && !lower.contains(term))
        || (outcomes.contains("slides") && !lower.contains("slides") && !lower.contains("doors"))
    {
        return false;
    }
    let numbers = |text: &str| {
        text.split(|c: char| !c.is_ascii_digit() && c != '.')
            .filter_map(|part| part.trim_matches('.').parse::<f64>().ok())
            .collect::<Vec<_>>()
    };
    let actual = numbers(&reply);
    if numbers(&outcomes)
        .iter()
        .any(|number| !actual.contains(number))
    {
        return false;
    }
    true
}

fn batch_reply(s: &mut crate::EngineState, batch: Batch) -> String {
    let mut text = String::new();
    if let Some((last, rest)) = batch.outcomes.split_last() {
        let items = if rest.is_empty() {
            last.clone()
        } else {
            say(
                "crew_completed_list",
                &[("first", rest.join(", ")), ("last", last.clone())],
            )
        };
        text = say("crew_batch_complete", &[("items", items)]);
        if !text.ends_with(['.', '?', '!']) {
            text.push('.');
        }
    }
    let busy_cabin = s.cabin.preparing_for_turbulence
        || matches!(s.session.mission.severity.as_str(), "urgent" | "emergency")
        || matches!(
            s.session.phase,
            openatc_core::state::PhaseCode::Departure
                | openatc_core::state::PhaseCode::Approach
                | openatc_core::state::PhaseCode::Landed
        );
    let mut requests = Vec::new();
    let mut coffee_count = 0;
    for service in batch.services {
        if service == openatc_core::crew::request::Service::PassengerStatus {
            if let Some(request) =
                openatc_core::dialogue::phrases("cabin_request_passengers").first()
                && let Some(report) = crate::cabin::report(s, request)
            {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(&report);
            }
            continue;
        }
        if matches!(
            service,
            openatc_core::crew::request::Service::LandingPreparation
                | openatc_core::crew::request::Service::PassengerWatch
        ) {
            if let Some(request) = openatc_core::dialogue::phrases(service.response()).first()
                && let Some(report) = crate::cabin::report(s, request)
            {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(&report);
            }
            continue;
        }
        if service == openatc_core::crew::request::Service::DeparturePreparation {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&say(service.response(), &[]));
            continue;
        }
        let count = s
            .crew
            .service_requests
            .entry(service.response().into())
            .or_default();
        *count = count.saturating_add(1);
        if service == openatc_core::crew::request::Service::Coffee {
            coffee_count = *count;
        }
        requests.push(say(service.response(), &[]));
    }
    if let Some((last, rest)) = requests.split_last() {
        if !text.is_empty() {
            text.push(' ');
        }
        let service = if rest.is_empty() {
            last.clone()
        } else {
            say(
                "crew_completed_list",
                &[("first", rest.join(", ")), ("last", last.clone())],
            )
        };
        text.push_str(&say(
            if busy_cabin {
                "crew_service_when_free"
            } else {
                "crew_service_reply"
            },
            &[("service", service)],
        ));
        if !busy_cabin && coffee_count >= 3 && coffee_count % 3 == 0 {
            text.push(' ');
            text.push_str(&say("crew_coffee_repeat", &[]));
        }
    }
    text
}

fn cross_check(s: &crate::EngineState) -> &'static str {
    match s.crew.values.get("slides").copied() {
        Some(1.0) => "crew_slides_armed_checked",
        Some(0.0) => "crew_slides_disarmed_checked",
        _ => "crew_slides_not_confirmed",
    }
}

pub async fn request(shared: &Shared, request: &Request) -> Option<Response> {
    let role = request.role.as_str();
    if !["copilot", "cabin", "ground"].contains(&role) {
        return None;
    }
    let text = request.text.to_lowercase();
    let mut s = shared.write().await;
    if role == "cabin"
        && let Some(text) = crate::missions::respond(&mut s, &request.text)
    {
        let pilot = s.session.plan.callsign.clone();
        add_transmission(&mut s.session, &pilot, &request.text, &SpeechTag::default());
        speak(&mut s, role, &text);
        return Some(Json(json!({"result":{"accepted":true,"message":text},"state":snapshot(&s.session,&s.settings)})).into_response());
    }
    let profile = s.crew.profile.clone();
    let grammar =
        openatc_core::crew::request::Grammar::load(&s.config_dir.join("cabin/language.toml")).ok();
    if grammar
        .as_ref()
        .is_some_and(|grammar| grammar.social_only(&text))
    {
        return None;
    }
    let parsed = grammar.as_ref().and_then(|grammar| {
        openatc_core::crew::request::parse_with(&profile, role, &text, grammar)
    });
    let recognized = parsed.is_some()
        || profile.controls.iter().any(|(id, c)| {
            c.role == role
                && openatc_core::dialogue::phrases(&format!("crew_request_{id}"))
                    .iter()
                    .any(|a| text.contains(a))
        })
        || [
            "set ",
            "remove ",
            "disconnect ",
            "connect ",
            "add ",
            "please ",
            "can you ",
            "could you ",
            "remov",
            "discon",
            "conn",
            "lower ",
            "raise ",
            "engage ",
            "disengage ",
            "turn ",
            "arm ",
            "disarm ",
        ]
        .iter()
        .any(|word| text.starts_with(word))
        || text.contains("checklist")
        || text.contains("cross check")
        || text.contains("crosscheck")
        || text.contains("xcheck")
        || (role == "ground" && (text.contains("pushback") || text.contains("push back")))
        || s.crew.checklist.is_some() && role == "copilot";
    let ai_recovery = s.settings.ai_enabled
        && !s.settings.ai_model.is_empty()
        && ![
            "how are",
            "hello",
            "hi",
            "good morning",
            "good evening",
            "thanks",
            "thank you",
        ]
        .iter()
        .any(|prefix| text.starts_with(prefix));
    if !recognized && !ai_recovery {
        return None;
    }
    if let Some(intent) = parsed
        .as_ref()
        .filter(|intent| intent.actions.is_empty() && !intent.cross_check)
    {
        let pilot = s.session.plan.callsign.clone();
        add_transmission(&mut s.session, &pilot, &request.text, &SpeechTag::default());
        let message = batch_reply(
            &mut s,
            Batch {
                outcomes: Vec::new(),
                request: request.text.clone(),
                services: intent.services.clone(),
                started: std::time::Instant::now(),
            },
        );
        let generation = s.session_generation;
        drop(s);
        let message = narrate(shared, role, &request.text, &message, &[]).await;
        s = shared.write().await;
        if s.session_generation != generation {
            return Some(reply(&mut s, role, "crew_action_clarify", &[], false));
        }
        speak(&mut s, role, &message);
        return Some(Json(json!({"result":{"accepted":true,"message":message},"state":snapshot(&s.session,&s.settings)})).into_response());
    }
    if s.crew
        .observed_at
        .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(3))
    {
        return Some(reply(&mut s, role, "crew_controls_wait", &[], false));
    }
    if !s.session.crew_actions.is_empty() {
        return Some(reply(&mut s, role, "crew_actions_busy", &[], false));
    }
    let pilot = s.session.plan.callsign.clone();
    let pilot_sequence = s.session.next_sequence;
    add_transmission(&mut s.session, &pilot, &request.text, &SpeechTag::default());
    if role == "ground" && (text.contains("pushback") || text.contains("push back")) {
        s.crew.pushback = openatc_core::crew::parse_pushback(&text);
        let words = text.trim().trim_end_matches(['.', '!']);
        let start_request = !text.contains('?')
            && (matches!(words, "pushback" | "push back")
                || [
                    "start pushback",
                    "start push back",
                    "request pushback",
                    "begin pushback",
                    "please start pushback",
                ]
                .iter()
                .any(|prefix| words == *prefix || words.starts_with(&format!("{prefix} "))));
        if start_request {
            if queue(&mut s, role, "pushback_start", 1.0, String::new()) {
                return Some(Json(json!({"result":{"accepted":true,"pending":true},"state":snapshot(&s.session,&s.settings)})).into_response());
            }
            return Some(reply(
                &mut s,
                role,
                "crew_action_unavailable",
                &[("item", say("crew_control_pushback_start", &[]))],
                false,
            ));
        }
        return Some(reply(&mut s, role, "crew_pushback_clarify", &[], false));
    }
    if parsed.is_none()
        && role == "cabin"
        && !["arm", "disarm", "automatic", "manual"].iter().any(|word| {
            text.split_whitespace()
                .any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == *word)
        })
        && ["cross check", "crosscheck", "xcheck"]
            .iter()
            .any(|word| text.contains(word))
    {
        let state = s.crew.values.get("slides").copied();
        return Some(match state {
            Some(1.0) => reply(&mut s, role, "crew_slides_armed_checked", &[], true),
            Some(0.0) => reply(&mut s, role, "crew_slides_disarmed_checked", &[], true),
            _ => reply(&mut s, role, "crew_slides_not_confirmed", &[], false),
        });
    }
    if role == "copilot" && text.contains("checklist") {
        if text.contains("cancel") || text.contains("stop") {
            s.crew.checklist = None;
            return Some(reply(&mut s, role, "crew_checklist_stopped", &[], true));
        }
        let name = profile
            .checklists
            .keys()
            .find(|name| text.contains(&name.replace('_', " ")))
            .cloned();
        if let Some(name) = name {
            s.crew.checklist = Some((name, 0, text.contains("perform") || text.contains("do the")));
            return Some(next_item(&mut s));
        }
        return Some(reply(
            &mut s,
            role,
            "crew_checklist_choose",
            &[(
                "checklist",
                profile
                    .checklists
                    .keys()
                    .map(|k| k.replace('_', " "))
                    .collect::<Vec<_>>()
                    .join(", "),
            )],
            false,
        ));
    }
    if role == "copilot"
        && let Some((name, index, false)) = s.crew.checklist.clone()
    {
        let item = profile
            .checklists
            .get(&name)
            .and_then(|items| items.get(index))
            .cloned()?;
        if text.split_whitespace().any(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .eq_ignore_ascii_case(&say(&item.response, &[]))
                || ["set", "checked"].contains(&word.trim_matches(|c: char| !c.is_alphanumeric()))
        }) {
            let checked = s
                .crew
                .values
                .get(&item.control)
                .is_some_and(|v| (*v - item.value).abs() <= 0.01);
            if checked {
                s.crew.checklist = Some((name, index + 1, false));
                return Some(next_item(&mut s));
            }
            return Some(reply(
                &mut s,
                role,
                "crew_checklist_mismatch",
                &[
                    ("item", say(&item.challenge, &[])),
                    ("state", say(&item.response, &[])),
                ],
                false,
            ));
        }
        return Some(reply(
            &mut s,
            role,
            "crew_checklist_repeat",
            &[
                ("item", say(&item.challenge, &[])),
                ("state", say(&item.response, &[])),
            ],
            false,
        ));
    }
    let explicit_action = [
        "connect ",
        "disconnect ",
        "turn ",
        "set ",
        "add ",
        "remove ",
        "arm ",
        "disarm ",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix));
    if ((!explicit_action
        && profile.controls.iter().any(|(id, c)| {
            c.role == role
                && !c.momentary
                && openatc_core::dialogue::phrases(&format!("crew_request_{id}"))
                    .iter()
                    .any(|alias| text.trim_end_matches('?') == *alias)
        }))
        || text.starts_with("is ")
        || text.starts_with("are ")
        || text.starts_with("check ")
        || text.starts_with("confirm "))
        && let Some((id, c)) = profile.controls.iter().find(|(id, c)| {
            c.role == role
                && openatc_core::dialogue::phrases(&format!("crew_request_{id}"))
                    .iter()
                    .any(|a| text.contains(a))
        })
    {
        let value = s.crew.values.get(id).copied();
        let item = say(&format!("crew_control_{id}"), &[]);
        return Some(if let Some(value) = value {
            let state = c
                .states
                .iter()
                .find(|(_, n)| (**n - value).abs() < 0.01)
                .map(|(name, _)| say(&format!("crew_state_{name}"), &[]))
                .unwrap_or_else(|| {
                    if value == c.off {
                        say("crew_state_off", &[])
                    } else if value == c.on {
                        say("crew_state_on", &[])
                    } else {
                        say("crew_state_value", &[("value", format!("{value:.0}"))])
                    }
                });
            reply(
                &mut s,
                role,
                "crew_action_confirmed",
                &[("item", item), ("state", state)],
                true,
            )
        } else {
            reply(&mut s, role, "crew_action_failed", &[("item", item)], false)
        });
    }
    if text.contains("don't ") || text.contains("do not ") || text.contains("never ") {
        return Some(reply(&mut s, role, "crew_no_action", &[], true));
    }
    let generation = s.session_generation;
    let aircraft = s.crew.aircraft.clone();
    drop(s);
    let parsed = if parsed.is_some() {
        parsed
    } else {
        let settings = shared.read().await.settings.clone();
        if settings.ai_enabled && !settings.ai_model.is_empty() {
            let client = shared.read().await.client.clone();
            let mut ranked = profile
                .controls
                .iter()
                .filter(|(_, c)| c.role == role)
                .map(|(id, c)| {
                    let words = format!(
                        "{} {}",
                        c.label,
                        openatc_core::dialogue::phrases(&format!("crew_request_{id}")).join(" ")
                    )
                    .to_lowercase();
                    let score = text
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|w| w.len() > 2 && words.contains(*w))
                        .count();
                    (score, id, c)
                })
                .collect::<Vec<_>>();
            ranked.sort_by_key(|(score, _, _)| std::cmp::Reverse(*score));
            let allowed=ranked.into_iter().take(32).map(|(_,id,c)|json!({"control":id,"label":c.label,"min":c.min,"max":c.max,"on":c.on,"off":c.off,"states":c.states,"button":c.momentary})).collect::<Vec<_>>();
            let instructions = shared
                .read()
                .await
                .prompts
                .crew_actions
                .replace("{{role}}", role);
            let prompt = format!("{instructions}\nAllowed controls: {}", json!(allowed));
            crate::llm::structured::<openatc_core::crew::request::Intent>(
                &client,
                &settings.ai_url,
                &settings.ai_model,
                0.0,
                &prompt,
                &request.text,
            )
            .await
            .ok()
            .filter(|intent| intent.valid(&profile, role))
        } else {
            None
        }
    };
    let mut s = shared.write().await;
    if s.session_generation != generation {
        return Some(
            (
                axum::http::StatusCode::CONFLICT,
                Json(json!({"error":say("notice_flight_was_reset_previous_request_discarded_c0a5c48d", &[])})),
            )
                .into_response(),
        );
    }
    if s.crew.aircraft != aircraft {
        return Some(reply(&mut s, role, "crew_action_clarify", &[], false));
    }
    if let Some(intent) = parsed {
        if intent.conversation_only {
            s.session
                .transcript
                .retain(|entry| entry.sequence != pilot_sequence);
            let settings = s.settings.clone();
            drop(s);
            let mut conversation = request.clone();
            conversation.intent = "conversation".into();
            return Some(crate::routes::actor_chat(shared, &settings, &conversation).await);
        }
        if !s.session.crew_actions.is_empty() {
            return Some(reply(&mut s, role, "crew_actions_busy", &[], false));
        }
        let mut batch = Batch {
            outcomes: Vec::new(),
            request: request.text.clone(),
            services: intent.services,
            started: std::time::Instant::now(),
        };
        if intent.cross_check && !intent.actions.iter().any(|a| a.control == "slides") {
            batch
                .outcomes
                .push(say(cross_check(&s), &[]).trim_end_matches('.').to_owned());
        }
        for action in intent.actions {
            let (id, value) = (action.control, action.value);
            let c = profile.controls.get(&id)?;
            let state = if c.momentary {
                say("crew_state_button", &[])
            } else {
                c.states
                    .iter()
                    .find(|(_, v)| (**v - value).abs() < 0.01)
                    .map(|(name, _)| say(&format!("crew_state_{name}"), &[]))
                    .unwrap_or_else(|| {
                        if value == c.off {
                            say("crew_state_off", &[])
                        } else if value == c.on {
                            say("crew_state_on", &[])
                        } else {
                            say("crew_state_value", &[("value", format!("{value:.0}"))])
                        }
                    })
            };
            let ok = queue(&mut s, role, &id, value, state);
            if !ok {
                s.session.crew_actions.clear();
                return Some(reply(
                    &mut s,
                    role,
                    "crew_action_unavailable",
                    &[("item", say(&format!("crew_control_{id}"), &[]))],
                    false,
                ));
            }
        }
        if s.session.crew_actions.is_empty() {
            let generation = s.session_generation;
            let protected = batch.outcomes.clone();
            let message = batch_reply(&mut s, batch);
            drop(s);
            let message = narrate(shared, role, &request.text, &message, &protected).await;
            s = shared.write().await;
            if s.session_generation != generation {
                return Some(reply(&mut s, role, "crew_action_clarify", &[], false));
            }
            speak(&mut s, role, &message);
            return Some(Json(json!({"result":{"accepted":true,"message":message},"state":snapshot(&s.session,&s.settings)})).into_response());
        }
        s.crew.batch = Some(batch);
        return Some(Json(json!({"result":{"accepted":true,"pending":true},"state":snapshot(&s.session,&s.settings)})).into_response());
    }
    if !recognized {
        s.session
            .transcript
            .retain(|entry| entry.sequence != pilot_sequence);
        return None;
    }
    Some(reply(&mut s, role, "crew_action_clarify", &[], false))
}

#[cfg(test)]
mod narration_tests {
    #[test]
    fn generated_reply_keeps_control_outcomes_and_operational_numbers() {
        let protected = vec!["Heading 275 set".to_owned()];
        assert!(super::valid_narration(
            "Heading 275 set. You're welcome.",
            "Heading 275 set",
            &protected
        ));
        assert!(!super::valid_narration(
            "Heading 270 set.",
            "Heading 275 set",
            &protected
        ));
        assert!(!super::valid_narration(
            "You're welcome.",
            "Heading 275 set",
            &protected
        ));
        assert!(!super::valid_narration(
            "Yes, Captain, Captain, I cannot confirm the slides?.",
            "Slides could not be confirmed",
            &[]
        ));
        let failed = vec!["Slides could not be confirmed".to_owned()];
        assert!(!super::valid_narration(
            "Doors armed and checked.",
            &failed[0],
            &failed
        ));
        assert!(!super::valid_narration(
            "The simulator shows heading 275.",
            "Heading 275 set",
            &protected
        ));
    }
}

#[cfg(test)]
mod conversation_routing_tests {
    use super::*;
    use axum::{Router, routing::post};

    #[tokio::test]
    async fn polite_joke_request_is_conversation_without_pushback_or_control_actions() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/v1/chat/completions", post(|Json(body):Json<serde_json::Value>| async move {
                let classifier = body["messages"][0]["content"].as_str().unwrap().contains("Allowed controls:");
                let content = if classifier { json!({"actions":[],"services":[],"cross_check":false,"conversation_only":true}).to_string() }
                    else if body.get("response_format").is_some() { json!({"reply":"Here's a quick one: why did the biscuit visit the doctor? It felt crummy!"}).to_string() }
                    else { "Here's a quick one: why did the biscuit visit the doctor? It felt crummy!".into() };
                Json(json!({"choices":[{"message":{"content":content}}]}))
            }))).await.unwrap();
        });
        let mut state = crate::test_state();
        state.settings.ai_enabled = true;
        state.settings.ai_model = "test".into();
        state.settings.ai_url = format!("http://{address}");
        state.session.phase = openatc_core::state::PhaseCode::Pushback;
        state.crew.observed_at = Some(std::time::Instant::now());
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(state));
        let response = request(
            &shared,
            &Request {
                role: "cabin".into(),
                intent: "pushback".into(),
                text: "can you tell me a joke while we are waiting for pushback?".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(response.status().is_success());
        let state = shared.read().await;
        assert!(state.session.crew_actions.is_empty());
        assert_eq!(
            state.session.phase,
            openatc_core::state::PhaseCode::Pushback
        );
        assert_eq!(state.session.transcript.len(), 2);
        assert_eq!(
            state.session.transcript.last().unwrap().speaker,
            "ATTENDANT"
        );
        assert!(
            state
                .session
                .transcript
                .last()
                .unwrap()
                .text
                .contains("biscuit")
        );
        drop(state);
        for (role, speaker, text) in [
            (
                "cabin",
                "ATTENDANT",
                "what about the joke? can you tell me one?",
            ),
            ("copilot", "COPILOT", "are you okay?"),
            ("ground", "GROUND", "can you tell me something funny?"),
        ] {
            shared.write().await.crew.observed_at = Some(std::time::Instant::now());
            let response = request(
                &shared,
                &Request {
                    role: role.into(),
                    intent: "conversation".into(),
                    text: text.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            assert!(response.status().is_success());
            let state = shared.read().await;
            assert!(state.session.crew_actions.is_empty());
            assert_eq!(state.session.transcript.last().unwrap().speaker, speaker);
            assert!(
                state
                    .session
                    .transcript
                    .last()
                    .unwrap()
                    .text
                    .contains("biscuit")
            );
        }
        server.abort();
    }
}
