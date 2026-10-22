//! Plan an early descent without treating a predicted FMS altitude as a restriction.
use serde::Deserialize;
use std::{io::Read, path::Path};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    version: u32,
    pub descent_degrees: f64,
    pub lead_nm: f64,
    pub level_step_feet: i32,
    pub minimum_change_feet: i32,
    #[serde(default = "default_select_ils")]
    pub select_runway_ils: bool,
    #[serde(default = "notice_seconds")]
    pub speed_notice_seconds: f64,
    #[serde(default = "notice_deceleration")]
    pub speed_notice_deceleration: f64,
    #[serde(default = "notice_min")]
    pub speed_notice_min_nm: f64,
    #[serde(default = "notice_max")]
    pub speed_notice_max_nm: f64,
}
fn notice_seconds() -> f64 {
    90.0
}
fn notice_deceleration() -> f64 {
    1.0
}
fn notice_min() -> f64 {
    8.0
}
fn notice_max() -> f64 {
    30.0
}
fn default_select_ils() -> bool {
    true
}
impl Default for Policy {
    fn default() -> Self {
        Self::parse(include_str!("../../../../traffic/vertical.toml"))
            .expect("bundled vertical policy")
    }
}
impl Policy {
    fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 65536 {
            return Err("Vertical policy exceeds 64 KiB".into());
        }
        let policy: Self =
            openatc_core::configuration::from_str(text).map_err(|error| error.to_string())?;
        if policy.version != 1
            || !(2.5..=3.5).contains(&policy.descent_degrees)
            || !(5.0..=30.0).contains(&policy.lead_nm)
            || !(100..=2000).contains(&policy.level_step_feet)
            || !(100..=1000).contains(&policy.minimum_change_feet)
            || !(30.0..=180.0).contains(&policy.speed_notice_seconds)
            || !(0.25..=3.0).contains(&policy.speed_notice_deceleration)
            || !(3.0..=15.0).contains(&policy.speed_notice_min_nm)
            || !(policy.speed_notice_min_nm..=50.0).contains(&policy.speed_notice_max_nm)
        {
            return Err("Vertical policy contains invalid planning margins".into());
        }
        Ok(policy)
    }
    pub fn load(config: &Path) -> Result<Self, String> {
        let mut policy = Self::default();
        for directory in crate::traffic_profiles::directories(config) {
            let path = directory.join("vertical.toml");
            let file = match std::fs::File::open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("{}: {error}", path.display())),
            };
            let mut text = String::new();
            file.take(65537)
                .read_to_string(&mut text)
                .map_err(|error| error.to_string())?;
            policy = Self::parse(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        }
        Ok(policy)
    }
    pub fn distance(&self, from: i32, to: i32) -> f64 {
        f64::from((from - to).max(0)) / openatc_core::arrival::gradient(self.descent_degrees)
            + self.lead_nm
    }
}
