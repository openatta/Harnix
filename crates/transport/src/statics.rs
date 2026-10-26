//! The static face: the interface itself, and the modules installed packages
//! contribute to it.
//!
//! **The interface ships inside the binary.** One artifact to install, and no
//! way to end up running a page and a backend that disagree — they are
//! compiled together. `--ui-dir` replaces it with a directory on disk, which
//! is how a different interface becomes a flag instead of a rebuild (§5.1),
//! and how the front end is worked on: edit, reload, no recompile.
//! `--headless` serves neither.
//!
//! A package's modules are **never** embedded, and could not be: a package is
//! installed while the process runs, so its files exist only on disk. That is
//! why the two are separate branches here rather than one lookup with a
//! fallback — they answer from different places for different reasons.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use include_dir::{include_dir, Dir};

use crate::AppState;

/// The interface, as built (`ui/dist`, or the application's own build; see `build.rs`). Compiled in whole;
/// `build.rs` refuses to build without it rather than embed an empty interface.
static INTERFACE: Dir<'_> = include_dir!("$HARNIX_UI_DIST");

fn embedded(path: &str) -> Option<&'static [u8]> {
    INTERFACE.get_file(path).map(|f| f.contents())
}

/// No external sources and no inline anything.
///
/// A package's interface module is a same-origin module, so this does not
/// have to be loosened for one to load — and it must not be: going out is a
/// capability to declare and have disclosed, not one to get by running in a
/// browser. `'self'` is about the **origin**, not about where the bytes were
/// stored, so embedding the interface changes nothing here: a page served
/// from the binary and a module served from the plugin cache come from the
/// same host and port.
///
/// The one inline thing is the page's import map (design §10.5), which a browser accepts only
/// inline. It is allowed by its own hash and nothing else ([`page_csp`]); no `'unsafe-inline'`,
/// no `'unsafe-eval'`. The token travels in a `<meta>` tag.
pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
     connect-src 'self'; img-src 'self' data:; font-src 'self' data:; \
     base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// Where the product's `[brand] icon` is served.
pub const BRAND_ICON_PATH: &str = "/brand/icon.svg";

/// The page's CSP: [`CSP`], with the page's inline import map allowed by its `sha256`.
///
/// Worked out from the page being served rather than written in at build time, so an interface
/// served from `--ui-dir` is covered the same as the one compiled in. A page with no import map
/// gets the plain policy.
pub fn page_csp(page: &str) -> String {
    let Some(map) = import_map(page) else {
        return CSP.to_string();
    };
    use base64::Engine as _;
    use sha2::Digest as _;
    let hash =
        base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(map.as_bytes()));
    CSP.replacen(
        "script-src 'self'",
        &format!("script-src 'self' 'sha256-{hash}'"),
        1,
    )
}

/// The text of the page's `<script type="importmap">`, exactly as the browser hashes it.
fn import_map(page: &str) -> Option<&str> {
    let open = page.find("<script type=\"importmap\">")?;
    let body = open + "<script type=\"importmap\">".len();
    let close = page[body..].find("</script>")?;
    Some(&page[body..body + close])
}

/// Where the interface comes from.
#[derive(Debug, Clone)]
pub enum Face {
    /// The one compiled in. The default, and the only one that cannot be
    /// out of step with this binary.
    Embedded,
    /// A directory on disk, replacing it whole.
    Directory(PathBuf),
    /// None at all — a pure RPC node.
    Headless,
}

pub struct StaticFace {
    face: Face,
    /// Each installed package's directory, by name.
    ///
    /// Served from a **same-origin** path, which is what makes a runtime
    /// `import()` possible. Resolved through this map rather than by joining
    /// a path, so a request can only reach a directory the engine actually
    /// installed.
    packages: RwLock<BTreeMap<String, PathBuf>>,
    /// The product's `[brand] icon`, served at [`BRAND_ICON_PATH`].
    brand_icon: RwLock<Option<PathBuf>>,
}

impl StaticFace {
    pub fn new(face: Face) -> Self {
        Self {
            face,
            packages: RwLock::new(BTreeMap::new()),
            brand_icon: RwLock::new(None),
        }
    }

    pub fn set_brand_icon(&self, icon: Option<PathBuf>) {
        *self.brand_icon.write().unwrap() = icon;
    }

    /// Replace what is being served. Called at assembly and again after an
    /// install, because a package that needed a restart to appear is a
    /// package nobody will believe installed.
    pub fn set_packages(&self, packages: BTreeMap<String, PathBuf>) {
        *self.packages.write().unwrap() = packages;
    }

    pub fn serves_anything(&self) -> bool {
        !matches!(self.face, Face::Headless)
    }

    /// Write the embedded interface out.
    ///
    /// For the deployment where something else serves it — a CDN, or a proxy
    /// with its own static root. One installed artifact still, and the files
    /// when they are wanted, rather than a second thing to download and keep
    /// in step.
    pub fn export(to: &Path) -> std::io::Result<usize> {
        fn write(dir: &Dir<'_>, to: &Path, count: &mut usize) -> std::io::Result<()> {
            for file in dir.files() {
                let path = to.join(file.path());
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, file.contents())?;
                *count += 1;
            }
            for sub in dir.dirs() {
                write(sub, to, count)?;
            }
            Ok(())
        }
        let mut count = 0;
        std::fs::create_dir_all(to)?;
        write(&INTERFACE, to, &mut count)?;
        Ok(count)
    }

    /// Resolve a request path.
    ///
    /// Anything containing `..` falls back to the index rather than being
    /// resolved: a deep link that reloads must not land on an error page, and
    /// a traversal must not land anywhere at all.
    fn read(&self, path: &str) -> Option<(Vec<u8>, &'static str)> {
        if matches!(self.face, Face::Headless) {
            return None;
        }
        let clean = path.trim_start_matches('/');
        if clean.is_empty() || clean.contains("..") {
            return self
                .interface("index.html")
                .map(|b| (b, "text/html; charset=utf-8"));
        }

        if clean == BRAND_ICON_PATH.trim_start_matches('/') {
            let icon = self.brand_icon.read().unwrap().clone()?;
            return read_file(&icon).map(|b| (b, "image/svg+xml"));
        }

        // `/plugins/<name>/ui/<file>` — always from disk. A package is
        // installed while this process runs, so its files were never here to
        // embed.
        if let Some(rest) = clean.strip_prefix("plugins/") {
            let (name, file) = rest.split_once("/ui/")?;
            if file.contains("..") || name.contains('/') {
                return None;
            }
            let dir = self.packages.read().unwrap().get(name).cloned()?;
            return read_file(&dir.join("ui").join(file)).map(|b| (b, content_type(file)));
        }

        match self.interface(clean) {
            Some(bytes) => Some((bytes, content_type(clean))),
            // Client-side routing: an unknown path is a route, not a 404.
            None => self
                .interface("index.html")
                .map(|b| (b, "text/html; charset=utf-8")),
        }
    }

    /// One interface file, from wherever this deployment's interface lives.
    fn interface(&self, path: &str) -> Option<Vec<u8>> {
        match &self.face {
            Face::Embedded => embedded(path).map(<[u8]>::to_vec),
            Face::Directory(root) => read_file(&root.join(path)),
            Face::Headless => None,
        }
    }
}

fn read_file(path: &Path) -> Option<Vec<u8>> {
    path.is_file().then(|| std::fs::read(path).ok()).flatten()
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "map" => "application/json; charset=utf-8",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "txt" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

pub async fn serve(
    State(state): State<AppState>,
    uri: Uri,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some((bytes, mime)) = state.statics.read(uri.path()) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    // The index is the only response the connection token is injected into.
    // The page itself does not need one — a cross-origin page cannot read the
    // body — and the token governs connecting, not reading.
    if mime.starts_with("text/html") {
        // Under launch links the page carries no token: the
        // session cookie stands in for it at connect time, and a request
        // without one is told how to get one instead of being handed a page
        // that cannot connect.
        let injected = match state.config.browser {
            crate::BrowserAdmission::PageToken => state.config.token.as_str(),
            crate::BrowserAdmission::LaunchLink => {
                if !state.config.links.admits(&headers) {
                    tracing::debug!(
                        path = uri.path(),
                        has_cookie = headers.contains_key(axum::http::header::COOKIE),
                        "page refused: no launch-link session"
                    );
                    return crate::launch::refusal(
                        StatusCode::UNAUTHORIZED,
                        "Open this interface through a launch link. \
                         The process that started it can make one.",
                    );
                }
                ""
            }
        };
        let page = String::from_utf8_lossy(&bytes).replace("__HARNIX_TOKEN__", injected);
        let csp = page_csp(&page);
        return (
            [
                (header::CONTENT_TYPE, mime.to_string()),
                (header::CONTENT_SECURITY_POLICY, csp),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ],
            page,
        )
            .into_response();
    }
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CONTENT_SECURITY_POLICY, CSP),
        ],
        bytes,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The import map is allowed by its own hash, and nothing else is loosened.
    #[test]
    fn the_import_map_is_allowed_by_its_hash_alone() {
        let page = r#"<head><script type="importmap">{"imports":{"react":"/shared/react.js"}}</script></head>"#;
        let csp = page_csp(page);
        assert!(csp.contains("script-src 'self' 'sha256-"), "{csp}");
        assert!(!csp.contains("unsafe"), "{csp}");
        // The hash is of the text between the tags, as the browser computes it.
        use base64::Engine as _;
        use sha2::Digest as _;
        let expected = base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(
            br#"{"imports":{"react":"/shared/react.js"}}"#,
        ));
        assert!(csp.contains(&expected), "{csp}");
    }

    #[test]
    fn a_page_without_one_keeps_the_plain_policy() {
        assert_eq!(page_csp("<head></head>"), CSP);
    }
}
