//! Companion engine for controller state, local HTTP requests and AI/STT/TTS services.

mod cabin;
mod context;
mod crew;
mod discipline;
mod flight;
mod generated;
mod initiative;
mod instructions;
mod intent;
mod lifecycle;
mod memory;
mod missions;
mod parking;
mod radio;
mod routes;
mod squawk;
mod support;
mod surface;
mod traffic;
mod traffic_models;
mod traffic_profiles;
mod workers;

use openatc_core::airport::Airport;
use openatc_core::regions::{Region, load_regions};
use openatc_core::state::{Controllers, State};
use openatc_settings::Settings;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Product version.
pub const VERSION: &str = "0.2.1";

/// Everything the engine owns: session state plus configuration.
pub struct EngineState {
    /// Generated scene preparation and future movement state.
    pub generated: generated::Runtime,
    /// Shared CPU budget for work that must not occupy the HTTP executor.
    pub workers: Arc<workers::Workers>,
    /// Model downloads run separately from flight and radio work.
    pub model_installer: traffic_models::Installer,
    /// Validated airline, hub and aircraft preferences, loaded on CPU workers.
    pub traffic_profiles: traffic_profiles::Store,
    /// Share one scenery scan between map, station and radio requests.
    pub station_index_lock: Arc<tokio::sync::Mutex<()>>,
    /// Optional controller offers and their expiry.
    pub initiative: initiative::Monitor,
    /// Arrival stand changes are retried at a bounded rate.
    pub parking: parking::Monitor,
    /// Bounded traffic history and active spacing instructions.
    pub traffic: traffic::Traffic,
    /// Selected mission clock and branching outcomes.
    pub mission: missions::Runtime,
    /// Codes recently assigned by this engine, including previous flights.
    pub squawks: squawk::Allocator,
    /// Passenger activity for the simulated cabin conversation.
    pub cabin: cabin::Cabin,
    /// Recent requests and verified outcomes for this flight.
    pub request_memory: memory::RequestMemory,
    /// Invalidates work started before a new-flight reset.
    pub session_generation: u64,
    /// Prepared crew radio exchange and its owning flight generation.
    pub pending_copilot_reply: Option<routes::PreparedReply>,
    /// Previous phrase choices and transmitted wording.
    pub phrase_history: radio::PhraseHistory,
    /// Aircraft crew controls, live readbacks and checklist progress.
    pub crew: crew::CrewSession,
    /// Automatic station transitions and arrival permissions.
    pub flow: flight::FlightFlow,
    /// Live session (plan, telemetry, phase, transcript, ...).
    pub session: State,
    /// Operator settings.
    pub settings: Settings,
    /// Cached published stations.
    pub stations: Vec<openatc_core::stations::Station>,
    /// Voice roster fetched from the configured TTS service.
    pub voices: Vec<openatc_core::voices::Voice>,
    /// TTS endpoint associated with the cached roster.
    pub voices_source: String,
    /// Last voice refresh attempt, including failed requests.
    pub voices_refreshed: Option<std::time::Instant>,
    /// Airport positions including fields without radio frequencies.
    pub map_airports: Vec<openatc_core::stations::AirportMarker>,
    /// Simulator folder used to build the airport marker index.
    pub map_root: String,
    /// Root used by the station index.
    pub station_root: String,
    /// Latest simulator airport weather, timestamped at receipt.
    /// Information letter advances only when the rounded airport observation changes.
    pub atis_information: BTreeMap<String, (String, usize)>,
    /// Bounded geometry cache for weather-based ATIS runway selection.
    pub atis_runways: BTreeMap<String, Vec<openatc_core::airport::Runway>>,
    /// A runway change advances the information letter independently of weather changes.
    pub atis_last_runway: BTreeMap<String, String>,
    /// Last information bulletin logged per airport and receiving radio.
    pub atis_logged: BTreeMap<String, String>,
    /// Surface observations received from X-Plane, with freshness timestamps.
    pub sim_weather: BTreeMap<String, (serde_json::Value, std::time::Instant)>,
    /// Loaded airport, if any.
    pub airport: Option<Airport>,
    /// Destination geometry retained while departure surface data is in use.
    pub arrival_airport: Option<Airport>,
    /// Published approach entry levels used to plan descent before final approach.
    pub arrival_profile: Option<openatc_core::arrival::ArrivalAirport>,
    /// Reviewed nominal descent margins loaded when a flight plan is submitted.
    pub vertical_policy: instructions::VerticalPolicy,
    /// Installed minimum off-route grid for vectoring checks.
    pub mora: openatc_core::navigation::Mora,
    /// Remembered controller voices.
    pub controllers: Controllers,
    /// Role prompts.
    pub prompts: support::Prompts,
    /// Region table from file, empty when built-in.
    pub regions: BTreeMap<String, Region>,
    /// Regions file path, empty when built-in.
    pub regions_path: String,
    /// Speech example pool for AI few-shot prompts, empty when missing.
    pub speech: openatc_core::speech::SpeechPool,
    /// Speech dir path, empty when missing.
    pub speech_source: String,
    /// Config directory holding settings/controllers/session JSON.
    pub config_dir: PathBuf,
    /// Handoff and clearance expectations awaiting pilot compliance.
    pub expectations: Vec<openatc_core::compliance::Expectation>,
    /// Shared HTTP client for AI/STT/TTS upstreams.
    pub client: reqwest::Client,
}

/// Engine state shared across handlers and the weather watch.
pub type Shared = Arc<RwLock<EngineState>>;

impl EngineState {
    fn prompts_text(&self, attendant: bool) -> String {
        if attendant {
            self.prompts.cabin.clone()
        } else {
            self.prompts.ground.clone()
        }
    }

    fn classify_prompt(&self) -> String {
        self.prompts.classify.clone()
    }

    fn copilot_chat_prompt(&self) -> String {
        self.prompts.copilot_chat.clone()
    }
}

/// Resolve the config dir: env override, then XDG, then home fallback.
fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OPENATC_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(dir).join("openatc");
    }
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("openatc");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config/openatc");
    }
    PathBuf::from("openatc-config")
}

/// Load settings.json when present, otherwise start from defaults.
fn load_settings(dir: &std::path::Path) -> Settings {
    let path = dir.join("settings.json");
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(settings) = serde_json::from_str(&text)
    {
        return settings;
    }
    Settings::default()
}

fn mtime(path: &std::path::Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

/// Settings hot-reload; live radio weather comes exclusively from the plugin.
async fn weather_loop(shared: Shared) {
    let mut settings_stamp = {
        let state = shared.read().await;
        mtime(&state.config_dir.join("settings.json"))
    };
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        reload_settings_if_changed(&shared, &mut settings_stamp).await;
        // Weather transmissions use simulator observations only. Internet
        // METAR fetches remain available as explicitly labelled planning data.
    }
}

/// Reload changed personal settings without blocking provider calls.
async fn reload_settings_if_changed(shared: &Shared, stamp: &mut std::time::SystemTime) {
    let state = shared.read().await;
    if mtime(&state.config_dir.join("settings.json")) == *stamp {
        return;
    }
    let path = state.config_dir.join("settings.json");
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(fresh) = serde_json::from_str::<Settings>(&text)
        && fresh.validate().is_ok()
    {
        drop(state);
        shared.write().await.settings = fresh;
    }
    *stamp = mtime(&path);
}

#[tokio::main]
async fn main() {
    use axum::routing::{get, post};
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--check-speech") {
        let Some(path) = args.get(2) else {
            eprintln!("Usage: open-atc-engine --check-speech /path/to/speech");
            std::process::exit(2);
        };
        if let Err(error) = openatc_core::dialogue::validate(
            &std::path::Path::new(path).join("runtime/responses.toml"),
        ) {
            eprintln!("Operational speech rejected: {error}");
            std::process::exit(1);
        }
        match openatc_core::speech::load_speech_dir(std::path::Path::new(path)) {
            Ok(pool) => {
                let samples = openatc_core::speech::sample_slots();
                let mut phrases = 0;
                for (id, entry) in pool.iter() {
                    for line in entry.say.iter().chain([&entry.accept, &entry.decline]) {
                        if line.is_empty() {
                            continue;
                        }
                        if let Err(error) = openatc_core::speech::render_checked(line, &samples) {
                            eprintln!("{id}: {error}");
                            std::process::exit(1);
                        }
                    }
                    phrases += entry.say.len();
                }
                println!(
                    "Speech library valid: {} situations, {phrases} phrases",
                    pool.len()
                );
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(8087);
    let plugin_parent = std::env::args().find_map(|arg| {
        arg.strip_prefix("--plugin-parent-pid=")
            .and_then(|pid| pid.parse::<u32>().ok())
    });
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_default();
    let config_dir = config_dir();
    for path in [
        exe_dir.join("../intents.toml"),
        config_dir.join("intents.toml"),
    ] {
        if path.is_file()
            && let Err(error) = openatc_core::catalog::activate(&path)
        {
            eprintln!("Intent catalogue rejected: {error}");
        }
    }
    let mut settings = load_settings(&config_dir);
    if settings.validate().is_err() {
        eprintln!("Settings invalid, starting from defaults");
        settings = Settings::default();
    }
    if let Ok(url) = std::env::var("OPENATC_AI_URL") {
        settings.ai_url = url;
        settings.ai_enabled = true;
    }
    if let Ok(model) = std::env::var("OPENATC_AI_MODEL") {
        settings.ai_model = model;
    }
    let mut controllers = Controllers::default();
    let controllers_path = config_dir.join("controllers.json");
    if let Ok(text) = std::fs::read_to_string(&controllers_path)
        && let Ok(saved) = serde_json::from_str(&text)
    {
        controllers = saved;
    }
    let prompts = support::load_prompts(&config_dir, &exe_dir);
    let regions_path = support::resolve_regions_file(&config_dir, &exe_dir);
    let regions = if regions_path.is_empty() {
        BTreeMap::new()
    } else {
        load_regions(std::path::Path::new(&regions_path)).unwrap_or_default()
    };
    let speech_source = support::resolve_speech_dir(&config_dir, &exe_dir);
    eprintln!(
        "Speech library: {speech_source}; phrase variety: {}",
        settings.llm_phrase_variety
    );
    if !speech_source.is_empty()
        && let Err(error) = openatc_core::dialogue::load(
            &std::path::Path::new(&speech_source).join("runtime/responses.toml"),
        )
    {
        eprintln!("Operational speech rejected: {error}");
        std::process::exit(1);
    }
    let speech = if speech_source.is_empty() {
        openatc_core::speech::SpeechPool::default()
    } else {
        match openatc_core::speech::load_speech_dir(std::path::Path::new(&speech_source)) {
            Ok(pool) => pool,
            Err(error) => {
                eprintln!("Speech library rejected: {error}");
                openatc_core::speech::SpeechPool::default()
            }
        }
    };
    let cabin = match cabin::Cabin::load(&config_dir, &exe_dir) {
        Ok(cabin) => cabin,
        Err(error) => {
            eprintln!("Cabin policy rejected: {error}");
            std::process::exit(1);
        }
    };
    let workers = Arc::new(workers::Workers::new(settings.cpu_workers));
    let traffic_profiles = traffic_profiles::Store::default();
    if let Err(error) = traffic_profiles.get(&config_dir, &workers, false).await {
        eprintln!("Traffic profiles rejected: {error}");
    }
    let shared: Shared = Arc::new(RwLock::new(EngineState {
        workers,
        model_installer: Default::default(),
        station_index_lock: Default::default(),
        cabin,
        request_memory: Default::default(),
        squawks: Default::default(),
        mission: Default::default(),
        parking: Default::default(),
        initiative: Default::default(),
        traffic: Default::default(),
        traffic_profiles,
        generated: Default::default(),
        session_generation: 0,
        pending_copilot_reply: None,
        phrase_history: radio::PhraseHistory::load(&config_dir),
        crew: crew::CrewSession::default(),
        flow: flight::FlightFlow::default(),
        session: State::default(),
        settings,
        stations: Vec::new(),
        voices: Vec::new(),
        voices_source: String::new(),
        voices_refreshed: None,
        map_airports: Vec::new(),
        map_root: String::new(),
        station_root: String::new(),
        sim_weather: BTreeMap::new(),
        atis_information: BTreeMap::new(),
        atis_runways: BTreeMap::new(),
        atis_last_runway: BTreeMap::new(),
        atis_logged: BTreeMap::new(),
        airport: None,
        arrival_airport: None,
        arrival_profile: None,
        vertical_policy: Default::default(),
        mora: Default::default(),
        controllers,
        prompts,
        regions,
        regions_path,
        speech,
        speech_source,
        config_dir,
        expectations: Vec::new(),
        client: reqwest::Client::new(),
    }));
    let weather = shared.clone();
    tokio::spawn(async move { weather_loop(weather).await });
    let lifetime = Arc::new(lifecycle::PluginSession::new(plugin_parent));
    let app = axum::Router::new()
        .route("/health", get(routes::health))
        .route("/voice-health", get(routes::voice_health))
        .route("/speech/voices", post(routes::post_voices))
        .route("/debug/map", post(radio::debug_map))
        .route("/state", get(routes::full_state))
        .route("/settings", post(routes::post_settings))
        .route("/missions/list", post(missions::list))
        .route("/missions/save", post(missions::save))
        .route("/missions/start", post(missions::start))
        .route("/missions/stop", post(missions::stop))
        .route("/missions/choose", post(missions::choose))
        .route("/traffic/talk", post(traffic::talk))
        .route("/traffic/profiles", post(traffic_profiles::inspect))
        .route("/traffic/models/status", post(traffic_models::status))
        .route("/traffic/models/index", post(traffic_models::index))
        .route("/traffic/models/install", post(traffic_models::install))
        .route("/traffic/models/cancel", post(traffic_models::cancel))
        .route("/atc/offer", post(initiative::answer))
        .route("/simulator/root", post(routes::post_simulator_root))
        .route("/debug/cancel-actions", post(routes::debug_cancel_actions))
        .route("/crew/observe", post(crew::observe))
        .route("/crew/ack", post(crew::acknowledge))
        .route("/request", post(routes::post_request))
        .route("/request/auto-reply", post(routes::post_auto_reply))
        .route("/request/copilot-reply", post(routes::post_copilot_reply))
        .route(
            "/request/copilot-checkin",
            post(routes::post_copilot_checkin),
        )
        .route(
            "/request/copilot-prepare",
            post(routes::post_copilot_prepare),
        )
        .route("/plan", post(routes::post_plan))
        .route("/plan/parking", post(routes::post_parking))
        .route("/simbrief", post(routes::post_simbrief))
        .route("/airport/load", post(routes::post_airport_load))
        .route("/airport/local", post(routes::post_airport_local))
        .route("/airport/arrival", post(routes::post_airport_arrival))
        .route("/stations/nearby", post(radio::nearby))
        .route("/weather/simulator", post(radio::weather))
        .route("/atis", post(radio::atis))
        .route("/weather", post(routes::post_weather))
        .route("/weather/online", post(routes::post_online_weather))
        .route("/speech/transcribe", post(routes::post_transcribe))
        .route("/speech/speak", post(routes::post_speak))
        .route("/suggest", post(routes::post_suggest))
        .route("/telemetry", post(routes::post_telemetry))
        .route("/plugin/shutdown", post(lifecycle::request_shutdown))
        .route("/plugin/heartbeat", post(lifecycle::heartbeat))
        .route("/demo", post(routes::post_demo))
        .route("/session/reset", post(routes::session_reset))
        .route("/session/save", post(routes::session_save))
        .route("/session/load", post(routes::session_load))
        .layer(axum::Extension(lifetime.clone()))
        .with_state(shared);
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    println!("OpenATC engine: http://127.0.0.1:{port}");
    lifecycle::serve(listener, app, lifetime).await;
}

#[cfg(test)]
fn test_state() -> EngineState {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let config_dir = std::env::temp_dir().join(format!(
        "openatc-unit-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    EngineState {
        workers: Arc::new(workers::Workers::new(1)),
        model_installer: Default::default(),
        station_index_lock: Default::default(),
        cabin: Default::default(),
        request_memory: Default::default(),
        squawks: Default::default(),
        mission: Default::default(),
        parking: Default::default(),
        initiative: Default::default(),
        traffic: Default::default(),
        traffic_profiles: Default::default(),
        generated: Default::default(),
        session_generation: 0,
        pending_copilot_reply: None,
        phrase_history: radio::PhraseHistory::load(&config_dir),
        crew: crew::CrewSession::default(),
        flow: flight::FlightFlow::default(),
        session: State::default(),
        settings: Settings::default(),
        stations: Vec::new(),
        voices: Vec::new(),
        voices_source: String::new(),
        voices_refreshed: None,
        map_airports: Vec::new(),
        map_root: String::new(),
        station_root: String::new(),
        sim_weather: BTreeMap::new(),
        atis_information: BTreeMap::new(),
        atis_runways: BTreeMap::new(),
        atis_last_runway: BTreeMap::new(),
        atis_logged: BTreeMap::new(),
        airport: None,
        arrival_airport: None,
        arrival_profile: None,
        vertical_policy: Default::default(),
        mora: Default::default(),
        controllers: Controllers::default(),
        prompts: support::load_prompts(&config_dir, &config_dir),
        regions: BTreeMap::new(),
        regions_path: String::new(),
        speech: Default::default(),
        speech_source: String::new(),
        config_dir,
        expectations: Vec::new(),
        client: reqwest::Client::new(),
    }
}
