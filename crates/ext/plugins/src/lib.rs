//! Uploading a package, asking Agix to install it, and serving what it brings to the interface.
//!
//! **Harnix has no plugin system.** Writing a plugin for Harnix is writing a plugin for Agix:
//! its manifest, its capability gate, its sandbox, its disclosure, its lifecycle. Harnix neither
//! reads a package nor runs one — anything here that parsed a manifest would be a second truth
//! about what an extension may do, and the two would diverge. What is read is one section the
//! engine has no opinion about, `[[ui]]` ([`packages`]); what is executed is nothing a package
//! brought.
//!
//! What is left for a host to do is the one thing Agix deliberately does not: **receive a file.**
//! `engine.plugin.install` fetches from a URL it is given and has no upload channel, so a person with a
//! `.zip` on their laptop and a backend on another machine has no way across. That gap is
//! `plugins.upload`.
//!
//! ```text
//!   browser --- POST /upload -------> harnix            the bulk semantic
//!   browser --- plugins.install ----> harnix
//!                                     harnix --- engine.plugin.install{download_url:
//!                                              "file:///…"} ---> Agix
//! ```
//!
//! # Disclosure comes back, not first
//!
//! Agix installs and *then* returns what the package will put in front of the model. That is the
//! opposite order from disclose-then-decide, and it is the engine's order, so it is the one the
//! interface shows: installed, here is what it will say to the model, keep it or remove it.
//! Inventing a pre-install inspection on this side would mean reading the package here, which
//! is the thing this crate exists not to do.
//!
//! # A build can still be one nothing installs into
//!
//! A build made without the package layer answers `PLUGINS_DISABLED`, and these methods pass
//! that through unchanged rather than dressing it up as an empty list: "there is no plugin
//! subsystem here" and "no plugins are installed" are different facts, and a client that cannot
//! tell them apart shows the wrong screen.
//!
//! # What a package's interface is
//!
//! Every change to what is installed is followed by a re-read of it (`plugins.install`,
//! `uninstall`, `enable`, `disable`, `reload`; and once at start). The enabled packages' `[[ui]]`
//! modules are published to the page (`cx.ui()`), and their files are served from the package's
//! own unpacked directory, which the engine says where is: under `/<this id>/ui/<package>/<file>`,
//! and only from the package's `ui/`. With `[ext.plugins] ui_modules = false` nothing is
//! published and nothing is served: a package is then the engine's alone.

pub mod packages;

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

use harnix_ext::http::{full, status, Body, Method, Request, Response, RouteContext, RouteHandler, StatusCode};
use harnix_ext::ui::UiEntry;
use harnix_ext::{caps, Access, Call, Extension, ExtensionContext, Manifest, Registrar, Rpc, RpcError};
use serde::Deserialize;
use serde_json::{json, Value};

/// Ceiling on an uploaded package.
const MAX_PACKAGE_BYTES: usize = 64 * 1024 * 1024;

/// `[ext.plugins]`.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Whether installed packages' `[[ui]]` modules load. Off, none does.
    pub ui_modules: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { ui_modules: true }
    }
}

/// The enabled packages whose files are served: name → unpacked root.
type Served = Arc<RwLock<BTreeMap<String, PathBuf>>>;

#[derive(Default)]
pub struct Plugins {
    served: Served,
}

#[async_trait::async_trait]
impl Extension for Plugins {
    type Config = Config;

    fn manifest(&self) -> Manifest {
        Manifest::new("plugins", env!("CARGO_PKG_VERSION"))
            .title("Plugins")
            .disclosure("接收插件包文件，经 Agix 安装、卸载、启停；读包里的界面段，把它的界面模块交给页面")
            .requires([caps::ENGINE_PLUGINS])
    }

    fn register(&self, r: &mut Registrar<'_, Config>) {
        r.method("upload")
            .access(Access::device())
            .describe("a one-shot URL to send a package's .zip to")
            .raw(|call, params| async move { upload(&call, &params) });
        r.method("install")
            .access(Access::device())
            .describe("hand the engine an uploaded package to install")
            .raw({
                let served = self.served.clone();
                move |call, params| {
                    let served = served.clone();
                    async move { install(&call, &served, &params).await }
                }
            });
        for (name, engine) in [
            ("uninstall", "engine.plugin.uninstall"),
            ("enable", "engine.plugin.enable"),
            ("disable", "engine.plugin.disable"),
            ("reload", "engine.plugin.reload"),
        ] {
            let served = self.served.clone();
            r.method(name)
                .access(Access::device())
                .describe("a lifecycle call for the engine, then a re-read of what is installed")
                .raw(move |call, params| {
                    let served = served.clone();
                    async move {
                        let result = call.call(engine, params).await?;
                        refresh(call.ext(), &served).await;
                        Ok(result)
                    }
                });
        }
        r.method("list")
            .access(Access::device())
            .describe("what is installed, and what of it reaches the interface")
            .raw(|call, _| async move { Ok(list(&call).await) });
        r.route("ui", Access::device(), PackageFiles { served: self.served.clone() });
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        // What is installed already, resolved before anything is served: one that needed a
        // restart to appear is one nobody will believe installed.
        refresh(&cx, &self.served).await;
        Ok(())
    }
}

/// A one-shot URL to PUT a `.zip` at. The grant is spent on use.
fn upload(call: &Call, params: &Value) -> Rpc<Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params("missing name"))?;
    let grant = call.ext().bulk().grant(MAX_PACKAGE_BYTES)?;
    Ok(json!({
        "token": grant.token(),
        "url": format!("/upload?token={}", grant.token()),
        "path": grant.path().display().to_string(),
        "max_bytes": grant.max_bytes(),
        "name": name,
    }))
}

/// What is installed, and what of it reaches this side.
///
/// The engine's list, plus the one section it ignores. One call, because "what is installed" and
/// "what will appear in my interface" are the same question asked by the same screen.
async fn list(call: &Call) -> Value {
    match call.call("engine.plugin.list", json!({})).await {
        Ok(value) => json!({
            "plugins": value.get("plugins").cloned().unwrap_or(json!([])),
            "contributes": packages::read_all(&value),
            "available": true,
        }),
        // Not an empty list. "This build carries no plugin subsystem" and "nothing is installed"
        // are different facts, and a client that cannot tell them apart sends someone looking
        // for a package that could never load.
        Err(e) => json!({
            "plugins": [],
            "contributes": [],
            "available": false,
            "reason": e.message,
        }),
    }
}

/// Hand Agix a local path and let it do the rest.
///
/// The `checksum` is passed through when given. Agix requires one for `http(s)` sources and
/// allows it to be omitted for `file://`, and that rule is the engine's to enforce — repeating it
/// here would be a second copy of a policy that can change.
async fn install(call: &Call, served: &Served, params: &Value) -> Rpc<Value> {
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params("missing path"))?;
    let mut request = json!({
        "name": params.get("name").cloned().unwrap_or(Value::Null),
        "version": params.get("version").cloned().unwrap_or(Value::Null),
        "download_url": format!("file://{path}"),
    });
    for key in ["checksum", "scope"] {
        if let Some(value) = params.get(key) {
            request[key] = value.clone();
        }
    }
    let result = call.call("engine.plugin.install", request).await?;
    // The package is on disk now, so what it contributes is knowable now.
    refresh(call.ext(), served).await;
    Ok(result)
}

/// Re-read what is installed and tell the page, and serve what is enabled.
///
/// Driven by what the engine says is installed, so a package that the engine refused, disabled
/// or never had is not here either. Asking the engine rather than scanning a directory is the
/// difference between "installed" and "some files are lying around".
async fn refresh(cx: &ExtensionContext, served: &Served) {
    let contributions = match cx.call("engine.plugin.list", json!({})).await {
        Ok(listed) => packages::read_all(&listed),
        Err(e) => {
            // A build without the package layer answers `PLUGINS_DISABLED` and nothing is
            // installed, which is a fact rather than a failure.
            tracing::debug!(reason = %e.message, "no installed packages to read");
            Vec::new()
        }
    };
    let ui_modules = cx.config::<Config>().map(|c| c.ui_modules).unwrap_or(true);
    publish(cx, served, contributions, ui_modules);
}

/// Serve what these packages contribute, and tell the page about it.
fn publish(cx: &ExtensionContext, served: &Served, contributions: Vec<packages::Contributions>, ui_modules: bool) {
    let contributions = if ui_modules {
        contributions
    } else {
        if contributions.iter().any(|c| !c.ui.is_empty()) {
            tracing::info!("[ext.plugins] ui_modules = false: packages' interface modules are not loaded");
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
    let enabled = || contributions.iter().filter(|c| c.enabled);
    *served.write().unwrap_or_else(|e| e.into_inner()) =
        enabled().map(|c| (c.plugin.clone(), c.root.clone())).collect();
    let id = cx.id();
    cx.ui().publish(
        enabled()
            .flat_map(|c| {
                c.ui.iter().map(move |entry| {
                    UiEntry::new(
                        &entry.point,
                        &format!("/apps/{id}/ui/{}/{}", c.plugin, entry.module),
                        entry.api_version.unwrap_or(1),
                    )
                    .owner(&c.plugin)
                })
            })
            .collect(),
    );
}

/// A package's `ui/` directory, read-only: a `GET` of a file in it, never a listing, never a path
/// that leaves it.
struct PackageFiles {
    served: Served,
}

#[async_trait::async_trait]
impl RouteHandler for PackageFiles {
    async fn handle(&self, cx: RouteContext, req: Request<Body>) -> Response<Body> {
        if req.method() != Method::GET && req.method() != Method::HEAD {
            return status(StatusCode::METHOD_NOT_ALLOWED, "read-only");
        }
        let mount = format!("/{}", cx.prefix().trim_matches('/'));
        let path = req.uri().path();
        let within = path.strip_prefix(mount.as_str()).unwrap_or(path).trim_start_matches('/');
        let Some((name, file)) = within.split_once('/') else {
            return status(StatusCode::NOT_FOUND, "not found");
        };
        let root = self.served.read().unwrap_or_else(|e| e.into_inner()).get(name).cloned();
        let Some(root) = root else {
            return status(StatusCode::NOT_FOUND, "not found");
        };
        let Some(file_path) = contained(&root.join("ui"), file) else {
            return status(StatusCode::NOT_FOUND, "not found");
        };
        match tokio::fs::read(&file_path).await {
            Ok(bytes) => Response::builder()
                .header("content-type", content_type(file))
                .body(full(bytes))
                .unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR, "not served")),
            Err(_) => status(StatusCode::NOT_FOUND, "not found"),
        }
    }
}

/// `base` joined with a request's path, if what that names is a file inside `base`.
///
/// `Path::join` with an absolute path *replaces* the base: `/plugins/x/ui//etc/passwd` split at
/// `/ui/` leaves `/etc/passwd`, and joining that onto the package's directory read the system's
/// file. So the request's part may only be plain names (no root, no `..`, no `.`), and what it
/// resolves to, links followed, has to still be under `base`: a link inside a package pointing
/// out of it is the same escape.
fn contained(base: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    if rel.as_os_str().is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let base = base.canonicalize().ok()?;
    let path = base.join(rel).canonicalize().ok()?;
    (path.starts_with(&base) && path.is_file()).then_some(path)
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "html" => "text/html; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The request's part names a file inside the package's `ui/` and nowhere else: not an
    /// absolute path, not a parent, not a link out.
    #[test]
    fn a_file_is_inside_the_package_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("pkg/ui/sub")).unwrap();
        std::fs::write(root.join("pkg/ui/sub/index.js"), "export {}").unwrap();
        std::fs::write(root.join("secret"), "no").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("secret"), root.join("pkg/ui/link")).unwrap();
        let ui = root.join("pkg/ui");

        assert!(contained(&ui, "sub/index.js").is_some());
        let secret = root.join("secret");
        for rel in [
            secret.to_str().unwrap(),
            "/etc/passwd",
            "../../secret",
            "..",
            "%2e%2e/secret",
            "link",
            "sub",
            "",
        ] {
            assert!(contained(&ui, rel).is_none(), "`{rel}` was served");
        }
    }
}
