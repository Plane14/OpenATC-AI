//! Spoken radio identifiers; displayed transmissions keep their original spelling.

const LETTERS: [&str; 26] = [
    "Alfa", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel", "India", "Juliett",
    "Kilo", "Lima", "Mike", "November", "Oscar", "Papa", "Quebec", "Romeo", "Sierra", "Tango",
    "Uniform", "Victor", "Whiskey", "X-ray", "Yankee", "Zulu",
];
const DIGITS: [&str; 10] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "niner",
];

/// Format radio identifiers for speech without changing the transcript.
pub fn spoken(text: &str, callsign: &str) -> String {
    spoken_with_airports(text, callsign, &[])
}

/// Resolve known airport identifiers to their names in spoken transmissions.
pub fn spoken_with_airports(text: &str, callsign: &str, airports: &[(String, String)]) -> String {
    let spelling = callsign
        .chars()
        .filter_map(|c| {
            if c.is_ascii_alphabetic() {
                Some(LETTERS[(c.to_ascii_uppercase() as u8 - b'A') as usize])
            } else if c.is_ascii_digit() {
                Some(DIGITS[(c as u8 - b'0') as usize])
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut output = String::new();
    let mut number_context = false;
    for part in text.split_inclusive(char::is_whitespace) {
        let token = part.trim();
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
                    .map(|c| DIGITS[(c as u8 - b'0') as usize])
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
                        DIGITS[(c as u8 - b'0') as usize]
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        } else if let Some((_, name)) = airports
            .iter()
            .find(|(id, name)| !name.is_empty() && bare.eq_ignore_ascii_case(id))
        {
            spoken(name.trim(), "")
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
