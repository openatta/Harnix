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
use harnix_contract::{codes, RpcError, Topology};
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use crate::frame::{self, Malformed};
use crate::handshake::{self, Refused};
use crate::session::Face;
use crate::AppState;

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
    // Neither face is subscribed yet: in this topology a client opts into
    // each stream by opening it, which is what makes a read-only client
    // possible at all.
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let opened = handshake::establish_from_body(
        &state,
        &body,
        Topology::SplitStreams,
        "this endpoint establishes the split-streams topology; \
         the single-duplex one is established on its own connection",
        None,
        user_agent,
    )
    .await;
    match opened {
        Ok(opened) => Json(opened.answer).into_response(),
        Err(Refused::NotAdmitted(reason)) => (StatusCode::UNAUTHORIZED, reason).into_response(),
        Err(Refused::Negotiation(e)) => Json(frame::error(Value::Null, &e)).into_response(),
    }
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
    let mut request = match frame::parse_request(&raw) {
        Ok(request) => request,
        Err(Malformed { id, error }) => return Json(frame::error(id, &error)).into_response(),
    };
    // 401 with the JSON-RPC error as its body: a credential that is gone —
    // never issued, idle too long, its device revoked — is a matter of
    // admission, and a client reads the reason the same way either way.
    let Some(credential) = credential_of(&headers, &query) else {
        return unauthorized(request.id, RpcError::new(codes::HANDSHAKE_REFUSED, "no credential"));
    };
    let Some(established) = state.sessions.get(&credential) else {
        return unauthorized(
            request.id,
            RpcError::new(codes::HANDSHAKE_REFUSED, "unknown credential; call /handshake first"),
        );
    };
    // `harnix.attach` arrives here while the frames it registers a watcher
    // for go out on a different socket entirely.
    request.say_client(established.client);

    let id = request.id;
    match crate::call_settled(state.gate.as_ref(), &established.subject, &request.method, request.params).await {
        Ok(result) => Json(frame::ok(id, result)).into_response(),
        Err(e) => Json(frame::error(id, &e)).into_response(),
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

fn unauthorized(id: Value, e: RpcError) -> Response {
    (StatusCode::UNAUTHORIZED, Json(frame::error(id, &e))).into_response()
}
