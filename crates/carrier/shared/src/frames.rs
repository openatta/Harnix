//! What the page is told of a running package's interface, through the component's `harnix.ui`
//! entries: a trusted publisher's `[[ui]]` modules, which the page imports; everyone else's (and a
//! trusted one that asked for it) as its surfaces, which the shell draws as isolated frames
//! (`harnix_package::surface`). A module that will not run is listed as such, with why, so the
//! page can say so in its diagnostics.

use harnix_package::manifest::Title;
use serde_json::{json, Value};

use crate::registry::Loaded;

/// Whether a running package's interface is drawn in frames rather than imported into the page.
pub fn frames(app: &Loaded) -> bool {
    harnix_package::surface::runs_in_frames(app.trust, &app.interface, !app.ui.is_empty())
}

/// What the page is told of one running package's interface.
pub fn ui_entries(app: &Loaded) -> Vec<Value> {
    let id = &app.id;
    if !frames(app) {
        return app
            .ui
            .iter()
            .map(|entry| {
                json!({
                    "plugin": id,
                    "point": entry.point,
                    "api_version": entry.api_version,
                    "module": format!("/ext/{id}/ui/{}", entry.module),
                })
            })
            .collect();
    }
    let interface = json!({
        "calls": app.interface.calls,
        "links": app.interface.links,
        "trust": app.trust.as_str(),
        "title": app.title,
        "spaces": app.backend_spaces,
    });
    // A module that runs in the page runs with the page's rights: not this package's.
    let refused = app.ui.iter().map(|entry| {
        json!({
            "plugin": id,
            "point": entry.point,
            "api_version": entry.api_version,
            "refused": entry.module,
            "reason": format!(
                "`{}` runs in the page, which only a trusted publisher's interface does ({} here); this \
                 package's interface is its isolated surfaces",
                entry.module,
                app.trust.as_str()
            ),
        })
    });
    let surfaces = app.surfaces.iter().map(|s| {
        // Where a space's sessions go: its own backend space when the package declares one by
        // that id, else the package's first.
        let sessions = s
            .space
            .as_ref()
            .and_then(|space| app.backend_spaces.iter().find(|b| *b == space).or(app.backend_spaces.first()).cloned());
        json!({
            "plugin": id,
            "point": s.point,
            "api_version": harnix_ext::CONTRIB_API_VERSION,
            "frame": s.page.as_ref().map(|page| format!("/ext/{id}/ui/{page}")),
            "surface": {
                "id": s.id(),
                "title": s.title.as_ref().map(title_json),
                "space": s.space,
                "sessions": sessions,
                "icon": s.icon,
                "order": s.order,
                "center": s.center,
                "kinds": s.kinds,
                "wide": s.wide,
            },
            "interface": interface,
        })
    });
    refused.chain(surfaces).collect()
}

fn title_json(title: &Title) -> Value {
    match title {
        Title::One(text) => json!({"en": text}),
        Title::ByLocale(by) => json!(by),
    }
}
