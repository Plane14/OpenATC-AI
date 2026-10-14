//! Editable deterministic operational speech, separate from AI example selection.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Template {
    slots: Vec<String>,
    say: Vec<String>,
    #[serde(default)]
    match_any: Vec<String>,
    #[serde(default)]
    match_priority: i32,
    #[serde(default)]
    humour: bool,
    #[serde(default)]
    fallback: bool,
    #[serde(default)]
    when: BTreeMap<String, serde_json::Value>,
}
type Templates = BTreeMap<String, Template>;

/// Select a conversation response using editable matching and humour rules.
#[must_use]
pub fn conversation_response(prefix: &str, text: &str, humour: bool) -> Option<String> {
    matched_response(prefix, text, humour, &BTreeMap::new()).map(|key| say(&key, &[]))
}

/// Find a response whose editable conditions match observed facts.
#[must_use]
pub fn matched_response(
    prefix: &str,
    text: &str,
    humour: bool,
    facts: &BTreeMap<String, serde_json::Value>,
) -> Option<String> {
    let lock =
        TEMPLATES.get_or_init(|| RwLock::new(parse(DEFAULTS).expect("shipped dialogue TOML")));
    let templates = lock.read().ok()?;
    select_response(&templates, prefix, text, humour, facts)
}

fn select_response(
    templates: &Templates,
    prefix: &str,
    text: &str,
    humour: bool,
    facts: &BTreeMap<String, serde_json::Value>,
) -> Option<String> {
    let words = |value: &str| {
        format!(
            " {} ",
            value
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    let text = words(text);
    let selected = {
        templates
            .iter()
            .filter(|(key, template)| {
                key.starts_with(prefix)
                    && (!template.humour || humour)
                    && template.when.iter().all(|(fact, condition)| {
                        facts
                            .get(fact)
                            .is_some_and(|value| condition_matches(condition, value))
                    })
            })
            .filter(|(_, template)| {
                template.fallback
                    || (template.match_any.is_empty() && !template.when.is_empty())
                    || template
                        .match_any
                        .iter()
                        .any(|phrase| !phrase.trim().is_empty() && text.contains(&words(phrase)))
            })
            .max_by_key(|(_, template)| (!template.fallback, template.match_priority))
            .map(|(key, _)| key.clone())
    };
    selected
}

fn condition_matches(condition: &serde_json::Value, value: &serde_json::Value) -> bool {
    if let Some(options) = condition.as_array() {
        return options.contains(value);
    }
    if let Some(range) = condition.as_object() {
        let Some(number) = value.as_f64() else {
            return false;
        };
        return !range.is_empty()
            && range.iter().all(|(key, bound)| {
                bound.as_f64().is_some_and(|bound| match key.as_str() {
                    "min" => number >= bound,
                    "max" => number <= bound,
                    _ => false,
                })
            });
    }
    condition == value
}
const DEFAULTS: &str = include_str!("../../../speech/runtime/responses.toml");
static EDITED: AtomicBool = AtomicBool::new(false);
static PHRASE_CHOICES: OnceLock<Mutex<BTreeMap<String, usize>>> = OnceLock::new();
static PLACEHOLDERS: OnceLock<regex::Regex> = OnceLock::new();
static TEMPLATES: OnceLock<RwLock<Templates>> = OnceLock::new();

fn parse(text: &str) -> Result<Templates, String> {
    let templates: Templates = toml::from_str(text).map_err(|e| e.to_string())?;
    let placeholders = regex::Regex::new(r"\{([a-z_0-9]+)\}").map_err(|e| e.to_string())?;
    for (id, entry) in &templates {
        if entry.when.len() > 16
            || entry.match_any.len() > 32
            || entry
                .match_any
                .iter()
                .any(|phrase| phrase.trim().is_empty() || phrase.len() > 160)
            || entry.when.values().any(|condition| {
                condition.as_object().is_some_and(|range| {
                    range.is_empty()
                        || range.iter().any(|(key, value)| {
                            !["min", "max"].contains(&key.as_str()) || value.as_f64().is_none()
                        })
                }) || condition.as_array().is_some_and(Vec::is_empty)
            })
        {
            return Err(format!(
                "{id}: invalid or excessive response matching conditions"
            ));
        }
        if entry.say.is_empty() || entry.say.iter().any(|s| s.trim().is_empty()) {
            return Err(format!("{id}: say must contain non-empty phrases"));
        }
        for line in &entry.say {
            let used: std::collections::BTreeSet<_> = placeholders
                .captures_iter(line)
                .map(|c| c[1].to_owned())
                .collect();
            let declared: std::collections::BTreeSet<_> = entry.slots.iter().cloned().collect();
            if used != declared || placeholders.replace_all(line, "").contains(['{', '}']) {
                return Err(format!(
                    "{id}: each phrase must use exactly the declared placeholders"
                ));
            }
        }
    }
    Ok(templates)
}

/// Validate all required response IDs and slot contracts against the shipped TOML.
pub fn validate(path: &Path) -> Result<(), String> {
    let edited =
        parse(&std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?)?;
    let defaults = parse(DEFAULTS)?;
    for (id, required) in defaults {
        let Some(entry) = edited.get(&id) else {
            return Err(format!("missing response {id}"));
        };
        if entry.slots != required.slots {
            return Err(format!("{id}: required placeholders changed"));
        }
    }
    Ok(())
}

/// Load edits at engine startup; invalid libraries cannot silently fall back to code wording.
pub fn load(path: &Path) -> Result<(), String> {
    validate(path)?;
    let edited = parse(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)?;
    let lock = TEMPLATES.get_or_init(|| RwLock::new(BTreeMap::new()));
    *lock.write().map_err(|e| e.to_string())? = edited;
    EDITED.store(true, Ordering::Relaxed);
    Ok(())
}

fn next_phrase(choices: &mut BTreeMap<String, usize>, id: &str, count: usize) -> usize {
    let next = choices.entry(id.to_owned()).or_default();
    let pick = *next % count;
    *next = pick + 1;
    pick
}

/// Render one operational response; only the caller supplies operational facts.
///
/// # Panics
/// Panics only if the compiled-in TOML or placeholder pattern is invalid.
#[must_use]
pub fn say(id: &str, slots: &[(&str, String)]) -> String {
    let lock =
        TEMPLATES.get_or_init(|| RwLock::new(parse(DEFAULTS).expect("shipped dialogue TOML")));
    let Ok(templates) = lock.read() else {
        return String::new();
    };
    let Some(entry) = templates.get(id) else {
        return String::new();
    };
    let pick = if EDITED.load(Ordering::Relaxed) {
        let mut choices = PHRASE_CHOICES
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .expect("phrase choices");
        next_phrase(&mut choices, id, entry.say.len())
    } else {
        0
    };
    let Some(line) = entry.say.get(pick) else {
        return String::new();
    };
    if entry
        .slots
        .iter()
        .any(|name| !slots.iter().any(|(key, _)| key == name))
    {
        return String::new();
    }
    PLACEHOLDERS
        .get_or_init(|| regex::Regex::new(r"\{([a-z_0-9]+)\}").expect("placeholder pattern"))
        .replace_all(line, |captures: &regex::Captures<'_>| {
            slots
                .iter()
                .find(|(key, _)| *key == &captures[1])
                .map_or_else(String::new, |(_, value)| value.clone())
        })
        .into_owned()
}

/// All editable phrases for request matching or prompt examples.
#[must_use]
pub fn phrases(id: &str) -> Vec<String> {
    let lock =
        TEMPLATES.get_or_init(|| RwLock::new(parse(DEFAULTS).expect("shipped dialogue TOML")));
    lock.read()
        .ok()
        .and_then(|templates| templates.get(id).map(|entry| entry.say.clone()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editable_matching_conditions_priorities_and_humour_select_responses() {
        let templates = parse("[chat_default]\nslots=[]\nsay=['Hello']\nfallback=true\n[chat_cuppa]\nslots=[]\nsay=['Thanks for the cuppa']\nmatch_any=['cuppa']\nmatch_priority=20\nhumour=true\nwhen={phase=['cruise'],temperature={min=10,max=30}}\n").unwrap();
        let facts = BTreeMap::from([
            ("phase".into(), serde_json::json!("cruise")),
            ("temperature".into(), serde_json::json!(20)),
        ]);
        assert_eq!(
            select_response(&templates, "chat_", "Fancy a cuppa?", true, &facts).as_deref(),
            Some("chat_cuppa")
        );
        assert_eq!(
            select_response(&templates, "chat_", "Fancy a cuppa?", false, &facts).as_deref(),
            Some("chat_default")
        );
        assert_eq!(
            select_response(
                &templates,
                "chat_",
                "Fancy a cuppa?",
                true,
                &BTreeMap::new()
            )
            .as_deref(),
            Some("chat_default")
        );
        assert_eq!(
            select_response(&templates, "chat_", "cuppaboard", true, &facts).as_deref(),
            Some("chat_default")
        );
        assert!(
            parse("[chat]\nslots=[]\nsay=['Hello']\nwhen={phase={execute='anything'}}").is_err()
        );
    }
    #[test]
    fn unrelated_responses_do_not_change_phrase_rotation() {
        let mut choices = BTreeMap::new();
        assert_eq!(next_phrase(&mut choices, "clearance", 3), 0);
        for _ in 0..5 {
            next_phrase(&mut choices, "radio_check", 4);
        }
        assert_eq!(next_phrase(&mut choices, "clearance", 3), 1);
        assert_eq!(next_phrase(&mut choices, "clearance", 3), 2);
        assert_eq!(next_phrase(&mut choices, "clearance", 3), 0);
    }
    #[test]
    fn shipped_templates_validate_and_reject_missing_facts() {
        let templates = parse(DEFAULTS).unwrap();
        assert!(templates.len() >= 120);
        assert!(parse("[example]\nslots=['runway']\nsay=['Taxi approved.']").is_err());
        assert!(
            parse("[example]\nslots=['runway']\nsay=['Hold short of {runway}.','Hold short.']")
                .is_err()
        );
        assert!(parse("[example]\nslots=[]\nsay=['{unknown}']").is_err());
    }
    #[test]
    fn response_values_do_not_interpolate_each_other() {
        let rendered = say(
            "copilot_repeat_instruction",
            &[
                ("instruction", "Keep {callsign} literal".into()),
                ("callsign", "VH-BIL".into()),
            ],
        );
        assert!(rendered.contains("Keep {callsign} literal"));
    }
}
