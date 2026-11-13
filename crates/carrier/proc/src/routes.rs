//! What a package serves over HTTP, under `/ext/<id>/` on the process's own listener: two routes
//! of its descriptor, both its carrier's.
//!
//! - `GET /ext/<id>/ui/<file>` — a running package's interface: the modules a trusted
//!   publisher's runs in the page, same-origin, so the page can `import()` them; and the pages (and
//!   their scripts, styles, images) the shell's isolated frames load (`harnix_package::surface`).
//!   Only files of a package that is running, only under its `ui/`; like the page's own files,
//!   served without a credential. Every one carries [`FRAME_CSP`]: a frame's page has no network,
//!   no plugins, no nested frames, and loads only what its own package has.
//! - `POST /ext/<id>/mcp` — the package's tools for a daemon this process spawned (concept §4.3):
//!   its sessions call the tools here, and they run in the package's own process, over its wire. The smallest MCP server Agix's client (streamable HTTP) needs:
//!   `initialize`, `ping`, `tools/list`, `tools/call`, answered as JSON; notifications accepted;
//!   no server-sent stream (`GET` is 405). Only an engine this process spawned reaches it: the
//!   transport admits its credential, from this machine, as `Subject::Engine`, and nothing else
//!   is let in here.
//!
//! The daemon's engine asks the person before each call, as for any MCP tool. What a call here
//! does not get, against one from an in-process session: the calling session's project (no
//! `/project`), progress, and the package's `events` (they are the in-process sessions').

use std::sync::Arc;

use harnix_ext::Subject;
use harnix_ext::http::{full, status, Body, Method, Request, Response, RouteContext, RouteHandler, StatusCode};
use harnix_wasm_port::socket::now_ms;
use harnix_wasm_port::{CallMeta, Failure, ToolCall, ToolReply};
use http_body_util::BodyExt;
use serde_json::{json, Value};

use crate::registry::{Loaded, Registry};

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

/// Protocol versions this server answers `initialize` with: the client's, when it is one of these.
const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The package a request under `/ext/<id>/` is for, while it runs: the extension the route was
/// declared by.
fn package_of(cx: &RouteContext, registry: &Registry) -> Option<Arc<Loaded>> {
    registry.get(cx.ext().id().as_str())
}

/// `/ext/<id>/ui/<file>`.
pub(crate) struct UiFiles(Arc<Registry>);

impl UiFiles {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self(registry)
    }
}

#[async_trait::async_trait]
impl RouteHandler for UiFiles {
    async fn handle(&self, cx: RouteContext, req: Request<Body>) -> Response<Body> {
        if req.method() != Method::GET && req.method() != Method::HEAD {
            return status(StatusCode::METHOD_NOT_ALLOWED, "read-only");
        }
        let Some(app) = package_of(&cx, &self.0) else {
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

/// `/ext/<id>/mcp`.
pub(crate) struct McpEndpoint(Arc<Registry>);

impl McpEndpoint {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self(registry)
    }
}

fn answer(id: Value, result: Result<Value, (i64, String)>) -> Response<Body> {
    let body = match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    };
    let mut response = Response::new(full(body.to_string()));
    response.headers_mut().insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("application/json"));
    response
}

#[async_trait::async_trait]
impl RouteHandler for McpEndpoint {
    async fn handle(&self, cx: RouteContext, req: Request<Body>) -> Response<Body> {
        // Its tools, for an engine this process spawned: the transport admitted its credential,
        // and only from this machine.
        if !matches!(req.extensions().get::<Subject>(), Some(Subject::Engine { .. })) {
            return status(StatusCode::UNAUTHORIZED, "this endpoint needs the credential of a daemon this process spawned");
        }
        if req.method() != Method::POST {
            return status(StatusCode::METHOD_NOT_ALLOWED, "this endpoint answers POSTs only");
        }
        let Some(app) = package_of(&cx, &self.0) else {
            return status(StatusCode::NOT_FOUND, "no such package is running");
        };
        let body = match req.into_body().collect().await {
            Ok(body) => body.to_bytes(),
            Err(e) => return answer(Value::Null, Err((-32700, format!("the body did not read: {e}")))),
        };
        let message: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return answer(Value::Null, Err((-32700, format!("not JSON: {e}")))),
        };
        if message.is_array() {
            return answer(Value::Null, Err((-32600, "batches are not supported".into())));
        }
        let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
        // A notification, or a response to something this server never asks: nothing to answer.
        let Some(rpc_id) = message.get("id").cloned().filter(|_| !method.is_empty()) else {
            let mut accepted = Response::new(harnix_ext::http::empty());
            *accepted.status_mut() = StatusCode::ACCEPTED;
            return accepted;
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
                let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).unwrap_or(&PROTOCOL_VERSIONS[1]);
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": format!("harnix-ext:{}", app.id), "version": app.version},
                    "instructions": format!(
                        "The tools of the Harnix package `{}`{}.",
                        app.id,
                        app.title.as_ref().map(|t| format!(" ({t})")).unwrap_or_default()
                    ),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": app.tools.iter().map(|t| {
                    let mut tool = json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": serde_json::from_str::<Value>(&t.input_schema).unwrap_or(json!({"type": "object"})),
                    });
                    if t.read_only {
                        tool["annotations"] = json!({"readOnlyHint": true});
                    }
                    tool
                }).collect::<Vec<_>>(),
            })),
            "tools/call" => call(&self.0, &app, &params).await,
            other => Err((-32601, format!("`{other}` is not served here"))),
        };
        answer(rpc_id, result)
    }
}

async fn call(registry: &Registry, app: &Loaded, params: &Value) -> Result<Value, (i64, String)> {
    let control = match registry.control(&app.id) {
        Some(control) if app.component => control,
        _ => return Err((-32601, format!("`{}` has no tools", app.id))),
    };
    let name = params.get("name").and_then(Value::as_str).ok_or((-32602, "missing name".to_string()))?;
    if !app.tools.iter().any(|t| t.name == name) {
        return Err((-32602, format!("`{}` has no tool `{name}`", app.id)));
    }
    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let call = ToolCall {
        app: app.id.clone(),
        tool: name.to_string(),
        input: arguments,
        call_id: uuid::Uuid::new_v4().simple().to_string(),
        project: None,
        progress: false,
    };
    // A client that goes away drops this future, and the wire takes the call back.
    let meta = CallMeta::default().with_deadline(Some(now_ms() + app.timeout.as_millis() as u64));
    match control.call_tool(meta, call).await {
        Ok(ToolReply::Output { content, structured, is_error }) => {
            let mut result = json!({"content": [{"type": "text", "text": content}], "isError": is_error});
            if let Some(structured) = structured {
                result["structuredContent"] = structured;
            }
            Ok(result)
        }
        Ok(ToolReply::Failed { failure: Failure::Cancelled, .. }) => Err((-32800, "cancelled".into())),
        Ok(ToolReply::Failed { message, .. }) => Ok(json!({
            "content": [{"type": "text", "text": format!("{}: {message}", app.id)}],
            "isError": true,
        })),
        Err(e) => Ok(json!({
            "content": [{"type": "text", "text": format!("{}: {}", app.id, e.message)}],
            "isError": true,
        })),
    }
}
