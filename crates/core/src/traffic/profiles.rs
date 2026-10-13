//! Editable traffic preferences and aircraft envelopes, independent of CSL paint names.
use super::csl::Catalogue;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Small bundled lookup for operational checks that cannot wait for an editable catalogue reload.
#[must_use]
pub fn aircraft(code: &str) -> Option<&'static Aircraft> {
    static DATA: std::sync::OnceLock<Option<Profiles>> = std::sync::OnceLock::new();
    let code = code.trim().to_ascii_uppercase();
    let code = match code.as_str() {
        "P28A" | "P28B" | "P28R" | "P28T" => "PA28",
        other => other,
    };
    DATA.get_or_init(|| Profiles::builtin().ok())
        .as_ref()?
        .aircraft
        .get(code)
}

/// Activity mix. These categories do not determine wake or approach category.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Mainline passenger operators.
    Airline,
    /// Regional passenger operators, including turboprops.
    Regional,
    /// Freight operators.
    Cargo,
    /// Private and business IFR aircraft.
    GeneralAviation,
}

/// Nominal movement envelope for generated aircraft, not an aircraft flight manual.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Aircraft {
    /// Human-readable type.
    pub name: String,
    /// Metres.
    pub wingspan: f64,
    /// Metres, used to keep the tail clear of intersections.
    pub length: f64,
    /// L, M, H or J; approach category is deliberately separate.
    pub wake: char,
    /// FAA consolidated wake category, separate from ICAO wake and approach category.
    #[serde(default)]
    pub cwt: Option<char>,
    /// ICAO approach category A through E.
    pub approach_category: char,
    /// Conservative dry-runway planning floor in metres.
    pub runway: f64,
    /// Nominal range in nautical miles; reserve is applied when selecting routes.
    pub range_nm: f64,
    /// Cruise true airspeed in knots.
    pub cruise_knots: f64,
    /// Maximum planned cruise altitude, feet MSL.
    pub ceiling: f64,
    /// Nominal final approach speed, knots.
    pub approach_knots: f64,
    /// Nominal climb rate, feet per minute.
    pub climb_fpm: f64,
    /// Nominal descent rate, feet per minute.
    pub descent_fpm: f64,
    /// Propeller animation is required when supported by the model.
    pub propeller: bool,
    /// Equipment assumed for this simulated aircraft.
    pub rnav: bool,
}

impl Aircraft {
    /// Parking size derived from wingspan, never from wake or approach category.
    #[must_use]
    pub fn parking_size(&self) -> char {
        [15.0, 24.0, 36.0, 52.0, 65.0]
            .iter()
            .position(|limit| self.wingspan < *limit)
            .map_or('F', |n| char::from(b'A' + n as u8))
    }
}

/// One operating fleet. A regional subsidiary can have its own entry and callsign.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Airline {
    /// Display name.
    pub name: String,
    /// ICAO operating code, empty for private registrations.
    pub operator: String,
    /// Spoken telephony name.
    pub callsign: String,
    /// Traffic-mix category.
    pub category: Category,
    /// At least one end must be in this home area.
    pub home_prefixes: Vec<String>,
    /// Both ends must be in these ICAO prefix areas; an empty list means worldwide.
    pub serves: Vec<String>,
    /// ICAO aircraft types and relative weights, not actual fleet counts.
    pub fleet: BTreeMap<String, u32>,
    /// Explicit permitted marketing liveries. The operating callsign stays unchanged.
    #[serde(default)]
    pub liveries: Vec<String>,
}

/// Airport-specific operator preferences, not market-share percentages.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Hub {
    /// Readable airport label.
    pub name: String,
    /// Keys refer to airline profile IDs, including regional fleets.
    pub airlines: BTreeMap<String, u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AircraftFile {
    version: u32,
    aircraft: BTreeMap<String, Aircraft>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AirlineFile {
    version: u32,
    airline: BTreeMap<String, Airline>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HubFile {
    version: u32,
    airport: BTreeMap<String, Hub>,
}

/// Validated profile set, shared between traffic generation and diagnostics.
#[derive(Clone, Debug, Serialize)]
pub struct Profiles {
    /// ICAO type to movement envelope.
    pub aircraft: BTreeMap<String, Aircraft>,
    /// Stable profile ID to operating fleet.
    pub airlines: BTreeMap<String, Airline>,
    /// ICAO airport to operator preferences.
    pub hubs: BTreeMap<String, Hub>,
}

/// Only known runway and parking capacities may be used to generate a flight.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Port {
    /// ICAO airport identifier.
    pub icao: String,
    /// Longest usable runway, metres.
    pub runway: f64,
    /// Largest known compatible parking size.
    pub parking_size: char,
}

/// A model-compatible choice. Stable seeds do not depend on render order.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Selection {
    /// Profile ID, independent of painted marketing airline.
    pub airline: String,
    /// Operating airline or empty for private aircraft.
    pub operator: String,
    /// Spoken operator name.
    pub spoken_operator: String,
    /// ICAO aircraft type.
    pub aircraft: String,
    /// CSL identity; renderer loads every part of this model.
    pub model: String,
    /// Selected painted operator, which may be an explicitly allowed partner.
    pub model_operator: String,
    /// Activity category for density controls.
    pub category: Category,
}

impl Profiles {
    /// Parse all three files together so unresolved fleet and hub references fail visibly.
    pub fn parse(aircraft: &str, airlines: &str, hubs: &str) -> Result<Self, String> {
        if [aircraft, airlines, hubs]
            .iter()
            .any(|s| s.len() > 1024 * 1024)
        {
            return Err("Traffic profile file exceeds 1 MiB".into());
        }
        let a: AircraftFile =
            toml::from_str(aircraft).map_err(|e| format!("aircraft.toml: {e}"))?;
        let l: AirlineFile = toml::from_str(airlines).map_err(|e| format!("airlines.toml: {e}"))?;
        let h: HubFile = toml::from_str(hubs).map_err(|e| format!("hubs.toml: {e}"))?;
        if [a.version, l.version, h.version] != [1; 3] {
            return Err("Traffic profiles require version = 1".into());
        }
        let result = Self {
            aircraft: a.aircraft,
            airlines: l.airline,
            hubs: h.airport,
        };
        result.validate()?;
        Ok(result)
    }

    /// Bundled profiles are also shipped as editable TOML files.
    pub fn builtin() -> Result<Self, String> {
        Self::parse(
            include_str!("../../../../traffic/aircraft.toml"),
            include_str!("../../../../traffic/airlines.toml"),
            include_str!("../../../../traffic/hubs.toml"),
        )
    }

    fn validate(&self) -> Result<(), String> {
        if self.aircraft.is_empty()
            || self.airlines.is_empty()
            || self.aircraft.len() > 512
            || self.airlines.len() > 2048
            || self.hubs.len() > 8192
        {
            return Err("Traffic profile counts are empty or exceed supported limits".into());
        }
        for (id, a) in &self.aircraft {
            if !code(id, 2, 4)
                || a.name.trim().is_empty()
                || !"LMHJ".contains(a.wake)
                || a.cwt.is_some_and(|c| !(('A'..='I').contains(&c)))
                || !"ABCDE".contains(a.approach_category)
                || !bounded(a.wingspan, 5.0, 90.0)
                || !bounded(a.length, 5.0, 90.0)
                || !bounded(a.runway, 300.0, 5000.0)
                || !bounded(a.range_nm, 50.0, 12000.0)
                || !bounded(a.cruise_knots, 70.0, 550.0)
                || !bounded(a.ceiling, 5000.0, 60000.0)
                || !bounded(a.approach_knots, 45.0, 180.0)
                || a.approach_knots >= a.cruise_knots
                || !bounded(a.climb_fpm, 200.0, 5000.0)
                || !bounded(a.descent_fpm, 200.0, 4000.0)
            {
                return Err(format!("Invalid aircraft envelope: {id}"));
            }
        }
        for (id, a) in &self.airlines {
            if id.is_empty()
                || id.len() > 40
                || a.name.trim().is_empty()
                || (a.category != Category::GeneralAviation
                    && (!code(&a.operator, 3, 3)
                        || a.callsign.trim().is_empty()
                        || a.home_prefixes.is_empty()))
                || a.home_prefixes
                    .iter()
                    .chain(&a.serves)
                    .any(|s| !code(s, 1, 4))
                || a.liveries.iter().any(|s| !code(s, 3, 3))
                || a.liveries.len() > 16
                || a.fleet.is_empty()
                || a.fleet
                    .iter()
                    .any(|(t, w)| !self.aircraft.contains_key(t) || *w == 0 || *w > 1000)
            {
                return Err(format!(
                    "Invalid airline profile or unknown fleet type: {id}"
                ));
            }
        }
        for (id, h) in &self.hubs {
            if !code(id, 3, 4)
                || h.name.trim().is_empty()
                || h.airlines.is_empty()
                || h.airlines
                    .iter()
                    .any(|(id, w)| !self.airlines.contains_key(id) || *w == 0 || *w > 1000)
            {
                return Err(format!("Invalid hub preference or unknown airline: {id}"));
            }
        }
        Ok(())
    }

    /// Match both airports, route range and the installed model inventory before drawing lots.
    /// Empty model/type intersections return no flight; another airline's paint is never invented.
    #[must_use]
    pub fn select(
        &self,
        origin: &Port,
        destination: &Port,
        distance_nm: f64,
        category: Category,
        catalogue: &Catalogue,
        seed: u64,
    ) -> Option<Selection> {
        if !distance_nm.is_finite()
            || distance_nm < 5.0
            || origin.icao == destination.icao
            || ![origin, destination].iter().all(|p| {
                code(&p.icao, 3, 4)
                    && bounded(p.runway, 300.0, 10000.0)
                    && "ABCDEF".contains(p.parking_size)
            })
        {
            return None;
        }
        let mut candidates = Vec::new();
        for (id, airline) in &self.airlines {
            if airline.category != category
                || !serves(airline, &origin.icao)
                || !serves(airline, &destination.icao)
            {
                continue;
            }
            let oh = self.hub_weight(id, &origin.icao);
            let dh = self.hub_weight(id, &destination.icao);
            if category != Category::GeneralAviation
                && ![origin, destination].iter().any(|p| {
                    airline
                        .home_prefixes
                        .iter()
                        .any(|prefix| p.icao.starts_with(prefix))
                })
            {
                continue;
            }
            let mut fleet = Vec::new();
            for (kind, weight) in &airline.fleet {
                let aircraft = &self.aircraft[kind];
                if distance_nm > aircraft.range_nm * 0.8
                    || [origin, destination].iter().any(|p| {
                        p.runway < aircraft.runway || p.parking_size < aircraft.parking_size()
                    })
                {
                    continue;
                }
                // Airline liveries take precedence over the generic same-type fallback.
                let paints: BTreeSet<_> = std::iter::once(airline.operator.as_str())
                    .chain(airline.liveries.iter().map(String::as_str))
                    .collect();
                let models: Vec<_> = catalogue
                    .models
                    .iter()
                    .filter_map(|m| {
                        let paint = m.matches.iter().find(|v| {
                            v.aircraft == *kind
                                && paints.contains(v.operator.as_str())
                                && !v.operator.is_empty()
                        })?;
                        Some((m, paint.operator.as_str()))
                    })
                    .collect();
                let choice = if models.is_empty() {
                    catalogue
                        .choose(kind, &airline.operator, "", mix(seed ^ stable_hash(kind)))
                        .map(|m| (m, ""))
                } else {
                    Some(models[(mix(seed ^ stable_hash(kind)) % models.len() as u64) as usize])
                };
                if let Some((model, paint)) = choice {
                    fleet.push((
                        *weight as u64,
                        Selection {
                            airline: id.clone(),
                            operator: airline.operator.clone(),
                            spoken_operator: airline.callsign.clone(),
                            aircraft: kind.clone(),
                            model: model.id.clone(),
                            model_operator: paint.into(),
                            category,
                        },
                    ));
                }
            }
            if let Some(selected) = pick(&fleet, mix(seed ^ stable_hash(id))) {
                let weight = (u64::from(oh) + 2) * (u64::from(dh) + 2);
                candidates.push((weight, selected.clone()));
            }
        }
        pick(&candidates, mix(seed)).cloned()
    }

    fn hub_weight(&self, airline: &str, airport: &str) -> u32 {
        self.hubs
            .get(airport)
            .and_then(|h| h.airlines.get(airline))
            .copied()
            .unwrap_or(0)
    }
}

fn serves(airline: &Airline, airport: &str) -> bool {
    airline.serves.is_empty() || airline.serves.iter().any(|p| airport.starts_with(p))
}
fn bounded(v: f64, low: f64, high: f64) -> bool {
    v.is_finite() && (low..=high).contains(&v)
}
fn code(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}
fn pick<T>(values: &[(u64, T)], seed: u64) -> Option<&T> {
    let sum: u64 = values.iter().map(|(w, _)| *w).sum();
    if sum == 0 {
        return None;
    }
    let mut draw = seed % sum;
    for (weight, value) in values {
        if draw < *weight {
            return Some(value);
        }
        draw -= *weight;
    }
    None
}
/// Stable mixer; results do not depend on Rust's hash-map seed or operating system.
#[must_use]
pub fn mix(mut v: u64) -> u64 {
    v = v.wrapping_add(0x9e3779b97f4a7c15);
    v = (v ^ (v >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    v = (v ^ (v >> 27)).wrapping_mul(0x94d049bb133111eb);
    v ^ (v >> 31)
}
/// FNV-1a string seed, stable across application restarts.
#[must_use]
pub fn stable_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::csl::{Match, Model};
    fn models(profiles: &Profiles) -> Catalogue {
        Catalogue {
            models: profiles
                .airlines
                .values()
                .flat_map(|a| {
                    a.fleet.keys().map(move |t| Model {
                        id: format!("{}/{t}", a.operator),
                        objects: vec![],
                        vertical_offset: None,
                        matches: vec![Match {
                            aircraft: t.clone(),
                            operator: a.operator.clone(),
                            livery: String::new(),
                        }],
                    })
                })
                .collect(),
            warnings: vec![],
        }
    }
    fn port(id: &str) -> Port {
        Port {
            icao: id.into(),
            runway: 4000.0,
            parking_size: 'F',
        }
    }
    #[test]
    fn profiles_resolve_all_references_and_preserve_type_categories() {
        let p = Profiles::builtin().unwrap();
        assert!(p.airlines.len() >= 20 && p.hubs.len() >= 30 && p.aircraft.len() >= 30);
        assert_eq!(p.aircraft["A320"].parking_size(), 'C');
        assert_eq!(p.aircraft["A388"].wake, 'J');
        assert!(p.aircraft["AT76"].propeller);
        assert_eq!(p.airlines["SIA"].fleet.get("B78X"), Some(&15));
        assert!(!p.airlines["SIA"].fleet.contains_key("B789"));
    }
    #[test]
    fn selection_uses_both_ends_hubs_suitability_and_available_paint() {
        let p = Profiles::builtin().unwrap();
        let c = models(&p);
        let mut local = 0;
        for seed in 0..500 {
            let s = p
                .select(
                    &port("YSSY"),
                    &port("YMML"),
                    400.0,
                    Category::Airline,
                    &c,
                    seed,
                )
                .unwrap();
            assert!(matches!(s.operator.as_str(), "QFA" | "JST" | "VOZ"));
            local += usize::from(s.operator == "QFA");
            assert_eq!(
                s,
                p.select(
                    &port("YSSY"),
                    &port("YMML"),
                    400.0,
                    Category::Airline,
                    &c,
                    seed
                )
                .unwrap()
            );
        }
        assert!(local > 200);
        let small = Port {
            icao: "KSMO".into(),
            runway: 1000.0,
            parking_size: 'A',
        };
        assert!(
            p.select(&small, &port("KLAX"), 20.0, Category::Airline, &c, 1)
                .is_none()
        );
        assert!(
            p.select(
                &small,
                &port("KLGB"),
                40.0,
                Category::GeneralAviation,
                &c,
                1
            )
            .is_some()
        );
        assert!(
            p.select(
                &port("KLAX"),
                &port("KSNA"),
                40.0,
                Category::Airline,
                &Catalogue::default(),
                1
            )
            .is_none()
        );
        assert!(
            p.select(
                &port("KLAX"),
                &port("KSNA"),
                f64::NAN,
                Category::Airline,
                &c,
                1
            )
            .is_none()
        );
        let distant = p
            .select(
                &port("WSSS"),
                &port("YSSY"),
                3400.0,
                Category::Airline,
                &c,
                5,
            )
            .unwrap();
        assert!(p.aircraft[&distant.aircraft].range_nm * 0.8 >= 3400.0);
    }
    #[test]
    fn malformed_or_unresolved_edits_fail_instead_of_silently_changing_traffic() {
        let a = include_str!("../../../../traffic/aircraft.toml");
        let l = include_str!("../../../../traffic/airlines.toml");
        let h = include_str!("../../../../traffic/hubs.toml");
        assert!(Profiles::parse(a, &l.replace("B78X = 15", "ZZZZ = 15"), h).is_err());
        assert!(Profiles::parse(&a.replacen("wingspan =", "wingpsan =", 1), l, h).is_err());
        assert!(Profiles::parse(a, l, &h.replacen("version = 1", "version = 2", 1)).is_err());
    }
}
