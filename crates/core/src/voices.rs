//! Provider voice names and language metadata used by the voice browser.
use serde::{Deserialize, Serialize};

/// A provider voice and its advertised or recognised language.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    /// Provider identifier sent with synthesis requests.
    pub id: String,
    /// Name shown in the voice browser.
    pub name: String,
    /// Provider language or a known Kokoro language label.
    pub language: String,
    /// Whether the language is English, regardless of accent.
    pub english: bool,
}

/// Identify known voice languages when the provider returns names only.
#[must_use]
pub fn language(id: &str) -> &'static str {
    match id.as_bytes() {
        [b'a', b'f' | b'm', b'_', ..] => "English (US)",
        [b'b', b'f' | b'm', b'_', ..] => "English (UK)",
        [b'e', b'f' | b'm', b'_', ..] => "Spanish",
        [b'f', b'f' | b'm', b'_', ..] => "French",
        [b'h', b'f' | b'm', b'_', ..] => "Hindi",
        [b'i', b'f' | b'm', b'_', ..] => "Italian",
        [b'j', b'f' | b'm', b'_', ..] => "Japanese",
        [b'p', b'f' | b'm', b'_', ..] => "Portuguese",
        [b'z', b'f' | b'm', b'_', ..] => "Mandarin",
        _ if crate::ops::english_voice(id) => "English",
        _ => "Unknown",
    }
}

/// Accept string rosters and provider voice records with language metadata.
/// # Panics
///
/// Panics if a voice identifier contains an underscore without a prefix.
pub fn parse(body: &serde_json::Value) -> Result<Vec<Voice>, String> {
    let list = body
        .get("voices")
        .or_else(|| body.get("data"))
        .unwrap_or(body)
        .as_array()
        .ok_or("Voice API did not return a voice list")?;
    let mut voices = Vec::new();
    for entry in list {
        let id = entry
            .as_str()
            .or_else(|| entry.get("id").and_then(serde_json::Value::as_str))
            .or_else(|| entry.get("voice_id").and_then(serde_json::Value::as_str));
        let Some(id) = id.filter(|id| !id.trim().is_empty()) else {
            continue;
        };
        if voices.iter().any(|v: &Voice| v.id == id) {
            continue;
        }
        let language = entry
            .get("language")
            .and_then(serde_json::Value::as_str)
            .or_else(|| entry.get("locale").and_then(serde_json::Value::as_str))
            .unwrap_or_else(|| language(id))
            .to_owned();
        let lower = language.to_ascii_lowercase();
        let english = lower == "en"
            || lower.starts_with("en-")
            || lower.starts_with("en_")
            || lower.starts_with("english");
        voices.push(Voice {
            id: id.to_owned(),
            name: entry
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map_or_else(
                    || {
                        if super::voices::language(id) != "Unknown" && id.contains('_') {
                            let name = id.split_once('_').unwrap().1;
                            let mut chars = name.chars();
                            chars.next().map_or_else(
                                || id.into(),
                                |c| format!("{}{}", c.to_uppercase(), chars.as_str()),
                            )
                        } else {
                            id.into()
                        }
                    },
                    str::to_owned,
                ),
            language,
            english,
        });
    }
    voices.sort_by(|a, b| a.language.cmp(&b.language).then(a.name.cmp(&b.name)));
    Ok(voices)
}

#[cfg(test)]
mod tests {
    #[test]
    fn accepts_roster_and_provider_metadata() {
        let list = super::parse(&serde_json::json!({"voices":["af_bella","ff_siwis","af_bella"]}))
            .unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().find(|v| v.id == "af_bella").unwrap().english);
        assert!(!list.iter().find(|v| v.id == "ff_siwis").unwrap().english);
        let list = super::parse(
            &serde_json::json!({"data":[{"id":"custom","name":"Anna","language":"en-AU"}]}),
        )
        .unwrap();
        assert!(list[0].english);
        assert!(super::parse(&serde_json::json!({"error":"unavailable"})).is_err());
    }
}
