//! Validated conversation policy, loaded once from installed and personal TOML.
use serde::Deserialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Chat {
    pub range_nm: f64,
    pub observation_age_seconds: u64,
    pub initiation_interval_seconds: u64,
    pub initiation_probability: f64,
    pub operational_quiet_seconds: u64,
    pub maximum_reply_characters: usize,
    pub maximum_message_bytes: usize,
    pub history_exchanges: usize,
    pub model_timeout_seconds: u64,
    pub allowed_phases: Vec<String>,
    pub stationary_speed_knots: f64,
    pub critical_airport_radius_nm: f64,
    pub critical_height_feet: f64,
    pub personality: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    pilot_chat: Chat,
}
impl Chat {
    pub fn load(config: &Path) -> Self {
        let mut policy = Self::parse(include_str!("../../../../traffic/behaviour.toml"))
            .expect("bundled traffic policy");
        let installed = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join("../traffic")));
        for directory in installed
            .into_iter()
            .chain(std::env::var_os("OPENATC_TRAFFIC_DIR").map(PathBuf::from))
            .chain([config.join("traffic")])
        {
            let path = directory.join("behaviour.toml");
            let Ok(file) = std::fs::File::open(&path) else {
                continue;
            };
            let mut text = String::new();
            let parsed = file
                .take(128 * 1024 + 1)
                .read_to_string(&mut text)
                .map_err(|error| error.to_string())
                .and_then(|_| Self::parse(&text));
            match parsed {
                Ok(loaded) => policy = loaded,
                Err(error) => eprintln!(
                    "{}: {error}; keeping the previous valid conversation policy",
                    path.display()
                ),
            }
        }
        policy
    }
    fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 128 * 1024 {
            return Err("Conversation policy exceeds 128 KiB".into());
        }
        let policy = toml::from_str::<File>(text)
            .map_err(|error| error.to_string())?
            .pilot_chat;
        if !policy.range_nm.is_finite()
            || !(0.1..=100.0).contains(&policy.range_nm)
            || !(1..=30).contains(&policy.observation_age_seconds)
            || !(30..=3600).contains(&policy.initiation_interval_seconds)
            || !policy.initiation_probability.is_finite()
            || !(0.0..=1.0).contains(&policy.initiation_probability)
            || !(10..=600).contains(&policy.operational_quiet_seconds)
            || !(40..=500).contains(&policy.maximum_reply_characters)
            || !(40..=4000).contains(&policy.maximum_message_bytes)
            || !(1..=12).contains(&policy.history_exchanges)
            || !(1..=30).contains(&policy.model_timeout_seconds)
            || !policy.stationary_speed_knots.is_finite()
            || !(0.0..=1.0).contains(&policy.stationary_speed_knots)
            || !policy.critical_airport_radius_nm.is_finite()
            || !(3.0..=30.0).contains(&policy.critical_airport_radius_nm)
            || !policy.critical_height_feet.is_finite()
            || !(1000.0..=5000.0).contains(&policy.critical_height_feet)
            || policy.allowed_phases.is_empty()
            || policy
                .allowed_phases
                .iter()
                .any(|phase| !["parked", "cruise", "taxi_in"].contains(&phase.as_str()))
            || policy.personality.trim().is_empty()
        {
            return Err("Conversation policy exceeds its supported limits or includes a critical flight phase".into());
        }
        Ok(policy)
    }
}
