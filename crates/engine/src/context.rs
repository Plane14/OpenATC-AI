//! Shared flight context for request interpretation and crew conversation.
use crate::EngineState;
use serde_json::json;

pub fn prompt(state: &EngineState, role: &str, request: &str) -> String {
    let role = if role == "attendant" { "cabin" } else { role };
    let mut plan = state.session.plan.clone();
    plan.fixes = plan
        .fixes
        .into_iter()
        .skip(state.flow.monitor.next_fix.saturating_sub(1))
        .take(8)
        .collect();
    let fresh = state
        .crew
        .observed_at
        .is_some_and(|at| at.elapsed().as_secs() < 5);
    let words: Vec<_> = request
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(str::to_owned)
        .collect();
    let mut controls: Vec<_> = state
        .session
        .crew_controls
        .iter()
        .filter(|c| c.role == role || words.iter().any(|w| c.label.to_lowercase().contains(w)))
        .collect();
    controls.sort_by_key(|c| {
        std::cmp::Reverse(
            words
                .iter()
                .filter(|w| c.label.to_lowercase().contains(w.as_str()))
                .count(),
        )
    });
    let controls:Vec<_>=controls.into_iter().take(40).map(|c|json!({"id":c.id,"label":c.label,"role":c.role,"available":c.available&&fresh,"value":if fresh{c.value}else{None}})).collect();
    let owners: Vec<_> = state
        .stations
        .iter()
        .filter(|s| {
            s.coverage
                .iter()
                .any(|v| v.contains(&state.session.telemetry))
        })
        .take(8)
        .map(|s| json!({"station":s.name,"service":s.service,"frequencyKhz":s.khz}))
        .collect();
    if role == "atc" {
        let recent: Vec<_> = state
            .session
            .transcript
            .iter()
            .rev()
            .filter(|t| {
                !t.background
                    && (t.speaker == "ATC"
                        || t.speaker == state.session.plan.callsign
                        || t.speaker == "COPILOT")
            })
            .take(6)
            .collect();
        return format!(
            "{}",
            state.prompts.guidance.get("atc_context").replace("{{facts}}", &json!({"role":role,"phase":state.session.phase,"telemetry":state.session.telemetry,
                "plan":plan,"clearance":state.session.clearance,"taxiClearance":state.session.taxi_clearance,
                "controllers":owners,"recentTransmissionsNewestFirst":recent}).to_string())
        );
    }
    let recent = conversation(state, role, 6, 1024);
    format!(
        "{}",
        state.prompts.guidance.get("crew_context").replace("{{facts}}", &json!({
            "role":role,"recentConversationNewestFirst":recent,"phase":state.session.phase,"telemetry":state.session.telemetry,
            "plan":plan,"clearance":state.session.clearance,
            "taxiClearance":state.session.taxi_clearance,"controllers":owners,
            "controlsFresh":fresh,"controls":controls,"pendingActions":state.session.crew_actions,
            "requestMemory":state.request_memory.context(role),
            "cabinServiceRequestCounts":state.crew.service_requests,
            "simulatedCabin":state.cabin,
            "mission":state.session.mission,
        }).to_string())
    )
}

/// Select exchanges for one recipient before applying the common history budget.
pub fn conversation(
    state: &EngineState,
    role: &str,
    count: usize,
    reply_characters: usize,
) -> Vec<serde_json::Value> {
    let speaker = match role {
        "cabin" | "attendant" => "ATTENDANT",
        "ground" => "GROUND",
        _ => "COPILOT",
    };
    crate::llm::history(
        state
            .session
            .transcript
            .windows(2)
            .rev()
            .filter(|pair| {
                pair[0].speaker == state.session.plan.callsign && pair[1].speaker == speaker
            })
            .map(|pair| (pair[0].text.as_str(), pair[1].text.as_str())),
        count,
        512,
        reply_characters,
    )
}
