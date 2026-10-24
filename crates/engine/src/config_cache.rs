//! Content-addressed TOML syntax cache. Typed deserialization and domain checks stay at callers.
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Mutex, OnceLock},
    time::Duration,
};

// Bump toml-v1 when the wire tree or TOML parser interpretation changes.
const MAX_SOURCE: usize = 32 * 1024 * 1024;
const MAX_PAYLOAD: usize = 64 * 1024 * 1024;
const MAX_STORED: i64 = 128 * 1024 * 1024;
const TABLE: &str = "CREATE TABLE documents (digest BLOB PRIMARY KEY, payload BLOB NOT NULL, checksum BLOB NOT NULL, used INTEGER NOT NULL); CREATE INDEX document_lru ON documents(used);";
const SETUP_TABLE: &str = "CREATE TABLE IF NOT EXISTS setup (id INTEGER PRIMARY KEY CHECK(id=1), complete INTEGER NOT NULL); INSERT OR IGNORE INTO setup VALUES(1,0);";
static CACHE: OnceLock<Cache> = OnceLock::new();
thread_local! {static WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };}
/// Opt in only on bounded CPU workers or the single pre-HTTP startup worker.
pub fn on_worker<T>(work: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            WORKER.set(self.0);
        }
    }
    let _restore = Restore(WORKER.replace(true));
    work()
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
    pub path: String,
    pub hits: u64,
    pub misses: u64,
    pub writes: u64,
    pub error: String,
    pub setup_complete: bool,
}
static STATUS: OnceLock<Mutex<Status>> = OnceLock::new();
fn update(f: impl FnOnce(&mut Status)) {
    f(&mut STATUS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner));
}
pub fn status() -> Status {
    STATUS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

// An explicit wire tree preserves TOML datetime and nonfinite float types. Serializing Value
// directly to JSON loses them; postcard cannot deserialize Value's deserialize_any visitor.
#[derive(Serialize, Deserialize)]
enum Node {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Datetime(String),
    Array(Vec<Node>),
    Table(BTreeMap<String, Node>),
}
impl Node {
    fn from_value(value: toml::Value) -> Self {
        match value {
            toml::Value::String(v) => Self::String(v),
            toml::Value::Integer(v) => Self::Integer(v),
            toml::Value::Float(v) => Self::Float(v),
            toml::Value::Boolean(v) => Self::Boolean(v),
            toml::Value::Datetime(v) => Self::Datetime(v.to_string()),
            toml::Value::Array(v) => Self::Array(v.into_iter().map(Self::from_value).collect()),
            toml::Value::Table(v) => Self::Table(
                v.into_iter()
                    .map(|(k, v)| (k, Self::from_value(v)))
                    .collect(),
            ),
        }
    }
    fn into_value(self) -> Option<toml::Value> {
        Some(match self {
            Self::String(v) => toml::Value::String(v),
            Self::Integer(v) => toml::Value::Integer(v),
            Self::Float(v) => toml::Value::Float(v),
            Self::Boolean(v) => toml::Value::Boolean(v),
            Self::Datetime(v) => toml::Value::Datetime(v.parse().ok()?),
            Self::Array(v) => {
                toml::Value::Array(v.into_iter().map(Self::into_value).collect::<Option<_>>()?)
            }
            Self::Table(v) => toml::Value::Table(
                v.into_iter()
                    .map(|(k, v)| Some((k, v.into_value()?)))
                    .collect::<Option<_>>()?,
            ),
        })
    }
}

pub(crate) struct Cache {
    connection: Mutex<Connection>,
}
impl Cache {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        connection
            .busy_timeout(Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        connection
            .execute_batch(SETUP_TABLE)
            .map_err(|e| e.to_string())?;
        connection.execute_batch("PRAGMA page_size=4096; PRAGMA max_page_count=65536; PRAGMA cache_size=-4096; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS generation (id INTEGER PRIMARY KEY CHECK(id=1), epoch INTEGER NOT NULL); INSERT OR IGNORE INTO generation VALUES(1,0);").map_err(|e|e.to_string())?;
        connection
            .execute_batch(
                &TABLE
                    .replace("CREATE TABLE ", "CREATE TABLE IF NOT EXISTS ")
                    .replace("CREATE INDEX ", "CREATE INDEX IF NOT EXISTS "),
            )
            .map_err(|e| e.to_string())?;
        update(|s| {
            s.path = path.display().to_string();
            s.error.clear();
            s.setup_complete = false;
        });
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
    fn read(&self, digest: &[u8]) -> Result<(i64, Option<Vec<u8>>), String> {
        let db = self.connection.lock().map_err(|e| e.to_string())?;
        let epoch = db
            .query_row("SELECT epoch FROM generation WHERE id=1", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let row = db
            .query_row(
                "SELECT payload,checksum FROM documents WHERE digest=?1 AND length(payload)<=?2",
                params![digest, 64_i64 * 1024 * 1024],
                |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let payload = row.and_then(|(payload, checksum)| {
            if Sha256::digest(&payload).as_slice() == checksum {
                Some(payload)
            } else {
                update(|s| s.error = "TOML cache checksum failed; reparsing source".into());
                None
            }
        });
        if payload.is_some() {
            let _ = db.execute(
                "UPDATE documents SET used=unixepoch() WHERE digest=?1 AND used<unixepoch()-60",
                [digest],
            );
        }
        Ok((epoch, payload))
    }
    fn write(&self, digest: &[u8], epoch: i64, value: toml::Value) -> Result<(), String> {
        let payload = postcard::to_allocvec(&Node::from_value(value)).map_err(|e| e.to_string())?;
        if payload.len() > MAX_PAYLOAD {
            return Err("Parsed TOML exceeds cache entry budget".into());
        }
        let mut db = self.connection.lock().map_err(|e| e.to_string())?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let current: i64 = tx
            .query_row("SELECT epoch FROM generation WHERE id=1", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if current != epoch {
            return Ok(());
        }
        tx.execute("DELETE FROM documents WHERE digest=?1", [digest])
            .map_err(|e| e.to_string())?;
        let mut size: i64 = tx
            .query_row(
                "SELECT COALESCE(SUM(length(payload)),0) FROM documents",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let mut count: i64 = tx
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        while size + i64::try_from(payload.len()).map_err(|e| e.to_string())? > MAX_STORED
            || count >= 8192
        {
            tx.execute("DELETE FROM documents WHERE digest=(SELECT digest FROM documents ORDER BY used LIMIT 1)",[]).map_err(|e|e.to_string())?;
            count -= 1;
            size = tx
                .query_row(
                    "SELECT COALESCE(SUM(length(payload)),0) FROM documents",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "INSERT INTO documents VALUES(?1,?2,?3,unixepoch())",
            params![digest, payload, Sha256::digest(&payload).as_slice()],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        update(|s| s.writes += 1);
        Ok(())
    }
    pub fn parse(&self, text: &str) -> Result<toml::Value, toml::de::Error> {
        if text.len() > MAX_SOURCE {
            return toml::from_str(text);
        }
        let digest = Sha256::digest(text.as_bytes());
        let row = self.read(digest.as_slice());
        let epoch = match row {
            Ok((epoch, payload)) => {
                if let Some(value) = payload
                    .and_then(|b| postcard::from_bytes::<Node>(&b).ok())
                    .and_then(Node::into_value)
                {
                    update(|s| s.hits += 1);
                    return Ok(value);
                }
                Some(epoch)
            }
            Err(error) => {
                update(|s| s.error = error);
                None
            }
        };
        update(|s| s.misses += 1);
        let value: toml::Value = toml::from_str(text)?;
        if let Some(epoch) = epoch
            && let Err(error) = self.write(digest.as_slice(), epoch, value.clone())
        {
            update(|s| s.error = error);
        }
        Ok(value)
    }
    pub fn rebuild(&self) -> Result<(), String> {
        let mut db = self.connection.lock().map_err(|e| e.to_string())?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let epoch: i64 = tx
            .query_row("SELECT epoch FROM generation WHERE id=1", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let next = epoch.checked_add(1).ok_or("Cache generation exhausted")?;
        tx.execute_batch("DROP TABLE documents; DROP TABLE generation; CREATE TABLE generation(id INTEGER PRIMARY KEY CHECK(id=1),epoch INTEGER NOT NULL);").map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO generation VALUES(1,?1)", [next])
            .map_err(|e| e.to_string())?;
        tx.execute_batch(TABLE).map_err(|e| e.to_string())?;
        tx.execute_batch("DROP TABLE IF EXISTS setup;")
            .map_err(|e| e.to_string())?;
        tx.execute_batch(SETUP_TABLE).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        update(|s| {
            s.hits = 0;
            s.misses = 0;
            s.writes = 0;
            s.error.clear();
            s.setup_complete = false;
        });
        Ok(())
    }
    pub fn setup_complete(&self) -> Result<bool, String> {
        self.connection
            .lock()
            .map_err(|e| e.to_string())?
            .query_row("SELECT complete FROM setup WHERE id=1", [], |r| r.get(0))
            .map_err(|e| e.to_string())
    }
    pub fn finish_setup(&self) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|e| e.to_string())?
            .execute("UPDATE setup SET complete=1 WHERE id=1", [])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

pub fn initialize() {
    let path = crate::data_dir().join("setup-cache/toml-v1.sqlite3");
    match Cache::open(&path) {
        Ok(cache) => {
            let complete = cache.setup_complete().unwrap_or(false);
            update(|s| s.setup_complete = complete);
            let _ = CACHE.set(cache);
            openatc_core::configuration::install_parser(parse);
        }
        Err(error) => {
            update(|s| s.error.clone_from(&error));
            eprintln!("TOML cache unavailable; parsing source directly: {error}");
        }
    }
}
pub fn parse(text: &str) -> Result<toml::Value, toml::de::Error> {
    if !WORKER.get() {
        return toml::from_str(text);
    }
    CACHE
        .get()
        .map_or_else(|| toml::from_str(text), |cache| cache.parse(text))
}
pub fn available() -> bool {
    CACHE.get().is_some()
}
pub fn setup_complete() -> bool {
    status().setup_complete
}
pub fn finish_setup() -> Result<(), String> {
    CACHE
        .get()
        .ok_or("TOML cache unavailable")?
        .finish_setup()?;
    update(|s| s.setup_complete = true);
    Ok(())
}
pub fn rebuild() -> Result<(), String> {
    if !available() {
        initialize();
    }
    CACHE
        .get()
        .ok_or("TOML cache unavailable; restart the engine after fixing its cache directory")?
        .rebuild()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temporary() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "openatc-toml-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn content_changes_corruption_reset_and_old_writers() {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Typed {
            value: u8,
        }

        let path = temporary();
        let cache = Cache::open(&path).unwrap();
        assert!(!cache.setup_complete().unwrap());
        cache.finish_setup().unwrap();
        let source = "value = 10\ntime = 1979-05-27T07:32:00Z\nfloat = inf";
        let expected = cache.parse(source).unwrap();
        drop(cache);
        let cache = Cache::open(&path).unwrap();
        assert!(cache.setup_complete().unwrap());
        assert_eq!(expected, cache.parse(source).unwrap());
        assert_eq!(
            cache.parse("value=11").unwrap()["value"].as_integer(),
            Some(11)
        );
        let digest = Sha256::digest(source.as_bytes());
        let (epoch, _) = cache.read(&digest).unwrap();
        cache
            .connection
            .lock()
            .unwrap()
            .execute("UPDATE documents SET payload=x'00'", [])
            .unwrap();
        assert_eq!(expected, cache.parse(source).unwrap());
        cache.rebuild().unwrap();
        assert!(!cache.setup_complete().unwrap());
        cache.write(&digest, epoch, expected.clone()).unwrap();
        assert!(cache.read(&digest).unwrap().1.is_none());
        assert_eq!(expected, cache.parse(source).unwrap());
        assert!(cache.parse("bad=[").is_err());
        let _: u8 = cache
            .parse("value=11")
            .unwrap()
            .try_into::<Typed>()
            .unwrap()
            .value;
        assert!(
            cache
                .parse("value=999")
                .unwrap()
                .try_into::<Typed>()
                .is_err()
        );
        assert!(
            cache
                .parse("value=11\nextra=true")
                .unwrap()
                .try_into::<Typed>()
                .is_err()
        );
        drop(cache);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    #[ignore = "release benchmark of source parse versus SQLite syntax reuse"]
    fn benchmark_shipped_toml() {
        let path = temporary();
        let cache = Cache::open(&path).unwrap();
        let sources = [
            include_str!("../../../speech/runtime/responses.toml"),
            include_str!("../../../traffic/aircraft.toml"),
            include_str!("../../../traffic/airlines.toml"),
            include_str!("../../../traffic/hubs.toml"),
            include_str!("../../../traffic/routes.toml"),
            include_str!("../../../prompts/prompts.toml"),
        ];
        let start = std::time::Instant::now();
        for text in sources {
            std::hint::black_box(cache.parse(text).unwrap());
        }
        let cold = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..30 {
            for text in sources {
                std::hint::black_box(toml::from_str::<toml::Value>(text).unwrap());
            }
        }
        let direct = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..30 {
            for text in sources {
                std::hint::black_box(cache.parse(text).unwrap());
            }
        }
        let warm = start.elapsed();
        eprintln!(
            "TOML syntax: cold SQLite population {cold:?}; 30 direct parses {direct:?}; 30 warm SQLite loads {warm:?}"
        );
        drop(cache);
        std::fs::remove_file(path).unwrap();
    }
}
