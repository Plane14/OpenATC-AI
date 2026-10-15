//! Spoken radio identifiers; displayed transmissions keep their original spelling.

/// Editable radiotelephony alphabet and taxi-route speech context.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pronunciation {
    version: u32,
    letters: Vec<String>,
    digits: Vec<String>,
    taxi_introducers: Vec<String>,
    taxi_connectors: Vec<String>,
    maximum_taxi_letters: usize,
    maximum_taxi_digits: usize,
}
impl Pronunciation {
    /// Parse a bounded pronunciation table without changing operational identifiers.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 16384 {
            return Err("Radio pronunciation exceeds 16 KiB".into());
        }
        let value: Self = toml::from_str(text).map_err(|error| error.to_string())?;
        if value.version != 1
            || value.letters.len() != 26
            || value.digits.len() != 10
            || !(1..=3).contains(&value.maximum_taxi_letters)
            || !(1..=4).contains(&value.maximum_taxi_digits)
            || value.taxi_introducers.is_empty()
            || value.taxi_introducers.len() > 16
            || value.taxi_connectors.len() > 16
            || value
                .letters
                .iter()
                .chain(&value.digits)
                .chain(&value.taxi_introducers)
                .chain(&value.taxi_connectors)
                .any(|word| {
                    word.is_empty()
                        || word.len() > 32
                        || !word.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
                })
        {
            return Err("Radio pronunciation has invalid alphabet or context entries".into());
        }
        Ok(value)
    }
    fn taxiway(&self, token: &str) -> Option<String> {
        let letters = token.bytes().take_while(u8::is_ascii_uppercase).count();
        if letters == 0
            || letters > self.maximum_taxi_letters
            || token.len() - letters > self.maximum_taxi_digits
            || !token.as_bytes()[letters..].iter().all(u8::is_ascii_digit)
        {
            return None;
        }
        Some(
            token
                .bytes()
                .map(|c| {
                    if c.is_ascii_uppercase() {
                        self.letters[(c - b'A') as usize].as_str()
                    } else {
                        self.digits[(c - b'0') as usize].as_str()
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}
impl Default for Pronunciation {
    fn default() -> Self {
        Self::parse(include_str!("../../../speech/runtime/radiotelephony.toml"))
            .expect("valid bundled radio pronunciation")
    }
}
fn bundled() -> &'static Pronunciation {
    static VALUE: std::sync::OnceLock<Pronunciation> = std::sync::OnceLock::new();
    VALUE.get_or_init(Pronunciation::default)
}

/// Format radio identifiers for speech without changing the transcript.
pub fn spoken(text: &str, callsign: &str) -> String {
    spoken_with_airports(text, callsign, &[])
}

/// Resolve known airport identifiers to their names in spoken transmissions.
pub fn spoken_with_airports(text: &str, callsign: &str, airports: &[(String, String)]) -> String {
    spoken_with_pronunciation(text, callsign, airports, bundled())
}

/// Use a loaded pronunciation policy for the spoken copy of a transmission.
pub fn spoken_with_pronunciation(
    text: &str,
    callsign: &str,
    airports: &[(String, String)],
    policy: &Pronunciation,
) -> String {
    let spelling = callsign
        .chars()
        .filter_map(|c| {
            if c.is_ascii_alphabetic() {
                Some(policy.letters[(c.to_ascii_uppercase() as u8 - b'A') as usize].as_str())
            } else if c.is_ascii_digit() {
                Some(policy.digits[(c as u8 - b'0') as usize].as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut output = String::new();
    let mut number_context = false;
    let mut taxi_context = false;
    for part in text.split_inclusive(|c: char| c.is_whitespace() || c == ',') {
        let token = part.trim();
        if token.is_empty() {
            output.push_str(part);
            continue;
        }
        let bare = token.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        let runway_suffix = if number_context && bare.len() > 1 {
            match bare.as_bytes().last() {
                Some(b'L') => Some("left"),
                Some(b'R') => Some("right"),
                Some(b'C') => Some("center"),
                _ => None,
            }
        } else {
            None
        };
        let replacement = if let Some(suffix) =
            runway_suffix.filter(|_| bare[..bare.len() - 1].chars().all(|c| c.is_ascii_digit()))
        {
            format!(
                "{} {suffix}",
                bare[..bare.len() - 1]
                    .chars()
                    .map(|c| policy.digits[(c as u8 - b'0') as usize].as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        } else if !callsign.is_empty() && bare.eq_ignore_ascii_case(callsign) {
            spelling.clone()
        } else if (number_context || bare.contains('.'))
            && bare.chars().all(|c| c.is_ascii_digit() || c == '.')
            && bare.chars().any(|c| c.is_ascii_digit())
        {
            bare.chars()
                .map(|c| {
                    if c == '.' {
                        "point"
                    } else {
                        policy.digits[(c as u8 - b'0') as usize].as_str()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        } else if taxi_context && let Some(taxiway) = policy.taxiway(bare) {
            taxiway
        } else if let Some((_, name)) = airports
            .iter()
            .find(|(id, name)| !name.is_empty() && bare.eq_ignore_ascii_case(id))
        {
            spoken_with_pronunciation(name.trim(), "", &[], policy)
        } else {
            match bare.to_ascii_uppercase().as_str() {
                "INTL" => "International",
                "RWY" => "runway",
                "TWY" => "taxiway",
                "CLNC" => "clearance",
                "DCT" => "direct",
                "ATC" => "air traffic control",
                "MSL" => "mean sea level",
                "AGL" => "above ground level",
                "APT" => "Airport",
                "GND" => "Ground",
                "TWR" => "Tower",
                "DEL" => "Delivery",
                "APP" | "APPR" => "Approach",
                "DEP" => "Departure",
                "CTR" => "Center",
                "ACC" => "Area Control",
                "IFR" => "I F R",
                "VFR" => "V F R",
                "SID" => "sid",
                "STAR" => "star",
                "ILS" => "I L S",
                "ATIS" => "ay tis",
                "FL" => "flight level",
                "NM" => "nautical miles",
                "KT" | "KTS" => "knots",
                "FT" => "feet",
                "HPA" => "hectopascals",
                "MHZ" => "megahertz",
                "QNH" => "Q N H",
                "QFE" => "Q F E",
                _ => bare,
            }
            .to_owned()
        };
        if let Some(start) = token.find(bare).filter(|_| !bare.is_empty()) {
            output.push_str(&token[..start]);
            output.push_str(&replacement);
            output.push_str(&token[start + bare.len()..]);
        } else {
            output.push_str(token);
        }
        output.push_str(&part[token.len()..]);
        let lower = bare.to_ascii_lowercase();
        taxi_context = policy.taxi_introducers.iter().any(|word| word == &lower)
            || (taxi_context
                && (policy.taxiway(bare).is_some()
                    || policy.taxi_connectors.iter().any(|word| word == &lower)));
        if token.ends_with(['.', ';', ':']) {
            taxi_context = false;
        }
        number_context = matches!(
            bare.to_ascii_lowercase().as_str(),
            "runway" | "rwy" | "squawk" | "frequency" | "heading"
        );
    }
    output
}

#[cfg(test)]
mod tests {
    #[test]
    fn taxi_identifiers_are_spoken_in_context_without_changing_log_or_other_numbers() {
        let input =
            "C-GTLT, Taxi via T, A, A1 and AB2, hold short of runway 04L. Maintain 5000 feet.";
        let output = super::spoken(input, "C-GTLT");
        assert!(output.contains("via Tango, Alfa, Alfa one and Alfa Bravo two,"));
        assert!(output.contains("runway zero four left. Maintain 5000 feet."));
        assert!(input.contains("T, A, A1"));
        assert_eq!(
            super::spoken("A passenger is in seat A1. Taxi via T,A,A1.", ""),
            "A passenger is in seat A1. Taxi via Tango,Alfa,Alfa one."
        );
        assert_eq!(
            super::spoken("Taxiway A1, then B. A passenger needs help.", ""),
            "Taxiway Alfa one, then Bravo. A passenger needs help."
        );
        let text =
            include_str!("../../../speech/runtime/radiotelephony.toml").replace("Alfa", "Alpha");
        let policy = super::Pronunciation::parse(&text).unwrap();
        assert_eq!(
            super::spoken_with_pronunciation("Taxi via A1", "", &[], &policy),
            "Taxi via Alpha one"
        );
        assert!(
            super::Pronunciation::parse(
                &text.replace("maximum_taxi_letters = 2", "maximum_taxi_letters = 99")
            )
            .is_err()
        );
    }
    #[test]
    fn expands_radio_names_without_changing_log_text() {
        let text = "Los Angeles GND West, C-GTLT, cleared to KSNA, IFR.";
        let spoken = super::spoken_with_airports(
            text,
            "C-GTLT",
            &[("KSNA".into(), "John Wayne Airport".into())],
        );
        assert!(spoken.contains("Ground West"));
        assert!(spoken.contains("cleared to John Wayne Airport"));
        assert!(spoken.contains("I F R"));
        assert!(text.contains("GND"));
    }

    #[test]
    fn spells_callsign_without_changing_display_or_altitude() {
        let input = "C-GTLT, runway 29L, squawk 2105, maintain 5000 feet, contact 121.400.";
        let spoken = super::spoken(input, "C-GTLT");
        assert!(spoken.starts_with("Charlie Golf Tango Lima Tango,"));
        assert!(spoken.contains("runway two niner left"));
        assert!(spoken.contains("squawk two one zero five"));
        assert!(spoken.contains("maintain 5000 feet"));
        assert!(spoken.contains("one two one point four zero zero"));
    }
}
