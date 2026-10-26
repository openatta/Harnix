//! Standing the engine up, in this process.
//!
//! The engine is linked as a library and called in-process: no socket, no
//! handshake, no subprocess to supervise (§9). This does what `agixd`'s
//! `main.rs` does — resolve scenes, load the settings tiers, build the model
//! client and the `SessionPool` — and stops before the part that binds
//! listeners. What it hands back is a `DaemonServer` whose `dispatch_public`
//! the hub calls directly.
//!
//! The assembly itself is Agix's (`daemon::assemble::pool`); this file only
//! decides what Harnix hands it: the root, the scenes, a client that may start
//! without credentials, and a transcript store the hub keeps a handle on.
//!
//! The engine root is **passed in**, never read from this process's
//! environment: otherwise one process could not serve two roots, and the
//! state would sit somewhere the caller cannot see (§3.1).

use std::path::PathBuf;
use std::sync::Arc;

use daemon::assemble::{Assembly, Transcripts};
use daemon::config::{load_daemon_config, StaticDaemonPaths};
use daemon::{DaemonServer, SessionPool};
use model::client::{AnthropicClient, AuthMode, HttpAnthropicClient};
use tokio_util::sync::CancellationToken;
use tracing::info;

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Default scene for `session.create` and the state root under
    /// `~/.agix/scenes/<scene>/`.
    pub scene: String,
    /// Scenes activated on top of `scene`.
    pub scenes: Vec<String>,
    pub model: String,
    pub max_tokens: u32,
    pub session_cap: usize,
    pub session_idle_timeout_secs: u64,
    pub permission_prompt_timeout_secs: u64,
    /// The engine's data root — settings tiers, transcripts, memory, skills.
    /// What `AGIX_CONFIG_HOME` names, resolved by the app and handed in.
    pub data_root: PathBuf,
    /// The project a session belongs to when it names none. `None` is this
    /// process's working directory — an embedder should not have to `chdir`
    /// to choose it, any more than to choose the data root.
    pub cwd: Option<PathBuf>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            scene: "coding".to_string(),
            scenes: Vec::new(),
            model: "claude-sonnet-4-6".to_string(),
            max_tokens: 2000,
            session_cap: 32,
            session_idle_timeout_secs: 3600,
            permission_prompt_timeout_secs: 300,
            data_root: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".agix"),
            cwd: None,
        }
    }
}

pub struct Engine {
    pub server: Arc<DaemonServer>,
    /// The root this engine was built on, for reporting.
    pub data_root: PathBuf,
    /// The transcript store, for readers that would otherwise page the RPC
    /// surface to get at text (see `search`).
    pub history: Option<Arc<dyn history::store::HistoryStore>>,
    /// Where a session's recording lands — one directory per session id. The
    /// engine has no reader for it, so the hub goes to the files directly;
    /// this is the one place that says which files.
    pub recordings_root: PathBuf,
    pub pool: Arc<SessionPool>,
    pub cancel: CancellationToken,
    /// Scenes actually active after startup, sorted.
    pub active_scenes: Vec<String>,
    /// The model name sessions get with nothing overriding it.
    pub model: String,
    /// Whether a usable credential was found at startup. False means the UI
    /// should send the user to the settings page before a first turn.
    pub has_credentials: bool,
}

impl Engine {
    /// Finish in-flight turns, close every session, cancel the pool's tasks.
    pub async fn shutdown(&self) {
        self.pool.shutdown_all().await;
        self.cancel.cancel();
    }
}

fn resolve_scene(name: &str) -> anyhow::Result<Arc<dyn base::interface::scene::AgentScene>> {
    let mut registry = scene::scene::SceneRegistry::new();
    registry.register_builtin();
    registry.resolve(name).ok_or_else(|| {
        let mut known = registry.ids();
        known.sort();
        anyhow::anyhow!(
            "unsupported scene `{name}` — supported scenes: {}",
            known.join(", ")
        )
    })
}

fn active_scenes(scene: &str, scenes: &[String]) -> Vec<String> {
    let mut all: Vec<String> = std::iter::once(scene.to_string())
        .chain(scenes.iter().cloned())
        .collect();
    all.sort();
    all.dedup();
    all
}

pub async fn build(config: EngineConfig) -> anyhow::Result<Engine> {
    let scene = resolve_scene(&config.scene)?;
    let scope = scene.id().to_string();
    for extra in &config.scenes {
        resolve_scene(extra)?; // fail before any state is touched
    }

    let cwd = config
        .cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // `{data_root}/scenes/<scope>/` over `{data_root}/` — the same layout
    // `DefaultDaemonPaths::from_env` derives, with the root handed in.
    let paths =
        StaticDaemonPaths::with_project(config.data_root.join("scenes").join(&scope), cwd.clone())
            .with_global(config.data_root.clone());
    let mut daemon_config = load_daemon_config(
        &config.model,
        config.max_tokens,
        // No socket: nothing listens on one. `load_daemon_config` still
        // derives a path for it, which we never create.
        None,
        &scope,
        &paths,
    );
    daemon_config.session_cap = config.session_cap;
    daemon_config.session_idle_timeout_secs = config.session_idle_timeout_secs;
    daemon_config.settings.session_dir = Some(daemon_config.settings.paths.local_data_dir.clone());

    // A configured provider is a complete answer on its own: the router serves
    // `main` and the client built here is never asked for anything. Requiring
    // an environment variable anyway would mean a fresh install could not be
    // set up from its own settings page.
    let has_router = !daemon_config.settings.providers.is_empty();
    let api_key = std::env::var("ANTHROPIC_AUTH_TOKEN")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .ok();
    if api_key.is_none() && !has_router {
        anyhow::bail!(
            "no model credentials: set ANTHROPIC_AUTH_TOKEN (or ANTHROPIC_API_KEY), \
             or configure a provider in settings.json"
        );
    }
    let has_credentials = api_key.is_some();
    let auth = AuthMode::ApiKey(api_key.unwrap_or_default());
    let client: Arc<dyn AnthropicClient> = match std::env::var("ANTHROPIC_BASE_URL").ok() {
        Some(mut url) => {
            if !url.ends_with('/') {
                url.push('/');
            }
            let base = reqwest::Url::parse(&url)
                .map_err(|e| anyhow::anyhow!("invalid ANTHROPIC_BASE_URL: {e}"))?;
            Arc::new(HttpAnthropicClient::with_base(auth, base)?)
        }
        None => Arc::new(HttpAnthropicClient::new(auth)?),
    };

    let global_dir = daemon_config.settings.paths.global_data_dir.clone();
    let settings = &daemon_config.settings;
    // What a turn will actually use: with a provider router in play, `main`
    // routes to that provider's model, and reporting the CLI default instead
    // would name a model no request ever reaches.
    let model_name = settings
        .default_provider
        .as_ref()
        .and_then(|id| settings.providers.get(id))
        .and_then(|provider| provider.default_model.clone())
        .unwrap_or_else(|| settings.model.model_name.clone());

    // Mirrors `daemon::session_pool::recordings_root` with no `dir` override,
    // which is how the hub asks for recording (see `harnix_hub::recordings`).
    // The two must agree: the daemon decides where a recording is written and
    // this decides where it is read from.
    let recordings_root = settings
        .recorder
        .as_ref()
        .map(|recorder| recorder.root.clone())
        .unwrap_or_else(|| {
            base::paths::ConfigPaths::from_settings(&settings.paths).global_recordings_dir()
        });

    // Built here rather than left to the assembly because the hub keeps a
    // handle on it (`Engine::history`).
    let history_roots = history::path::HistoryRoots::under(&global_dir);
    let migration = history::migrate::migrate_layout(&global_dir);
    if !migration.did_nothing() {
        info!(
            transcripts = migration.transcripts_moved,
            sidecars = migration.sidecars_moved,
            "migrated session state to the projects/ + sessions/ layout"
        );
    }
    let history_store: Option<Arc<dyn history::store::HistoryStore>> =
        match history::store::JsonlHistoryStore::with_roots(&cwd, history_roots).await {
            Ok(store) => Some(Arc::new(store)),
            Err(e) => {
                tracing::warn!(error = %e, "history store unavailable; sessions are in-memory only");
                None
            }
        };
    let transcripts = match history_store.clone() {
        Some(store) => Transcripts::In(store),
        None => Transcripts::Nowhere,
    };

    let pool = daemon::assemble::pool(
        &daemon_config,
        scene,
        Assembly {
            cwd: Some(cwd),
            extra_scenes: config.scenes.clone(),
            scenes: Vec::new(),
            permission_prompt_timeout: std::time::Duration::from_secs(
                config.permission_prompt_timeout_secs,
            ),
            model_client: Some(client),
            credentials: None,
            transcripts,
            permission: None,
            customize_session: None,
            mcp_oauth_resolver: None,
        },
    )
    .await?;

    let cancel = CancellationToken::new();
    let server = Arc::new(DaemonServer::new(pool.clone(), cancel.clone()));

    Ok(Engine {
        server,
        data_root: config.data_root.clone(),
        history: history_store,
        recordings_root,
        pool,
        cancel,
        active_scenes: active_scenes(&config.scene, &config.scenes),
        model: model_name,
        has_credentials,
    })
}
