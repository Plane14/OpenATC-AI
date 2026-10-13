//! Load editable traffic data away from the simulator and keep a last known good set.
use crate::{Shared, workers::Workers};
use axum::{
    Json,
    extract::State as AxumState,
    response::{IntoResponse, Response},
};
use openatc_core::traffic::profiles::Profiles;
use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Default, Clone)]
pub struct Store(Arc<tokio::sync::Mutex<Option<Loaded>>>);

struct Loaded {
    profile: Arc<Profiles>,
    sources: Vec<String>,
    errors: Vec<String>,
}

fn directories(config: &Path) -> Vec<PathBuf> {
    let installed = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("../traffic")));
    installed
        .into_iter()
        .chain(std::env::var_os("OPENATC_TRAFFIC_DIR").map(PathBuf::from))
        .chain([config.join("traffic")])
        .collect()
}

fn load(dirs: &[PathBuf]) -> Result<Loaded, String> {
    let mut texts = [
        include_str!("../../../traffic/aircraft.toml").to_owned(),
        include_str!("../../../traffic/airlines.toml").to_owned(),
        include_str!("../../../traffic/hubs.toml").to_owned(),
    ];
    let mut sources = vec!["Bundled profiles".to_owned()];
    for dir in dirs {
        for (i, name) in ["aircraft.toml", "airlines.toml", "hubs.toml"]
            .iter()
            .enumerate()
        {
            let path = dir.join(name);
            let file = match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("{}: {e}", path.display())),
            };
            let mut text = String::new();
            file.take(1024 * 1024 + 1)
                .read_to_string(&mut text)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if text.len() > 1024 * 1024 {
                return Err(format!("{} exceeds 1 MiB", path.display()));
            }
            texts[i] = text;
            sources.push(path.display().to_string());
        }
    }
    Ok(Loaded {
        profile: Arc::new(Profiles::parse(&texts[0], &texts[1], &texts[2])?),
        sources,
        errors: vec![],
    })
}

impl Store {
    pub async fn get(
        &self,
        config: &Path,
        workers: &Workers,
        refresh: bool,
    ) -> Result<Arc<Profiles>, String> {
        let mut cached = self.0.lock().await;
        if refresh || cached.is_none() {
            let dirs = directories(config);
            match workers.run(move || load(&dirs)).await? {
                Ok(loaded) => *cached = Some(loaded),
                Err(error) => {
                    if let Some(current) = cached.as_mut() {
                        current.errors = vec![error.clone()];
                    }
                    return Err(error);
                }
            }
        }
        Ok(cached.as_ref().unwrap().profile.clone())
    }
}

pub async fn inspect(AxumState(shared): AxumState<Shared>, Json(body): Json<Value>) -> Response {
    let (store, config, workers) = {
        let s = shared.read().await;
        (
            s.traffic_profiles.clone(),
            s.config_dir.clone(),
            s.workers.clone(),
        )
    };
    let result = store.get(&config, &workers, body["refresh"] == true).await;
    let cache = store.0.lock().await;
    let Some(loaded) = cache.as_ref() else {
        return crate::routes::bad_request(
            result
                .err()
                .unwrap_or_else(|| "Traffic profiles unavailable".into()),
        );
    };
    Json(
        json!({"accepted":result.is_ok(), "aircraft":loaded.profile.aircraft,
        "airlines":loaded.profile.airlines,"hubs":loaded.profile.hubs,
        "sources":loaded.sources,"errors":loaded.errors}),
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn edited_profiles_load_and_invalid_reload_keeps_previous_set() {
        let dir =
            std::env::temp_dir().join(format!("openatc-traffic-profiles-{}", std::process::id()));
        let data = dir.join("traffic");
        std::fs::create_dir_all(&data).unwrap();
        let source = include_str!("../../../traffic/hubs.toml").replace("QFA = 55", "QFA = 75");
        std::fs::write(data.join("hubs.toml"), source).unwrap();
        let store = Store::default();
        let workers = Workers::new(1);
        let first = store.get(&dir, &workers, false).await.unwrap();
        assert_eq!(first.hubs["YSSY"].airlines["QFA"], 75);
        std::fs::write(data.join("hubs.toml"), "version = 99").unwrap();
        assert!(store.get(&dir, &workers, true).await.is_err());
        let next = store.get(&dir, &workers, false).await.unwrap();
        assert!(Arc::ptr_eq(&first, &next));
        assert_eq!(store.0.lock().await.as_ref().unwrap().errors.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
