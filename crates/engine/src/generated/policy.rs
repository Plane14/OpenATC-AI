//! Read traffic overrides during scene preparation, never while sampling movement.
use super::{Operations, animation, parse_operations};
use std::{collections::BTreeMap, io::Read, path::Path};

pub(super) fn load(
    config: &Path,
) -> Result<
    (
        BTreeMap<animation::Kind, animation::Handling>,
        Operations,
        BTreeMap<String, animation::Channel>,
    ),
    String,
> {
    let mut handling = animation::builtin()?;
    let mut operations = parse_operations(include_str!("../../../../traffic/operations.toml"))?;
    let mut channels =
        animation::channels(include_str!("../../../../traffic/animation-channels.toml"))?;
    for directory in crate::traffic_profiles::directories(config) {
        for name in [
            "handling.toml",
            "operations.toml",
            "animation-channels.toml",
        ] {
            let path = directory.join(name);
            let file = match std::fs::File::open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("{}: {error}", path.display())),
            };
            let mut text = String::new();
            file.take(65537)
                .read_to_string(&mut text)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let result = if name == "handling.toml" {
                animation::parse(&text).map(|loaded| handling = loaded)
            } else if name == "operations.toml" {
                parse_operations(&text).map(|loaded| operations = loaded)
            } else {
                animation::channels(&text).map(|loaded| channels = loaded)
            };
            result.map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    Ok((handling, operations, channels))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn personal_policy_changes_rates_and_rejects_invalid_geometry() {
        let directory =
            std::env::temp_dir().join(format!("openatc-generated-policy-{}", std::process::id()));
        let overrides = directory.join("traffic");
        std::fs::create_dir_all(&overrides).unwrap();
        std::fs::write(
            overrides.join("operations.toml"),
            include_str!("../../../../traffic/operations.toml").replace(
                "surface_segment_meters = 25.0",
                "surface_segment_meters = 30.0",
            ),
        )
        .unwrap();
        let (_, policy, _) = load(&directory).unwrap();
        assert_eq!(policy.surface_segment_meters, 30.0);
        let source = include_str!("../../../../traffic/operations.toml");
        let legacy_enroute = source
            .lines()
            .filter(|line| {
                ![
                    "enroute_segment_nm",
                    "airborne_sample_meters",
                    "airborne_detour_meters",
                    "airborne_turn_rate_degrees_sec",
                ]
                .iter()
                .any(|name| line.starts_with(name))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let default = parse_operations(&legacy_enroute).unwrap();
        assert_eq!(default.airborne_sample_meters, 250.0);
        assert_eq!(default.airborne_detour_meters, 5000.0);
        assert_eq!(default.airborne_turn_rate_degrees_sec, 3.0);
        for (from, to) in [
            ("network_airport_limit = 4", "network_airport_limit = 7"),
            (
                "network_candidate_limit = 16",
                "network_candidate_limit = 33",
            ),
            ("network_radius_nm = 750.0", "network_radius_nm = nan"),
            (
                "airborne_sample_meters = 250.0",
                "airborne_sample_meters = 251.0",
            ),
            (
                "airborne_detour_meters = 5000.0",
                "airborne_detour_meters = 10001.0",
            ),
            (
                "airborne_turn_rate_degrees_sec = 3.0",
                "airborne_turn_rate_degrees_sec = 0.0",
            ),
        ] {
            assert!(parse_operations(&source.replace(from, to)).is_err());
        }

        assert_eq!(
            parse_operations(&legacy_enroute)
                .unwrap()
                .enroute_segment_nm,
            20.0
        );
        for value in ["0.0", "nan", "51.0"] {
            assert!(
                parse_operations(&source.replace(
                    "enroute_segment_nm = 20.0",
                    &format!("enroute_segment_nm = {value}")
                ))
                .is_err()
            );
        }

        let legacy = source.replace("terminal_intercept_limit_nm = 20.0", "");
        assert_eq!(
            parse_operations(&legacy)
                .unwrap()
                .terminal_intercept_limit_nm,
            20.0
        );
        assert!(
            parse_operations(&source.replace(
                "terminal_intercept_limit_nm = 20.0",
                "terminal_intercept_limit_nm = 0.0"
            ))
            .is_err()
        );
        let legacy = source
            .lines()
            .filter(|line| {
                ![
                    "visible_aircraft_limit",
                    "display_enter_nm",
                    "display_leave_nm",
                    "display_replacement_margin_nm",
                ]
                .iter()
                .any(|name| line.starts_with(name))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let defaults = parse_operations(&legacy).unwrap();
        assert_eq!(defaults.visible_aircraft_limit, 63);
        assert_eq!(defaults.display_enter_nm, 40.0);
        for (from, to) in [
            ("visible_aircraft_limit = 63", "visible_aircraft_limit = 64"),
            ("display_enter_nm = 40.0", "display_enter_nm = nan"),
            ("display_leave_nm = 50.0", "display_leave_nm = 39.0"),
            (
                "display_replacement_margin_nm = 2.0",
                "display_replacement_margin_nm = -1.0",
            ),
        ] {
            assert!(parse_operations(&source.replace(from, to)).is_err());
        }
        std::fs::write(overrides.join("operations.toml"), "version = 999").unwrap();
        assert!(load(&directory).unwrap_err().contains("operations.toml"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
