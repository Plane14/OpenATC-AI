//! Interpret a whole crew request before queuing any aircraft controls.
use crate::crew::{CrewProfile, parse_many, validate};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// All parts of one pilot request, validated together before execution.
pub struct Intent {
    /// Spoken conversation from the role, held separately from executable controls.
    pub reply: String,
    /// Compact conversational continuity; never authoritative aircraft state.
    pub memory: String,
    /// Aircraft controls in execution order.
    pub actions: Vec<ControlAction>,
    /// Verify slide indications after any requested change.
    pub cross_check: bool,
    /// Legacy hint; validated actions determine whether execution is needed.
    pub conversation_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
/// A target selected from the loaded aircraft profile.
pub struct ControlAction {
    /// Profile control identifier, never a raw simulator dataref.
    pub control: String,
    /// Requested target in the control's declared units.
    pub value: f64,
}

impl Intent {
    /// Enforce role, range, count and duplicate-control limits.
    pub fn valid(&self, profile: &CrewProfile, role: &str) -> bool {
        if self.actions.is_empty() && !self.cross_check {
            return !self.reply.trim().is_empty() || self.conversation_only;
        }
        if self.actions.len() > 8
            || (role != "cabin" && self.cross_check)
        {
            return false;
        }
        self.actions.iter().enumerate().all(|(i, action)| {
            validate(profile, role, &action.control, action.value)
                && !self.actions[..i]
                    .iter()
                    .any(|a| a.control == action.control)
        })
    }
}

/// Reply checks are separate from language understanding and control mappings.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyPolicy {
    version: u32,
    narration_reject: Vec<String>,
}
impl ReplyPolicy {
    /// Load editable response constraints, falling back to bundled policy.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                include_str!("../../../cabin/response-policy.toml").into()
            }
            Err(error) => return Err(error.to_string()),
        };
        if text.len() > 16384 {
            return Err("Reply policy is too large".into());
        }
        let policy: Self = toml::from_str(&text).map_err(|error| error.to_string())?;
        if policy.version != 1 || policy.narration_reject.len() > 16 {
            return Err("Unsupported reply policy".into());
        }
        for pattern in &policy.narration_reject {
            regex::Regex::new(pattern).map_err(|error| error.to_string())?;
        }
        Ok(policy)
    }
    /// Keep role replies from issuing aircraft movement permissions.
    pub fn narration_allowed(&self, text: &str) -> bool {
        self.narration_reject
            .iter()
            .all(|pattern| regex::Regex::new(pattern).is_ok_and(|pattern| !pattern.is_match(text)))
    }
}

/// Explicit mapped-control fallback when role conversations are disabled.
pub fn parse(profile: &CrewProfile, role: &str, text: &str) -> Option<Intent> {
    let intent = Intent {
        actions: parse_many(profile, role, text)?
            .into_iter()
            .map(|(control, value)| ControlAction { control, value })
            .collect(),
        ..Default::default()
    };
    intent.valid(profile, role).then_some(intent)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversation_has_no_menu_or_control_side_effects() {
        let profile = CrewProfile::default();
        let intent: Intent =
            serde_json::from_str(r#"{"actions":[],"cross_check":false,"conversation_only":true}"#)
                .unwrap();
        assert!(intent.valid(&profile, "cabin"));
        let reply: Intent = serde_json::from_str(r#"{"reply":"I'll check the galley for you.","actions":[]}"#).unwrap();
        assert!(reply.valid(&profile, "cabin"));
        assert!(parse(&profile, "cabin", "coffee and a sandwich please").is_none());
        assert!(serde_json::from_str::<Intent>(r#"{"services":["dessert","something to drink"]}"#).is_err());
        let invalid: Intent = serde_json::from_str(
            r#"{"actions":[{"control":"unlisted","value":1}],"conversation_only":true}"#,
        )
        .unwrap();
        assert!(!invalid.valid(&profile, "cabin"));
    }
}
