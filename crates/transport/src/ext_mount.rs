//! Extension routes, `/ext/<id>/…` (concept §3.3.5).
//!
//! An extension's own HTTP and WebSocket, served from this listener so they are
//! same-origin with the interface. Transport does not know what an extension
//! is: it is handed routers by id and a [`RouteDecider`] to ask, and does the
//! two things only it can — find out who the request is from, and refuse
//! before the extension's code runs.
//!
//! Who the request is from, in order: a credential from a handshake
//! (`Authorization: Bearer <credential>`, or `?credential=` where a browser
//! API cannot set a header — a WebSocket); the credential of an engine this
//! process spawned (`Subject::Engine`); on a loopback listener, the
//! per-process token the same two ways, or a launch-link session cookie.
//!
//! What is under `/ext/` and no native extension's is the packages', which
//! come and go while the process runs: handed whole to the router for them,
//! with whoever the request was admitted as in its extensions (none when it
//! carried no credential — a package's interface files are served to a frame,
//! which has none).

use std::sync::Arc;

use axum::extract::Request;
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use harnix_contract::Subject;
use harnix_kernel::RouteDecider;

use crate::{Admission, AppState};

/// One extension's router, to be served at `/ext/<id>/`.
pub struct ExtRoutes {
    pub id: String,
    pub router: axum::Router,
}

pub(crate) fn mount(
    mut router: axum::Router<AppState>,
    state: &AppState,
    routes: Vec<ExtRoutes>,
    packages: Option<axum::Router>,
) -> axum::Router<AppState> {
    for ext in routes {
        let guard_state = state.clone();
        let id = ext.id.clone();
        let guarded = ext.router.layer(axum::middleware::from_fn(move |req: Request, next: Next| {
            let state = guard_state.clone();
            let id = id.clone();
            async move { guard(state, id, req, next).await }
        }));
        router = router.nest_service(&format!("/ext/{}", ext.id), guarded);
    }
    // Anything else under `/ext/` is a package's, when this process runs them,
    // or an extension's that is not here — disabled, or never was — and must
    // not fall through to the interface's index.
    match packages {
        Some(packages) => {
            let admit_state = state.clone();
            let admitted = packages.layer(axum::middleware::from_fn(move |req: Request, next: Next| {
                let state = admit_state.clone();
                async move { admit(state, req, next).await }
            }));
            router.route_service("/ext/{*rest}", admitted)
        }
        None => router.route(
            "/ext/{*rest}",
            axum::routing::any(|| async { (StatusCode::NOT_FOUND, "no extension serves this path") }),
        ),
    }
}

/// A package's path: whoever it was admitted as goes with it, and the router for the packages
/// decides what that subject may have.
async fn admit(state: AppState, mut req: Request, next: Next) -> Response {
    if !crate::origin_allowed(req.headers(), &state.config.extra_origins) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    let query = req.uri().query().unwrap_or("").to_string();
    if let Some(subject) = subject_of(&state, req.headers(), &query).await {
        // An engine's credential is presented from this machine, or not at all: a token can leak,
        // and being on the other side of a network is not something a token says.
        let peer = req.extensions().get::<axum::extract::ConnectInfo<std::net::SocketAddr>>().map(|c| c.0);
        let local = peer.is_some_and(|addr| addr.ip().to_canonical().is_loopback());
        if local || !matches!(subject, Subject::Engine { .. }) {
            req.extensions_mut().insert(subject);
        }
    }
    next.run(req).await
}

async fn guard(state: AppState, ext: String, req: Request, next: Next) -> Response {
    let headers = req.headers();
    if !crate::origin_allowed(headers, &state.config.extra_origins) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    let query = req.uri().query().unwrap_or("").to_string();
    let Some(subject) = subject_of(&state, headers, &query).await else {
        return (StatusCode::UNAUTHORIZED, "no credential").into_response();
    };
    let Some(gate) = state.route_gate.as_ref() else {
        return (StatusCode::NOT_FOUND, "no extension routes here").into_response();
    };
    if let Err(e) = gate.decide_route(&subject, &ext, req.uri().path()) {
        let status = if e.code == harnix_contract::codes::REFUSED {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::NOT_FOUND
        };
        return (status, e.message).into_response();
    }
    next.run(req).await
}

async fn subject_of(state: &AppState, headers: &HeaderMap, query: &str) -> Option<Subject> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    let param = |name: &str| {
        query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == name).then(|| v.to_string())
        })
    };
    for credential in [bearer.clone(), param("credential")].into_iter().flatten() {
        if let Some(established) = state.sessions.get(&credential) {
            return Some(established.subject.clone());
        }
    }
    if let Some(engine) = bearer.as_deref().and_then(|token| state.engines.as_ref()?.admit(token)) {
        return Some(engine);
    }
    if state.config.admission == Admission::Token {
        let token = state.config.token.as_str();
        let is_token = |t: Option<String>| t.is_some_and(|t| harnix_authz::tokens_equal(&t, token));
        let by_token = is_token(bearer) || is_token(param("token"));
        if by_token || state.config.links.admits(headers) {
            return Some(Subject::Device {
                id: harnix_contract::LOCAL_DEVICE.into(),
            });
        }
    }
    None
}

pub(crate) type SharedGate = Option<Arc<dyn RouteDecider>>;
