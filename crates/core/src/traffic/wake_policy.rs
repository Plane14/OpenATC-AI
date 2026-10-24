//! Editable wake-turbulence and runway-relationship policy.
//!
#![allow(missing_docs)]
//!
//! The data lives in `traffic/wake.toml` so regional procedures can be changed without
//! rebuilding. The compiled fallback is the same file, embedded for tests and first start.
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{OnceLock, RwLock},
};

#[derive(Clone, Debug, Deserialize)]
pub struct File {
    pub version: u32,
    pub runways: RunwayRules,
    pub regions: BTreeMap<String, Region>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct RunwayRules {
    pub parallel_independent_m: f64,
    pub parallel_wake_m: f64,
    pub crossing_degrees: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Region {
    pub label: String,
    #[serde(default)]
    pub prefixes: Vec<String>,
    /// `cwt` reads the aircraft CWT letter; `wake` reads the wake letter or simulator category.
    pub category: String,
    pub categories: Vec<String>,
    pub lua_w_small_behind_heavy: bool,
    /// Region override for wake-related parallel runways.
    #[serde(default)]
    pub parallel_wake_m: Option<f64>,
    /// Region override for independent parallel operations.
    #[serde(default)]
    pub parallel_independent_m: Option<f64>,
    #[serde(default = "default_lua_w_seconds")]
    pub max_lua_w_seconds: u16,
    /// Rows are leader categories, columns follower categories, values nautical miles.
    pub arrival: Vec<Vec<f64>>,
    /// Rows are leader categories, columns follower categories,
    /// each cell `[full_length, intersection, opposite_direction]` seconds.
    pub departure: Vec<Vec<[u16; 3]>>,
}

fn default_lua_w_seconds() -> u16 {
    90
}

#[derive(Clone, Debug)]
pub struct Policy {
    pub file: File,
    pub standard: String,
}

impl Policy {
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: File = toml::from_str(text).map_err(|error| error.to_string())?;
        if file.version != 1 {
            return Err(format!("Unsupported wake policy version {}", file.version));
        }
        if !file.runways.parallel_independent_m.is_finite()
            || !file.runways.parallel_wake_m.is_finite()
            || !(0.0..=180.0).contains(&file.runways.crossing_degrees)
        {
            return Err("Wake policy runway limits are invalid".into());
        }
        for (key, region) in &file.regions {
            let width = region.categories.len();
            if width == 0
                || region.arrival.len() != width
                || region.departure.len() != width
                || region.arrival.iter().any(|row| row.len() != width)
                || region.departure.iter().any(|row| row.len() != width)
            {
                return Err(format!("Wake policy region {key} has inconsistent tables"));
            }
        }
        if !file.regions.contains_key("icao") {
            return Err("Wake policy needs an `icao` fallback region".into());
        }
        Ok(Self {
            file,
            standard: "auto".into(),
        })
    }

    pub fn with_standard(mut self, standard: &str) -> Self {
        self.standard = normalize_standard(standard).to_owned();
        self
    }

    #[must_use]
    pub fn runways(&self) -> RunwayRules {
        self.file.runways
    }

    #[must_use]
    pub fn region(&self, airport: &str) -> &Region {
        let key = if self.standard == "auto" {
            self.file
                .regions
                .iter()
                .filter(|(_, region)| {
                    region
                        .prefixes
                        .iter()
                        .any(|prefix| airport.starts_with(prefix))
                })
                .max_by_key(|(_, region)| {
                    region
                        .prefixes
                        .iter()
                        .filter(|prefix| airport.starts_with(prefix.as_str()))
                        .map(String::len)
                        .max()
                        .unwrap_or(0)
                })
                .map_or("icao", |(key, _)| key.as_str())
        } else {
            match self.standard.as_str() {
                "faa_cwt" => "faa",
                "mats" => "mats",
                "recat_eu" => "recat_eu",
                _ => "icao",
            }
        };
        self.file
            .regions
            .get(key)
            .or_else(|| self.file.regions.get("icao"))
            .expect("validated icao region")
    }

    #[must_use]
    pub fn category(&self, region: &Region, aircraft: &str, sim_wake: Option<u8>) -> Option<char> {
        if region.category == "cwt" {
            return super::profiles::aircraft(aircraft)
                .and_then(|profile| profile.cwt)
                .or_else(|| {
                    sim_wake.map(|value| match value {
                        0 => 'I',
                        1 => 'F',
                        2 => 'B',
                        3 => 'A',
                        _ => 'I',
                    })
                });
        }
        sim_wake
            .and_then(sim_category)
            .or_else(|| super::profiles::aircraft(aircraft).map(|profile| profile.wake))
    }

    #[must_use]
    pub fn arrival(
        &self,
        airport: &str,
        leader: &str,
        leader_wake: Option<u8>,
        follower: &str,
        follower_wake: Option<u8>,
    ) -> Option<f64> {
        let region = self.region(airport);
        let leader = self.category(region, leader, leader_wake)?;
        let follower = self.category(region, follower, follower_wake)?;
        let row = region
            .categories
            .iter()
            .position(|category| category.len() == 1 && category.starts_with(leader))?;
        let column = region
            .categories
            .iter()
            .position(|category| category.len() == 1 && category.starts_with(follower))?;
        region.arrival.get(row)?.get(column).copied()
    }

    #[must_use]
    pub fn departure(
        &self,
        airport: &str,
        leader: &str,
        leader_wake: Option<u8>,
        follower: &str,
        follower_wake: Option<u8>,
        kind: usize,
    ) -> Option<u16> {
        let region = self.region(airport);
        let leader = self.category(region, leader, leader_wake)?;
        let follower = self.category(region, follower, follower_wake)?;
        let row = region
            .categories
            .iter()
            .position(|category| category.len() == 1 && category.starts_with(leader))?;
        let column = region
            .categories
            .iter()
            .position(|category| category.len() == 1 && category.starts_with(follower))?;
        region
            .departure
            .get(row)?
            .get(column)
            .map(|cell| cell[kind.clamp(0, 2)])
    }

    #[must_use]
    pub fn lua_w_allowed(
        &self,
        airport: &str,
        leader: &str,
        leader_wake: Option<u8>,
        follower: &str,
        follower_wake: Option<u8>,
    ) -> bool {
        let region = self.region(airport);
        if region.lua_w_small_behind_heavy {
            return true;
        }
        let leader = self.category(region, leader, leader_wake);
        let follower = self.category(region, follower, follower_wake);
        !(matches!(leader, Some('A' | 'B' | 'C' | 'H' | 'J'))
            && matches!(follower, Some('G' | 'H' | 'I' | 'L')))
    }

    #[must_use]
    pub fn max_lua_w_seconds(&self, airport: &str) -> u16 {
        self.region(airport).max_lua_w_seconds
    }

    #[must_use]
    pub fn parallel_wake_m(&self, airport: &str) -> f64 {
        self.region(airport)
            .parallel_wake_m
            .unwrap_or(self.file.runways.parallel_wake_m)
    }

    #[must_use]
    pub fn parallel_independent_m(&self, airport: &str) -> f64 {
        self.region(airport)
            .parallel_independent_m
            .unwrap_or(self.file.runways.parallel_independent_m)
    }
}

fn sim_category(value: u8) -> Option<char> {
    Some(match value {
        0 => 'L',
        1 => 'M',
        2 => 'H',
        3 => 'J',
        _ => return None,
    })
}

fn normalize_standard(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "faa" | "faa_cwt" | "cwt" => "faa_cwt",
        "mats" | "australia" => "mats",
        "recat" | "recat_eu" | "eurocontrol" => "recat_eu",
        "icao" | "intl" => "icao",
        _ => "auto",
    }
}

static POLICY: OnceLock<RwLock<Policy>> = OnceLock::new();

fn global() -> &'static RwLock<Policy> {
    POLICY.get_or_init(|| {
        let policy = Policy::parse(include_str!("../../../../traffic/wake.toml"))
            .expect("bundled wake policy must parse");
        RwLock::new(policy)
    })
}

pub fn load(text: &str) -> Result<(), String> {
    let policy = Policy::parse(text)?;
    let standard = global()
        .read()
        .map_or("auto".into(), |current| current.standard.clone());
    *global()
        .write()
        .map_err(|_| "Wake policy lock poisoned".to_owned())? = policy.with_standard(&standard);
    Ok(())
}

pub fn set_standard(value: &str) {
    if let Ok(mut policy) = global().write() {
        policy.standard = normalize_standard(value).into();
    }
}

/// Borrow the active policy without cloning its tables.
pub fn read<R>(apply: impl FnOnce(&Policy) -> R) -> R {
    let guard = global()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    apply(&guard)
}

#[must_use]
pub fn current() -> Policy {
    global().read().map_or_else(
        |_| Policy::parse(include_str!("../../../../traffic/wake.toml")).unwrap(),
        |policy| policy.clone(),
    )
}
