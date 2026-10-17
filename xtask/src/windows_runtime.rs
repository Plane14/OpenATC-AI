//! App-local Microsoft runtime deployment from the native Visual Studio toolchain.
use crate::Result;
use std::{
    fs,
    path::{Path, PathBuf},
};

// These are imported by the launcher and the speech/ONNX native dependencies.
const REQUIRED: &[&str] = &[
    "msvcp140.dll",
    "msvcp140_1.dll",
    "vcruntime140.dll",
    "vcruntime140_1.dll",
    "vcomp140.dll",
];

fn find_libraries(root: &Path) -> Result<Vec<PathBuf>> {
    let mut libraries = Vec::new();
    let x64 = root.join("x64");
    if x64.is_dir() {
        for entry in fs::read_dir(&x64)? {
            let folder = entry?.path();
            let name = folder.file_name().unwrap().to_string_lossy();
            // Only Microsoft's release CRT and OpenMP redistributable directories.
            if !name.starts_with("Microsoft.VC")
                || !(name.ends_with(".CRT") || name.ends_with(".OpenMP"))
            {
                continue;
            }
            for entry in fs::read_dir(folder)? {
                let path = entry?.path();
                if path.is_file()
                    && path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
                {
                    libraries.push(path);
                }
            }
        }
    }
    libraries.sort();
    for name in REQUIRED {
        if !libraries
            .iter()
            .any(|path| path.file_name().unwrap().eq_ignore_ascii_case(name))
        {
            return Err(format!("Missing Microsoft redistributable {name} under {}. Install the Visual Studio C++ x64 runtime components or set VCToolsRedistDir to their release redist directory.", root.display()).into());
        }
    }
    Ok(libraries)
}

pub(crate) fn libraries() -> Result<Vec<PathBuf>> {
    if let Some(root) = std::env::var_os("VCToolsRedistDir") {
        return find_libraries(Path::new(&root));
    }
    let vc = std::env::var_os("VCINSTALLDIR")
        .ok_or("Windows packaging requires VCToolsRedistDir or a Visual Studio developer shell (VCINSTALLDIR)")?;
    let root = Path::new(&vc).join("Redist/MSVC");
    let mut versions: Vec<_> = fs::read_dir(&root)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(|c: char| c.is_ascii_digit())
        })
        .collect();
    versions.sort();
    let latest = versions
        .last()
        .ok_or("Visual Studio has no versioned C++ redistributable directory")?;
    find_libraries(latest)
}

pub(crate) fn install(destination: &Path) -> Result {
    let libraries = libraries()?;
    fs::create_dir_all(destination)?;
    for source in libraries {
        fs::copy(&source, destination.join(source.file_name().unwrap()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redistributables_require_complete_release_crt_and_openmp() -> Result {
        let temp = tempfile::tempdir()?;
        let crt = temp.path().join("x64/Microsoft.VC143.CRT");
        let openmp = temp.path().join("x64/Microsoft.VC143.OpenMP");
        fs::create_dir_all(&crt)?;
        fs::create_dir_all(&openmp)?;
        for name in REQUIRED.iter().filter(|name| **name != "vcomp140.dll") {
            fs::write(crt.join(name), b"fixture")?;
        }
        assert!(
            find_libraries(temp.path())
                .unwrap_err()
                .to_string()
                .contains("vcomp140.dll")
        );
        fs::write(openmp.join("VCOMP140.DLL"), b"fixture")?;
        let debug = temp.path().join("x64/Microsoft.VC143.DebugCRT");
        fs::create_dir_all(&debug)?;
        fs::write(debug.join("vcruntime140d.dll"), b"not redistributable")?;
        let libraries = find_libraries(temp.path())?;
        assert_eq!(libraries.len(), REQUIRED.len());
        assert!(libraries.iter().all(|path| !path.starts_with(&debug)));
        assert!(find_libraries(&temp.path().join("missing")).is_err());
        Ok(())
    }
}
