//! Rebuildable compiled scenery stacks. Only planning workers open the database.
use super::{Footprint, Geometry, Obstacle, Placement, Tile};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

// Bump when the stored format or scenery interpretation changes.
const VERSION: u32 = 6;
const MAX_ENTRY: usize = 256 * 1024 * 1024;
const MAX_STORED: i64 = 1536 * 1024 * 1024;
const MAX_DEPENDENCIES: usize = 50_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct Fingerprint {
    exists: bool,
    directory: bool,
    bytes: u64,
    modified: Option<(u64, u32)>,
}
impl Fingerprint {
    fn read(path: &Path) -> Option<Self> {
        match path.metadata() {
            Ok(meta) => Some(Self {
                exists: true,
                directory: meta.is_dir(),
                bytes: meta.len(),
                modified: Some({
                    let duration = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
                    (duration.as_secs(), duration.subsec_nanos())
                }),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(Self {
                exists: false,
                directory: false,
                bytes: 0,
                modified: None,
            }),
            Err(_) => None,
        }
    }
}
pub(super) type Dependencies = BTreeMap<PathBuf, Option<Fingerprint>>;
thread_local! { static DEPENDENCIES: RefCell<Dependencies> = const { RefCell::new(BTreeMap::new()) }; }
pub(super) fn begin() {
    DEPENDENCIES.with(|deps| deps.borrow_mut().clear());
}
pub(super) fn observe(path: &Path) {
    DEPENDENCIES.with(|deps| {
        let mut deps = deps.borrow_mut();
        if deps.len() <= MAX_DEPENDENCIES {
            deps.entry(path.to_path_buf())
                .or_insert_with(|| Fingerprint::read(path));
        }
    });
}
pub(super) fn open_file(path: impl AsRef<Path>) -> std::io::Result<std::fs::File> {
    observe(path.as_ref());
    std::fs::File::open(path)
}
pub(super) fn snapshot() -> Dependencies {
    DEPENDENCIES.with(|deps| deps.borrow().clone())
}
pub(super) fn restore(dependencies: &Dependencies) {
    DEPENDENCIES.with(|deps| {
        for (path, stamp) in dependencies {
            deps.borrow_mut()
                .entry(path.clone())
                .or_insert_with(|| stamp.clone());
        }
    });
}
fn valid(dependencies: &Dependencies) -> bool {
    !dependencies.is_empty()
        && dependencies.len() <= MAX_DEPENDENCIES
        && dependencies
            .iter()
            .all(|(path, stamp)| stamp.is_some() && Fingerprint::read(path) == *stamp)
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Progress {
    pub path: String,
    pub phase: String,
    pub hits: u64,
    pub misses: u64,
    pub writes: u64,
    pub retained_bytes: usize,
    pub current_tile: String,
    pub error: String,
}
static PROGRESS: OnceLock<Mutex<Progress>> = OnceLock::new();
fn update(change: impl FnOnce(&mut Progress)) {
    change(
        &mut PROGRESS
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
}
pub(crate) fn progress() -> Progress {
    PROGRESS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}
pub(super) fn retained(bytes: usize) {
    update(|status| status.retained_bytes = bytes);
}
pub(super) fn preparing(tile: (i32, i32)) {
    update(|status| {
        status.current_tile = format!("{:+03}{:+04}", tile.0, tile.1);
        status.phase = "Preparing scenery".into();
    });
}
fn location() -> PathBuf {
    std::env::var_os("OPENATC_SCENERY_CACHE_DIR")
        .map_or_else(|| crate::data_dir().join("scenery-index"), PathBuf::from)
}

/// Remove scenery indexes written by an earlier interpretation.
///
/// The index is rebuildable, so a superseded file only occupies disk. Only files named for the
/// scenery index are removed, and the current one is left alone.
fn remove_superseded(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(version) = name
            .strip_prefix("scenery-v")
            .and_then(|rest| rest.strip_suffix(".sqlite3"))
        else {
            continue;
        };
        if version
            .parse::<u32>()
            .is_ok_and(|version| version != VERSION)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
pub(super) struct Cache {
    connection: Connection,
    root: String,
    epoch: i64,
    capacity: i64,
}
impl Cache {
    pub fn configured(root: &Path) -> Option<Self> {
        // Installed-data tests opt in to a separate cache directory.
        if cfg!(test) && std::env::var_os("OPENATC_SCENERY_CACHE_DIR").is_none() {
            return None;
        }
        match Self::open(&location(), root) {
            Ok(cache) => Some(cache),
            Err(error) => {
                update(|status| {
                    status.phase = "Cache unavailable; parsing scenery directly".into();
                    status.error = error;
                });
                None
            }
        }
    }
    fn open(directory: &Path, root: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        remove_superseded(directory);
        let path = directory.join(format!("scenery-v{VERSION}.sqlite3"));
        let connection = Connection::open(&path).map_err(|e| e.to_string())?;
        connection
            .busy_timeout(Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        connection.execute_batch("PRAGMA page_size=4096; PRAGMA max_page_count=524288; PRAGMA cache_size=-8192; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS stacks (key TEXT PRIMARY KEY, dependencies BLOB NOT NULL, payload BLOB NOT NULL, checksum BLOB NOT NULL, used INTEGER NOT NULL); CREATE INDEX IF NOT EXISTS stack_lru ON stacks(used); CREATE TABLE IF NOT EXISTS generation (id INTEGER PRIMARY KEY CHECK(id=1), epoch INTEGER NOT NULL); INSERT OR IGNORE INTO generation VALUES(1,0);").map_err(|e| e.to_string())?;
        update(|status| {
            status.path = path.display().to_string();
            status.error.clear();
        });
        let epoch = connection
            .query_row("SELECT epoch FROM generation WHERE id=1", [], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            connection,
            epoch,
            capacity: MAX_STORED,
            root: root
                .canonicalize()
                .unwrap_or_else(|_| root.to_path_buf())
                .to_string_lossy()
                .into_owned(),
        })
    }
    fn key(&self, packs: &[PathBuf], tile: (i32, i32)) -> String {
        format!(
            "{}|{}|{},{}",
            self.root,
            serde_json::to_string(packs).unwrap_or_default(),
            tile.0,
            tile.1
        )
    }
    pub fn load(&self, packs: &[PathBuf], tile: (i32, i32)) -> Option<Vec<Arc<Tile>>> {
        let key = self.key(packs, tile);
        let row = self.connection.query_row("SELECT dependencies,payload,checksum FROM stacks WHERE key=?1 AND length(payload)<=?2 AND length(dependencies)<=16777216", params![key, MAX_ENTRY as i64], |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?))).optional();
        let row = match row {
            Ok(row) => row,
            Err(error) => {
                update(|status| {
                    status.error = format!("Cache read failed; rebuilding scenery: {error}");
                });
                return None;
            }
        };
        let loaded = (|| {
            let (manifest, payload, checksum) = row?;
            let dependencies: Dependencies = postcard::from_bytes(&manifest).ok()?;
            if !valid(&dependencies) {
                return None;
            }
            if Sha256::digest(&payload).as_slice() != checksum {
                update(|status| {
                    status.error = "Cache entry failed its checksum; rebuilding scenery".into();
                });
                return None;
            }
            let tiles: Vec<Tile> = postcard::from_bytes(&payload).ok()?;
            if tiles.is_empty() || tiles.len() > 1024 {
                return None;
            }
            // The obstacle index is rebuilt rather than stored, so a cached tile costs a pass
            // over its obstacles and no extra bytes.
            let mut tiles = tiles;
            for tile in &mut tiles {
                tile.index_obstacles();
            }
            restore(&dependencies);
            Some(tiles.into_iter().map(Arc::new).collect())
        })();
        if loaded.is_some() {
            let _ = self.connection.execute(
                "UPDATE stacks SET used=?1 WHERE key=?2",
                params![now(), key],
            );
            update(|status| {
                status.hits += 1;
                status.phase = "Using cached scenery".into();
            });
        } else {
            update(|status| status.misses += 1);
        }
        loaded
    }
    pub fn store(&mut self, packs: &[PathBuf], tile: (i32, i32), tiles: &[Arc<Tile>]) {
        let dependencies = snapshot();
        if !valid(&dependencies) {
            return;
        }
        let result = (|| -> Result<(), String> {
            let payload = postcard::to_allocvec(tiles).map_err(|e| e.to_string())?;
            if payload.len() > MAX_ENTRY {
                return Err(
                    "Compiled tile exceeds the disk-entry budget; using memory only".into(),
                );
            }
            let manifest = postcard::to_allocvec(&dependencies).map_err(|e| e.to_string())?;
            if !valid(&dependencies) {
                return Err("Scenery changed while indexing; entry discarded".into());
            }
            let key = self.key(packs, tile);
            let tx = self.connection.transaction().map_err(|e| e.to_string())?;
            let current: i64 = tx
                .query_row("SELECT epoch FROM generation WHERE id=1", [], |row| {
                    row.get(0)
                })
                .map_err(|e| e.to_string())?;
            if current != self.epoch {
                return Err("Scenery index cleared during preparation; old entry discarded".into());
            }
            tx.execute("DELETE FROM stacks WHERE key=?1", [&key])
                .map_err(|e| e.to_string())?;
            let mut stored: i64 = tx
                .query_row(
                    "SELECT coalesce(sum(length(payload)+length(dependencies)),0) FROM stacks",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            let incoming = (payload.len() + manifest.len()) as i64;
            while stored + incoming > self.capacity {
                let (old, size): (String, i64) = tx.query_row("SELECT key,length(payload)+length(dependencies) FROM stacks ORDER BY used LIMIT 1", [], |row| Ok((row.get(0)?, row.get(1)?))).map_err(|e| e.to_string())?;
                tx.execute("DELETE FROM stacks WHERE key=?1", [old])
                    .map_err(|e| e.to_string())?;
                stored = stored.saturating_sub(size);
            }
            tx.execute(
                "INSERT INTO stacks VALUES (?1,?2,?3,?4,?5)",
                params![
                    key,
                    manifest,
                    payload,
                    Sha256::digest(&payload).as_slice(),
                    now()
                ],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })();
        update(|status| match result {
            Ok(()) => {
                status.writes += 1;
                status.phase = "Scenery cached".into();
            }
            Err(error) => status.error = error,
        });
    }
    pub fn clear(&self) -> Result<(), String> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let epoch: i64 = tx
            .query_row("SELECT epoch FROM generation WHERE id=1", [], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;
        let next = epoch.checked_add(1).ok_or("Cache generation exhausted")?;
        tx.execute_batch("DROP TABLE stacks; DROP TABLE generation; CREATE TABLE stacks(key TEXT PRIMARY KEY, dependencies BLOB NOT NULL, payload BLOB NOT NULL, checksum BLOB NOT NULL, used INTEGER NOT NULL); CREATE INDEX stack_lru ON stacks(used); CREATE TABLE generation(id INTEGER PRIMARY KEY CHECK(id=1),epoch INTEGER NOT NULL);").map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO generation VALUES(1,?1)", [next])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.connection
            .execute_batch("VACUUM")
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| i64::try_from(time.as_secs()).unwrap_or(i64::MAX))
}

/// Preserve shared object geometry when serialising. Serde's default Arc support would write
/// the same model for every placement, then allocate a separate model for each on load.
pub(super) mod obstacles {
    use super::{Arc, BTreeMap, Deserialize, Footprint, Geometry, Obstacle, Placement, Serialize};
    type Record = (
        Placement,
        usize,
        Option<Arc<Footprint>>,
        bool,
        f64,
        Option<(f64, f64)>,
    );
    pub fn serialize<S: serde::Serializer>(
        items: &[Obstacle],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut indices = BTreeMap::new();
        let mut geometries = Vec::new();
        let mut records = Vec::with_capacity(items.len());
        for item in items {
            let index = *indices.entry(item.geometry.key()).or_insert_with(|| {
                let index = geometries.len();
                geometries.push(&item.geometry);
                index
            });
            records.push((
                &item.placement,
                index,
                &item.footprint,
                item.agl,
                item.radius,
                item.local_ground,
            ));
        }
        (geometries, records).serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Obstacle>, D::Error> {
        let (geometries, records): (Vec<Geometry>, Vec<Record>) =
            Deserialize::deserialize(deserializer)?;
        records
            .into_iter()
            .map(|(placement, index, footprint, agl, radius, local_ground)| {
                let geometry = geometries
                    .get(index)
                    .ok_or_else(|| serde::de::Error::custom("Invalid cached model reference"))?
                    .clone();
                Ok(Obstacle {
                    local_ground,
                    placement,
                    geometry,
                    footprint,
                    agl,
                    radius,
                })
            })
            .collect()
    }
}

pub(crate) fn rebuild(root: &Path) -> Result<(), String> {
    Cache::open(&location(), root)?.clear()?;
    update(|status| {
        let path = status.path.clone();
        *status = Progress {
            path,
            phase: "Tables recreated; scenery will rebuild on demand".into(),
            ..Progress::default()
        };
    });
    Ok(())
}

pub(crate) async fn status(
    axum::extract::State(shared): axum::extract::State<crate::Shared>,
    axum::Json(_): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let state = shared.read().await;
    axum::Json(
        serde_json::json!({"cache":progress(), "traffic":state.generated.status, "preparing":state.generated.preparation_job.is_some(), "aircraft":state.session.generated_traffic.len(), "enabled":state.settings.generated_traffic}),
    )
}
pub(crate) async fn clear(
    axum::extract::State(shared): axum::extract::State<crate::Shared>,
    axum::Json(_): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let (root, workers) = {
        let state = shared.read().await;
        (state.settings.simulator_root.clone(), state.workers.clone())
    };
    let result = workers
        .run(move || Cache::open(&location(), Path::new(&root)).and_then(|cache| cache.clear()))
        .await;
    match result {
        Ok(Ok(())) => {
            update(|status| {
                *status = Progress {
                    phase: "Index cleared; scenery will rebuild on demand".into(),
                    ..Progress::default()
                };
            });
            axum::Json(serde_json::json!({"cleared":true})).into_response()
        }
        error => (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(
                serde_json::json!({"error":format!("Could not clear scenery index: {error:?}")}),
            ),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temporary() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "openatc-scenery-cache-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn restart_reuses_geometry_and_invalidates_changed_or_new_dependencies() {
        let root = temporary();
        let file = root.join("pack/Earth nav data/+00+000/+00+000.dsf");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            super::super::tests::mesh_tile(0, 0, 100.0, false, false),
        )
        .unwrap();
        begin();
        let mut tile = Tile::load(&file).unwrap();
        let geometry = Geometry::Many(Arc::new(vec![super::super::object::Bounds {
            low: [-1.0, 0.0, -1.0],
            high: [1.0, 10.0, 1.0],
        }]));
        for longitude in [0.4, 0.5] {
            tile.obstacles.push(Obstacle {
                local_ground: Some((10.0, 50.0)),
                placement: Placement {
                    resource: "tower.obj".into(),
                    coordinate: [0.5, longitude],
                    heading: 0.0,
                    height: None,
                },
                geometry: geometry.clone(),
                footprint: None,
                agl: true,
                radius: 2.0,
            });
        }
        let missing = root.join("not-installed.obj");
        observe(&missing);
        let packs = vec![root.join("pack")];
        let tiles = vec![Arc::new(tile)];
        let mut cache = Cache::open(&root.join("cache"), &root).unwrap();
        cache.store(&packs, (0, 0), &tiles);
        drop(cache);
        begin();
        let mut cache = Cache::open(&root.join("cache"), &root).unwrap();
        let warm = cache
            .load(&packs, (0, 0))
            .expect("persistent compiled stack");
        assert_eq!(warm[0].obstacles[0].local_ground, Some((10.0, 50.0)));
        assert_eq!(
            warm[0].mesh_height([0.5, 0.5]),
            tiles[0].mesh_height([0.5, 0.5])
        );
        assert_eq!(
            warm[0].obstacles[0].geometry.key(),
            warm[0].obstacles[1].geometry.key()
        );
        assert!(cache.load(&[root.join("other")], (0, 0)).is_none());
        std::fs::write(&missing, "new geometry").unwrap();
        assert!(
            cache.load(&packs, (0, 0)).is_none(),
            "newly available missing object invalidates"
        );
        std::fs::remove_file(&missing).unwrap();
        assert!(cache.load(&packs, (0, 0)).is_some());
        std::fs::write(&file, "different DSF").unwrap();
        assert!(
            cache.load(&packs, (0, 0)).is_none(),
            "changed DSF invalidates"
        );
        // A source change while building must not be stored with the old dependency stamp.
        cache.store(&packs, (1, 0), &tiles);
        assert!(cache.load(&packs, (1, 0)).is_none());
        drop(cache);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corruption_and_clear_do_not_publish_a_cached_or_old_worker_entry() {
        let root = temporary();
        let file = root.join("source.dsf");
        std::fs::write(&file, "source").unwrap();
        begin();
        observe(&file);
        let tile =
            super::super::dsf::parse(&super::super::tests::mesh_tile(0, 0, 0.0, false, false))
                .unwrap();
        let tiles = vec![Arc::new(tile)];
        let packs = vec![root.clone()];
        let mut cache = Cache::open(&root.join("cache"), &root).unwrap();
        cache.store(&packs, (0, 0), &tiles);
        assert!(cache.load(&packs, (0, 0)).is_some());
        cache
            .connection
            .execute("UPDATE stacks SET payload=x'00'", [])
            .unwrap();
        assert!(cache.load(&packs, (0, 0)).is_none());
        let clearer = Cache::open(&root.join("cache"), &root).unwrap();
        clearer.clear().unwrap();
        cache.store(&packs, (0, 0), &tiles);
        assert!(
            cache.load(&packs, (0, 0)).is_none(),
            "pre-clear worker must not repopulate"
        );
        drop(cache);
        drop(clearer);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn disk_budget_evicts_an_old_entry_before_committing_the_next() {
        let root = temporary();
        let source = root.join("source.dsf");
        std::fs::write(&source, "source").unwrap();
        begin();
        observe(&source);
        let tile =
            super::super::dsf::parse(&super::super::tests::mesh_tile(0, 0, 0.0, false, false))
                .unwrap();
        let tiles = vec![Arc::new(tile)];
        let packs = vec![root.clone()];
        let mut cache = Cache::open(&root.join("cache"), &root).unwrap();
        cache.store(&packs, (0, 0), &tiles);
        let bytes: i64 = cache
            .connection
            .query_row(
                "SELECT sum(length(payload)+length(dependencies)) FROM stacks",
                [],
                |row| row.get(0),
            )
            .unwrap();
        cache.capacity = bytes + 1;
        cache.store(&packs, (0, 1), &tiles);
        assert!(cache.load(&packs, (0, 0)).is_none());
        assert!(cache.load(&packs, (0, 1)).is_some());
        let retained: i64 = cache
            .connection
            .query_row(
                "SELECT sum(length(payload)+length(dependencies)) FROM stacks",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(retained <= cache.capacity);
        drop(cache);
        std::fs::remove_dir_all(root).unwrap();
    }
}
