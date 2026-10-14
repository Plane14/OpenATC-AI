//! Resolve only the terminal references a worker needs, keeping region and owner identity.
use super::procedures::{Catalogue, Leg};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Fix,
    Vor,
    Ndb,
    Localizer,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    kind: Kind,
    identifier: String,
    region: String,
    owner: String,
}

fn key(leg: &Leg, airport: &str) -> Option<Key> {
    reference_key(
        &leg.identifier,
        &leg.region,
        &leg.section,
        &leg.subsection,
        airport,
    )
}
fn reference_key(
    identifier: &str,
    region: &str,
    section: &str,
    subsection: &str,
    airport: &str,
) -> Option<Key> {
    let (kind, owner) = match (section, subsection) {
        ("E", "A") => (Kind::Fix, "ENRT"),
        ("P", "C") => (Kind::Fix, airport),
        ("D", "") => (Kind::Vor, "ENRT"),
        ("D", "B") => (Kind::Ndb, "ENRT"),
        ("P", "N") => (Kind::Ndb, airport),
        ("P", "I") => (Kind::Localizer, airport),
        _ => return None,
    };
    if identifier.is_empty() {
        return None;
    }
    Some(Key {
        kind,
        identifier: identifier.into(),
        region: region.into(),
        owner: owner.into(),
    })
}

/// Coordinate and navaid reference without guessing a same-name fix in another region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    /// Latitude/longitude in degrees.
    pub coordinate: [f64; 2],
    /// Published navaid elevation, absent on ordinary fixes.
    pub elevation_feet: Option<f64>,
    /// Published VOR slaved variation, absent on fixes and localizers.
    pub magnetic_variation_degrees: Option<f64>,
}

/// Only requested references are retained; conflicting records stay unresolved.
#[derive(Clone, Debug, Default)]
pub struct Points {
    /// AIRAC cycle shared by the loaded fix and navaid files.
    pub cycle: Option<String>,
    wanted: BTreeSet<Key>,
    names: BTreeSet<String>,
    found: BTreeMap<Key, Option<Position>>,
}
impl Points {
    /// Resolve a runway fix against the installed landing threshold, including displacement.
    pub fn runway(leg: &Leg, airport: &crate::airport::Airport) -> Option<Position> {
        if leg.section != "P" || leg.subsection != "G" {
            return None;
        }
        let name = leg.identifier.strip_prefix("RW")?;
        let runway = airport
            .runways
            .iter()
            .find(|runway| runway.first_name == name || runway.second_name == name)?;
        let (start, end, displacement) = if runway.first_name == name {
            (runway.first, runway.second, runway.first_displaced_meters)
        } else {
            (runway.second, runway.first, runway.second_displaced_meters)
        };
        let length = (end.east - start.east).hypot(end.north - start.north);
        if length < 100.0
            || !(0.0..length).contains(&displacement)
            || airport.reference_latitude.abs() >= 85.0
        {
            return None;
        }
        let east = start.east + (end.east - start.east) * displacement / length;
        let north = start.north + (end.north - start.north) * displacement / length;
        Some(Position {
            coordinate: [
                airport.reference_latitude + north / 111320.0,
                airport.reference_longitude
                    + east / (111320.0 * airport.reference_latitude.to_radians().cos()),
            ],
            elevation_feet: Some(airport.elevation_feet),
            magnetic_variation_degrees: None,
        })
    }
    /// Collect both terminal airports before scanning global navdata once.
    pub fn requested(catalogues: &[(&str, &Catalogue)]) -> Self {
        let wanted: BTreeSet<Key> = catalogues
            .iter()
            .flat_map(|(airport, catalogue)| {
                catalogue.0.values().flatten().flat_map(|leg| {
                    key(leg, airport)
                        .into_iter()
                        .chain(leg.recommended.as_ref().and_then(|reference| {
                            reference_key(
                                &reference.identifier,
                                &reference.region,
                                &reference.section,
                                &reference.subsection,
                                airport,
                            )
                        }))
                })
            })
            .collect();
        Self {
            names: wanted.iter().map(|key| key.identifier.clone()).collect(),
            wanted,
            ..Default::default()
        }
    }
    /// A missing or ambiguous identifier cannot authorize a generated leg.
    pub fn resolve(&self, leg: &Leg, airport: &str) -> Option<Position> {
        self.found.get(&key(leg, airport)?).copied().flatten()
    }
    /// A recommended VOR can retain its published slaved variation instead of using today's field model.
    pub fn course_variation(&self, leg: &Leg, airport: &str) -> Option<f64> {
        let recommended = leg.recommended.as_ref().and_then(|reference| {
            reference_key(
                &reference.identifier,
                &reference.region,
                &reference.section,
                &reference.subsection,
                airport,
            )
        });
        recommended
            .as_ref()
            .and_then(|key| self.found.get(key).copied().flatten())
            .and_then(|position| position.magnetic_variation_degrees)
            .or_else(|| {
                self.resolve(leg, airport)
                    .and_then(|position| position.magnetic_variation_degrees)
            })
    }
    fn insert(&mut self, key: Key, position: Position) {
        if !self.wanted.contains(&key) {
            return;
        }
        self.found
            .entry(key)
            .and_modify(|old| {
                if old.is_some_and(|old| {
                    crate::arrival::distance_nm(old.coordinate, position.coordinate) > 0.001
                }) {
                    *old = None;
                }
            })
            .or_insert(Some(position));
    }
    fn line(&mut self, line: &str, navaids: bool) {
        let mut tokens = line.split_whitespace();
        let fields: [&str; 11] = std::array::from_fn(|_| tokens.next().unwrap_or_default());
        let identifier = if navaids { fields[7] } else { fields[2] };
        if !self.names.contains(identifier) {
            return;
        }
        let parsed = if navaids && !fields[9].is_empty() {
            let kind = match fields[0] {
                "2" => Kind::Ndb,
                "3" => Kind::Vor,
                "4" | "5" => Kind::Localizer,
                _ => return,
            };
            Some((
                kind,
                fields[1],
                fields[2],
                fields[7],
                fields[9],
                fields[8],
                Some(fields[3]),
                Some(fields[6]),
            ))
        } else if !navaids && !fields[4].is_empty() {
            Some((
                Kind::Fix,
                fields[0],
                fields[1],
                fields[2],
                fields[4],
                fields[3],
                None,
                None,
            ))
        } else {
            None
        };
        let Some((kind, latitude, longitude, identifier, region, owner, elevation, variation)) =
            parsed
        else {
            return;
        };
        let (Ok(latitude), Ok(longitude)) = (latitude.parse::<f64>(), longitude.parse::<f64>())
        else {
            return;
        };
        if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
            return;
        }
        let magnetic_variation_degrees = if kind == Kind::Vor {
            variation
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| (-180.0..=180.0).contains(value))
        } else {
            None
        };
        self.insert(
            Key {
                kind,
                identifier: identifier.into(),
                region: region.into(),
                owner: owner.into(),
            },
            Position {
                coordinate: [latitude, longitude],
                elevation_feet: elevation
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| (-2000.0..=30000.0).contains(value)),
                magnetic_variation_degrees,
            },
        );
    }
    /// Bounded streaming reads belong on the scenery/navigation worker pool.
    pub fn load(&mut self, root: &Path) -> Result<(), String> {
        let mut loaded = Self {
            wanted: self.wanted.clone(),
            names: self.names.clone(),
            ..Default::default()
        };
        let mut cycle = None;
        for (name, navaids) in [("earth_fix.dat", false), ("earth_nav.dat", true)] {
            let custom = root.join("Custom Data").join(name);
            let path = if custom.is_file() {
                custom
            } else {
                root.join("Resources/default data").join(name)
            };
            let file = std::fs::File::open(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            if file.metadata().map_err(|error| error.to_string())?.len() > 64 * 1024 * 1024 {
                return Err("Navigation file exceeds 64 MiB".into());
            }
            for (index, line) in BufReader::new(file).lines().enumerate() {
                let line = line.map_err(|error| error.to_string())?;
                if index > 500000 || line.len() > 4096 {
                    return Err("Navigation file exceeds record limits".into());
                }
                if index < 4
                    && let Some((_, suffix)) = line.split_once("data cycle ")
                {
                    let current = suffix.chars().take(4).collect::<String>();
                    if cycle.as_ref().is_some_and(|previous| previous != &current) {
                        return Err("Fix and navaid data cycles do not match".into());
                    }
                    cycle = Some(current);
                }
                loaded.line(&line, navaids);
            }
            let user = root.join("Custom Data").join(if navaids {
                "user_nav.dat"
            } else {
                "user_fix.dat"
            });
            if user.is_file() {
                let file = std::fs::File::open(&user).map_err(|error| error.to_string())?;
                if file.metadata().map_err(|error| error.to_string())?.len() > 4 * 1024 * 1024 {
                    return Err("User navigation file exceeds 4 MiB".into());
                }
                let mut overrides = Self {
                    wanted: loaded.wanted.clone(),
                    names: loaded.names.clone(),
                    ..Default::default()
                };
                for line in BufReader::new(file).lines() {
                    let line = line.map_err(|error| error.to_string())?;
                    if line.len() > 4096 {
                        return Err("User navigation record exceeds its limit".into());
                    }
                    overrides.line(&line, navaids);
                }
                loaded.found.extend(overrides.found);
            }
        }
        self.found = loaded.found;
        self.cycle = cycle;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed licensed navigation data"]
    fn installed_milan_restriction_and_missed_hold_resolve_the_correct_references() {
        let root = std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap();
        let catalogue = Catalogue::load(Path::new(&root), "LIMC").unwrap();
        let mut points = Points::requested(&[("LIMC", &catalogue)]);
        points.load(Path::new(&root)).unwrap();
        let crossing = catalogue
            .0
            .iter()
            .filter(|(key, _)| key.name == "DEVO3E")
            .flat_map(|(_, legs)| legs)
            .find(|leg| leg.identifier == "MC588")
            .unwrap();
        let coordinate = points.resolve(crossing, "LIMC").unwrap().coordinate;
        assert!((45.0..46.0).contains(&coordinate[0]));
        assert!((8.0..10.0).contains(&coordinate[1]));
        let hold = catalogue
            .0
            .iter()
            .filter(|(key, _)| key.name == "I35RZ" && key.transition.is_empty())
            .flat_map(|(_, legs)| legs)
            .find(|leg| leg.terminator == "HM")
            .unwrap();
        let station = points.resolve(hold, "LIMC").unwrap();
        assert!((station.coordinate[0] - 45.645972222).abs() < 0.00001);
        assert!(station.magnetic_variation_degrees.is_some());
        let clearance = super::super::holding::Clearance {
            turn_rate_degrees_sec: 3.0,
            level_feet: 6000,
            geometric_feet: 6100.0,
            indicated_knots: 180.0,
            ground_knots: 190.0,
            bank_degrees: 25.0,
        };
        let pattern = super::super::holding::Pattern::published(hold, station, &clearance).unwrap();
        assert_eq!(pattern.leg_seconds, 60.0);
        assert!(pattern.compile(|_, _| true).is_ok());
        assert!(
            super::super::holding::Pattern::published(
                hold,
                station,
                &super::super::holding::Clearance {
                    level_feet: 3500,
                    ..clearance
                }
            )
            .is_err()
        );
        assert!(points.resolve(crossing, "LFMN").is_none());
    }
    #[test]
    fn terminal_owner_and_region_disambiguate_repeated_names() {
        let mut fields = vec![""; 38];
        fields[0] = "010";
        fields[1] = "I";
        fields[2] = "TEST";
        fields[4] = "SAME";
        fields[5] = "LI";
        fields[6] = "P";
        fields[7] = "C";
        fields[11] = "IF";
        let catalogue = Catalogue::parse(&format!("APPCH:{};", fields.join(","))).unwrap();
        let leg = &catalogue.0.values().next().unwrap()[0];
        let mut points = Points::requested(&[("LIMC", &catalogue)]);
        points.line("45.0 8.0 SAME ENRT LI", false);
        points.line("44.0 7.0 SAME LFMN LF", false);
        assert!(points.resolve(leg, "LIMC").is_none());
        points.line("45.6 8.7 SAME LIMC LI", false);
        assert_eq!(points.resolve(leg, "LIMC").unwrap().coordinate, [45.6, 8.7]);
        points.line("45.7 8.8 SAME LIMC LI", false);
        assert!(points.resolve(leg, "LIMC").is_none());
    }
}
