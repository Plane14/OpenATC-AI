//! Engine support: config paths, atomic saves, prompt loading, upstream HTTP.

use std::path::{Path, PathBuf};

/// Role prompts with file-loaded overrides and shipped fallbacks.
pub struct Prompts {
    /// Shared framing and context instructions loaded from TOML.
    pub guidance: Guidance,
    /// Wording rules for an already validated ATC decision.
    pub atc: String,
    /// Structured interpretation of multi-part crew requests.
    pub crew_actions: String,
    /// Conversation grounded in an already verified crew result.
    pub crew_response: String,
    /// Cabin crew system prompt.
    pub cabin: String,
    /// Ground crew system prompt.
    pub ground: String,
    /// Copilot readback prompt.
    pub copilot: String,
    /// Intent classifier prompt.
    pub classify: String,
    /// Copilot chat prompt.
    pub copilot_chat: String,
    /// Directory the files came from, or built-in.
    pub source: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PromptFile {
    atc_reply: String,
    crew_actions: String,
    #[serde(default = "default_crew_response")]
    crew_response: String,
    cabin: String,
    ground: String,
    copilot_readback: String,
    intent_classify: String,
    copilot_chat: String,
}

fn default_crew_response() -> String {
    include_str!("../../../prompts/crew_response.txt").to_owned()
}

impl PromptFile {
    fn into_prompts(self, source: String) -> Result<Prompts, String> {
        if [
            &self.atc_reply,
            &self.crew_actions,
            &self.crew_response,
            &self.cabin,
            &self.ground,
            &self.copilot_readback,
            &self.intent_classify,
            &self.copilot_chat,
        ]
        .iter()
        .any(|text| text.trim().is_empty() || text.len() > 65536)
        {
            return Err("Role prompts must be nonempty and at most 64 KiB each".into());
        }
        Ok(Prompts {
            guidance: Guidance::default(),
            atc: self.atc_reply,
            crew_actions: self.crew_actions,
            crew_response: self.crew_response,
            cabin: self.cabin,
            ground: self.ground,
            copilot: self.copilot_readback,
            classify: self.intent_classify,
            copilot_chat: self.copilot_chat,
            source,
        })
    }
}

/// Read a prompt file and trim trailing newlines.
fn read_prompt_file(dir: &Path, name: &str) -> String {
    let mut text = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
    while text.ends_with('\n') || text.ends_with('\r') {
        text.pop();
    }
    text
}

/// Replace every `{{callsign}}` placeholder.
#[must_use]
pub fn fill_prompt(text: &str, callsign: &str) -> String {
    text.replace("{{callsign}}", callsign)
}

/// Load prompts: env override, then config dir, then exe-relative, else the
/// shipped texts compiled in. A directory only wins when it holds cabin.txt.
#[must_use]
pub fn load_prompts(config_dir: &Path, exe_dir: &Path) -> Prompts {
    let mut prompts = toml::from_str::<PromptFile>(include_str!("../../../prompts/prompts.toml"))
        .expect("bundled role prompts")
        .into_prompts("built-in".into())
        .expect("valid bundled prompts");
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("OPENATC_PROMPTS_DIR") {
        candidates.push(PathBuf::from(dir));
    }
    candidates.push(config_dir.join("prompts"));
    candidates.push(exe_dir.join("../prompts"));
    for candidate in candidates {
        if let Ok(text) = std::fs::read_to_string(candidate.join("prompts.toml")) {
            match toml::from_str::<PromptFile>(&text)
                .map_err(|error| error.to_string())
                .and_then(|file| file.into_prompts(candidate.to_string_lossy().into_owned()))
            {
                Ok(mut loaded) => {
                    loaded.guidance = Guidance::load(&candidate.join("interaction.toml"));
                    return loaded;
                }
                Err(error) => eprintln!(
                    "Invalid role prompt catalogue {}: {error}",
                    candidate.display()
                ),
            }
            continue;
        }
        if read_prompt_file(&candidate, "cabin.txt").is_empty() {
            continue;
        }
        prompts.source = candidate.to_string_lossy().into_owned();
        prompts.guidance = Guidance::load(&candidate.join("interaction.toml"));
        for (name, target) in [
            ("atc_reply.txt", &mut prompts.atc),
            ("crew_actions.txt", &mut prompts.crew_actions),
            ("crew_response.txt", &mut prompts.crew_response),
            ("cabin.txt", &mut prompts.cabin),
            ("ground.txt", &mut prompts.ground),
            ("copilot_readback.txt", &mut prompts.copilot),
            ("copilot_chat.txt", &mut prompts.copilot_chat),
            ("intent_classify.txt", &mut prompts.classify),
        ] {
            let text = read_prompt_file(&candidate, name);
            if !text.is_empty() {
                *target = text;
            }
        }
        break;
    }
    prompts
}

/// Resolve the speech library dir: env override, then exe-relative.
#[must_use]
pub fn resolve_speech_dir(config_dir: &Path, exe_dir: &Path) -> String {
    if let Ok(override_path) = std::env::var("OPENATC_SPEECH_DIR") {
        return override_path;
    }
    for candidate in [config_dir.join("speech"), exe_dir.join("../speech")] {
        if candidate.is_dir() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    String::new()
}

/// Resolve the regions file: env override, then config dir, then exe-relative.
#[must_use]
pub fn resolve_regions_file(config_dir: &Path, exe_dir: &Path) -> String {
    if let Ok(override_path) = std::env::var("OPENATC_REGIONS_FILE") {
        return override_path;
    }
    for candidate in [
        config_dir.join("regions.toml"),
        exe_dir.join("../regions.toml"),
    ] {
        if candidate.is_file() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    String::new()
}

/// Write JSON atomically through a temp file plus rename.
pub fn save_json(path: &Path, data: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "Cannot save configuration".to_owned())?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_string_pretty(data).unwrap_or_default(),
    )
    .map_err(|_| "Cannot save configuration".to_owned())?;
    std::fs::rename(&temporary, path).map_err(|_| "Cannot save configuration".to_owned())
}

/// Bearer header from an env key, absent when unset.
/// GET a upstream /health with short timeouts.
pub async fn service_ok(client: &reqwest::Client, url: &str) -> bool {
    if url.is_empty() {
        return false;
    }
    client
        .get(format!("{url}/health"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .is_ok_and(|reply| reply.status().is_success())
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn toml_prompts_preserve_role_instructions_and_reject_empty_overrides() {
        let text = include_str!("../../../prompts/prompts.toml");
        let file: PromptFile = toml::from_str(text).unwrap();
        let prompts = file.into_prompts("test".into()).unwrap();
        assert_eq!(prompts.atc, include_str!("../../../prompts/atc_reply.txt"));
        assert_eq!(
            prompts.crew_actions,
            include_str!("../../../prompts/crew_actions.txt")
        );
        assert_eq!(
            prompts.crew_response,
            include_str!("../../../prompts/crew_response.txt")
        );
        assert_eq!(prompts.cabin, include_str!("../../../prompts/cabin.txt"));
        assert_eq!(prompts.ground, include_str!("../../../prompts/ground.txt"));
        assert_eq!(
            prompts.copilot,
            include_str!("../../../prompts/copilot_readback.txt")
        );
        assert_eq!(
            prompts.classify,
            include_str!("../../../prompts/intent_classify.txt")
        );
        assert_eq!(
            prompts.copilot_chat,
            include_str!("../../../prompts/copilot_chat.txt")
        );
        let mut file: PromptFile = toml::from_str(text).unwrap();
        file.cabin.clear();
        assert!(file.into_prompts("bad".into()).is_err());
        assert!(toml::from_str::<PromptFile>(&format!("{text}\nunknown = true")).is_err());
    }
}

/// Shared instruction strings; missing fields retain bundled TOML defaults.
pub struct Guidance(std::collections::BTreeMap<String, String>);
impl Default for Guidance {
    fn default() -> Self {
        Self(
            toml::from_str(include_str!("../../../prompts/interaction.toml"))
                .expect("bundled interaction instructions"),
        )
    }
}
impl Guidance {
    fn load(path: &Path) -> Self {
        let mut result = Self::default();
        let Ok(text) = std::fs::read_to_string(path) else {
            return result;
        };
        let overrides = toml::from_str::<std::collections::BTreeMap<String, String>>(&text);
        match overrides {
            Ok(values)
                if text.len() <= 65536
                    && values.iter().all(|(key, value)| {
                        result.0.contains_key(key)
                            && !value.trim().is_empty()
                            && value.len() <= 16384
                    }) =>
            {
                result.0.extend(values)
            }
            _ => eprintln!("Invalid shared prompt instructions: {}", path.display()),
        }
        result
    }
    pub fn get(&self, key: &str) -> &str {
        self.0.get(key).expect("known shared prompt instruction")
    }
}

#[cfg(test)]
mod guidance_tests {
    use super::*;
    #[test]
    fn editable_framing_loads_without_requiring_a_complete_new_catalogue() {
        let directory =
            std::env::temp_dir().join(format!("openatc-guidance-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("interaction.toml");
        std::fs::write(&path, "examples = 'Custom example guidance'\n").unwrap();
        let loaded = Guidance::load(&path);
        assert_eq!(loaded.get("examples"), "Custom example guidance");
        assert!(!loaded.get("crew_context").is_empty());
        assert_eq!(
            Guidance::default().get("speech_rules"),
            include_str!("../../../prompts/speech_rules.txt").trim()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
