//! Turning a carrier description into a running link.
//!
//! The one place that knows how each carrier is stood up: an engine built in
//! this process, `harnix daemon` spawned with its socket in a private
//! directory, or a connection to a daemon somebody else runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use harnix_engine::{Chains, Endpoint, Engine, EngineConfig, HostSide, InProcessEngine, Recording, ReplayLink};
use harnix_engine_api::{EngineContribution, EngineControl, EngineLink, SceneLayers};

use crate::daemon::{DaemonSpec, SpawnOptions, SpawnSpec, Supervised};
use crate::profile::EngineProfile;
use crate::servers::SharedServers;
use crate::templates::{InstanceTemplate, TemplateCarrier};

/// What is known about an instance without asking it. Only an in-process
/// engine can answer most of these; a daemon's are its own.
#[derive(Clone, Default)]
pub struct Facts {
    pub data_root: PathBuf,
    pub recordings_root: Option<PathBuf>,
    pub history: Option<Arc<dyn harnix_engine::HistoryStore>>,
    pub active_scenes: Vec<String>,
    /// The scene `session.create` uses when it names none.
    pub scene: Option<String>,
    pub model: Option<String>,
    pub has_credentials: bool,
}

impl Facts {
    /// Where the instance keeps its data, as the instance list and `hello` show it. What it holds
    /// is this layer's to write: the hub reports the place, and never opens it.
    pub fn root_label(&self) -> String {
        self.data_root.display().to_string()
    }
}

/// What an engine started in this process says about itself.
impl From<&Engine> for Facts {
    fn from(engine: &Engine) -> Self {
        Self {
            data_root: engine.data_root.clone(),
            recordings_root: Some(engine.recordings_root.clone()),
            history: engine.history.clone(),
            active_scenes: engine.active_scenes.clone(),
            scene: Some(engine.scene.clone()),
            model: Some(engine.model.clone()),
            has_credentials: engine.has_credentials,
        }
    }
}

/// What a credential name resolves to.
pub type Resolve = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// An instance as it was stood up: the link the hub is handed, and what the registry keeps to
/// stop it — the same link, or what keeps a daemon running.
pub struct Built {
    pub link: Arc<dyn EngineLink>,
    pub control: Arc<dyn EngineControl>,
    pub facts: Facts,
}

impl Built {
    /// A link that is its own control.
    pub fn new(link: Arc<dyn EngineLink>, facts: Facts) -> Self {
        let control: Arc<dyn EngineControl> = link.clone();
        Self { link, control, facts }
    }

    fn supervised(daemon: Arc<Supervised>, facts: Facts) -> Self {
        Self { link: daemon.link(), control: daemon, facts }
    }
}

pub struct Factory {
    /// The binary `spawn` starts when a template names none: this one.
    pub program: PathBuf,
    /// Where spawned daemons' sockets go. Short, because a unix socket path
    /// has a length limit (104 bytes on macOS) that a data directory nested
    /// in a temporary directory can exceed; private, because anyone who can
    /// open the socket drives the daemon.
    pub socket_dir: PathBuf,
    /// What a credential reference resolves to (an attached daemon's token).
    pub credential: Resolve,
    /// Tool providers every engine built in this process gets — the in-process
    /// carrier's alone: a spawned daemon reaches a package's tools another way, over MCP.
    pub providers: Vec<Arc<dyn harnix_engine_api::ToolProvider>>,
    /// The MCP servers every daemon this process spawns is configured with — an extension
    /// package's tools — written into each one's data root as it is built, and kept in line
    /// with the set after (`servers.rs`). Shared with whoever sets them.
    pub servers: Arc<SharedServers>,
    /// The data root of the instance a template's `inherit_providers` inherits from: the
    /// main instance's. `None`, nothing is inherited.
    pub providers_from: Option<PathBuf>,
    /// What every link built here records with.
    pub recording: Arc<Recording>,
    /// Which calls each session's running turn is on behalf of, as the hub writes it: what the
    /// tool calls of every engine built in this process carry.
    pub turns: Arc<harnix_kernel::TurnChains>,
}

/// The hub's turns, as an engine's tool calls read them.
struct TurnSource(Arc<harnix_kernel::TurnChains>);

impl harnix_engine_api::ChainSource for TurnSource {
    fn chain(&self, session: &harnix_engine_api::SessionRef) -> Vec<harnix_engine_api::ChainLink> {
        self.0.chain(&session.instance, &session.session_id).links().to_vec()
    }
}

impl Factory {
    pub fn new(credential: Resolve) -> anyhow::Result<Self> {
        let program = std::env::current_exe()?;
        let socket_dir = std::env::temp_dir().join(format!("harnix-{}", std::process::id()));
        create_private_dir(&socket_dir)?;
        Ok(Self {
            program,
            socket_dir,
            credential,
            providers: Vec::new(),
            servers: Arc::default(),
            providers_from: None,
            recording: Recording::new(None),
            turns: Arc::default(),
        })
    }

    /// Where the tool calls of an engine this process links to read their turn's chain.
    pub fn chains(&self) -> Chains {
        Chains(Arc::new(TurnSource(self.turns.clone())))
    }

    /// An engine started in this process from `profile` on `data_root`, `contrib` offered on its
    /// link: the main instance's, or a template's. The registry stops the engine with the link.
    pub async fn in_process(
        &self,
        id: &str,
        profile: &EngineProfile,
        data_root: PathBuf,
        contrib: EngineContribution,
    ) -> anyhow::Result<Built> {
        let host = HostSide::new(id, self.chains(), &contrib).map_err(anyhow::Error::msg)?;
        let profile = engine_scenes(id, profile, host.layers())?;
        let engine = Engine::start(EngineConfig::from_profile(&profile, &data_root)).await?;
        let facts = Facts::from(&engine);
        let here = InProcessEngine::start(engine, id, host).await?;
        let recorder = self.recording.of(id, facts.recordings_root.clone());
        Ok(Built { link: ReplayLink::daemon(here.link().clone(), recorder), control: here, facts })
    }

    pub async fn build(
        &self,
        id: &str,
        carrier: &TemplateCarrier,
        data_root: PathBuf,
    ) -> anyhow::Result<Built> {
        self.build_with(id, carrier, data_root, None).await
    }

    /// An instance of `template`: its carrier, with what the template adds to it — native
    /// contributions (ipdrun-10) and reaping on stop (ipdrun-07).
    pub async fn build_template(
        &self,
        id: &str,
        template: &InstanceTemplate,
        data_root: PathBuf,
    ) -> anyhow::Result<Built> {
        self.build_with(id, &template.carrier, data_root, Some(template)).await
    }

    async fn build_with(
        &self,
        id: &str,
        carrier: &TemplateCarrier,
        data_root: PathBuf,
        template: Option<&InstanceTemplate>,
    ) -> anyhow::Result<Built> {
        std::fs::create_dir_all(&data_root)?;
        if template.is_some_and(|t| t.inherit_providers) {
            if let Err(e) = self.inherit(&data_root) {
                tracing::warn!(instance = id, error = %e, "the inherited providers were not written");
            }
        }
        let reap_on_stop = template.is_some_and(|t| t.reap_on_stop);
        if reap_on_stop {
            crate::reap::engine_reaps(&data_root)?;
        }
        match carrier {
            TemplateCarrier::InProcess { engine } => {
                let mut contrib = template.and_then(InstanceTemplate::contribution).unwrap_or_default();
                for provider in &self.providers {
                    contrib.provider(provider.clone());
                }
                self.in_process(id, engine, data_root, contrib).await
            }
            TemplateCarrier::Spawn {
                engine,
                program,
                args,
            } => {
                let socket = self.socket_dir.join(format!("{}.sock", short_name(id)));
                // The template's native contributions are offered by this process, on the link: a
                // daemon has nowhere to build them. Its scene layers are the hub's to set a
                // session up from, and their tools are answered on the link. Not the providers
                // every engine in this process gets: the daemon reaches those over MCP.
                let contrib = template.and_then(InstanceTemplate::contribution).unwrap_or_default();
                let host = HostSide::new(id, self.chains(), &contrib).map_err(anyhow::Error::msg)?;
                let engine = &engine_scenes(id, engine, host.layers())?;
                let mut argv = daemon_args(engine, &data_root, &socket);
                // This binary's daemon holds the instance's lock on its data root (ipdrun-08); a
                // program of somebody else's gets the arguments it always got.
                let lock = program.is_none().then(|| {
                    argv.push("--instance".into());
                    argv.push(id.to_string());
                    crate::data_lock::lock_path(&data_root, id)
                });
                argv.extend(args.iter().cloned());
                let mut options = if reap_on_stop {
                    SpawnOptions::default().reap_on_stop()
                } else {
                    SpawnOptions::default()
                };
                // Before the daemon reads its settings: the servers it reaches this process's
                // packages by, with a credential of its own.
                options.servers = Some(self.servers.configure(id, &data_root));
                // Where a daemon writes its recordings is its own: a deleted session's stays.
                options.recorder = Some(self.recording.of(id, None));
                options.host = Some(host);
                let daemon = Supervised::start_with(DaemonSpec {
                    endpoint: Endpoint::Unix(socket),
                    spawn: Some(SpawnSpec {
                        program: program.clone().unwrap_or_else(|| self.program.clone()),
                        args: argv,
                        env: loopback_without_proxy(),
                        lock,
                    }),
                }, options);
                let facts = Facts {
                    data_root,
                    active_scenes: harnix_engine::active_scenes(&engine.scene, &engine.scenes),
                    scene: Some(engine.scene.clone()),
                    model: Some(engine.model.clone()),
                    ..Facts::default()
                };
                Ok(Built::supervised(daemon, facts))
            }
            TemplateCarrier::Attach(to) => {
                let endpoint = match (&to.socket, &to.tcp) {
                    (Some(socket), None) => Endpoint::Unix(socket.clone()),
                    (None, Some(addr)) => {
                        let name = to.token.as_deref().ok_or_else(|| {
                            anyhow::anyhow!("attaching over TCP needs `token`, a credential name")
                        })?;
                        let token = (self.credential)(name).ok_or_else(|| {
                            anyhow::anyhow!("the credential `{name}` has no value")
                        })?;
                        Endpoint::Tcp {
                            addr: addr.clone(),
                            token,
                        }
                    }
                    _ => anyhow::bail!("attach takes exactly one of `socket` and `tcp`"),
                };
                let options = SpawnOptions { recorder: Some(self.recording.of(id, None)), ..SpawnOptions::default() };
                let daemon = Supervised::start_with(DaemonSpec { endpoint, spawn: None }, options);
                Ok(Built::supervised(daemon, Facts { data_root, ..Facts::default() }))
            }
            _ => anyhow::bail!("instance `{id}`: a carrier this build does not know"),
        }
    }

    /// `main`'s providers into the instance whose data `facts` describes; `Ok(true)` when its
    /// settings changed and it should reload. For `harnix.instances.inheritProviders`, after the
    /// settings page changed one; an instance built from a template that inherits got them as it
    /// was built.
    pub fn inherit_into(&self, facts: &Facts) -> std::io::Result<bool> {
        // An instance that was never built (listed down) has no data root to write into.
        if facts.data_root.as_os_str().is_empty() {
            return Ok(false);
        }
        self.inherit(&facts.data_root)
    }

    fn inherit(&self, data_root: &Path) -> std::io::Result<bool> {
        match &self.providers_from {
            Some(from) => crate::inherit::merge(from, data_root),
            None => Ok(false),
        }
    }
}

/// A daemon this process spawns reaches this process back over loopback (an extension
/// package's MCP endpoint). A proxy configured in the environment (`HTTP_PROXY`) would carry
/// that to somewhere that is not this machine, so loopback is added to whatever `NO_PROXY` says.
fn loopback_without_proxy() -> Vec<(String, String)> {
    const LOOPBACK: &str = "127.0.0.1,localhost,::1";
    ["NO_PROXY", "no_proxy"]
        .iter()
        .map(|name| {
            let value = match std::env::var(name) {
                Ok(given) if !given.trim().is_empty() => format!("{given},{LOOPBACK}"),
                _ => LOOPBACK.to_string(),
            };
            (name.to_string(), value)
        })
        .collect()
}

/// `profile` with the engine's own scenes alone: a scene layer of the instance's is none of the
/// engine's, and is not activated in it. A default scene that is a layer is refused: the engine
/// creates a session in its default without the hub, which sets one up from a layer.
fn engine_scenes(id: &str, profile: &EngineProfile, layers: &SceneLayers) -> anyhow::Result<EngineProfile> {
    if layers.get(&profile.scene).is_some() {
        anyhow::bail!(
            "instance `{id}`: `EngineProfile.scene` names one of the engine's own scenes; give a scene layer's id (`{}`) as the space's or the session's `scene`",
            profile.scene
        );
    }
    let mut profile = profile.clone();
    profile.scenes.retain(|scene| {
        let layer = layers.get(scene).is_some();
        if layer {
            tracing::debug!(instance = id, scene = %scene, "a scene layer, not one of the engine's: not activated in it");
        }
        !layer
    });
    Ok(profile)
}

/// The `harnix daemon` command line for these settings. Read back by [`crate::DaemonArgs`] on the
/// other side.
fn daemon_args(engine: &EngineProfile, data_root: &Path, socket: &Path) -> Vec<String> {
    let scene = engine.scene.clone();
    let mut scenes: Vec<String> = Vec::new();
    for other in &engine.scenes {
        if *other != scene && !scenes.contains(other) {
            scenes.push(other.clone());
        }
    }
    let mut argv = vec![
        "daemon".to_string(),
        "--data-root".into(),
        data_root.display().to_string(),
        "--socket".into(),
        socket.display().to_string(),
        "--scene".into(),
        scene,
        "--model".into(),
        engine.model.clone(),
        "--max-tokens".into(),
        engine.max_tokens.to_string(),
        "--session-cap".into(),
        engine.session_cap.to_string(),
        "--session-idle-timeout".into(),
        engine.session_idle_timeout_secs.to_string(),
        "--permission-prompt-timeout".into(),
        engine.permission_prompt_timeout_secs.to_string(),
        "--exit-with-stdin".into(),
    ];
    if !scenes.is_empty() {
        argv.push("--scenes".into());
        argv.push(scenes.join(","));
    }
    argv
}

/// Instance ids can be 64 characters; socket names should not be.
fn short_name(id: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    let safe: String = id.chars().take(16).collect();
    format!("{safe}-{:08x}", h.finish() as u32)
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.socket_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scene layer is not activated in the engine, and is no engine's default.
    #[test]
    fn the_engine_is_started_with_its_own_scenes_alone() {
        use harnix_engine_api::{SceneBase, SceneLayer};
        let layers = SceneLayers::new(&[SceneLayer::new("probe", SceneBase::builtin("harnix"))]).unwrap();
        let profile = EngineProfile::new().scene("harnix").scenes(["probe", "mini"]);
        assert_eq!(engine_scenes("i", &profile, &layers).unwrap().scenes, ["mini"]);
        let wrong = EngineProfile::new().scene("probe");
        assert!(engine_scenes("i", &wrong, &layers).unwrap_err().to_string().contains("scene layer"));
    }

    #[test]
    fn providers_go_only_where_there_is_a_data_root() {
        let dir = tempfile::tempdir().unwrap();
        let (system, instance) = (dir.path().join("system"), dir.path().join("instance"));
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(system.join("settings.json"), r#"{"providers": {"a": {"api_type": "anthropic"}}}"#).unwrap();
        let mut factory = Factory::new(Arc::new(|_: &str| None)).unwrap();
        let built = Facts {
            data_root: instance.clone(),
            ..Facts::default()
        };
        assert!(!factory.inherit_into(&built).unwrap(), "inherited with nowhere to inherit from");
        factory.providers_from = Some(system);
        assert!(factory.inherit_into(&built).unwrap());
        assert!(instance.join("settings.json").is_file());
        // An instance listed down was never built and has none: not the working directory's.
        assert!(!factory.inherit_into(&Facts::default()).unwrap());
    }
}
