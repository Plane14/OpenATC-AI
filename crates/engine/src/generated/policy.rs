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
        std::fs::write(overrides.join("operations.toml"), "version = 999").unwrap();
        assert!(load(&directory).unwrap_err().contains("operations.toml"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
