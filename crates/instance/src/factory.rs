//! Turning a carrier description into a running link.
//!
//! The one place that knows how each carrier is stood up: an engine built in
//! this process, `harnix daemon` spawned with its socket in a private
//! directory, or a connection to a daemon somebody else runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::daemon_link::{DaemonLink, DaemonSpec, Endpoint, SpawnOptions, SpawnSpec};
use crate::engine::{self, EngineConfig};
use crate::inproc::InProcessLink;
use crate::link::EngineLink;
use crate::profile::EngineProfile;
use crate::templates::{InstanceTemplate, TemplateCarrier};

/// What is known about an instance without asking it. Only an in-process
/// engine can answer most of these; a daemon's are its own.
#[derive(Clone, Default)]
pub struct Facts {
    pub data_root: PathBuf,
    pub recordings_root: Option<PathBuf>,
    pub history: Option<Arc<dyn history::store::HistoryStore>>,
    pub active_scenes: Vec<String>,
    /// The scene `session.create` uses when it names none.
    pub scene: Option<String>,
    pub model: Option<String>,
    pub has_credentials: bool,
}

/// What an engine built in this process says about itself.
impl From<&crate::Engine> for Facts {
    fn from(engine: &crate::Engine) -> Self {
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

pub struct Built {
    pub link: Arc<dyn EngineLink>,
    pub facts: Facts,
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
    /// Agix plugins every engine built in this process gets — the in-process
    /// carrier's alone: a daemon is another process, and what it should have
    /// of this one's reaches it another way (an application's tools, over MCP).
    pub plugins: Vec<Arc<dyn runtime::plugin::Plugin>>,
    /// The tools those plugins register, which every scene of such an engine admits
    /// (`EngineContrib::admit_tools`).
    pub admitted_tools: Vec<String>,
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
            plugins: Vec::new(),
            admitted_tools: Vec::new(),
        })
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
        let reap_on_stop = template.is_some_and(|t| t.reap_on_stop);
        if reap_on_stop {
            crate::reap::engine_reaps(&data_root)?;
        }
        match carrier {
            TemplateCarrier::InProcess { engine, contrib } => {
                let mut contrib = contrib.clone();
                if let Some(native) = template.and_then(InstanceTemplate::native_contrib) {
                    contrib.extend(native);
                }
                for plugin in &self.plugins {
                    contrib.plugin(plugin.clone());
                }
                for pattern in &self.admitted_tools {
                    contrib.admit_tools(pattern.clone());
                }
                let engine = engine::build(engine_config(engine, &data_root, contrib)).await?;
                let facts = Facts::from(&engine);
                let link: Arc<dyn EngineLink> = InProcessLink::start(engine).await;
                Ok(Built { link, facts })
            }
            TemplateCarrier::Spawn {
                engine,
                program,
                args,
            } => {
                let socket = self.socket_dir.join(format!("{}.sock", short_name(id)));
                let mut argv = daemon_args(engine, &data_root, &socket);
                // This binary's daemon holds the instance's lock on its data root (ipdrun-08); a
                // program of somebody else's gets the arguments it always got.
                let lock = program.is_none().then(|| {
                    argv.push("--instance".into());
                    argv.push(id.to_string());
                    crate::data_lock::lock_path(&data_root, id)
                });
                // Only this binary's daemon knows how to build the template's native contributions
                // again (ipdrun-10).
                if program.is_none() {
                    if let Some(template) = template.filter(|t| t.native.is_some()) {
                        argv.push("--template".into());
                        argv.push(template.id.clone());
                    }
                }
                argv.extend(args.iter().cloned());
                let options = if reap_on_stop {
                    SpawnOptions::default().reap_on_stop()
                } else {
                    SpawnOptions::default()
                };
                let link: Arc<dyn EngineLink> = DaemonLink::start_with(DaemonSpec {
                    endpoint: Endpoint::Unix(socket),
                    spawn: Some(SpawnSpec {
                        program: program.clone().unwrap_or_else(|| self.program.clone()),
                        args: argv,
                        env: loopback_without_proxy(),
                        lock,
                    }),
                }, options);
                Ok(Built {
                    link,
                    facts: Facts {
                        data_root,
                        active_scenes: scenes_of(engine),
                        scene: Some(engine.scene.clone()),
                        model: Some(engine.model.clone()),
                            ..Facts::default()
                    },
                })
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
                let link: Arc<dyn EngineLink> = DaemonLink::start(DaemonSpec {
                    endpoint,
                    spawn: None,
                });
                Ok(Built {
                    link,
                    facts: Facts {
                        data_root,
                        ..Facts::default()
                    },
                })
            }
        }
    }
}

/// A daemon this process spawns reaches this process back over loopback (a runtime
/// application's MCP endpoint). A proxy configured in the environment (`HTTP_PROXY`) would carry
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

pub fn engine_config(engine: &EngineProfile, data_root: &Path, contrib: crate::EngineContrib) -> EngineConfig {
    EngineConfig {
        scene: engine.scene.clone(),
        scenes: engine.scenes.clone(),
        model: engine.model.clone(),
        max_tokens: engine.max_tokens,
        session_cap: engine.session_cap,
        session_idle_timeout_secs: engine.session_idle_timeout_secs,
        permission_prompt_timeout_secs: engine.permission_prompt_timeout_secs,
        data_root: data_root.to_path_buf(),
        cwd: None,
        contrib,
    }
}

/// The `harnix daemon` command line for these settings. Read back by
/// [`crate::DaemonArgs`] on the other side.
fn daemon_args(engine: &EngineProfile, data_root: &Path, socket: &Path) -> Vec<String> {
    let mut argv = vec![
        "daemon".to_string(),
        "--data-root".into(),
        data_root.display().to_string(),
        "--socket".into(),
        socket.display().to_string(),
        "--scene".into(),
        engine.scene.clone(),
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
    if !engine.scenes.is_empty() {
        argv.push("--scenes".into());
        argv.push(engine.scenes.join(","));
    }
    argv
}

fn scenes_of(engine: &EngineProfile) -> Vec<String> {
    let mut all: Vec<String> = std::iter::once(engine.scene.clone())
        .chain(engine.scenes.iter().cloned())
        .collect();
    all.sort();
    all.dedup();
    all
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
