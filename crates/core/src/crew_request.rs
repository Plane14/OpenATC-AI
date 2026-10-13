//! Interpret a whole crew request before queuing any aircraft controls.
use crate::crew::{CrewProfile, parse_many, validate};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Cabin services with no aircraft-control side effects.
pub enum Service {
    /// Request a coffee.
    Coffee,
    /// Request a tea.
    Tea,
    /// Request drinking water.
    Water,
    /// Request a meal without assuming dietary preferences.
    Meal,
    /// Request a light snack.
    Snack,
    /// Request juice.
    Juice,
}

impl Service {
    /// Editable action fragment inserted into the attendant's reply.
    pub fn response(self) -> &'static str {
        match self {
            Self::Coffee => "crew_service_coffee",
            Self::Tea => "crew_service_tea",
            Self::Water => "crew_service_water",
            Self::Meal => "crew_service_meal",
            Self::Snack => "crew_service_snack",
            Self::Juice => "crew_service_juice",
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// All parts of one pilot request, validated together before execution.
pub struct Intent {
    /// Aircraft controls in execution order.
    pub actions: Vec<ControlAction>,
    /// Cabin services to acknowledge in the combined reply.
    pub services: Vec<Service>,
    /// Verify slide indications after any requested change.
    pub cross_check: bool,
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
        if self.actions.is_empty() && self.services.is_empty() && !self.cross_check {
            return false;
        }
        if self.actions.len() > 8
            || self.services.len() > 6
            || (role != "cabin" && (!self.services.is_empty() || self.cross_check))
        {
            return false;
        }
        self.actions.iter().enumerate().all(|(i, action)| {
            validate(profile, role, &action.control, action.value)
                && !self.actions[..i]
                    .iter()
                    .any(|a| a.control == action.control)
        }) && self
            .services
            .iter()
            .enumerate()
            .all(|(i, service)| !self.services[..i].contains(service))
    }
}

/// Parse common requests without relying on a language model.
pub fn parse(profile: &CrewProfile, role: &str, text: &str) -> Option<Intent> {
    let lower = text.to_lowercase();
    let mut intent = Intent::default();
    let mut controls = Vec::new();
    if role == "cabin" {
        let split = regex::Regex::new(r"\s+(?:and|then)\s+|[,;]").ok()?;
        let check = regex::Regex::new(r"^(?:(?:can|could|would) you\s+)?(?:please\s+)?(?:cross[ -]?check|xcheck)(?: the doors| the slides)?(?: please)?[.!?]?$").ok()?;
        let drink = regex::Regex::new(r"^(?:(?:(?:can|could|may) i (?:please )?(?:have|get)|(?:can|could|would) you (?:please )?(?:bring|get)(?: me)?|(?:please )?(?:bring|get)(?: me)?)\s+)?(?:a |some |my )?(?:quick )?(coffee|coffe|tea|water|meal|food|lunch|dinner|breakfast|snack|juice)(?: please)?(?: before (?:we )?push ?back)?[.!?]?$").ok()?;
        for clause in split.split(&lower).map(str::trim).filter(|s| !s.is_empty()) {
            if check.is_match(clause) {
                intent.cross_check = true;
            } else if let Some(capture) = drink.captures(clause) {
                let service = match &capture[1] {
                    "coffee" | "coffe" => Service::Coffee,
                    "tea" => Service::Tea,
                    "water" => Service::Water,
                    "meal" | "food" | "lunch" | "dinner" | "breakfast" => Service::Meal,
                    "snack" => Service::Snack,
                    "juice" => Service::Juice,
                    _ => return None,
                };
                if !intent.services.contains(&service) {
                    intent.services.push(service);
                }
            } else {
                controls.push(clause);
            }
        }
    } else {
        controls.push(&lower);
    }
    if !controls.is_empty() {
        intent.actions = parse_many(profile, role, &controls.join(" and "))?
            .into_iter()
            .map(|(control, value)| ControlAction { control, value })
            .collect();
    }
    intent.valid(profile, role).then_some(intent)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> CrewProfile {
        let aircraft: toml::Value =
            toml::from_str(include_str!("../../../aircraft/toliss_a320.toml")).unwrap();
        aircraft["aircraft"]["crew"].clone().try_into().unwrap()
    }
    #[test]
    fn cabin_service_and_control_order_do_not_depend_on_a_comma() {
        for text in [
            "can i please have a coffe and can you cross check and arm doors",
            "arm and cross check, can you bring me a quick coffee before we push back?",
            "arm doors and cross-check and coffee please",
        ] {
            let intent = parse(&profile(), "cabin", text).unwrap();
            assert_eq!(intent.services, vec![Service::Coffee]);
            assert!(intent.cross_check);
            assert_eq!(intent.actions.len(), 1);
            assert_eq!(intent.actions[0].control, "slides");
            assert_eq!(intent.actions[0].value, 1.0);
        }
    }
    #[test]
    fn no_unrequested_or_unknown_controls() {
        for text in [
            "coffee please and set mystery switch",
            "do not arm doors and coffee please",
            "are the doors armed? and coffee please",
        ] {
            assert!(parse(&profile(), "cabin", text).is_none());
        }
        assert!(parse(&profile(), "ground", "coffee please").is_none());
        let intent = parse(&profile(), "cabin", "cross check and tea please").unwrap();
        assert!(intent.actions.is_empty());
    }
}
