//! Editable bindings for a deliberate code selection.
use serde::Deserialize;
/// Controls changed by clicking a displayed squawk.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    /// Writable integer transponder code.
    pub code_dataref: String,
    /// Standard aircraft control used when the custom digit group is absent.
    #[serde(default)]
    pub default_code_dataref: Option<String>,
    /// Optional individual code digits used by custom aircraft.
    #[serde(default)]
    pub code_digits: Vec<String>,
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
        if (!result.code_digits.is_empty()
            && (result.code_digits.len() != 4
                || result.code_digits.iter().any(|name| !valid(name))))
            || !valid(&result.code_dataref)
            || result
                .default_code_dataref
                .as_ref()
                .is_some_and(|name| !valid(name))
            || result
                .power
                .as_ref()
                .is_some_and(|power| !valid(&power.dataref) || !(0..=10).contains(&power.on))
        {
            return Err("Invalid transponder binding".into());
        }
        Ok(result)
    }
    /// Build only writes supported by the current aircraft; mode controls are never included.
    pub fn writes(
        &self,
        code: i32,
        available: impl Fn(&str) -> bool,
    ) -> Result<Vec<(String, i32)>, String> {
        let digits = digits(code)?;
        let custom =
            self.code_digits.len() == 4 && self.code_digits.iter().all(|name| available(name));
        let code_ref = if custom {
            &self.code_dataref
        } else {
            self.default_code_dataref
                .as_ref()
                .unwrap_or(&self.code_dataref)
        };
        if !available(code_ref) {
            return Err("Aircraft transponder code control unavailable".into());
        }
        let mut writes = Vec::new();
        if custom {
            writes.extend(self.code_digits.iter().cloned().zip(digits));
        }
        writes.push((code_ref.clone(), code));
        if let Some(power) = &self.power
            && available(&power.dataref)
        {
            writes.push((power.dataref.clone(), power.on));
        }
        Ok(writes)
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
    fn default_aircraft_use_only_the_standard_code_control() {
        let controls =
            super::Controls::parse(include_str!("../../../aircraft/transponder.toml")).unwrap();
        let name = "sim/cockpit2/radios/actuators/transponder_code";
        assert_eq!(
            controls
                .writes(6450, |candidate| candidate == name)
                .unwrap(),
            vec![(name.into(), 6450)]
        );
        assert_eq!(
            controls.writes(42, |candidate| candidate == name).unwrap(),
            vec![(name.into(), 42)]
        );
        assert!(controls.writes(8911, |_| true).is_err());
        assert!(controls.writes(6450, |_| false).is_err());
    }
    #[test]
    fn bindings_keep_power_and_mode_separate() {
        let controls =
            super::Controls::parse(include_str!("../../../aircraft/transponder.toml")).unwrap();
        assert_eq!(controls.code_dataref, "sim/cockpit/radios/transponder_code");
        assert_eq!(
            controls.code_digits,
            [
                "AirbusFBW/XPDR4",
                "AirbusFBW/XPDR3",
                "AirbusFBW/XPDR2",
                "AirbusFBW/XPDR1"
            ]
        );
        let writes = controls
            .writes(6450, |_| true)
            .unwrap()
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(writes["AirbusFBW/XPDR4"], 6);
        assert_eq!(writes["AirbusFBW/XPDR1"], 0);
        assert_eq!(writes["sim/cockpit/radios/transponder_code"], 6450);
        assert!(!writes.contains_key("sim/cockpit2/radios/actuators/transponder_mode"));
        let selected = super::digits(6450).unwrap();
        let bindings = controls
            .code_digits
            .iter()
            .zip(selected)
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(bindings[&"AirbusFBW/XPDR4".to_owned()], 6);
        assert_eq!(bindings[&"AirbusFBW/XPDR3".to_owned()], 4);
        assert_eq!(bindings[&"AirbusFBW/XPDR2".to_owned()], 5);
        assert_eq!(bindings[&"AirbusFBW/XPDR1".to_owned()], 0);
        let power = controls.power.unwrap();
        assert_eq!(power.dataref, "AirbusFBW/XPDRTCASMode");
        assert_eq!(power.on, 1);
        assert!(super::Controls::parse("code_dataref = 'bad name'").is_err());
        assert!(super::Controls::parse("code_dataref = 'sim/code'\nmode = 0").is_err());
    }
}

/// Preserve leading zeros while expanding an octal squawk into display digits.
pub fn digits(code: i32) -> Result<[i32; 4], String> {
    if !(0..=7777).contains(&code) {
        return Err("Invalid transponder code".into());
    }
    let result = [code / 1000, code / 100 % 10, code / 10 % 10, code % 10];
    if result.iter().any(|digit| *digit > 7) {
        return Err("Invalid transponder code".into());
    }
    Ok(result)
}
#[cfg(test)]
mod digit_tests {
    #[test]
    fn code_order_and_leading_zeros_are_preserved() {
        assert_eq!(super::digits(6111).unwrap(), [6, 1, 1, 1]);
        assert_eq!(super::digits(42).unwrap(), [0, 0, 4, 2]);
        assert!(super::digits(8911).is_err());
    }
}
