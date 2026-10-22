//! Published holding defaults from XPHOLD1140, restricted to the requested route references.
use crate::traffic::procedures::{Catalogue as Procedures, Leg};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    identifier: String,
    region: String,
    owner: String,
    kind: u8,
}
fn reference(leg: &Leg, airport: &str) -> Option<Key> {
    let (kind, owner) = match (leg.section.as_str(), leg.subsection.as_str()) {
        ("E", "A") => (11, "ENRT"),
        ("P", "C") => (11, airport),
        ("D", "") => (3, "ENRT"),
        ("D", "B") => (2, "ENRT"),
        ("P", "N") => (2, airport),
        _ => return None,
    };
    Some(Key {
        identifier: leg.identifier.clone(),
        region: leg.region.clone(),
        owner: owner.into(),
        kind,
    })
}

/// Alternatives remain separate when a fix has several published levels or patterns.
#[derive(Clone, Debug, Default)]
pub struct Catalogue {
    entries: BTreeMap<Key, Vec<Leg>>,
}
impl Catalogue {
    /// Find only the same fix type, region and terminal owner as the procedure leg.
    pub fn at(&self, leg: &Leg, airport: &str) -> &[Leg] {
        reference(leg, airport)
            .and_then(|key| self.entries.get(&key))
            .map_or(&[], Vec::as_slice)
    }
    /// Read on a planning worker, checking the cycle against resolved navigation points.
    pub fn load(
        root: &Path,
        procedures: &[(&str, &Procedures)],
        cycle: &str,
    ) -> Result<Self, String> {
        if root.join("Custom Data/earth_424.dat").is_file()
            || root.join("Custom Data/FAACIFP18").is_file()
        {
            return Err("ARINC holding overrides need their own decoder".into());
        }
        let wanted: BTreeSet<_> = procedures
            .iter()
            .flat_map(|(airport, catalogue)| {
                catalogue
                    .0
                    .values()
                    .flatten()
                    .filter_map(|leg| reference(leg, airport))
            })
            .collect();
        if wanted.len() > 8192 {
            return Err("Holding reference budget exceeded".into());
        }
        let custom = root.join("Custom Data/earth_hold.dat");
        let path = if custom.is_file() {
            custom
        } else {
            root.join("Resources/default data/earth_hold.dat")
        };
        let file =
            std::fs::File::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        if file.metadata().map_err(|error| error.to_string())?.len() > 16 * 1024 * 1024 {
            return Err("Holding file exceeds 16 MiB".into());
        }
        Self::read(BufReader::new(file), &wanted, cycle)
    }
    fn read(reader: impl BufRead, wanted: &BTreeSet<Key>, cycle: &str) -> Result<Self, String> {
        let mut result = Self::default();
        let names: BTreeSet<_> = wanted.iter().map(|key| key.identifier.as_str()).collect();
        let mut header = false;
        let mut terminated = false;
        for (index, line) in reader.lines().enumerate() {
            let line = line.map_err(|error| error.to_string())?;
            if index >= 200_000 || line.len() > 4096 {
                return Err("Holding file exceeds record limits".into());
            }
            let line = line.trim();
            if line.is_empty() || matches!(line, "I" | "A") {
                continue;
            }
            if !header {
                let found = line
                    .split_once("data cycle ")
                    .map(|(_, value)| value.chars().take(4).collect::<String>());
                if !line.starts_with("1140 Version") || found.as_deref() != Some(cycle) {
                    return Err(
                        "Holding format or AIRAC cycle does not match navigation data".into(),
                    );
                }
                header = true;
                continue;
            }
            if line == "99" {
                terminated = true;
                break;
            }
            let mut tokens = line.split_whitespace();
            let fields: [&str; 11] = std::array::from_fn(|_| tokens.next().unwrap_or_default());
            if !names.contains(fields[0]) {
                continue;
            }
            let key = Key {
                identifier: fields[0].into(),
                region: fields[1].into(),
                owner: fields[2].into(),
                kind: fields[3].parse().unwrap_or(0),
            };
            if !wanted.contains(&key) {
                continue;
            }
            if tokens.next().is_some() || fields.iter().any(|field| field.is_empty()) {
                return Err("Incomplete published holding record".into());
            }
            let number = |field: &str, maximum| -> Result<f64, String> {
                let value = field
                    .parse::<f64>()
                    .map_err(|_| "Invalid published holding number")?;
                if !value.is_finite() || !(0.0..=maximum).contains(&value) {
                    return Err("Published holding value exceeds its limits".into());
                }
                Ok(value)
            };
            let course = number(fields[4], 360.0)?;
            let minutes = number(fields[5], 10.0)?;
            let distance = number(fields[6], 50.0)?;
            if (minutes > 0.0) == (distance > 0.0) || !matches!(fields[7], "L" | "R") {
                return Err("Holding needs one leg measure and a turn direction".into());
            }
            let level = |field: &str| -> Result<Option<i32>, String> {
                let value = field.parse::<i32>().map_err(|_| "Invalid holding level")?;
                if !(-2000..=60000).contains(&value) {
                    return Err("Invalid holding level".into());
                }
                Ok((value != 0).then_some(value))
            };
            let minimum = level(fields[8])?;
            let maximum = level(fields[9])?;
            if minimum
                .zip(maximum)
                .is_some_and(|(minimum, maximum)| minimum > maximum)
            {
                return Err("Holding level band is reversed".into());
            }
            let speed = fields[10]
                .parse::<u16>()
                .map_err(|_| "Invalid holding speed")?;
            if speed > 300 {
                return Err("Holding speed exceeds supported limits".into());
            }
            let (condition, first, second) = match (minimum, maximum) {
                (Some(low), Some(high)) => ("B", Some(low), Some(high)),
                (Some(low), None) => ("+", Some(low), None),
                (None, Some(high)) => ("-", Some(high), None),
                (None, None) => ("", None, None),
            };
            let leg = Leg {
                recommended: None,
                sequence: 0,
                identifier: key.identifier.clone(),
                region: key.region.clone(),
                section: if key.kind == 11 {
                    if key.owner == "ENRT" { "E" } else { "P" }
                } else if key.owner == "ENRT" {
                    "D"
                } else {
                    "P"
                }
                .into(),
                subsection: if key.kind == 11 {
                    if key.owner == "ENRT" { "A" } else { "C" }
                } else if key.kind == 2 {
                    if key.owner == "ENRT" { "B" } else { "N" }
                } else {
                    ""
                }
                .into(),
                description: "    ".into(),
                turn: fields[7].into(),
                terminator: "HM".into(),
                course: Some(course.rem_euclid(360.0)),
                distance_time: if minutes > 0.0 {
                    format!("T{:03.0}", minutes * 10.0)
                } else {
                    format!("{:.0}", distance * 10.0)
                },
                altitude_description: condition.into(),
                altitude_one: first,
                altitude_one_is_flight_level: None,
                altitude_two: second,
                altitude_two_is_flight_level: None,
                speed_description: if speed == 0 { "" } else { "-" }.into(),
                speed_knots: (speed != 0).then_some(f64::from(speed)),
                vertical_angle_degrees: None,
            };
            let alternatives = result.entries.entry(key).or_default();
            if !alternatives.contains(&leg) {
                if alternatives.len() >= 32 {
                    return Err("Too many holding alternatives at one reference".into());
                }
                alternatives.push(leg);
            }
        }
        if !header || !terminated {
            return Err("Holding file is incomplete".into());
        }
        Ok(result)
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    #[test]
    fn holding_defaults_keep_owner_alternatives_and_clearance_limits() {
        let key = Key {
            identifier: "INLER".into(),
            region: "LI".into(),
            owner: "LIMC".into(),
            kind: 11,
        };
        let wanted = BTreeSet::from([key.clone()]);
        let text = "I\n1140 Version - data cycle 2609, build 20260826\nINLER LI LIMC 11 356.0 1.0 0.0 R 4000 0 190\nINLER LI ENRT 11 100.0 1.0 0.0 L 9000 0 210\nINLER LI LIMC 11 356.0 1.0 0.0 R 4000 0 190\nINLER LI LIMC 11 356.0 1.5 0.0 R 6000 8000 210\nINLER LI LIMC 11 356.0 0.0 3.9 R 10000 0 210\n99\n";
        let catalogue = Catalogue::read(text.as_bytes(), &wanted, "2609").unwrap();
        let alternatives = &catalogue.entries[&key];
        assert_eq!(alternatives.len(), 3);
        assert_eq!(
            alternatives[0].altitude_limits().unwrap(),
            (Some(4000), None)
        );
        assert_eq!(
            alternatives[1].altitude_limits().unwrap(),
            (Some(6000), Some(8000))
        );
        assert_eq!(alternatives[2].distance_time, "39");
        let position = crate::traffic::coordinates::Position {
            coordinate: [45.5, 8.8],
            elevation_feet: None,
            magnetic_variation_degrees: Some(3.0),
        };
        let mut clearance = crate::traffic::holding::Clearance {
            level_feet: 4000,
            geometric_feet: 4200.0,
            indicated_knots: 180.0,
            ground_knots: 190.0,
            bank_degrees: 25.0,
            turn_rate_degrees_sec: 3.0,
        };
        let pattern =
            crate::traffic::holding::Pattern::published(&alternatives[0], position, &clearance)
                .unwrap();
        assert_eq!(pattern.inbound_degrees, 359.0);
        assert_eq!(pattern.leg_seconds, 60.0);
        assert_eq!(pattern.fix.altitude_feet, 4200.0);
        clearance.level_feet = 3000;
        assert!(
            crate::traffic::holding::Pattern::published(&alternatives[0], position, &clearance)
                .is_err()
        );
        clearance.level_feet = 4000;
        clearance.indicated_knots = 200.0;
        assert!(
            crate::traffic::holding::Pattern::published(&alternatives[0], position, &clearance)
                .is_err()
        );
        let mut high = alternatives[0].clone();
        high.altitude_one = Some(35000);
        high.speed_knots = Some(230.0);
        let atmosphere = crate::traffic::air::Atmosphere::surface(1013.25, 15.0, 0.0).unwrap();
        let altitude = atmosphere.geometric(35000.0, true).unwrap();
        let mut high_clearance = crate::traffic::holding::Clearance {
            level_feet: 35000,
            geometric_feet: altitude,
            indicated_knots: 230.0,
            ground_knots: atmosphere.true_knots(230.0, altitude).unwrap(),
            bank_degrees: 25.0,
            turn_rate_degrees_sec: 3.0,
        };
        assert!(high_clearance.ground_knots > 300.0);
        assert!(
            crate::traffic::holding::Pattern::published(&high, position, &high_clearance)
                .unwrap()
                .compile(|_, _| true)
                .is_ok()
        );
        high_clearance.indicated_knots = 231.0;
        assert!(
            crate::traffic::holding::Pattern::published(&high, position, &high_clearance).is_err()
        );
        assert!(Catalogue::read(text.as_bytes(), &wanted, "2608").is_err());
        assert!(
            Catalogue::read(text.trim_end_matches("99\n").as_bytes(), &wanted, "2609").is_err()
        );
        assert!(
            Catalogue::read(
                text.replace("1.0 0.0", "0.0 0.0").as_bytes(),
                &wanted,
                "2609"
            )
            .is_err()
        );
    }
    #[test]
    #[ignore = "requires installed licensed navigation data"]
    fn installed_inler_holding_default_matches_the_approach_reference() {
        let root = std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap();
        let root = Path::new(&root);
        let procedures = Procedures::load(root, "LIMC").unwrap();
        let catalogues = [("LIMC", &procedures)];
        let mut points = crate::traffic::coordinates::Points::requested(&catalogues);
        points.load(root).unwrap();
        let holds = Catalogue::load(root, &catalogues, points.cycle.as_deref().unwrap()).unwrap();
        let approach = procedures.approach("I35RZ", "INLER").unwrap();
        let entry = approach.transition.first().unwrap();
        assert_eq!(entry.identifier, "INLER");
        let alternatives = holds.at(entry, "LIMC");
        assert!(!alternatives.is_empty());
        assert!(alternatives.iter().any(|leg| leg.altitude_limits().unwrap()
            == (Some(4000), None)
            && leg.course == Some(356.0)
            && leg.turn == "R"
            && leg.distance_time == "T010"
            && leg.speed_knots == Some(190.0)));
    }
}
