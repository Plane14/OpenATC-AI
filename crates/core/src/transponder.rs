//! Editable bindings for a deliberate code selection.
use serde::Deserialize;
/// Controls changed by clicking a displayed squawk.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    /// Writable integer transponder code.
    pub code_dataref: String,
    /// Optional aircraft-specific power selector, distinct from the mode selector.
    pub power: Option<Power>,
}
/// Aircraft-specific power control.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Power {
    /// Writable integer dataref.
    pub dataref: String,
    /// ON position.
    pub on: i32,
}
impl Controls {
    /// Parse a bounded binding file.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 4096 {
            return Err("Transponder bindings exceed 4 KiB".into());
        }
        let result: Self = toml::from_str(text).map_err(|error| error.to_string())?;
        let valid = |name: &str| {
            !name.is_empty()
                && name.len() <= 256
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"/_-".contains(&byte))
        };
        if !valid(&result.code_dataref)
            || result
                .power
                .as_ref()
                .is_some_and(|power| !valid(&power.dataref) || !(0..=10).contains(&power.on))
        {
            return Err("Invalid transponder binding".into());
        }
        Ok(result)
    }
    /// Installed overrides use the same schema as the bundled mapping.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Self::parse(include_str!("../../../aircraft/transponder.toml"))
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bindings_keep_power_and_mode_separate() {
        let controls =
            super::Controls::parse(include_str!("../../../aircraft/transponder.toml")).unwrap();
        assert_eq!(
            controls.code_dataref,
            "sim/cockpit2/radios/actuators/transponder_code"
        );
        let power = controls.power.unwrap();
        assert_eq!(power.dataref, "AirbusFBW/XPDRTCASMode");
        assert_eq!(power.on, 1);
        assert!(super::Controls::parse("code_dataref = 'bad name'").is_err());
        assert!(super::Controls::parse("code_dataref = 'sim/code'\nmode = 0").is_err());
    }
}
