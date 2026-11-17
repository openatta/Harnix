//! The engine, started in this process — Agix's embedding entry (`agix::embed`) — and what Harnix
//! reads about the engine it started before any link reaches it.
//!
//! The engine is linked as a library and runs in this process: no subprocess to supervise
//! (concept §9). It is reached the one way a daemon in another process is, over its protocol: an
//! engine Harnix keeps is behind a [`DaemonLink`] on one end of an in-memory stream
//! ([`InProcessEngine`]), and `harnix daemon` serves it on a socket ([`serve_daemon`]). Nothing of
//! Harnix's goes into it but its settings: what extensions contribute is registered on the link.
//!
//! The engine root is **passed in**, never read from this process's environment: otherwise one
//! process could not serve two roots, and the state would sit somewhere the caller cannot see
//! (concept §3.1).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use agix::embed::{EmbedConfig, EmbedError, Embedded};
use harnix_contract::sync::lock;
use harnix_contract::RpcError;
use harnix_engine_link::link::{EngineControl, EngineIdentity, InstanceCarrier, LinkStatus};
use harnix_engine_api::{EngineProfile, PROJECT_DIR};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::daemon_link::{Connection, DaemonLink};
use crate::host::HostSide;

/// What the engine answers a request a connection has no room for: an answer of the link's is
/// sent again then.
pub(crate) const TOO_MANY_IN_FLIGHT: i32 = agix::rpc::codes::TOO_MANY_IN_FLIGHT;

/// The name the engine goes by: what the model is told it is, the `User-Agent` of its requests,
/// its worktrees' branch prefix (`harnix/worktree-`).
const PRODUCT_NAME: &str = "Harnix";

/// What one direction of an in-process engine's stream holds before its writer waits for the
/// reader: the pipe's, not a frame's, limit — a larger frame goes through in pieces.
const DUPLEX_BYTES: usize = 1 << 20;

/// An engine's settings: an instance profile's `[engine]`, on a data root.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// The scene a session that names none is created in: one of the engine's own.
    pub scene: String,
    /// The engine's own scenes activated besides it. A scene layer is none of them: the hub sets
    /// a session up from one.
    pub scenes: Vec<String>,
    pub model: String,
    pub max_tokens: u32,
    pub session_cap: usize,
    pub session_idle_timeout_secs: u64,
    pub permission_prompt_timeout_secs: u64,
    /// The engine's data root — settings tiers, transcripts, memory, skills.
    pub data_root: PathBuf,
    /// The project a session belongs to when it names none. `None` is this process's working
    /// directory — an embedder should not have to `chdir` to choose it.
    pub cwd: Option<PathBuf>,
    /// Where [`Engine::serve_unix`] listens: a spawned daemon's socket. `None` in process.
    pub socket: Option<PathBuf>,
    /// Where the engine records the process groups its sessions' commands run in, so that the
    /// next start on the same file stops what a killed run left behind ([`Self::reaping`]). The
    /// record is the process's, one file for every engine in it: the first engine to start keeps
    /// it, and one started beside it runs without (its commands are recorded there all the same).
    /// `None` records nothing: what a killed run left keeps running.
    pub groups_file: Option<PathBuf>,
}

impl EngineConfig {
    /// The engine `profile` describes, on `data_root`.
    pub fn from_profile(profile: &EngineProfile, data_root: &Path) -> Self {
        Self {
            scene: profile.scene.clone(),
            scenes: profile.scenes.clone(),
            model: profile.model.clone(),
            max_tokens: profile.max_tokens,
            session_cap: profile.session_cap,
            session_idle_timeout_secs: profile.session_idle_timeout_secs,
            permission_prompt_timeout_secs: profile.permission_prompt_timeout_secs,
            data_root: data_root.to_path_buf(),
            cwd: None,
            socket: None,
            groups_file: None,
        }
    }

    /// Recording its commands' process groups in its data root, so that a start after a run that
    /// was killed stops what that run left behind: an instance that reaps on stop.
    pub fn reaping(mut self) -> Self {
        self.groups_file = Some(groups_file(&self.data_root));
        self
    }

    /// Agix's settings for it.
    fn embed(&self) -> EmbedConfig {
        let mut config = EmbedConfig::new(self.data_root.clone());
        config.project_dir = PROJECT_DIR.into();
        config.product_name = PRODUCT_NAME.into();
        config.cwd = self.cwd.clone();
        config.scene = self.scene.clone();
        config.scenes = Vec::new();
        for scene in &self.scenes {
            if *scene != self.scene && !config.scenes.contains(scene) {
                config.scenes.push(scene.clone());
            }
        }
        config.model = self.model.clone();
        config.max_tokens = self.max_tokens;
        config.session_cap = self.session_cap;
        config.session_idle_timeout = Duration::from_secs(self.session_idle_timeout_secs);
        config.permission_prompt_timeout = Some(Duration::from_secs(self.permission_prompt_timeout_secs));
        // The data root is guarded by Harnix's own lock on it (the instance's), not the engine's.
        config.instance = None;
        config.socket = self.socket.clone();
        config.groups_file = self.groups_file.clone();
        config
    }
}

/// Where an engine on `data_root` records its process groups: beside Agix's own records.
fn groups_file(data_root: &Path) -> PathBuf {
    data_root.join("daemon").join("groups").join("harnix.json")
}

/// An engine running in this process, and what was read of it as it started.
pub struct Engine {
    embedded: Embedded,
    /// The root it was started on, for reporting.
    pub data_root: PathBuf,
    /// The scenes it serves, sorted, once each.
    pub active_scenes: Vec<String>,
    /// The scene a session that names none is created in.
    pub scene: String,
    /// The model name sessions get with nothing overriding it.
    pub model: String,
    /// Whether an `ANTHROPIC_*` key was found at startup. False, with no provider configured,
    /// means the UI should send the user to the settings page before a first turn.
    pub has_credentials: bool,
}

impl Engine {
    /// Start the engine `config` describes.
    pub async fn start(config: EngineConfig) -> anyhow::Result<Engine> {
        let embed = config.embed();
        let base = embed.scene.clone();
        let embedded = match Embedded::start(embed.clone()).await {
            Ok(embedded) => embedded,
            // Another engine in this process keeps the process's record, which takes this one's
            // commands too.
            Err(EmbedError::GroupsInProcess { running }) => {
                tracing::debug!(record = %running.display(), "process groups recorded by another engine in this process");
                let mut embed = embed;
                embed.groups_file = None;
                Embedded::start(embed).await.map_err(|e| anyhow::anyhow!("{e}"))?
            }
            Err(e) => anyhow::bail!("{e}"),
        };

        // What the engine does not report, read the way it reads it: its global and scene tiers. A
        // project's tier is that project's sessions', and the directory the process started in is
        // no project of the person's; the data root stands in as the project, a directory that is
        // never one.
        let paths = agix::pool::StaticDaemonPaths::with_project(config.data_root.join("scenes").join(&base), config.data_root.clone())
            .with_global(config.data_root.clone());
        let loaded = agix::pool::load_daemon_config(&config.model, config.max_tokens, None, &base, &paths).settings;
        let settings = &loaded.values;
        // What a turn will actually use: with a provider router in play, `main` routes to that
        // provider's model, and reporting the profile's instead would name a model no request
        // ever reaches.
        let model = settings
            .default_provider
            .as_ref()
            .and_then(|id| settings.providers.get(id))
            .and_then(|provider| provider.default_model.clone())
            .unwrap_or_else(|| settings.model.model_name.clone());
        // An empty variable is how a key is unset in a profile or a launcher: not a credential.
        let set = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let has_credentials = set("ANTHROPIC_AUTH_TOKEN").or_else(|| set("ANTHROPIC_API_KEY")).is_some();
        if !has_credentials && settings.providers.is_empty() {
            tracing::warn!("no model credentials yet: add a provider from the settings page");
        }
        Ok(Engine {
            embedded,
            data_root: config.data_root.clone(),
            active_scenes: active_scenes(&config.scene, &config.scenes),
            scene: config.scene.clone(),
            model,
            has_credentials,
        })
    }

    /// Serve one connection on `stream` — an inherited descriptor, say — until the other end
    /// closes it. No token is asked for: whoever holds the other end was handed it.
    pub fn serve<S: AsyncRead + AsyncWrite + Send + 'static>(&self, stream: S) -> JoinHandle<()> {
        self.embedded.serve_stream(stream)
    }

    /// One connection to it in this process: the far end of an in-memory stream it serves, and
    /// the task serving it.
    fn connect(&self) -> (Connection, JoinHandle<()>) {
        let (ours, theirs) = tokio::io::duplex(DUPLEX_BYTES);
        let serving = self.embedded.serve_stream(theirs);
        let (read, write) = tokio::io::split(ours);
        (Connection::new(read, write), serving)
    }

    /// Listen on its socket until it begins to shut down.
    pub async fn serve_unix(&self) -> std::io::Result<()> {
        self.embedded.serve_unix().await
    }

    /// Returns once it has begun shutting down.
    pub async fn closed(&self) {
        self.embedded.closed().await
    }

    /// Finish in-flight turns and close every session.
    pub async fn shutdown(&self) {
        self.embedded.shutdown().await
    }
}

/// An engine started in this process, behind a [`DaemonLink`] on one end of an in-memory stream it
/// serves on the other. What the registry stops: the link, then the engine.
///
/// The link is the instance's link as it is for a daemon — the same handshake, the same
/// registration of what it offers, the same reader routing replies to their callers and everything
/// else to the event stream in the order read. Unlike a daemon's, the connection is never found
/// again: the engine is in this process, and only it going away closes the stream, so a lost
/// connection is a link that is down.
pub struct InProcessEngine {
    engine: Engine,
    link: Arc<DaemonLink>,
    serving: StdMutex<Option<JoinHandle<()>>>,
}

impl InProcessEngine {
    /// Connect a link offering what `host` offers to `engine`; `label` names the instance in the
    /// log. The link is `Ready`, subscribed to the engine's events and with `host`'s tools and
    /// hooks registered, once this returns.
    pub async fn start(engine: Engine, label: impl Into<String>, host: Arc<HostSide>) -> anyhow::Result<Arc<Self>> {
        let (connection, serving) = engine.connect();
        let link = DaemonLink::with_host(InstanceCarrier::InProcess, label, host);
        let taken = match link.take(connection, None).await {
            Ok(taken) => taken,
            Err(refusal) => {
                serving.abort();
                engine.shutdown().await;
                anyhow::bail!("the engine in this process refused its own link: {refusal:?}");
            }
        };
        // Holding `taken` keeps the reader going; letting it go, once the link is shut down, ends
        // the reader and with it this end of the stream.
        let watched = link.clone();
        tokio::spawn(async move {
            let stopping = watched.stopping();
            tokio::select! {
                _ = taken.lost() => {
                    watched.disconnect().await;
                    watched.down("the engine in this process closed its connection".into());
                }
                _ = stopping.cancelled() => {}
            }
        });
        Ok(Arc::new(Self { engine, link, serving: StdMutex::new(Some(serving)) }))
    }

    /// The link the hub is handed, as a daemon's would be.
    pub fn link(&self) -> &Arc<DaemonLink> {
        &self.link
    }

    /// The engine itself: to serve it on another stream too.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
}

#[async_trait::async_trait]
impl EngineControl for InProcessEngine {
    fn status(&self) -> watch::Receiver<LinkStatus> {
        self.link.status()
    }

    fn identity(&self) -> EngineIdentity {
        self.link.identity()
    }

    fn carrier(&self) -> InstanceCarrier {
        InstanceCarrier::InProcess
    }

    async fn reload_settings(&self) -> Result<(), RpcError> {
        self.link.reload_settings().await
    }

    /// The link first: what is still sending through it — a hub's queued turn — is told the
    /// instance is unavailable rather than starting a session the engine then waits out. Its end
    /// of the stream closing ends the connection, and its subscriptions, before the engine stops.
    async fn shutdown(&self) {
        self.link.shutdown().await;
        let serving = lock(&self.serving).take();
        if let Some(serving) = serving {
            let _ = serving.await;
        }
        self.engine.shutdown().await;
    }
}

/// `harnix daemon`: the engine `config` describes, on its socket until told to stop — or, with
/// `exit_with_stdin`, until stdin closes: how a spawned daemon learns its parent is gone, even
/// one that was killed outright. Whatever guards the data root is its caller's to hold. The
/// scenes are the engine's own: the parent sets a session up from a scene layer.
pub async fn serve_daemon(config: EngineConfig, exit_with_stdin: bool) -> anyhow::Result<()> {
    let socket = config.socket.clone().ok_or_else(|| anyhow::anyhow!("a daemon needs a socket to serve on"))?;
    let data_root = config.data_root.clone();
    let engine = Arc::new(Engine::start(config).await?);
    if exit_with_stdin {
        let engine = engine.clone();
        tokio::spawn(async move {
            let mut stdin = tokio::io::stdin();
            let mut buf = [0u8; 64];
            while matches!(stdin.read(&mut buf).await, Ok(n) if n > 0) {}
            tracing::info!("stdin closed; the parent is gone");
            engine.shutdown().await;
        });
    }
    tracing::info!(
        socket = %socket.display(),
        data_root = %data_root.display(),
        scenes = ?engine.active_scenes,
        "daemon serving"
    );
    engine.serve_unix().await?;
    engine.shutdown().await;
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

/// The scenes an engine serves: its default and the others, once each and sorted. The same for one
/// built here and one spawned with the same profile.
pub fn active_scenes(scene: &str, scenes: &[String]) -> Vec<String> {
    let mut all: Vec<String> = std::iter::once(scene.to_string()).chain(scenes.iter().cloned()).collect();
    all.sort();
    all.dedup();
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use harnix_engine_link::{EngineEvent, EngineLink};
    use serde_json::json;

    fn config(dir: &Path) -> EngineConfig {
        let mut config = EngineConfig::from_profile(&EngineProfile::new(), dir);
        config.model = "test-model".into();
        config.max_tokens = 1000;
        config.session_cap = 4;
        config.cwd = Some(dir.to_path_buf());
        config
    }

    async fn started(dir: &Path) -> Arc<InProcessEngine> {
        started_with(config(dir)).await
    }

    async fn started_with(config: EngineConfig) -> Arc<InProcessEngine> {
        let engine = Engine::start(config).await.unwrap();
        InProcessEngine::start(engine, "main", HostSide::none()).await.unwrap()
    }

    /// The engine's error codes the hub tells apart are the engine's own numbers.
    #[test]
    fn the_codes_the_hub_tells_apart_are_the_engines() {
        use harnix_engine_link::ops::codes;
        assert_eq!(codes::SESSION_NOT_FOUND, agix::rpc::codes::SESSION_NOT_FOUND);
        assert_eq!(codes::SESSION_BUSY, agix::rpc::codes::SESSION_BUSY);
        assert_eq!(codes::PROJECT_REQUIRED, agix::rpc::codes::PROJECT_REQUIRED);
        assert_eq!(TOO_MANY_IN_FLIGHT, -32017);
    }

    /// What the build's constants say is what a running engine says in `daemon.ping`: the
    /// manifest `build.rs` reads and the engine this links are the same Agix, and it has what a
    /// link needs of it.
    #[tokio::test]
    async fn the_engine_says_what_the_build_constants_say() {
        let dir = tempfile::tempdir().unwrap();
        let here = started(dir.path()).await;
        let identity = here.identity();
        assert_eq!(identity.version.as_deref(), Some(crate::AGIX_VERSION));
        assert_eq!(identity.protocol_version, Some(crate::AGIX_PROTOCOL));
        let has = identity.capabilities.as_array().cloned().unwrap_or_default();
        for needed in crate::REQUIRED_CAPABILITIES {
            assert!(has.contains(&json!(needed)), "{needed} in {has:?}");
        }
        assert_eq!(here.carrier(), InstanceCarrier::InProcess);
        here.shutdown().await;
    }

    /// A project's tier is its `.harnix/settings.json`; an `.agix/` beside it is not read.
    #[tokio::test]
    async fn a_projects_tier_is_in_its_harnix_directory() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("p");
        for (sub, tokens) in [(PROJECT_DIR, 1234), (".agix", 4321)] {
            std::fs::create_dir_all(project.join(sub)).unwrap();
            std::fs::write(project.join(sub).join("settings.json"), json!({"model": {"max_tokens": tokens}}).to_string()).unwrap();
        }
        let here = started(&dir.path().join("root")).await;
        let tier = here.link().ops().passthrough("config.get", json!({"scene": here.engine().scene, "tier": "project", "project_root": project})).await.unwrap();
        assert_eq!(tier["settings"]["model"]["max_tokens"], 1234, "{tier}");
        here.shutdown().await;
    }

    /// A call's reply goes to its caller and never into the event stream, and a mark made after
    /// it comes out behind the events the engine wrote before the reply: one ordered stream.
    /// Shut down, the link is down and calls through it are refused.
    #[tokio::test]
    async fn replies_go_to_their_callers_and_marks_come_after_them() {
        let dir = tempfile::tempdir().unwrap();
        let here = started(dir.path()).await;
        let link = here.link().clone();
        let mut events = link.take_events().unwrap();
        let created = link.ops().passthrough("session.create", json!({})).await.unwrap();
        assert!(created.get("session_id").is_some(), "{created}");
        link.mark(7).await;
        let mut seen = Vec::new();
        loop {
            match tokio::time::timeout(Duration::from_secs(10), events.recv()).await.unwrap().unwrap() {
                EngineEvent::Barrier { id } => {
                    assert_eq!(id, 7);
                    break;
                }
                other => seen.push(other),
            }
        }
        assert!(
            seen.iter().any(|e| matches!(e, EngineEvent::Daemon { .. })),
            "the session's creation was told before the mark: {seen:?}"
        );
        here.shutdown().await;
        assert!(matches!(*here.status().borrow(), LinkStatus::Down { .. }));
        assert!(link.ops().passthrough("daemon.status", json!({})).await.is_err());
    }

    /// The process keeps one record of its commands' process groups: a second engine beside the
    /// first starts all the same, and neither leaves a record behind once both are shut down.
    #[tokio::test]
    async fn two_engines_in_a_process_start_and_leave_no_record() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        assert_eq!(config(a.path()).groups_file, None);
        assert_eq!(config(a.path()).reaping().groups_file, Some(a.path().join("daemon/groups/harnix.json")));
        let first = started_with(config(a.path()).reaping()).await;
        let second = started_with(config(b.path()).reaping()).await;
        second.shutdown().await;
        first.shutdown().await;
        assert!(!groups_file(a.path()).exists());
        assert!(!groups_file(b.path()).exists());
    }

    #[test]
    fn the_active_scenes_are_the_default_and_the_others_once_each() {
        let others = ["mini".to_string(), "code".to_string(), "mini".to_string()];
        assert_eq!(active_scenes("harnix", &others), ["code", "harnix", "mini"]);
        assert_eq!(active_scenes("code", &["code".to_string()]), ["code"]);
        assert_eq!(active_scenes("harnix", &[]), ["harnix"]);
    }
}
