//! What a package serves over HTTP, under `/ext/<id>/`: its carrier's route of its descriptor,
//! made here and given which packages the carrier runs ([`Hosted`]).
//!
//! `GET /ext/<id>/ui/<file>` — a running package's interface: the modules a trusted publisher's
//! runs in the page, same-origin, so the page can `import()` them; and the pages (and their
//! scripts, styles, images) the shell's isolated frames load (`harnix_package::surface`). Only
//! files of a package that is running, only under its `ui/`; like the page's own files, served
//! without a credential. Every one carries [`FRAME_CSP`]: a frame's page has no network, no
//! plugins, no nested frames, and loads only what its own package has.

use std::sync::Arc;

use harnix_ext::http::{full, status, Body, Method, Request, Response, RouteContext, RouteHandler, StatusCode};

use crate::registry::Loaded;

/// The policy every file under `/ext/<id>/ui/` is served with (it is a document's that matters: a
/// module the page imports runs under the page's own). What a frame's page may load is its
/// package's files and inline styles; it may not connect anywhere (`connect-src 'none'`: its data
/// comes over the shell's bridge), submit a form anywhere, frame anything, or be framed by
/// anything but this page. Its sandbox (`allow-scripts allow-forms`, no `allow-same-origin`) is the
/// shell's; together they make it an opaque origin that cannot read the page's token, storage or
/// cookies. (`allow-forms` is for a form's `submit` event, which never fires without it; where a
/// form may submit to is `form-action 'none'`.) Neither stops it navigating itself away; the
/// page's `frame-src 'self'` keeps it to this origin, and the shell empties a frame that does.
pub const FRAME_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data: blob:; font-src 'self'; connect-src 'none'; frame-src 'none'; \
     form-action 'none'; base-uri 'none'; frame-ancestors 'self'";

/// What the route needs of a carrier: which packages run.
pub trait Hosted: Send + Sync + 'static {
    /// The running package `id`.
    fn running(&self, id: &str) -> Option<Arc<Loaded>>;
}

/// The package a request under `/ext/<id>/` is for, while it runs: the extension the route was
/// declared by.
fn package_of(cx: &RouteContext, hosted: &dyn Hosted) -> Option<Arc<Loaded>> {
    hosted.running(cx.ext().id().as_str())
}

/// `/ext/<id>/ui/<file>`.
pub struct UiFiles<H>(Arc<H>);

impl<H: Hosted> UiFiles<H> {
    pub fn new(hosted: Arc<H>) -> Self {
        Self(hosted)
    }
}

#[async_trait::async_trait]
impl<H: Hosted> RouteHandler for UiFiles<H> {
    async fn handle(&self, cx: RouteContext, req: Request<Body>) -> Response<Body> {
        if req.method() != Method::GET && req.method() != Method::HEAD {
            return status(StatusCode::METHOD_NOT_ALLOWED, "read-only");
        }
        let Some(app) = package_of(&cx, &*self.0) else {
            return status(StatusCode::NOT_FOUND, "no such package is running");
        };
        let file = req.uri().path().trim_start_matches('/').strip_prefix("ui/").unwrap_or_default().to_string();
        if !harnix_package::manifest::safe_relative(&file) {
            return status(StatusCode::NOT_FOUND, "not found");
        }
        match tokio::fs::read(app.root.join("ui").join(&file)).await {
            Ok(bytes) => {
                let mut response = Response::new(full(bytes));
                let headers = response.headers_mut();
                for (name, value) in [
                    (http::header::CONTENT_TYPE, content_type(&file)),
                    (http::header::CACHE_CONTROL, "no-store"),
                    (http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                    (http::header::CONTENT_SECURITY_POLICY, FRAME_CSP),
                    (http::header::REFERRER_POLICY, "no-referrer"),
                ] {
                    headers.insert(name, http::HeaderValue::from_static(value));
                }
                response
            }
            Err(_) => status(StatusCode::NOT_FOUND, "not found"),
        }
    }
}

fn content_type(file: &str) -> &'static str {
    match file.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}
