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
    held_reply: String,
    failed: bool,
    started: std::time::Instant,
}

pub struct CrewSession {
    pub memories: BTreeMap<String, String>,
    pub aircraft: String,
    pub profile: CrewProfile,
    pub values: BTreeMap<String, f64>,
    pub checklist: Option<(String, usize, bool)>,
    pub sequence: u32,
    pub pushback: Option<openatc_core::crew::PushbackRequest>,
    pub observed_at: Option<std::time::Instant>,
    pub batch: Option<Batch>,
}
impl Default for CrewSession {
    fn default() -> Self {
        Self {
            memories: BTreeMap::new(),
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
#[derive(Deserialize)]
pub struct CalloutLog { entries: Vec<String>, callsign: String }
/// Record clips already played by the cockpit's independent callout channel.
pub async fn callout_log(AxumState(shared): AxumState<Shared>, Json(input): Json<CalloutLog>) -> Response {
    let mut state = shared.write().await;
    if input.callsign != state.session.plan.callsign || input.entries.is_empty() || input.entries.len() > 2
        || input.entries.iter().any(|text| text.is_empty() || text.len() > 128) {
        return Json(json!({"accepted":false})).into_response();
    }
    for text in input.entries {
        add_transmission(&mut state.session,"COPILOT",&text,&SpeechTag::default());
        if let Some(entry) = state.session.transcript.last_mut() {
            entry.delivery = "preloaded-callout".into();
            entry.information_only = true;
        }
    }
    Json(json!({"accepted":true,"state":snapshot(&state.session,&state.settings)})).into_response()
}
pub async fn observe(
    AxumState(shared): AxumState<Shared>,
    Json(input): Json<Observation>,
) -> Response {
    let mut s = shared.write().await;
    let profile = input.profile.unwrap_or_else(|| s.crew.profile.clone());
    if s.crew.aircraft != input.aircraft || s.crew.profile != profile {
        s.crew.memories.clear();
        s.request_memory.cancel_pending();
        s.crew.checklist = None;
        s.crew.batch = None;
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
            batch.failed = true;
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
            } else if !openatc_core::dialogue::phrases(&format!("crew_completed_{}_value",action.control)).is_empty() {
                say(&format!("crew_completed_{}_value",action.control),&[("value",action.value.to_string())])
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
        if let Some(batch)=s.crew.batch.as_mut() {batch.failed=true;}
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
        let text = batch_reply(&mut s, batch);
        speak(&mut s, &action.role, &text);
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
    compose(shared,role,request,fallback,protected,&[],false).await
}
async fn compose(shared:&Shared,role:&str,request:&str,fallback:&str,protected:&[String],points:&[String],social_only:bool) -> String {
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
    let mut readings=s.session.crew_controls.iter().filter(|control|control.role==role).collect::<Vec<_>>();
    readings.sort_by_key(|control|s.crew.profile.controls.get(&control.id).is_some_and(|profile|profile.momentary));
    let controls=readings.into_iter().take(24).map(|control|json!({"id":control.id,"available":control.available,"value":s.crew.values.get(&control.id)})).collect::<Vec<_>>();
    let humour_allowed = !s.cabin.preparing_for_turbulence && s.session.taxi_clearance.crossing_runway.is_empty() && !matches!(s.session.phase,openatc_core::state::PhaseCode::Departure|openatc_core::state::PhaseCode::Approach|openatc_core::state::PhaseCode::Landed) && s.session.mission.incident.is_empty();
    let mut context=json!({"phase":s.session.phase,"onGround":s.session.telemetry.on_ground,"groundSpeedKnots":s.session.telemetry.ground_speed_knots,
        "departure":s.session.plan.departure,"destination":s.session.plan.destination,"cabin":s.cabin,"controls":if social_only{Vec::new()}else{controls},
        "missionIncident":s.session.mission.incident,"conversationNotes":s.crew.memories.get(role),
        "humourAllowed":humour_allowed}).to_string();
    if social_only {
        context = json!({"phase":s.session.phase,"onGround":s.session.telemetry.on_ground,
            "passengers":s.session.plan.passengers,"humourAllowed":humour_allowed,
            "conversationNotes":s.crew.memories.get(role),"continuity":"Use the notes for facts about prior conversation; answer the current message, never recite an earlier reply"}).to_string();
    }
    let system = crate::llm::prompt(
        &[
            role_prompt.as_str(),
            personality.as_str(),
            if social_only {s.prompts.guidance.get("crew_social_only")} else {&s.prompts.crew_response},
        ]
        .join("\n"),
        s.prompts.guidance.get("examples"),
        if social_only {&[]} else {&examples},
        &context,
    );
    let generation = s.session_generation;
    let aircraft = s.crew.aircraft.clone();
    let client = s.client.clone();
    let settings = s.settings.clone();
    let language_path = s.config_dir.join("cabin/response-policy.toml");
    drop(s);
    let language = openatc_core::crew::request::ReplyPolicy::load(&language_path).ok();
    let user = if !social_only && points.is_empty() && protected.is_empty() { request.to_owned() } else if social_only { json!({"captainMessage":points.join(" "),"conversationPoints":points}).to_string() } else { json!({"conversationContext":if points.is_empty() && protected.is_empty(){Some(request)}else{None},"confirmedResult":if protected.is_empty(){serde_json::Value::Null}else{json!(fallback)},"conversationPoints":points}).to_string() };
    if crate::llm::prompt_fits(&client, &settings.ai_url, &system, &user).await == Some(false) {
        eprintln!("Crew response exceeds the configured context; retaining verified outcomes");
        return fallback.to_owned();
    }
    let mut social_memory = String::new();
    let response = if social_only {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct SocialReply { answers: Vec<String>, #[serde(default)] memory: String }
        let schema = json!({"type":"object","additionalProperties":false,"required":["answers","memory"],
            "properties":{"memory":{"type":"string","maxLength":700},"answers":{"type":"array","minItems":points.len(),"maxItems":points.len(),
                "items":{"type":"string"}}}});
        crate::llm::structured_format::<SocialReply>(&client,&settings.ai_url,&settings.ai_model,
            if settings.llm_phrase_variety {0.6} else {0.2},&system,&user,Some(schema)).await
            .and_then(|reply| {
                if reply.answers.len() != points.len() || reply.answers.iter().any(|answer| answer.trim().is_empty()) {
                    return Err(crate::llm::StructuredError::Invalid("Incomplete conversational answers".into()));
                }
                social_memory = reply.memory;
                Ok(reply.answers.join(" "))
            }).map_err(|error|error.to_string())
    } else { crate::llm::narration(
        &client,
        &settings.ai_url,
        &settings.ai_model,
        if settings.llm_phrase_variety {0.6} else {0.2},
        &system,
        &user,
    )
    .await };
    let Ok(reply) = response else {
        eprintln!(
            "Crew response could not be composed: {}",
            response.unwrap_err()
        );
        return fallback.to_owned();
    };
    if (if social_only { crate::llm::spoken(&reply).is_ok() && reply.len() <= 3000 } else { valid_narration(&reply, fallback, protected) })
        && language
            .as_ref()
            .is_some_and(|language| language.narration_allowed(&reply))
    {
        if !social_memory.is_empty() && social_memory.len() <= 700 && crate::llm::spoken(&social_memory).is_ok() {
            let mut state = shared.write().await;
            if state.session_generation == generation && state.crew.aircraft == aircraft {
                state.crew.memories.insert(role.to_owned(), social_memory);
            }
        }
        reply.trim().to_owned()
    } else {
        eprintln!("Crew response did not preserve confirmed outcomes");
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

fn batch_reply(_s: &mut crate::EngineState, batch: Batch) -> String {
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
        text = if !batch.held_reply.trim().is_empty() && !batch.failed {
            say("crew_batch_with_conversation", &[("items",items.trim_end_matches('.').to_owned()),("reply",batch.held_reply)])
        } else {say("crew_batch_complete", &[("items",items)])};
        if !text.ends_with(['.', '?', '!']) {
            text.push('.');
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
    let pilot = s.session.plan.callsign.clone();
    let mut pilot_sequence = s.session.next_sequence;
    let using_ai = s.settings.ai_enabled && !s.settings.ai_model.is_empty();
    if using_ai {
        add_transmission(&mut s.session, &pilot, &request.text, &SpeechTag::default());
    }
    let profile = s.crew.profile.clone();
    let parsed = if s.settings.ai_enabled && !s.settings.ai_model.is_empty() {
        let generation = s.session_generation;
        let aircraft = s.crew.aircraft.clone();
        let settings = s.settings.clone();
        let client = s.client.clone();
        let interpretation_context = s.prompts.guidance.get("crew_interpret_context").to_owned();
        let authority_prompt = s.prompts.guidance.get("crew_control_authority").to_owned();
        let role_prompt = match role { "ground" => &s.prompts.ground, "cabin" => &s.prompts.cabin, _ => &s.prompts.copilot_chat };
        let personality = match role { "ground" => &s.settings.ground_personality, "cabin" => &s.settings.attendant_personality, _ => &s.settings.copilot_personality };
        let instructions = format!("{}\n{}\n{}\n{}",crate::support::fill_prompt(role_prompt,&s.session.plan.callsign),personality,
            s.prompts.crew_actions.replace("{{role}}", role),s.prompts.guidance.get(&format!("crew_actions_{role}")));
        let query = request.text.to_lowercase();
        let words: Vec<_> = query.split(|character: char| !character.is_alphanumeric()).filter(|word| word.len() >= 3).collect();
        let mut candidates: Vec<_> = profile.controls.iter().filter(|(_, control)| control.role == role).collect();
        candidates.sort_by_key(|(id, control)| {
            let label = format!("{id} {}", control.label).to_lowercase();
            (control.momentary, std::cmp::Reverse(words.iter().filter(|word| label.contains(**word)).count()))
        });
        let mut allowed: Vec<_> = candidates.into_iter().take(64)
            .map(|(id,c)| json!({"control":id,"label":c.label,"min":c.min,"max":c.max,"on":c.on,"off":c.off,"states":c.states,"value":s.crew.values.get(id),"momentary":c.momentary})).collect();
        let humour_allowed = !s.cabin.preparing_for_turbulence && s.session.taxi_clearance.crossing_runway.is_empty()
            && !matches!(s.session.phase,openatc_core::state::PhaseCode::Departure|openatc_core::state::PhaseCode::Approach|openatc_core::state::PhaseCode::Landed)
            && s.session.mission.incident.is_empty();
        let facts = json!({"recipient":role,"onGround":s.session.telemetry.on_ground,"phase":format!("{:?}",s.session.phase),
            "passengers":s.session.plan.passengers,"conversationNotes":s.crew.memories.get(role),"humourAllowed":humour_allowed});
        let mut prompt = format!("{instructions}\n{}", interpretation_context.replace("{{controls}}", &json!(allowed).to_string()).replace("{{facts}}", &facts.to_string()));
        drop(s);
        while crate::llm::prompt_fits(&client, &settings.ai_url, &prompt, &request.text).await == Some(false) {
            if allowed.len() > 8 {
                allowed.truncate(allowed.len() / 2);
            } else {
                let mut state = shared.write().await;
                return Some(reply(&mut state, role, "crew_action_clarify", &[], false));
            }
            prompt = format!("{instructions}\n{}", interpretation_context.replace("{{controls}}", &json!(allowed).to_string()).replace("{{facts}}", &facts.to_string()));
        }
        let control_schema = if allowed.is_empty() { json!({"type":"string"}) } else {json!({"type":"string","enum":allowed.iter().filter_map(|control|control["control"].as_str()).collect::<Vec<_>>()})};
        let schema = json!({"type":"object","additionalProperties":false,
            "required":["actions","cross_check","conversation_reply","memory"],"properties":{
                "cross_check":{"type":"boolean"},"conversation_reply":{"type":"string"},"memory":{"type":"string","maxLength":700},
                "actions":{"type":"array","maxItems":8,"items":{"type":"object","additionalProperties":false,"required":["control","value"],
                    "properties":{"control":control_schema,"value":{"type":"number"}}}}
            }});
        let intent = crate::llm::structured_format::<openatc_core::crew::request::Intent>(
            &client,
            &settings.ai_url,
            &settings.ai_model,
            if settings.llm_phrase_variety {0.6} else {0.2},
            &prompt,
            &request.text,
            Some(schema),
        )
        .await
        .inspect_err(|error| eprintln!("Crew interpretation failed for {role}: {error}"))
        .ok()
        .map(|mut intent| {
            intent.actions.extend(std::mem::take(&mut intent.control_targets).into_iter().filter_map(|(control,value)|value.map(|value|openatc_core::crew::request::ControlAction{control,value})));
            // Repeating an identical proposal must not discard the rest of a mixed request.
            let mut unique = Vec::new();
            for action in std::mem::take(&mut intent.actions) {
                if !unique.iter().any(|previous: &openatc_core::crew::request::ControlAction| previous.control == action.control && previous.value == action.value) {
                    unique.push(action);
                }
            }
            intent.actions = unique;
            intent.conversation_only = intent.actions.is_empty() && !intent.cross_check;
            intent
        });

        if std::env::var_os("OPENATC_ACTOR_TRACE").is_some() { eprintln!("Crew interpretation: {:?}", intent); }
        let intent = intent.filter(|intent| intent.valid_controls(&profile, role)
            && intent.conversation_points.len() <= 16
            && intent.conversation_points.iter().map(String::len).sum::<usize>() <= 8192);
        let mut vetoed = false;
        let intent = if let Some(mut intent) = intent {
            if !intent.actions.is_empty() || intent.cross_check {
                let mut targets = intent.actions.iter().map(|action|(action.control.clone(),json!({"label":profile.controls[&action.control].label,"value":action.value}))).collect::<serde_json::Map<_,_>>();
                if intent.cross_check {targets.insert("@cross_check".into(),json!("Verify cabin door/slide arming indications"));}
                let properties=targets.keys().map(|id|(id.clone(),json!({"type":"boolean"}))).collect::<serde_json::Map<_,_>>();
                let schema=json!({"type":"object","additionalProperties":false,"required":targets.keys().collect::<Vec<_>>(),"properties":properties});
                let current=json!({"currentRequest":request.text,"targets":targets}).to_string();
                match crate::llm::structured_format::<BTreeMap<String,bool>>(&client,&settings.ai_url,&settings.ai_model,0.0,&authority_prompt,&current,Some(schema)).await {
                    Ok(approved) => {
                        vetoed = intent.actions.iter().any(|action|approved.get(&action.control)!=Some(&true)) || (intent.cross_check && approved.get("@cross_check")!=Some(&true));
                        intent.actions.retain(|action|approved.get(&action.control)==Some(&true));
                        intent.cross_check &= approved.get("@cross_check")==Some(&true);
                        // Repeating an identical proposal must not discard the rest of a mixed request.
            let mut unique = Vec::new();
            for action in std::mem::take(&mut intent.actions) {
                if !unique.iter().any(|previous: &openatc_core::crew::request::ControlAction| previous.control == action.control && previous.value == action.value) {
                    unique.push(action);
                }
            }
            intent.actions = unique;
            intent.conversation_only = intent.actions.is_empty() && !intent.cross_check;
                    }
                    Err(error) => {eprintln!("Crew control request could not be checked: {error}");let mut state=shared.write().await;return Some(reply(&mut state,role,"crew_action_clarify",&[],false));}
                }
            }
            Some(intent)
        } else {None};
        s = shared.write().await;
        if s.session_generation != generation || s.crew.aircraft != aircraft {
            return Some(reply(&mut s, role, "crew_action_clarify", &[], false));
        }
        let Some(mut intent) = intent else {
            return Some(reply(&mut s, role, "crew_action_clarify", &[], false));
        };
        if intent.actions.is_empty() && !intent.cross_check {
            if vetoed || intent.reply.trim().is_empty() {
                drop(s);
                intent.reply=compose(shared,role,&request.text,&intent.reply,&[],&[],false).await;
                s=shared.write().await;
                if s.session_generation != generation || s.crew.aircraft != aircraft {return Some(reply(&mut s,role,"crew_action_clarify",&[],false));}
            }
        if !intent.memory.is_empty() && intent.memory.chars().count() <= 700 && crate::llm::spoken(&intent.memory).is_ok() {
            s.crew.memories.insert(role.to_owned(), intent.memory.clone());
        }
            if crate::llm::spoken(&intent.reply).is_ok() && intent.reply.len() <= 3000 {
                speak(&mut s, role, &intent.reply);
                return Some(Json(json!({"result":{"accepted":true,"message":intent.reply},"state":snapshot(&s.session,&s.settings)})).into_response());
            }
            return Some(reply(&mut s, role, "crew_action_clarify", &[], false));
        }
        if intent.reply.trim().is_empty() && !intent.conversation_points.is_empty() {
            drop(s);
            intent.reply = compose(shared, role, &request.text, &intent.reply, &[], &intent.conversation_points, true).await;
            s = shared.write().await;
            if s.session_generation != generation || s.crew.aircraft != aircraft {
                return Some(reply(&mut s, role, "crew_action_clarify", &[], false));
            }
        }
        if !intent.memory.is_empty() && intent.memory.chars().count() <= 700 && crate::llm::spoken(&intent.memory).is_ok() {
            s.crew.memories.insert(role.to_owned(), intent.memory.clone());
        }
        Some(intent)
    } else {
        openatc_core::crew::request::parse(&profile, role, &text)
    };
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
    if !using_ai {
        pilot_sequence = s.session.next_sequence;
        add_transmission(&mut s.session, &pilot, &request.text, &SpeechTag::default());
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
    if !using_ai && (text.contains("don't ") || text.contains("do not ") || text.contains("never ")) {
        return Some(reply(&mut s, role, "crew_no_action", &[], true));
    }
    let generation = s.session_generation;
    let aircraft = s.crew.aircraft.clone();
    drop(s);
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
            held_reply: intent.reply.clone(),
            failed: false,
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
    async fn refreshed_observations_allow_actions_after_slow_inference() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/v1/chat/completions", post(|Json(body):Json<serde_json::Value>| async move {
                if let Some(properties)=body.pointer("/response_format/json_schema/schema/properties").and_then(serde_json::Value::as_object)
                    && properties.values().all(|value|value["type"]=="boolean") {
                    let approved=properties.keys().map(|key|(key.clone(),json!(true))).collect::<serde_json::Map<_,_>>();
                    return Json(json!({"choices":[{"message":{"content":json!(approved).to_string()}}]}));
                }
                tokio::time::sleep(std::time::Duration::from_millis(3200)).await;
                Json(json!({"choices":[{"message":{"content":json!({"reply":"I'll take care of both. Coffee when you're back sounds good!","actions":[{"control":"external_power","value":0},{"control":"chocks","value":0}],"cross_check":false,"memory":"Captain would like to catch up after the flight."}).to_string()}}]}))
            }))).await.unwrap();
        });
        let catalogue: toml::Value = toml::from_str(include_str!("../../../aircraft/toliss_a320.toml")).unwrap();
        let profile: CrewProfile = serde_json::from_value(serde_json::to_value(&catalogue["aircraft"]["crew"]).unwrap()).unwrap();
        let available = profile.controls.keys().cloned().collect::<Vec<_>>();
        let mut state = crate::test_state();
        state.settings.ai_enabled = true;
        state.settings.ai_model = "test".into();
        state.settings.ai_url = format!("http://{address}");
        state.session.telemetry.position_valid = true;
        state.session.telemetry.on_ground = true;
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(state));
        observe(AxumState(shared.clone()), Json(Observation {aircraft:"test".into(),profile:Some(profile),values:BTreeMap::new(),available:available.clone()})).await;
        let observed = shared.clone();
        let sampling = tokio::spawn(async move {
            loop {
                observe(AxumState(observed.clone()), Json(Observation {aircraft:"test".into(),profile:None,values:BTreeMap::new(),available:available.clone()})).await;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        });
        let response = request(&shared, &Request {role:"ground".into(),intent:"conversation".into(),text:"Hey buddy, disconnect external power and remove chocks. Coffee when I return?".into(),..Default::default()}).await.unwrap();
        assert!(response.status().is_success());
        let state = shared.read().await;
        assert_eq!(state.session.crew_actions.len(),2);
        assert!(state.session.transcript[0].text.contains("Hey buddy"));
        sampling.abort();
        server.abort();
    }

    #[tokio::test]
    async fn a_controls_wait_reply_keeps_the_captains_request_in_the_log() {
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(crate::test_state()));
        let text = "disconnect external power and remove chocks please";
        let response = request(&shared, &Request {role:"ground".into(),intent:"conversation".into(),text:text.into(),..Default::default()}).await.unwrap();
        assert!(response.status().is_success());
        let state = shared.read().await;
        assert_eq!(state.session.transcript.len(),2);
        assert_eq!(state.session.transcript[0].text,text);
        assert_eq!(state.session.transcript[1].speaker,"GROUND");
        assert!(state.session.crew_actions.is_empty());
    }


    #[tokio::test]
    async fn polite_joke_request_is_conversation_without_pushback_or_control_actions() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let generations = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls = generations.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/v1/chat/completions", post(move |Json(body):Json<serde_json::Value>| { let calls = calls.clone(); async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let classifier = body["messages"][0]["content"].as_str().unwrap().contains("Allowed controls:");
                let content = if classifier { json!({"reply":"Here is a quick one: why did the biscuit visit the doctor? It felt crummy!","actions":[],"cross_check":false,"memory":"Captain enjoys biscuit jokes."}).to_string() }
                    else if body.get("response_format").is_some() { json!({"reply":"Here's a quick one: why did the biscuit visit the doctor? It felt crummy!"}).to_string() }
                    else { "Here's a quick one: why did the biscuit visit the doctor? It felt crummy!".into() };
                Json(json!({"choices":[{"message":{"content":content}}]}))
            }}))).await.unwrap();
        });
        let mut state = crate::test_state();
        state.settings.ai_enabled = true;
        state.settings.ai_model = "test".into();
        state.settings.llm_phrase_variety = false;
        state.settings.ai_url = format!("http://{address}");
        state.session.phase = openatc_core::state::PhaseCode::Pushback;
        state.crew.observed_at = None;
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
            shared.write().await.crew.observed_at = None;
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
        assert_eq!(generations.load(std::sync::atomic::Ordering::SeqCst), 4);
        let state = shared.read().await;
        for role in ["cabin", "copilot", "ground"] {
            assert_eq!(state.crew.memories.get(role).unwrap(), "Captain enjoys biscuit jokes.");
        }
        server.abort();
    }
}

#[cfg(test)]
mod callout_log_tests {
    use super::*;
    #[tokio::test]
    async fn preloaded_calls_are_logged_without_replacing_radio_instructions() {
        let state = crate::test_state();
        let callsign = state.session.plan.callsign.clone();
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(state));
        callout_log(AxumState(shared.clone()), Json(CalloutLog { callsign, entries: vec!["V1 · 127 kt".into(),"Rotate · 146 kt".into()] })).await;
        let state = shared.read().await;
        assert_eq!(state.session.transcript.len(),2);
        assert!(state.session.transcript.iter().all(|entry| entry.speaker == "COPILOT" && entry.delivery == "preloaded-callout" && !entry.is_controller_instruction()));
        assert!(state.session.crew_actions.is_empty());
    }
}
