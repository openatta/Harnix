//! An extension's own HTTP, under `/<its id>/<prefix>`.
//!
//! Same origin as the interface, admitted by the same token or device credential, and decided
//! in the route table as the extension's route, then audited. What it does not get is what
//! Harnix's channels promise — topology independence, `seq` catch-up: a route is for what those
//! do not fit, a streaming protocol of its own or a library that wants an HTTP shape.
//!
//! The types are `http` 1.x, so a route is not bound to a web framework's major version. With
//! the `axum` feature, [`axum::router`] serves an `axum::Router` — outside the stability promise,
//! because axum's major version is in it.

use std::path::{Component, Path, PathBuf};

use bytes::Bytes;
use http_body_util::BodyExt;

pub use ::http::{HeaderMap, Method, Request, Response, StatusCode};

use crate::context::ExtensionContext;

#[cfg(feature = "axum")]
pub mod axum;

/// What a body's errors are.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A request's or a response's body.
pub type Body = http_body_util::combinators::UnsyncBoxBody<Bytes, BoxError>;

/// A body of these bytes.
pub fn full(bytes: impl Into<Bytes>) -> Body {
    http_body_util::Full::new(bytes.into()).map_err(|never| match never {}).boxed_unsync()
}

/// An empty body.
pub fn empty() -> Body {
    http_body_util::Empty::new().map_err(|never| match never {}).boxed_unsync()
}

/// A plain-text response with this status.
pub fn status(code: StatusCode, text: &str) -> Response<Body> {
    let mut response = Response::new(full(text.to_string()));
    *response.status_mut() = code;
    response
}

/// What answers an extension's route. The request's path is what follows `/<its id>`: it
/// begins with the route's prefix.
#[async_trait::async_trait]
pub trait RouteHandler: Send + Sync + 'static {
    async fn handle(&self, cx: RouteContext, req: Request<Body>) -> Response<Body>;
}

/// Who a request is answered as: the extension, and the route it came in under.
#[derive(Clone)]
pub struct RouteContext {
    ext: ExtensionContext,
    prefix: String,
}

impl RouteContext {
    pub(crate) fn new(ext: ExtensionContext, prefix: &str) -> Self {
        Self { ext, prefix: prefix.to_string() }
    }

    pub fn ext(&self) -> &ExtensionContext {
        &self.ext
    }

    /// The prefix this route was declared under.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }
}

/// A directory, read-only: a `GET` or `HEAD` of a file in it, never a listing, never a path that
/// leaves it.
pub(crate) struct StaticDir {
    root: PathBuf,
}

impl StaticDir {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The file a request path names, if it stays inside the directory. The path is what follows
    /// the route's prefix.
    fn resolve(&self, path: &str) -> Option<PathBuf> {
        let relative = Path::new(path.trim_start_matches('/'));
        if relative.as_os_str().is_empty() {
            return None;
        }
        relative.components().all(|c| matches!(c, Component::Normal(_))).then(|| self.root.join(relative))
    }
}

#[async_trait::async_trait]
impl RouteHandler for StaticDir {
    async fn handle(&self, cx: RouteContext, req: Request<Body>) -> Response<Body> {
        if req.method() != Method::GET && req.method() != Method::HEAD {
            return status(StatusCode::METHOD_NOT_ALLOWED, "read-only");
        }
        let mount = format!("/{}", cx.prefix().trim_matches('/'));
        let path = req.uri().path();
        let within = path.strip_prefix(mount.as_str()).unwrap_or(path);
        let Some(file) = self.resolve(within) else {
            return status(StatusCode::NOT_FOUND, "not found");
        };
        match tokio::fs::read(&file).await {
            Ok(bytes) if file.is_file() => {
                let mut response = Response::new(full(bytes));
                if let Some(kind) = content_type(&file) {
                    response.headers_mut().insert(::http::header::CONTENT_TYPE, ::http::HeaderValue::from_static(kind));
                }
                response
            }
            _ => status(StatusCode::NOT_FOUND, "not found"),
        }
    }
}

/// The few types a page's files are.
fn content_type(path: &Path) -> Option<&'static str> {
    Some(match path.extension()?.to_str()? {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_static_directory_never_leaves_itself() {
        let dir = StaticDir::new(PathBuf::from("/srv/ui"));
        assert_eq!(dir.resolve("/index.html"), Some(PathBuf::from("/srv/ui/index.html")));
        assert_eq!(dir.resolve("a/b.js"), Some(PathBuf::from("/srv/ui/a/b.js")));
        assert_eq!(dir.resolve("/../secret"), None);
        assert_eq!(dir.resolve("a/../../x"), None);
        assert_eq!(dir.resolve("/"), None, "no listing");
    }
}
