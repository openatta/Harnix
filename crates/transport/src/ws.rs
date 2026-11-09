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

use axum::extract::ws::{close_code, CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use harnix_contract::{codes, RpcError, Topology};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::warn;

use crate::frame::{self, Malformed, Request};
use crate::handshake::{self, Refused};
use crate::session::Ended;
use crate::AppState;

/// In-flight requests per connection.
///
/// The engine caps one connection at 64 in flight and **refuses** past that
/// rather than queueing; every client shares the hub's one engine connection,
/// so each is held well below that. A long request also gets its own task, or
/// it would hold up the read loop.
///
/// Published in `hello` (`limits.max_in_flight`), and one request over it is
/// answered with `BUSY` rather than dropped: a dropped request is a promise
/// that never settles, with nothing to say why (ipdrun-01).
const MAX_IN_FLIGHT: usize = harnix_contract::MAX_IN_FLIGHT;

/// Notifications dropped for being over the in-flight cap, process-wide. A
/// notification has no id to answer, so the count in `harnix.diagnostics` is
/// the only trace of one.
static DROPPED_NOTIFICATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many notifications the single-duplex topology has dropped for being over
/// the in-flight cap since this process started.
pub fn dropped_notifications() -> u64 {
    DROPPED_NOTIFICATIONS.load(std::sync::atomic::Ordering::Relaxed)
}

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
    let token_ok = query
        .token
        .as_deref()
        .is_some_and(|t| harnix_authz::tokens_equal(t, &state.config.token));
    let admitted_by = if token_ok {
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
    // Bounded, like every queue a client has (`session::OUTBOUND_FRAMES`): replies wait for room,
    // frames from the hub do not — a full queue ends the client.
    let (tx, outbound) = mpsc::channel::<String>(crate::session::OUTBOUND_FRAMES);
    let (ws_sink, mut stream) = socket.split();
    tracing::debug!(
        conn = peer.conn,
        admitted_by = peer.admitted_by,
        origin = %peer.origin,
        user_agent = %peer.user_agent,
        "websocket upgraded"
    );
    // How the connection is closed when it is the server that closes it: a code and a reason the
    // client can act on, rather than the socket simply going.
    let (close_with, closing) = tokio::sync::oneshot::channel::<CloseFrame>();
    let pump = tokio::spawn(pump(ws_sink, outbound, closing, peer.conn));

    let mut connection = Connection {
        state,
        peer,
        tx,
        established: None,
        ended: None,
        permits: Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT)),
    };
    let why = connection.read(&mut stream).await;
    connection.end(why).await;
    match why.map(close_frame) {
        // Said, then given a moment to go out before the connection is dropped.
        Some(close) => {
            let _ = close_with.send(close);
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), pump).await;
        }
        None => pump.abort(),
    }
    // Only now: its sender is what kept the pump waiting for the close frame rather than finding
    // the queue closed and going without one.
    drop(connection);
}

/// Everything that goes down the socket, in order, until the queue closes or the server closes
/// the connection with a reason.
async fn pump(
    mut ws_sink: futures::stream::SplitSink<WebSocket, Message>,
    mut outbound: mpsc::Receiver<String>,
    mut closing: tokio::sync::oneshot::Receiver<CloseFrame>,
    conn: u64,
) {
    loop {
        let frame = tokio::select! {
            frame = outbound.recv() => frame,
            close = &mut closing => {
                if let Ok(close) = close {
                    let _ = ws_sink.send(Message::Close(Some(close))).await;
                }
                break;
            }
        };
        let Some(frame) = frame else { break };
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
}

/// One connection's side of the protocol: its handshake, once there is one, and its calls.
struct Connection {
    state: AppState,
    peer: Peer,
    /// What goes down the socket.
    tx: mpsc::Sender<String>,
    /// Nothing is established until the handshake. The credential is what every later channel of
    /// this subject would carry; in this topology there is only one channel, so it never travels
    /// — but the *identity* comes from it either way, which is what keeps `__client` meaning the
    /// same thing in both topologies.
    established: Option<(String, Arc<crate::session::Established>)>,
    /// Changes when the handshake is over for a reason of the server's: its device revoked, its
    /// queue full, the process stopping. Then this connection closes, saying which.
    ended: Option<tokio::sync::watch::Receiver<Option<Ended>>>,
    permits: Arc<tokio::sync::Semaphore>,
}

impl Connection {
    /// Read frames until the client goes (`None`) or the server ends the handshake (why).
    async fn read(&mut self, stream: &mut futures::stream::SplitStream<WebSocket>) -> Option<Ended> {
        loop {
            let message = tokio::select! {
                message = stream.next() => message,
                reason = ended_of(&mut self.ended) => return Some(reason),
            };
            let Some(Ok(message)) = message else { return None };
            let text = match message {
                Message::Text(t) => t,
                Message::Close(_) => return None,
                _ => continue,
            };
            // A bad frame is refused and the connection stays open: dropping it instead would
            // leave the client guessing at which frame did it.
            match frame::parse_request(text.as_bytes()) {
                Ok(request) if request.method == "harnix.handshake" => self.handshake(request).await,
                Ok(request) => self.call(request).await,
                Err(Malformed { id, error }) => self.send(frame::error(id, &error)).await,
            }
        }
    }

    async fn send(&self, frame: Value) {
        let _ = self.tx.send(frame::text(&frame)).await;
    }

    async fn refuse(&self, id: Value, message: &str) {
        self.send(frame::error(id, &RpcError::new(codes::HANDSHAKE_REFUSED, message)))
            .await;
    }

    async fn handshake(&mut self, request: Request) {
        if self.established.is_some() {
            return self
                .refuse(
                    request.id,
                    "already established; a negotiation held twice is one that \
                     will eventually produce two answers",
                )
                .await;
        }
        // The upgrade was admitted by the token or a launch-link cookie; the handshake carries
        // the token the same as the split topology's does.
        let mut body = request.params;
        if let Some(obj) = body.as_object_mut() {
            obj.insert("token".into(), json!(self.state.config.token));
        }
        // This connection's own sender, handed in: both faces are carried here, so frames go
        // straight down it. No queue to drain and no task to drain it.
        let opened = handshake::establish_from_body(
            &self.state,
            &body,
            Topology::SingleDuplex,
            "this connection carries the single-duplex topology; \
             the split-streams one is established at POST /handshake",
            Some(self.tx.clone()),
            &self.peer.user_agent,
        )
        .await;
        match opened {
            Ok(opened) => {
                let peer = &self.peer;
                tracing::info!(
                    conn = peer.conn,
                    client = opened.session.client.0,
                    admitted_by = peer.admitted_by,
                    origin = %peer.origin,
                    user_agent = %peer.user_agent,
                    "single-duplex channel established"
                );
                self.ended = Some(opened.session.ended());
                self.established = Some((opened.credential, opened.session));
                self.send(frame::ok(request.id, opened.answer)).await;
            }
            Err(Refused::NotAdmitted(reason)) => self.refuse(request.id, reason).await,
            Err(Refused::Negotiation(e)) => self.send(frame::error(request.id, &e)).await,
        }
    }

    async fn call(&self, mut request: Request) {
        // Nothing before the handshake. Version and topology have to be
        // settled before a call can mean anything.
        let Some((_, session)) = self.established.as_ref() else {
            return self.refuse(request.id, "call harnix.handshake first").await;
        };
        request.say_client(session.client);
        let Ok(permit) = self.permits.clone().try_acquire_owned() else {
            if request.is_notification {
                let n = DROPPED_NOTIFICATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                warn!(conn = self.peer.conn, method = %request.method, dropped = n, "client exceeded its in-flight budget; notification dropped");
            } else {
                self.send(frame::error(request.id, &busy())).await;
            }
            return;
        };
        // A long request gets its own task, or it would hold up the read loop.
        let gate = self.state.gate.clone();
        let subject = session.subject.clone();
        let reply = self.tx.clone();
        tokio::spawn(async move {
            let Request {
                id,
                is_notification,
                method,
                params,
            } = request;
            let outcome = crate::call_settled(gate.as_ref(), &subject, &method, params).await;
            if !is_notification {
                let frame = match outcome {
                    Ok(result) => frame::ok(id, result),
                    Err(e) => frame::error(id, &e),
                };
                let _ = reply.send(frame::text(&frame)).await;
            }
            drop(permit);
        });
    }

    /// The handshake this connection carried, ended. The connection's own sender lives on until
    /// it is dropped, so the reason it closes with still has a way out.
    async fn end(&self, why: Option<Ended>) {
        if let Some((credential, _)) = &self.established {
            self.state.sessions.end(self.state.hub.as_ref(), credential).await;
            tracing::info!(conn = self.peer.conn, ended = ?why, "single-duplex channel closed");
        }
    }
}

/// The reason, once the handshake this connection carries is over; never, before there is one.
async fn ended_of(ended: &mut Option<tokio::sync::watch::Receiver<Option<Ended>>>) -> Ended {
    let Some(rx) = ended else {
        return std::future::pending().await;
    };
    match rx.wait_for(Option::is_some).await {
        Ok(why) => why.unwrap_or(Ended::Closed),
        // The handshake's sender is held for as long as it is; gone is over.
        Err(_) => Ended::Closed,
    }
}

/// What the client is told when the server ends its connection.
fn close_frame(why: Ended) -> CloseFrame {
    let (code, reason) = match why {
        // Policy: this device may no longer connect. Reconnecting will be refused.
        Ended::Revoked => (close_code::POLICY, "this device was revoked"),
        // Try again: reconnecting catches up from the hub's buffer.
        Ended::Overflowed => (close_code::AGAIN, "too far behind; reconnect to catch up"),
        Ended::ShuttingDown => (close_code::AWAY, "the server is shutting down"),
        Ended::Closed => (close_code::NORMAL, "closed"),
    };
    CloseFrame {
        code,
        reason: reason.into(),
    }
}

/// The answer to one request over the in-flight cap.
fn busy() -> RpcError {
    RpcError {
        code: codes::BUSY,
        message: format!(
            "{MAX_IN_FLIGHT} requests in flight on this connection; retry when one answers"
        ),
        data: Some(json!({"max_in_flight": MAX_IN_FLIGHT})),
    }
}
