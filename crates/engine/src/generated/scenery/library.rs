//! Conservative library variants for obstacle bounds, including seasonal and regional alternatives.
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Component, Path, PathBuf},
};

pub(super) struct Library {
    exports: BTreeMap<String, BTreeSet<PathBuf>>,
    incomplete: BTreeSet<String>,
}

fn relative(text: &str) -> Result<&Path, String> {
    let path = Path::new(text);
    if text.is_empty()
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err("Scenery library resource leaves its package".into());
    }
    Ok(path)
}

impl Library {
    pub fn load(root: &Path, packs: &[PathBuf]) -> Result<Self, String> {
        let mut folders = packs.iter().cloned().collect::<BTreeSet<_>>();
        let defaults = root.join("Resources/default scenery");
        if defaults.is_dir() {
            for (index, entry) in std::fs::read_dir(defaults)
                .map_err(|e| e.to_string())?
                .enumerate()
            {
                if index >= 1024 {
                    return Err("Default scenery package budget exceeded".into());
                }
                let entry = entry.map_err(|e| e.to_string())?;
                if entry.path().is_dir() {
                    folders.insert(entry.path());
                }
            }
        }
        if folders.len() > 2048 {
            return Err("Scenery library package budget exceeded".into());
        }
        let mut result = Self {
            exports: BTreeMap::new(),
            incomplete: BTreeSet::new(),
        };
        let mut bytes = 0;
        for folder in folders {
            let path = folder.join("library.txt");
            if !path.is_file() {
                continue;
            }
            let mut text = String::new();
            std::fs::File::open(path)
                .map_err(|e| e.to_string())?
                .take(8 * 1024 * 1024 + 1)
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            bytes += text.len();
            if text.len() > 8 * 1024 * 1024 || bytes > 32 * 1024 * 1024 {
                return Err("Scenery library input budget exceeded".into());
            }
            result
                .add(&folder, &text)
                .map_err(|error| format!("{}: {error}", folder.display()))?;
        }
        Ok(result)
    }

    fn add(&mut self, folder: &Path, text: &str) -> Result<(), String> {
        let mut lines = text
            .lines()
            .map(|line| line.split('#').next().unwrap_or("").trim())
            .filter(|line| !line.is_empty());
        if !matches!(lines.next(), Some("A" | "I"))
            || !matches!(lines.next(), Some("800" | "1200"))
            || lines.next() != Some("LIBRARY")
        {
            return Err("Unsupported scenery library header".into());
        }
        for line in lines {
            let mut rest = line;
            let mut next = || {
                rest = rest.trim_start();
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let token = &rest[..end];
                rest = &rest[end..];
                token
            };
            let command = next();
            if !command.starts_with("EXPORT") {
                continue;
            }
            if !matches!(
                command,
                "EXPORT"
                    | "EXPORT_RATIO"
                    | "EXPORT_EXTEND"
                    | "EXPORT_EXCLUDE"
                    | "EXPORT_BACKUP"
                    | "EXPORT_SEASON"
                    | "EXPORT_RATIO_SEASON"
                    | "EXPORT_EXTEND_SEASON"
                    | "EXPORT_EXCLUDE_SEASON"
            ) {
                return Err(format!("Unresolved scenery library directive {command}"));
            }
            if command.ends_with("_SEASON") {
                let seasons = next();
                if seasons.is_empty()
                    || !seasons
                        .split(',')
                        .all(|season| matches!(season, "spr" | "sum" | "fal" | "win"))
                {
                    return Err("Invalid library season list".into());
                }
            }
            if command.starts_with("EXPORT_RATIO") {
                let ratio = next()
                    .parse::<f64>()
                    .map_err(|_| "Invalid library export ratio")?;
                if !ratio.is_finite() || ratio < 0.0 {
                    return Err("Invalid library export ratio".into());
                }
            }
            // A leading slash belongs to the virtual namespace, never the host filesystem.
            let name = next().trim_start_matches('/');
            relative(name)?;
            if name.len() > 8192 || rest.len() > 8192 || self.exports.len() >= 200_000 {
                return Err("Scenery library export budget exceeded".into());
            }
            let destination = match relative(rest.trim()) {
                Ok(path) => path,
                Err(_) => {
                    self.incomplete.insert(name.to_owned());
                    continue;
                }
            };
            let variants = self.exports.entry(name.to_owned()).or_default();
            let path = folder.join(destination);
            if variants.len() >= 256 && !variants.contains(&path) {
                self.incomplete.insert(name.to_owned());
                continue;
            }
            // Keeping lower-priority and inactive variants can reject a path, but cannot shrink a building.
            variants.insert(path);
        }
        Ok(())
    }

    pub fn resources(&self, pack: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
        let name = name.trim_start_matches('/');
        if self.incomplete.contains(name) {
            return Err(format!(
                "Scenery resource exceeds its variant budget: {name}"
            ));
        }
        let local = pack.join(relative(name)?);
        let mut paths = self.exports.get(name).cloned().unwrap_or_default();
        if paths.is_empty() || local.is_file() {
            paths.insert(local);
        }
        Ok(paths.into_iter().collect())
    }

    pub fn paved_surface(&self, pack: &Path, name: &str) -> Result<bool, String> {
        for path in self.resources(pack, name)? {
            let mut text = String::new();
            std::fs::File::open(&path)
                .map_err(|e| e.to_string())?
                .take(64 * 1024 + 1)
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            if text.len() > 64 * 1024 {
                return Err("Draped surface definition exceeds its budget".into());
            }
            if !paved_definition(&text) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn ground_only(&self, pack: &Path, name: &str) -> Result<bool, String> {
        for path in self.resources(pack, name)? {
            let mut text = String::new();
            std::fs::File::open(path)
                .map_err(|e| e.to_string())?
                .take(16384)
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            let mut lines = text
                .lines()
                .map(|line| line.split('#').next().unwrap_or("").trim())
                .filter(|line| !line.is_empty());
            if !matches!(lines.next(), Some("A" | "I"))
                || lines.next() != Some("850")
                || !matches!(lines.next(), Some("DRAPED_POLYGON" | "LINE_PAINT"))
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn paved_definition(text: &str) -> bool {
    let mut lines = text
        .lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty());
    if !matches!(lines.next(), Some("A" | "I"))
        || lines.next() != Some("850")
        || lines.next() != Some("DRAPED_POLYGON")
    {
        return false;
    }
    let mut surface = false;
    for line in lines {
        let mut words = line.split_whitespace();
        if words.next() == Some("SURFACE") {
            if surface
                || !matches!(words.next(), Some("asphalt" | "concrete"))
                || words.next().is_some()
            {
                return false;
            }
            surface = true;
        }
    }
    surface
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_library_variant_must_declare_physical_pavement() {
        let root = std::env::temp_dir().join(format!(
            "openatc-surface-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut library = Library {
            exports: BTreeMap::new(),
            incomplete: BTreeSet::new(),
        };
        library.add(&root, "A\n1200\nLIBRARY\nEXPORT surface.pol paved.pol\nEXPORT_SEASON win surface.pol winter.pol\n").unwrap();
        std::fs::write(
            root.join("paved.pol"),
            "A\n850\nDRAPED_POLYGON\nSURFACE concrete\n",
        )
        .unwrap();
        std::fs::write(
            root.join("winter.pol"),
            "A\n850\nDRAPED_POLYGON\nTEXTURE snow.png\n",
        )
        .unwrap();
        assert!(!library.paved_surface(&root, "surface.pol").unwrap());
        std::fs::write(
            root.join("winter.pol"),
            "A\n850\nDRAPED_POLYGON\nSURFACE asphalt\n",
        )
        .unwrap();
        assert!(library.paved_surface(&root, "surface.pol").unwrap());
        std::fs::write(root.join("winter.pol"), "x".repeat(64 * 1024 + 1)).unwrap();
        assert!(library.paved_surface(&root, "surface.pol").is_err());
        std::fs::remove_file(root.join("winter.pol")).unwrap();
        assert!(library.paved_surface(&root, "surface.pol").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pavement_requires_an_explicit_unambiguous_hard_surface() {
        for surface in ["asphalt", "concrete"] {
            assert!(paved_definition(&format!(
                "A\n850\nDRAPED_POLYGON\nSURFACE {surface} # apron\n"
            )));
        }
        for tail in [
            "TEXTURE concrete.png",
            "SURFACE grass",
            "SURFACE concrete\nSURFACE asphalt",
            "SURFACE concrete extra",
        ] {
            assert!(!paved_definition(&format!(
                "I\n850\nDRAPED_POLYGON\n{tail}\n"
            )));
        }
        assert!(!paved_definition("A\n850\nLINE_PAINT\nSURFACE concrete\n"));
    }

    #[test]
    fn all_library_variants_are_retained_without_following_paths_outside_the_package() {
        let mut library = Library {
            exports: BTreeMap::new(),
            incomplete: BTreeSet::new(),
        };
        library.add(Path::new("scenery/A"), "I\n1200\nLIBRARY\nEXPORT lib/tower.obj small.obj\nREGION elsewhere\nEXPORT_RATIO 0.2 lib/tower.obj large tower.obj\nEXPORT_SEASON win lib/tower.obj snow.obj\nEXPORT_EXTEND_SEASON spr,sum,fal lib/tower.obj spring.obj\n").unwrap();
        library
            .add(
                Path::new("scenery/B"),
                "A\n800\nLIBRARY\nEXPORT_BACKUP lib/tower.obj backup.obj\n",
            )
            .unwrap();
        let variants = library
            .resources(Path::new("scenery/C"), "lib/tower.obj")
            .unwrap();
        assert_eq!(variants.len(), 5);
        assert!(variants.contains(&PathBuf::from("scenery/A/large tower.obj")));
        for mapping in [
            "EXPORT_RATIO NaN lib/a.obj a.obj",
            "EXPORT_SEASON invalid lib/a.obj a.obj",
            "EXPORT_NEW lib/a.obj a.obj",
        ] {
            assert!(
                library
                    .add(
                        Path::new("scenery/A"),
                        &format!("I\n1200\nLIBRARY\n{mapping}\n")
                    )
                    .is_err()
            );
        }
        assert!(
            library
                .resources(Path::new("scenery/C"), "../escape.obj")
                .is_err()
        );
        for path in ["../a.obj", "/absolute.obj", ""] {
            library
                .add(
                    Path::new("scenery/A"),
                    &format!("I\n800\nLIBRARY\nEXPORT lib/missing.obj {path}\n"),
                )
                .unwrap();
            assert!(
                library
                    .resources(Path::new("scenery/A"), "lib/missing.obj")
                    .is_err()
            );
        }
        let mut crowded = "I\n800\nLIBRARY\n".to_owned();
        for index in 0..257 {
            crowded.push_str(&format!("EXPORT /lib/crowded.obj model{index}.obj\n"));
        }
        library.add(Path::new("scenery/A"), &crowded).unwrap();
        assert!(
            library
                .resources(Path::new("scenery/A"), "lib/crowded.obj")
                .is_err()
        );
        assert_eq!(
            library
                .resources(Path::new("scenery/C"), "/lib/tower.obj")
                .unwrap()
                .len(),
            5
        );
    }
}
