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
    /// Report the observed simulated cabin state.
    PassengerStatus,
    /// Acknowledge preparation without claiming the checks are complete.
    DeparturePreparation,
    /// Ask the cabin crew to prepare for arrival without changing controls.
    LandingPreparation,
    /// Ask the cabin crew to keep watch over the passengers.
    PassengerWatch,
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
            Self::PassengerStatus => "cabin_request_passengers",
            Self::DeparturePreparation => "cabin_departure_preparing",
            Self::LandingPreparation => "cabin_request_landing",
            Self::PassengerWatch => "cabin_request_watch",
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

/// Editable spelling corrections and clause recognisers for interphone requests.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grammar {
    split: String,
    cross_check: String,
    drink: String,
    #[serde(default)]
    statement: Option<String>,
    replacements: std::collections::BTreeMap<String, String>,
    services: std::collections::BTreeMap<String, Service>,
}
impl Grammar {
    /// Load the installed grammar, retaining bundled defaults for older installations.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                include_str!("../../../cabin/language.toml").to_owned()
            }
            Err(error) => return Err(error.to_string()),
        };
        if text.len() > 16384 {
            return Err("Crew language file exceeds 16 KiB".into());
        }
        let result: Self = toml::from_str(&text).map_err(|error| error.to_string())?;
        for pattern in [&result.split, &result.cross_check, &result.drink] {
            regex::Regex::new(pattern).map_err(|error| error.to_string())?;
        }
        if let Some(pattern) = &result.statement {
            regex::Regex::new(pattern).map_err(|error| error.to_string())?;
        }
        if result.replacements.len() > 128
            || result.replacements.iter().any(|(from, to)| {
                from.is_empty()
                    || to.is_empty()
                    || !from.chars().all(char::is_alphabetic)
                    || !to.chars().all(char::is_alphabetic)
            })
        {
            return Err("Crew corrections must contain words only".into());
        }
        Ok(result)
    }
    /// Preserve punctuation and numbers while replacing only explicitly listed words.
    pub fn normalize(&self, text: &str) -> String {
        let lower = text.to_lowercase();
        let token = regex::Regex::new(r"\p{L}+").unwrap();
        token
            .replace_all(&lower, |capture: &regex::Captures<'_>| {
                self.replacements
                    .get(&capture[0])
                    .cloned()
                    .unwrap_or_else(|| capture[0].to_owned())
            })
            .into_owned()
    }
}

/// Parse common requests without relying on a language model.
pub fn parse(profile: &CrewProfile, role: &str, text: &str) -> Option<Intent> {
    let grammar: Grammar = toml::from_str(include_str!("../../../cabin/language.toml")).ok()?;
    parse_with(profile, role, text, &grammar)
}

/// Interpret all clauses using the selected editable grammar.
pub fn parse_with(
    profile: &CrewProfile,
    role: &str,
    text: &str,
    grammar: &Grammar,
) -> Option<Intent> {
    let lower = grammar.normalize(text);
    let mut intent = Intent::default();
    let mut controls = Vec::new();
    if role == "cabin" {
        let split = regex::Regex::new(&grammar.split).ok()?;
        let check = regex::Regex::new(&grammar.cross_check).ok()?;
        let drink = regex::Regex::new(&grammar.drink).ok()?;
        let statement = grammar
            .statement
            .as_ref()
            .map(|pattern| regex::Regex::new(pattern))
            .transpose()
            .ok()?;
        for clause in split.split(&lower).map(str::trim).filter(|s| !s.is_empty()) {
            if check.is_match(clause) {
                intent.cross_check = true;
            } else if let Some(service) = [
                ("cabin_request_passengers", Service::PassengerStatus),
                ("cabin_request_departure", Service::DeparturePreparation),
                ("cabin_request_landing", Service::LandingPreparation),
                ("cabin_request_watch", Service::PassengerWatch),
            ]
            .into_iter()
            .find_map(|(key, service)| {
                crate::dialogue::phrases(key)
                    .iter()
                    .any(|phrase| clause.contains(phrase))
                    .then_some(service)
            }) {
                if !intent.services.contains(&service) {
                    intent.services.push(service);
                }
            } else if let Some(capture) = drink.captures(clause) {
                let service = *grammar.services.get(&capture[1])?;
                if !intent.services.contains(&service) {
                    intent.services.push(service);
                }
            } else if statement
                .as_ref()
                .is_some_and(|pattern| pattern.is_match(clause))
            {
                continue;
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
    fn exact_compound_requests_and_typos_keep_all_intents() {
        let intent = parse(&profile(),"cabin","get passengers ready, can i have a coffee and can you cross check and arm doors please").unwrap();
        assert_eq!(
            intent.services,
            vec![Service::DeparturePreparation, Service::Coffee]
        );
        assert!(intent.cross_check);
        assert_eq!(intent.actions[0].control, "slides");
        for text in [
            "corss check and arm doors please",
            "cross chek and arm dorrs please",
            "arm dorrs and corss chek please",
        ] {
            let intent = parse(&profile(), "cabin", text).unwrap();
            assert!(intent.cross_check);
            assert_eq!(intent.actions[0].control, "slides");
            assert_eq!(intent.actions[0].value, 1.0);
        }
        let intent = parse(&profile(), "ground", "remove ext pwr and chocks").unwrap();
        assert_eq!(intent.actions.len(), 2);
        assert!(intent.actions.iter().all(|action| action.value == 0.0));
        assert!(parse(&profile(), "cabin", "do not arm dorrs and coffee please").is_none());
    }
    #[test]
    fn cabin_preparation_and_watch_are_services_not_switch_changes() {
        let watch = parse(&profile(), "cabin", "I have turned off the seat belt sign, it won't be for long so keep an eye on the passengers please.").unwrap();
        assert!(watch.actions.is_empty());
        assert_eq!(watch.services, vec![Service::PassengerWatch]);
        let landing = parse(&profile(), "cabin", "prepare cabin for landing").unwrap();
        assert!(landing.actions.is_empty());
        assert_eq!(landing.services, vec![Service::LandingPreparation]);
        let combined = parse(
            &profile(),
            "cabin",
            "prepare cabin for landing and turn off cabin lights please",
        )
        .unwrap();
        assert_eq!(combined.actions[0].value, 0.0);
        assert_eq!(combined.services, vec![Service::LandingPreparation]);
    }
    #[test]
    fn light_switch_requests_preserve_stacked_services() {
        for text in [
            "turn off cabin lights, can i have a coffee",
            "can i have a coffee and turn cabin lights off please",
            "cabin lights off and coffee please",
            "turn off cabin ligths and can i have a cofee",
            "set cabin lights to 0% and coffee please",
        ] {
            let intent = parse(&profile(), "cabin", text).unwrap();
            assert_eq!(intent.actions.len(), 1, "{text}");
            assert_eq!(intent.actions[0].control, "cabin_brightness");
            assert_eq!(intent.actions[0].value, 0.0);
            assert_eq!(intent.services, vec![Service::Coffee]);
        }
        let intent = parse(
            &profile(),
            "cabin",
            "how are the passengers, turn off cabin lights, can i have a coffee",
        )
        .unwrap();
        assert_eq!(
            intent.services,
            vec![Service::PassengerStatus, Service::Coffee]
        );
        assert_eq!(intent.actions[0].value, 0.0);
        assert!(
            parse(
                &profile(),
                "cabin",
                "do not turn off cabin lights and coffee please"
            )
            .is_none()
        );
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
    #[test]
    fn passenger_question_does_not_discard_explicit_light_command() {
        let intent = parse(
            &profile(),
            "cabin",
            "how are the passengers and get you turn off cabin lights and get ready for depature?",
        )
        .unwrap();
        assert_eq!(intent.actions.len(), 1);
        assert_eq!(intent.actions[0].control, "cabin_brightness");
        assert_eq!(intent.actions[0].value, 0.0);
        assert_eq!(
            intent.services,
            vec![Service::PassengerStatus, Service::DeparturePreparation]
        );
        assert!(
            parse(
                &profile(),
                "cabin",
                "how are the passengers and do not turn off cabin lights"
            )
            .is_none()
        );
        assert!(
            parse(
                &profile(),
                "cabin",
                "how are the passengers and set mystery switch"
            )
            .is_none()
        );
    }
}
