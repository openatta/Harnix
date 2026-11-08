//! What a person is shown before an application is installed or updated, and what consenting to
//! it means.
//!
//! The disclosure is read from the package itself — its manifest, its parts, its signatures —
//! never from the catalog, which only lists offers. Every capability is a line; for an update, the
//! lines that are new since the installed version are listed apart (`added`), and only those ask
//! for consent again: an update that asks for nothing more goes ahead.
//!
//! Consent is a digest: of the package's bytes, the trust its signatures give it, and the version
//! it replaces. The first `harnix.apps.install` answers the disclosure and the digest; the second
//! passes the digest back. A package that changed in between — another download, another
//! signature, another installed version — has another digest, and is disclosed again rather than
//! installed on a consent given to something else.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::manifest::Manifest;
use crate::package::Inspected;
use crate::signature::{sha256, Verified};

/// One capability line: `{capability, value}`, with `from` when a ceiling grew.
fn line(capability: &str, value: impl Into<Value>) -> Value {
    json!({"capability": capability, "value": value.into()})
}

/// Every line a manifest asks for, in a fixed order; `agent`, whether the package has an agent
/// package (the instances it goes into are a line each).
pub fn lines(manifest: &Manifest, agent: bool) -> Vec<Value> {
    let caps = &manifest.capabilities;
    let mut out = Vec::new();
    out.extend(caps.net.iter().map(|h| line("net", h.as_str())));
    out.extend(caps.env.iter().map(|e| line("env", e.as_str())));
    if caps.storage_mb > 0 {
        out.push(line("storage_mb", caps.storage_mb));
    }
    out.extend(caps.hub.iter().map(|m| line("hub", m.as_str())));
    out.extend(caps.fs.iter().map(|p| line("fs", p.as_str())));
    // The interface modules run in the page, with its rights: each is a line of its own.
    out.extend(
        manifest
            .ui
            .iter()
            .map(|u| line("ui", format!("{} ({})", u.point, u.module))),
    );
    if agent {
        out.extend(
            manifest
                .agent
                .install_into
                .iter()
                .map(|i| line("agent", i.as_str())),
        );
    }
    out
}

/// The lines `next` asks for that `previous` did not: a host, a variable, a method, a path, an
/// interface module or an instance more; a storage ceiling that grew.
pub fn added(previous: (&Manifest, bool), next: (&Manifest, bool)) -> Vec<Value> {
    let key = |v: &Value| format!("{}={}", v["capability"], v["value"]);
    let before: BTreeSet<String> = lines(previous.0, previous.1)
        .iter()
        .filter(|l| l["capability"] != "storage_mb")
        .map(key)
        .collect();
    let mut out: Vec<Value> = lines(next.0, next.1)
        .into_iter()
        .filter(|l| l["capability"] != "storage_mb" && !before.contains(&key(l)))
        .collect();
    let (old, new) = (
        previous.0.capabilities.storage_mb,
        next.0.capabilities.storage_mb,
    );
    if new > old {
        out.push(json!({"capability": "storage_mb", "value": new, "from": old}));
    }
    out
}

/// The disclosure of a package that read and verified; `previous`, the manifest of the version it
/// would replace, and whether that one had an agent package.
pub fn disclose(
    inspected: &Inspected,
    verified: &Verified,
    previous: Option<(&Manifest, bool)>,
) -> Value {
    let manifest = &inspected.manifest;
    let agent = inspected.agent.is_some();
    let app = &manifest.app;
    let caps = &manifest.capabilities;
    let mut disclosure = json!({
        "id": app.id,
        "version": app.version,
        "title": app.title,
        "publisher": app.publisher,
        "description": app.description,
        "trust": verified.trust.as_str(),
        "publisher_key": verified.publisher_key.map(|k| k.hex()),
        "capabilities": {
            "net": caps.net, "env": caps.env, "storage_mb": caps.storage_mb, "hub": caps.hub, "fs": caps.fs,
        },
        "lines": lines(manifest, agent),
        "component": manifest.component.as_ref().map(|c| json!({
            "world": c.world, "memory_mb": c.memory_mb, "timeout_ms": c.timeout_ms,
        })),
        "ui": manifest.ui.iter().map(|u| json!({"point": u.point, "module": u.module})).collect::<Vec<_>>(),
        // Until interfaces run isolated, every one runs in the page, whoever signed it.
        "ui_in_page": !manifest.ui.is_empty(),
        "agent": inspected.agent.as_ref().map(|a| json!({
            "package": a.name, "version": a.version, "install_into": manifest.agent.install_into,
        })),
        "skills": inspected.skills,
    });
    if let Some(previous) = previous {
        let added = added(previous, (manifest, agent));
        disclosure["update"] = json!({
            "from": previous.0.app.version,
            "added": added,
            "needs_consent": !added.is_empty(),
        });
    }
    disclosure
}

/// What consenting to a package is (see the module's docs).
pub fn digest(package: &[u8], verified: &Verified, replaces: Option<&str>) -> String {
    sha256(
        format!(
            "harnix-app-consent@1\n{}\n{}\n{}\n",
            sha256(package),
            verified.trust,
            replaces.unwrap_or("-")
        )
        .as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(caps: &str, ui: bool) -> Manifest {
        Manifest::parse(&format!(
            r#"
[app]
id = "acme.notes"
version = "1.0.0"
harnix_api = 1
publisher = "acme"

[capabilities]
{caps}
{}
"#,
            if ui {
                "[[ui]]\npoint = \"home.card\"\nmodule = \"card.js\"\napi_version = 2\n"
            } else {
                ""
            }
        ))
        .unwrap()
    }

    #[test]
    fn an_update_lists_only_what_it_asks_for_more() {
        let before = manifest(
            "net = [\"a.example.com\"]\nstorage_mb = 2\nfs = [\"${data}\"]",
            false,
        );
        let same = manifest(
            "fs = [\"${data}\"]\nnet = [\"a.example.com\"]\nstorage_mb = 2",
            false,
        );
        assert!(added((&before, false), (&same, false)).is_empty());
        // Asking for less is not asking.
        let less = manifest("storage_mb = 1", false);
        assert!(added((&before, false), (&less, false)).is_empty());

        let more = manifest(
            "net = [\"a.example.com\", \"b.example.com\"]\nenv = [\"TOKEN\"]\nstorage_mb = 8\nfs = [\"${data}\"]",
            true,
        );
        assert_eq!(
            added((&before, false), (&more, true)),
            [
                json!({"capability": "net", "value": "b.example.com"}),
                json!({"capability": "env", "value": "TOKEN"}),
                json!({"capability": "ui", "value": "home.card (card.js)"}),
                json!({"capability": "agent", "value": "system"}),
                json!({"capability": "storage_mb", "value": 8, "from": 2}),
            ]
        );
        // An agent package that was there before is not asked for again.
        assert!(added((&more, true), (&more, true)).is_empty());
    }

    #[test]
    fn every_capability_is_a_line() {
        let m = manifest("net = [\"h\"]\nenv = [\"E\"]\nstorage_mb = 3\nhub = [\"harnix.sessions\"]\nfs = [\"${project}\"]", true);
        let kinds: Vec<String> = lines(&m, true)
            .iter()
            .map(|l| l["capability"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            kinds,
            ["net", "env", "storage_mb", "hub", "fs", "ui", "agent"]
        );
    }
}
