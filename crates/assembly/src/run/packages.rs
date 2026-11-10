//! The interface modules the page is told about: the installed packages' and the running
//! applications'.

use std::sync::Arc;

use serde_json::json;

use crate::kernel_rows::UiModules;

/// Serve what these packages contribute, and tell clients about it.
///
/// With `[ui] runtime_modules = false` nothing is served and nothing is told: a package is then
/// the engine's alone, and its interface modules are not even reachable over HTTP.
pub(super) fn apply_packages(
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
    ui_modules.set_packages(modules);
}
