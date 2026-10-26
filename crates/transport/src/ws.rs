//! Topology one: a single bidirectional connection carrying all five
//! semantics.
//!
//! The edge default. Fewest connections, simplest ordering, and an
//! out-of-band answer has a return path for free — where a download-only
//! event stream would need a separate endpoint bolted on.
//!
//! Bulk still goes around this connection, always. That is not a property of
//! this topology; it is what stops one image from filling the frame ceiling
//! (§3.3.3, fourth invariant).

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use harnix_contract::{codes, RpcError, Subject, Topology};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::warn;

use crate::handshake;
use crate::AppState;

/// In-flight requests per connection.
///
/// The engine caps one connection at 64 in flight and **refuses** past that
/// rather than queueing; every client shares the hub's one engine connection,
/// so each is held well below that. A long request also gets its own task, or
/// it would hold up the read loop.
const MAX_IN_FLIGHT: usize = 16;

#[derive(serde::Deserialize)]
pub struct TokenQuery {
    token: Option<String>,
}

pub async fn upgrade(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !crate::origin_allowed(&headers, &state.config.extra_origins) {
        warn!("refused a WebSocket upgrade from a non-loopback Origin");
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    // The token gates the upgrade; on a reachable listener the device
    // signature is checked in the handshake below, where there is a frame to
    // carry it. A query parameter is all a browser's WebSocket API can send.
    //
    // Or a launch-link session cookie, which a browser sends on
    // a same-site upgrade by itself — the page under launch links has no
    // token to put in the query.
    let admitted_by = if query.token.as_deref() == Some(state.config.token.as_str()) {
        "token"
    } else if state.config.links.admits(&headers) {
        "cookie"
    } else {
        // A refused upgrade is otherwise invisible server-side.
        tracing::debug!(
            origin = %header_text(&headers, axum::http::header::ORIGIN),
            has_cookie = headers.contains_key(axum::http::header::COOKIE),
            "websocket upgrade refused: neither token nor launch-link session"
        );
        return (StatusCode::UNAUTHORIZED, "bad token").into_response();
    };
    // Who is connecting, said once per connection: which
    // admission it passed and what it says it is. A desktop shell's web view
    // and a script client are otherwise indistinguishable in the log.
    let peer = Peer {
        conn: NEXT_CONN.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        admitted_by,
        origin: header_text(&headers, axum::http::header::ORIGIN),
        user_agent: header_text(&headers, axum::http::header::USER_AGENT),
    };
    if !state.config.topologies.contains(&Topology::SingleDuplex) {
        return (
            StatusCode::CONFLICT,
            format!(
                "this deployment does not carry the single-duplex topology; it offers {}",
                state
                    .config
                    .topologies
                    .iter()
                    .map(Topology::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
            .into_response();
    }
    // Above Harnix's own ceiling on purpose. The WebSocket library's default
    // frame cap is 16 MiB — the same size — so a frame one byte over the
    // published limit was killed at the protocol layer and the connection
    // simply died: no error, no reason, and a client left to guess which
    // frame did it. Raised so the check below is the one that answers.
    upgrade
        .max_message_size(harnix_contract::MAX_FRAME_BYTES + 1024 * 1024)
        .max_frame_size(harnix_contract::MAX_FRAME_BYTES + 1024 * 1024)
        .on_upgrade(move |socket| connection(socket, state, peer))
}

/// Connection numbers, for the log only.
static NEXT_CONN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// What the log says about one connection.
struct Peer {
    conn: u64,
    admitted_by: &'static str,
    origin: String,
    user_agent: String,
}

fn header_text(headers: &HeaderMap, name: axum::http::header::HeaderName) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string()
}

/// The method of an outbound notification, or `reply` for an
/// answer. Read only when the frame is going to be logged.
fn frame_kind(frame: &str) -> &str {
    frame
        .split_once(r#""method":""#)
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(method, _)| method)
        .unwrap_or("reply")
}

async fn connection(socket: WebSocket, state: AppState, peer: Peer) {
    let (tx, mut outbound) = mpsc::unbounded_channel::<String>();
    let (mut ws_sink, mut stream) = socket.split();
    let conn = peer.conn;
    tracing::debug!(
        conn,
        admitted_by = peer.admitted_by,
        origin = %peer.origin,
        user_agent = %peer.user_agent,
        "websocket upgraded"
    );

    let pump = tokio::spawn(async move {
        while let Some(frame) = outbound.recv().await {
            if tracing::enabled!(tracing::Level::TRACE) {
                tracing::trace!(
                    conn,
                    kind = frame_kind(&frame),
                    bytes = frame.len(),
                    "frame out"
                );
            }
            if ws_sink.send(Message::Text(frame.into())).await.is_err() {
                break;
            }
        }
        let _ = ws_sink.close().await;
    });

    // Nothing is established until the handshake. `credential` is what every
    // later channel of this subject would carry; in this topology there is
    // only one channel, so it never travels — but the *identity* comes from
    // it either way, which is what keeps `__client` meaning the same thing in
    // both topologies.
    let mut established: Option<(String, Arc<crate::session::Established>)> = None;
    let permits = Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT));

    while let Some(Ok(message)) = stream.next().await {
        let text = match message {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };
        // Refused with the ceiling in the message, and the connection stays
        // open. Letting it through would mean `hello` publishes a limit that
        // nothing checks; dropping the connection instead would leave the
        // client guessing at which frame did it.
        if text.len() > harnix_contract::MAX_FRAME_BYTES {
            let id = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("id").cloned())
                .unwrap_or(Value::Null);
            let _ = tx.send(error_frame(
                id,
                codes::INVALID_REQUEST,
                format!(
                    "frame is {} bytes; the ceiling is {} — use the bulk channel",
                    text.len(),
                    harnix_contract::MAX_FRAME_BYTES
                ),
            ));
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            let _ = tx.send(error_frame(Value::Null, codes::PARSE_ERROR, "invalid JSON"));
            continue;
        };
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        let is_notification = value.get("id").is_none();
        let Some(method) = value
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            let _ = tx.send(error_frame(id, codes::INVALID_REQUEST, "missing method"));
            continue;
        };
        let mut params = value.get("params").cloned().unwrap_or(json!({}));

        if method == "harnix.handshake" {
            if established.is_some() {
                let _ = tx.send(error_frame(
                    id,
                    codes::HANDSHAKE_REFUSED,
                    "already established; a negotiation held twice is one that \
                     will eventually produce two answers",
                ));
                continue;
            }
            let mut body = params.clone();
            if let Some(obj) = body.as_object_mut() {
                obj.insert("token".into(), json!(state.config.token));
            }
            let device = match crate::admission::admit(
                state.config.admission,
                &state.config.token,
                &state.devices,
                &state.challenges,
                &body,
            )
            .await
            {
                crate::admission::Verdict::Admit { device } => device,
                crate::admission::Verdict::Refuse(reason) => {
                    let _ = tx.send(error_frame(id, codes::HANDSHAKE_REFUSED, reason));
                    continue;
                }
            };
            let subject = Subject::Device {
                id: device.unwrap_or_else(|| "local".into()),
            };
            match handshake::negotiate(&params, &state.config.topologies, subject) {
                Ok(negotiated) if negotiated.topology != Topology::SingleDuplex => {
                    let _ = tx.send(error_frame(
                        id,
                        codes::HANDSHAKE_REFUSED,
                        "this connection carries the single-duplex topology; \
                         the split-streams one is established at POST /handshake",
                    ));
                }
                Ok(negotiated) => {
                    // This connection's own sender, handed in: both faces are
                    // carried here, so frames go straight down it. No queue to
                    // drain and no task to drain it.
                    let (credential, session, sink) = state
                        .sessions
                        .establish(
                            &state.hub,
                            negotiated.subject.clone(),
                            negotiated.topology,
                            Some(tx.clone()),
                        )
                        .await;
                    let _ = sink;
                    let mut body = handshake::accepted(&negotiated, &state.config.topologies);
                    body["client"] = json!(session.client.0);
                    // What a module route (`/apps/<id>/`) is presented with:
                    // the same credential the split topology hands out.
                    body["credential"] = json!(credential);
                    tracing::info!(
                        conn,
                        client = session.client.0,
                        admitted_by = peer.admitted_by,
                        origin = %peer.origin,
                        user_agent = %peer.user_agent,
                        "single-duplex channel established"
                    );
                    established = Some((credential, session));
                    let _ = tx.send(ok_frame(id, body));
                }
                Err(e) => {
                    let _ = tx.send(error_frame(id, e.code, e.message));
                }
            }
            continue;
        }

        // Nothing before the handshake. Version and topology have to be
        // settled before a call can mean anything.
        let Some((_, session)) = established.as_ref() else {
            let _ = tx.send(error_frame(
                id,
                codes::HANDSHAKE_REFUSED,
                "call harnix.handshake first",
            ));
            continue;
        };
        let subject = session.subject.clone();

        // Which client this is, from the credential the handshake produced —
        // not from this socket. In a topology where the call and the frames
        // it registers a watcher for arrive on different connections, socket
        // identity is the wrong answer; taking it from the credential makes
        // both topologies mean the same thing here (§3.3.3, item 2).
        if let Some(obj) = params.as_object_mut() {
            obj.insert("__client".into(), json!(session.client.0));
        }

        let Ok(permit) = permits.clone().try_acquire_owned() else {
            warn!("client exceeded its in-flight budget; frame dropped");
            continue;
        };
        let gate = state.gate.clone();
        let reply = tx.clone();
        tokio::spawn(async move {
            let outcome = gate.call(&subject, &method, params).await;
            if !is_notification {
                let _ = reply.send(match outcome {
                    Ok(result) => ok_frame(id, result),
                    Err(RpcError { code, message, .. }) => error_frame(id, code, message),
                });
            }
            drop(permit);
        });
    }

    if let Some((credential, _)) = established {
        state.sessions.end(&state.hub, &credential).await;
        tracing::info!(conn, "single-duplex channel closed");
    }
    pump.abort();
}

fn ok_frame(id: Value, result: Value) -> String {
    serde_json::to_string(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
        .unwrap_or_default()
}

fn error_frame(id: Value, code: i32, message: impl Into<String>) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0", "id": id,
        "error": {"code": code, "message": message.into()},
    }))
    .unwrap_or_default()
}
