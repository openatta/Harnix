//! The interface modules the page is told about: the installed packages' and the running
//! applications'.

use std::sync::Arc;

use harnix_contract::sync::lock;
use harnix_hub::Hub;
use serde_json::json;

/// Serve what these packages contribute, and tell clients about it.
///
/// Spawned rather than awaited, because this is called from a callback that
/// cannot be async — the alternative was making the whole notification path
/// async for the sake of two assignments.
///
/// With `[ui] runtime_modules = false` nothing is served and nothing is told: a package is then
/// the engine's alone, and its interface modules are not even reachable over HTTP.
pub(super) fn apply_packages(
    hub: &Arc<Hub>,
    statics: &Arc<harnix_transport::StaticFace>,
    contributions: Vec<harnix_builtin::packages::Contributions>,
    runtime_modules: bool,
    ui_modules: &Arc<UiModules>,
) {
    let contributions = if runtime_modules {
        contributions
    } else {
        if contributions.iter().any(|c| !c.ui.is_empty()) {
            tracing::info!(
                "[ui] runtime_modules = false: packages' interface modules are not loaded"
            );
        }
        Vec::new()
    };
    if !contributions.is_empty() {
        tracing::info!(
            packages = contributions.len(),
            ui = contributions.iter().map(|c| c.ui.len()).sum::<usize>(),
            "packages contribute to the interface"
        );
    }
    statics.set_packages(
        contributions
            .iter()
            .filter(|c| c.enabled)
            .map(|c| (c.plugin.clone(), c.root.clone()))
            .collect(),
    );
    let modules: Vec<serde_json::Value> = contributions
        .iter()
        .filter(|c| c.enabled)
        .flat_map(|c| {
            c.ui.iter().map(|entry| {
                json!({
                    "plugin": c.plugin,
                    "point": entry.point,
                    "api_version": entry.api_version,
                    "module": format!("/plugins/{}/ui/{}", c.plugin, entry.module),
                })
            })
        })
        .collect();
    ui_modules.set_packages(hub, modules);
}

/// The interface modules the page is told at hello: the installed packages', and the running
/// applications'. Two sources that change apart, told as one list.
#[derive(Default)]
pub(super) struct UiModules {
    lists: std::sync::Mutex<(Vec<serde_json::Value>, Vec<serde_json::Value>)>,
}

impl UiModules {
    pub(super) fn set_packages(self: &Arc<Self>, hub: &Arc<Hub>, modules: Vec<serde_json::Value>) {
        lock(&self.lists).0 = modules;
        self.publish(hub);
    }

    #[cfg_attr(not(feature = "apps"), allow(dead_code))]
    pub(super) fn set_apps(self: &Arc<Self>, hub: &Arc<Hub>, modules: Vec<serde_json::Value>) {
        lock(&self.lists).1 = modules;
        self.publish(hub);
    }

    /// Spawned rather than awaited: this is called from callbacks that cannot be async. The lists
    /// are read when the task runs, so whichever task runs last sets the latest of both.
    fn publish(self: &Arc<Self>, hub: &Arc<Hub>) {
        let (hub, this) = (hub.clone(), self.clone());
        tokio::spawn(async move {
            let all: Vec<serde_json::Value> = {
                let lists = lock(&this.lists);
                lists.0.iter().chain(&lists.1).cloned().collect()
            };
            hub.set_ui_contributions(json!(all)).await;
        });
    }
}
