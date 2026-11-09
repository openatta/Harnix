//! What the runtime serves over HTTP, under `/apps/<id>/` on the process's own listener:
//!
//! - `GET /apps/<id>/ui/<file>` — a running application's interface: the modules a trusted
//!   publisher's runs in the page, same-origin, so the page can `import()` them like an installed
//!   package's (`/plugins/<name>/ui/…`); and the pages (and their scripts, styles, images) the
//!   shell's isolated frames load (`harnix_app_package::surface`). Only files of an application that is
//!   running, only under its `ui/`; like the page's own files, served without a credential. Every
//!   one carries [`FRAME_CSP`]: a frame's page has no network, no plugins, no nested frames, and
//!   loads only what its own package has.
//! - `POST /apps/<id>/mcp` — the application's tools for a daemon this process spawned (design
//!   §5.1): a daemon is another process and has no VM, so its sessions call the tools here, and
//!   they run in this process's VM. The smallest MCP server Agix's client (streamable HTTP) needs:
//!   `initialize`, `ping`, `tools/list`, `tools/call`, answered as JSON; notifications accepted;
//!   no server-sent stream (`GET` is 405). Admission is a peer on this machine — a daemon this
//!   process spawned is one, and the listener may face a network — and then the application's own
//!   token (`Authorization: Bearer …`), minted when it starts, gone when this process exits, and
//!   written only into the settings of daemons this process spawned.
//!
//! The daemon's engine asks the person before each call, as for any MCP tool. What a call here
//! does not get, against one from an in-process session: the calling session's project (no
//! `/project`), progress, and the application's `events` (they are the in-process sessions').

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::Extension;
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use harnix_app_port::{Failure, ToolCall, ToolReply};
use serde_json::{json, Value};

use crate::runtime::{Loaded, Registry};

/// The policy every file under `/apps/<id>/ui/` is served with (it is a document's that matters:
/// a module the page imports runs under the page's own). What a frame's page may load is its
/// package's files and inline styles; it may not connect anywhere (`connect-src 'none'`: its data
/// comes over the shell's bridge), submit a form anywhere, frame anything, or be framed by
/// anything but this page. Its sandbox (`allow-scripts allow-forms`, no `allow-same-origin`) is the
/// shell's; together they make it an opaque origin that cannot read the page's token, storage or
/// cookies. (`allow-forms` is for a form's `submit` event, which never fires without it; where a
/// form may submit to is `form-action 'none'`.)
/// Neither stops it navigating itself away; the page's `frame-src 'self'` keeps it to this origin,
/// and the shell empties a frame that does.
pub const FRAME_CSP: &str =
    "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data: blob:; font-src 'self'; connect-src 'none'; frame-src 'none'; \
     form-action 'none'; base-uri 'none'; frame-ancestors 'self'";

/// Protocol versions this server answers `initialize` with: the client's, when it is one of these.
const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Routed by reading the path rather than by the router's own captures: this router is mounted
/// under the listener's `/apps/{*rest}`, and a capture there would be handed to these handlers too.
pub fn router(registry: Arc<Registry>) -> Router {
    Router::new().fallback(dispatch).with_state(registry)
}

async fn dispatch(
    State(registry): State<Arc<Registry>>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().strip_prefix("/apps/").unwrap_or_default();
    let Some((id, rest)) = path.split_once('/') else {
        return (StatusCode::NOT_FOUND, "no application serves this path").into_response();
    };
    let (id, rest) = (id.to_string(), rest.to_string());
    match (rest.as_str(), &method) {
        // Its tools, for a daemon this process spawned: on this machine, always. Not a listener's
        // peer from elsewhere, token or no token — a token is something that can leak, and being
        // on the other side of a network is not.
        ("mcp", _) if !on_this_machine(peer.as_ref().map(|Extension(ConnectInfo(addr))| *addr)) => (
            StatusCode::FORBIDDEN,
            "an application's tools are served to this machine only",
        )
            .into_response(),
        ("mcp", &Method::POST) => mcp(registry, id, headers, body).await,
        ("mcp", _) => no_stream().await,
        (file, &Method::GET) if file.starts_with("ui/") => {
            ui(registry, id, file["ui/".len()..].to_string()).await
        }
        _ => (StatusCode::NOT_FOUND, "no application serves this path").into_response(),
    }
}

async fn ui(registry: Arc<Registry>, id: String, file: String) -> Response {
    let Some(app) = registry.get(&id) else {
        return (StatusCode::NOT_FOUND, "no such application is running").into_response();
    };
    if !harnix_app_package::manifest::safe_relative(&file) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let path = app.root.join("ui").join(&file);
    match std::fs::read(&path) {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, content_type(&file)),
                (header::CACHE_CONTROL, "no-store"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::CONTENT_SECURITY_POLICY, FRAME_CSP),
                (header::REFERRER_POLICY, "no-referrer"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// Whether a request came from this machine. Unknown is not: a listener that does not say who
/// its peer is serves nobody the tools.
fn on_this_machine(peer: Option<SocketAddr>) -> bool {
    peer.is_some_and(|addr| addr.ip().to_canonical().is_loopback())
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

async fn no_stream() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        "this endpoint answers POSTs only",
    )
        .into_response()
}

fn authorized(app: &Loaded, headers: &HeaderMap) -> bool {
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    // Not a timing oracle worth defending against on loopback, but it costs nothing to compare
    // every byte.
    let (a, b) = (given.as_bytes(), app.token.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn answer(id: Value, result: Result<Value, (i64, String)>) -> Response {
    let body = match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    };
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

async fn mcp(registry: Arc<Registry>, id: String, headers: HeaderMap, body: Bytes) -> Response {
    let Some(app) = registry.get(&id) else {
        return (StatusCode::NOT_FOUND, "no such application is running").into_response();
    };
    if !authorized(&app, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            "this endpoint needs the application's token",
        )
            .into_response();
    }
    let message: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return answer(Value::Null, Err((-32700, format!("not JSON: {e}")))),
    };
    if message.is_array() {
        return answer(
            Value::Null,
            Err((-32600, "batches are not supported".into())),
        );
    }
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    // A notification, or a response to something this server never asks: nothing to answer.
    let Some(rpc_id) = message.get("id").cloned().filter(|_| !method.is_empty()) else {
        return StatusCode::ACCEPTED.into_response();
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let asked = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let version = PROTOCOL_VERSIONS
                .iter()
                .find(|v| **v == asked)
                .unwrap_or(&PROTOCOL_VERSIONS[1]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": format!("harnix-app:{}", app.id), "version": app.version},
                "instructions": format!(
                    "The tools of the Harnix application `{}`{}.",
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
        "tools/call" => call(&registry, &app, &params).await,
        other => Err((-32601, format!("`{other}` is not served here"))),
    };
    answer(rpc_id, result)
}

async fn call(registry: &Registry, app: &Loaded, params: &Value) -> Result<Value, (i64, String)> {
    let control = match registry.control() {
        Some(control) if app.component => control,
        _ => return Err((-32601, format!("`{}` has no tools", app.id))),
    };
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "missing name".to_string()))?;
    if !app.tools.iter().any(|t| t.name == name) {
        return Err((-32602, format!("`{}` has no tool `{name}`", app.id)));
    }
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let call = ToolCall {
        app: app.id.clone(),
        tool: name.to_string(),
        input: arguments,
        call_id: uuid::Uuid::new_v4().simple().to_string(),
        project: None,
        progress: false,
    };
    // A client that goes away drops this future, and the call's instance with it.
    match control.call_tool(call).await {
        Ok(ToolReply::Output {
            content,
            structured,
            is_error,
        }) => {
            let mut result = json!({
                "content": [{"type": "text", "text": content}],
                "isError": is_error,
            });
            if let Some(structured) = structured {
                result["structuredContent"] = structured;
            }
            Ok(result)
        }
        Ok(ToolReply::Failed {
            failure: Failure::Cancelled,
            ..
        }) => Err((-32800, "cancelled".into())),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_machine_reaches_the_tools() {
        let at = |s: &str| Some(s.parse::<SocketAddr>().unwrap());
        assert!(on_this_machine(at("127.0.0.1:4000")));
        assert!(on_this_machine(at("[::1]:4000")));
        assert!(on_this_machine(at("[::ffff:127.0.0.1]:4000")));
        assert!(!on_this_machine(at("192.168.1.20:4000")));
        assert!(!on_this_machine(at("[2001:db8::1]:4000")));
        assert!(!on_this_machine(None), "a peer nobody named is not this machine");
    }
}
