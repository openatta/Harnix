//! What the `apps` feature adds to starting a process: the runtime applications run in.
//!
//! Every step of a start that the feature touches asks [`AppsWiring`], which is the one place it
//! is compiled in or out. Without the feature it is empty and every method does nothing, so the
//! steps read the same either way.

#[cfg(feature = "apps")]
use std::collections::BTreeMap;
use std::sync::Arc;

use harnix_authz::MethodTable;
use harnix_hub::Hub;
use harnix_instance::{EngineContrib, Factory, Profile};
use serde_json::Value;

use super::packages::UiModules;
use crate::AppBuilder;

/// The topic a running application's tool calls report their progress on (`host.progress`).
#[cfg(feature = "apps")]
pub(super) const APPS_PROGRESS_TOPIC: &str = "harnix.apps.progress";

/// The application runtime, when this build has one and the profile asks for it, and what the
/// steps of a start connect to it.
#[derive(Clone, Default)]
pub(super) struct AppsWiring {
    #[cfg(feature = "apps")]
    runtime: Option<Arc<harnix_apps::Runtime>>,
    /// The Agix plugin that gives a session the applications' tools: one, for every engine built
    /// in this process (what `Runtime::plugin` makes).
    #[cfg(feature = "apps")]
    plugin: Option<Arc<harnix_apps::AppsPlugin>>,
    /// Every spawned daemon's MCP servers, which follow what is running: set when the factory is.
    #[cfg(feature = "apps")]
    servers: Option<Arc<harnix_instance::SharedServers>>,
}

impl AppsWiring {
    /// Made before anything is registered: the `apps` module drives the runtime, and the
    /// instances built after carry its plugin.
    #[cfg_attr(not(feature = "apps"), allow(unused_variables))]
    pub fn new(profile: &Profile, app: &AppBuilder) -> anyhow::Result<Self> {
        #[cfg(feature = "apps")]
        if profile.apps.enabled && app.builtins {
            let runtime = harnix_apps::Runtime::new(harnix_apps::Options {
                dev: profile.apps.dev,
                config: profile.apps.config.clone(),
                secrets: profile.apps.secrets.clone(),
                keys: signing_keys(&profile.apps, app.store_keys.as_deref())?,
                store: profile.apps.store.clone(),
                idle: app.app_idle,
            })?;
            return Ok(Self {
                plugin: Some(Arc::new(harnix_apps::AppsPlugin::new(runtime.registry()))),
                runtime: Some(runtime),
                servers: None,
            });
        }
        Ok(Self::default())
    }

    /// The `apps` module, driving the runtime when there is one.
    pub fn apps_module(
        &self,
        module: harnix_builtin::apps::AppsModule,
    ) -> harnix_builtin::apps::AppsModule {
        #[cfg(feature = "apps")]
        if let Some(runtime) = &self.runtime {
            return module.runtime(runtime.clone());
        }
        module
    }

    /// A running application's tool calls, step by step, as their components report them.
    pub fn allow(&self, table: MethodTable) -> MethodTable {
        #[cfg(feature = "apps")]
        if self.runtime.is_some() {
            return table.allow(&harnix_authz::topic(APPS_PROGRESS_TOPIC), harnix_authz::Reach::Device);
        }
        table
    }

    /// Every engine built in this process gives its sessions the running applications' tools; a
    /// spawned daemon reaches them over MCP instead (`factory.servers`), with a credential of its
    /// own that the runtime issues and revokes.
    #[cfg_attr(not(feature = "apps"), allow(unused_variables))]
    pub fn factory(&mut self, factory: &mut Factory) {
        #[cfg(feature = "apps")]
        if let (Some(runtime), Some(plugin)) = (&self.runtime, &self.plugin) {
            factory.plugins.push(plugin.clone());
            factory.admitted_tools.push(harnix_apps::TOOL_PATTERN.to_string());
            factory.servers.issue_with(Arc::new(DaemonTokens(runtime.registry())));
            self.servers = Some(factory.servers.clone());
        }
    }

    /// The system instance's engine, which is built in this process: the same tools.
    #[cfg_attr(not(feature = "apps"), allow(unused_variables))]
    pub fn engine(&self, contrib: &mut EngineContrib) {
        #[cfg(feature = "apps")]
        if let Some(plugin) = &self.plugin {
            contrib.plugin(plugin.clone()).admit_tools(harnix_apps::TOOL_PATTERN);
        }
    }

    /// What a store has to have compiled an application for, for this process to run it.
    #[cfg_attr(not(feature = "apps"), allow(unused_variables))]
    pub fn process(&self, process: &mut Value) {
        #[cfg(feature = "apps")]
        if let Some(runtime) = &self.runtime {
            process["apps_engine"] = serde_json::json!(runtime.engine().fingerprint());
        }
    }

    /// The progress topic, declared, and what the runtime reports published on it.
    #[cfg_attr(not(feature = "apps"), allow(unused_variables))]
    pub async fn hub(&self, hub: &Arc<Hub>) {
        #[cfg(feature = "apps")]
        if let Some(runtime) = &self.runtime {
            hub.declare_topic(APPS_PROGRESS_TOPIC, Vec::new()).await;
            let hub = hub.clone();
            runtime.on_progress(move |value| {
                let hub = hub.clone();
                tokio::spawn(async move { hub.publish(APPS_PROGRESS_TOPIC, serde_json::json!({}), value).await });
            });
        }
    }

    /// What answers `/apps/<id>/…` for an id no module took: the running applications' paths.
    #[cfg_attr(not(feature = "apps"), allow(unused_variables))]
    pub fn routes(&self, extensions: &mut harnix_transport::Extensions) {
        #[cfg(feature = "apps")]
        if let Some(runtime) = &self.runtime {
            extensions.runtime_apps = Some(harnix_apps::serve::router(runtime.registry()));
        }
    }

    /// A running application's interface modules go to the page with the packages' (under the
    /// same `[ui] runtime_modules`); its tools go to every daemon this process spawned, as an MCP
    /// endpoint each. Both follow what is running, from the start (the enabled applications were
    /// started with the `apps` module) to every enable, disable, install and uninstall.
    #[cfg_attr(not(feature = "apps"), allow(unused_variables))]
    pub fn follow(&self, hub: &Arc<Hub>, ui_modules: &Arc<UiModules>, runtime_modules: bool) {
        #[cfg(feature = "apps")]
        if let (Some(runtime), Some(servers)) = (&self.runtime, &self.servers) {
            let follow = {
                let runtime = Arc::downgrade(runtime);
                let hub = hub.clone();
                let ui_modules = ui_modules.clone();
                let servers = servers.clone();
                move || {
                    let Some(runtime) = runtime.upgrade() else { return };
                    if runtime_modules {
                        ui_modules.set_apps(&hub, runtime.ui_contributions());
                    }
                    servers.set(runtime.mcp_servers());
                }
            };
            follow();
            runtime.on_change(follow);
        }
    }

    /// Where this process listens, once it does.
    #[cfg_attr(not(feature = "apps"), allow(unused_variables))]
    pub fn listening(&self, url: &str) {
        #[cfg(feature = "apps")]
        if let Some(runtime) = &self.runtime {
            runtime.set_listening(url);
        }
    }
}

/// What issues and revokes the credential each daemon this process spawns presents at a runtime
/// application's MCP endpoint: the application runtime, which serves them.
#[cfg(feature = "apps")]
struct DaemonTokens(Arc<harnix_apps::Registry>);

#[cfg(feature = "apps")]
impl harnix_instance::ServerTokens for DaemonTokens {
    fn issue(&self) -> String {
        self.0.issue_daemon_token()
    }
    fn revoke(&self, token: &str) {
        self.0.revoke_daemon_token(token)
    }
}

/// What the application runtime verifies packages with: the store's key built in (or, for a test
/// application, the ones it was assembled with), and the profile's trusted publishers. A key that
/// does not read stops the start, naming it: a typo here would otherwise quietly trust nobody.
#[cfg(feature = "apps")]
fn signing_keys(
    apps: &harnix_instance::profile::AppsProfile,
    store: Option<&[String]>,
) -> anyhow::Result<harnix_apps::signature::Keys> {
    use harnix_apps::signature::PublicKey;
    let store = match store {
        Some(keys) => keys
            .iter()
            .map(|k| PublicKey::parse(k).map_err(anyhow::Error::msg))
            .collect::<anyhow::Result<Vec<_>>>()?,
        None => harnix_apps::store::store_keys(),
    };
    let mut trusted = BTreeMap::new();
    for (publisher, keys) in &apps.trusted_publishers {
        let keys = keys
            .iter()
            .map(|k| {
                PublicKey::parse(k)
                    .map_err(|e| anyhow::anyhow!("[apps.trusted_publishers] {publisher}: {e}"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        trusted.insert(publisher.clone(), keys);
    }
    Ok(harnix_apps::signature::Keys { store, trusted })
}
