//! Topology two: unary and out-of-band over ordinary HTTP, each subscription
//! face on its own download-only stream.
//!
//! For a deployment behind a reverse proxy. A WebSocket is opaque to
//! everything a proxy is good at — per-request rate limiting, caching, access
//! logs, an authenticating front door — because after the upgrade the proxy
//! sees one long connection and nothing about what travels inside it. Putting
//! unary back on plain HTTP buys all of that with no code.
//!
//! It also lets the two faces disconnect independently, and lets a read-only
//! client open only the stream it wants: watching the sidebar should not mean
//! receiving every session's body.
//!
//! ```text
//!   POST /handshake            once — version, topology, and a credential
//!   POST /rpc                  unary
//!   POST /respond              out-of-band answers
//!   GET  /events/session       server → client
//!   GET  /events/host          server → client
//!   POST /upload               bulk, and identical in every topology
//! ```
//!
//! **The methods, params, error codes and event shapes are the same as the
//! other topology's.** Switching topology is not switching protocol; that is
//! the whole claim, and `tests/topology-parity.mjs` is what checks it.

use std::convert::Infallible;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::Stream;
use harnix_contract::{codes, RpcError, Subject, Topology};
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use crate::session::Face;
use crate::{handshake, AppState};

#[derive(serde::Deserialize)]
pub struct Credential {
    /// Also accepted as the `Authorization: Bearer …` header; a query
    /// parameter is what an `EventSource` can send, because that API cannot
    /// set headers.
    token: Option<String>,
}

fn credential_of(headers: &HeaderMap, query: &Credential) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        .or_else(|| query.token.clone())
}

/// One handshake, before any channel is opened.
pub async fn establish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Response {
    if !crate::origin_allowed(&headers, &state.config.extra_origins) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    // A launch-link session cookie stands in for the token. The
    // device half of admission (off loopback) is untouched by it.
    if state.config.links.admits(&headers) {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("token".into(), json!(state.config.token));
        }
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
            return (StatusCode::UNAUTHORIZED, reason).into_response()
        }
    };
    let subject = Subject::Device {
        id: device.unwrap_or_else(|| harnix_contract::LOCAL_DEVICE.into()),
    };
    let negotiated = match handshake::negotiate(&body, &state.config.topologies, subject) {
        Ok(negotiated) => negotiated,
        Err(e) => return rpc_error(e).into_response(),
    };
    if negotiated.topology != Topology::SplitStreams {
        return rpc_error(RpcError::new(
            codes::HANDSHAKE_REFUSED,
            "this endpoint establishes the split-streams topology; \
             the single-duplex one is established on its own connection",
        ))
        .into_response();
    }
    // Neither face is subscribed yet: in this topology a client opts into
    // each stream by opening it, which is what makes a read-only client
    // possible at all.
    let (credential, established) = state
        .sessions
        .establish(
            state.hub.as_ref(),
            negotiated.subject.clone(),
            negotiated.topology,
            None,
            headers
                .get(axum::http::header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
        )
        .await;
    let mut answer = handshake::accepted(&negotiated, &state.config.topologies);
    answer["credential"] = json!(credential);
    answer["client"] = json!(established.client.0);
    Json(answer).into_response()
}

/// One request, one reply.
pub async fn rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Credential>,
    raw: Bytes,
) -> Response {
    // Taken as bytes and parsed here rather than extracted as `Json`, so the
    // two answers a client can get for a bad frame are the same on both
    // topologies. The extractor's own refusals are plain text with an HTTP
    // status — "failed to buffer the request body" — where the bidirectional
    // topology answers a JSON-RPC error naming the ceiling. One semantic, two
    // topologies: a limit that reads differently depending on which one is
    // carrying it is the semantics leaking into the choice (§3.3).
    if raw.len() > harnix_contract::MAX_FRAME_BYTES {
        return rpc_error(RpcError::new(
            codes::INVALID_REQUEST,
            format!(
                "frame is {} bytes; the ceiling is {} — use the bulk channel",
                raw.len(),
                harnix_contract::MAX_FRAME_BYTES
            ),
        ))
        .into_response();
    }
    let Ok(mut body) = serde_json::from_slice::<Value>(&raw) else {
        return rpc_error(RpcError::new(codes::PARSE_ERROR, "invalid JSON")).into_response();
    };
    // 401 with the JSON-RPC error as its body: a credential that is gone —
    // never issued, idle too long, its device revoked — is a matter of
    // admission, and a client reads the reason the same way either way.
    let Some(credential) = credential_of(&headers, &query) else {
        return unauthorized(RpcError::new(codes::HANDSHAKE_REFUSED, "no credential"));
    };
    let Some(established) = state.sessions.get(&credential) else {
        return unauthorized(RpcError::new(
            codes::HANDSHAKE_REFUSED,
            "unknown credential; call /handshake first",
        ));
    };
    let Some(method) = body
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return rpc_error(RpcError::new(codes::INVALID_REQUEST, "missing method")).into_response();
    };
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let mut params = body
        .get_mut("params")
        .map(Value::take)
        .unwrap_or_else(|| json!({}));

    // Which client this is, from the **credential** — not from this
    // connection. `harnix.attach` arrives here while the frames it registers a
    // watcher for go out on a different socket entirely, so connection
    // identity would be the wrong answer, and quietly (§3.3.3, item 2).
    if let Some(obj) = params.as_object_mut() {
        obj.insert("__client".into(), json!(established.client.0));
    }

    match crate::call_settled(state.gate.as_ref(), &established.subject, &method, params).await {
        Ok(result) => Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response(),
        Err(e) => Json(json!({"jsonrpc": "2.0", "id": id, "error": e})).into_response(),
    }
}

/// Answering a question the server asked.
///
/// A separate endpoint because a download-only stream has no return path.
/// Treating it as its own semantic is what lets every topology implement it
/// hoharnixly instead of one of them growing a patch (§3.3.1).
pub async fn respond(
    state: State<AppState>,
    headers: HeaderMap,
    query: Query<Credential>,
    raw: Bytes,
) -> Response {
    rpc(state, headers, query, raw).await
}

/// This client's session events.
pub async fn session_events(
    state: State<AppState>,
    query: Query<Credential>,
    headers: HeaderMap,
) -> Response {
    stream_for(state, headers, query, Face::Session).await
}

/// Everything not tied to a session.
pub async fn host_events(
    state: State<AppState>,
    query: Query<Credential>,
    headers: HeaderMap,
) -> Response {
    stream_for(state, headers, query, Face::Host).await
}

async fn stream_for(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Credential>,
    face: Face,
) -> Response {
    let Some(credential) = credential_of(&headers, &query) else {
        return (StatusCode::UNAUTHORIZED, "no credential").into_response();
    };
    // Opening the stream *is* the subscription: this is where a read-only
    // client says which face it wants. One reader per face: opening it again
    // — the browser's own reconnect — replaces the one before, whose stream
    // ends, rather than splitting frames between two readers.
    let (Some(established), Some(receiver)) =
        (state.sessions.get(&credential), state.sessions.open(&credential, face))
    else {
        return (StatusCode::UNAUTHORIZED, "unknown credential").into_response();
    };
    // When the stream goes, its handshake is still in use as of now: the
    // idle clock starts here, not at the last `/rpc`.
    let open = Touch(established);
    let stream = ReceiverStream::new(receiver).map(move |payload| {
        let _ = &open;
        Ok::<Event, Infallible>(Event::default().data(payload))
    });
    sse(stream)
}

/// Marks its handshake used when it is dropped, with the stream it rides in.
struct Touch(std::sync::Arc<crate::session::Established>);

impl Drop for Touch {
    fn drop(&mut self) {
        self.0.touch();
    }
}

fn sse<S>(stream: S) -> Response
where
    S: Stream<Item = Result<Event, Infallible>> + Send + 'static,
{
    // A comment every 15s: an idle proxy that times out a quiet connection is
    // the ordinary way a download-only stream dies without either end
    // noticing.
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn unauthorized(e: RpcError) -> Response {
    (StatusCode::UNAUTHORIZED, rpc_error(e)).into_response()
}

fn rpc_error(e: RpcError) -> Json<Value> {
    Json(
        json!({"jsonrpc": "2.0", "id": Value::Null, "error": {"code": e.code, "message": e.message}}),
    )
}
