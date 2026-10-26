//! Module routes, `/apps/<id>/…` (design §7.7).
//!
//! A module's own HTTP and WebSocket, served from this listener so they are
//! same-origin with the interface. Transport does not know what a module is:
//! it is handed routers by id and a [`RouteGate`] to ask, and does the two
//! things only it can — find out who the request is from, and refuse before
//! the module's code runs.
//!
//! Who the request is from, in order: a credential from a handshake
//! (`Authorization: Bearer <credential>`, or `?credential=` where a browser
//! API cannot set a header — a WebSocket); on a loopback listener, the
//! per-process token the same two ways, or a launch-link session cookie.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use harnix_contract::{RpcError, Subject};

use crate::{Admission, AppState};

/// Decides whether a subject may reach a module's routes. Implemented by the
/// application, over the authorization table.
pub trait RouteGate: Send + Sync {
    fn decide(&self, subject: &Subject, module: &str, path: &str) -> Result<(), RpcError>;
}

/// One module's router, to be served at `/apps/<id>/`.
pub struct AppRoutes {
    pub id: String,
    pub router: axum::Router,
}

pub(crate) fn mount(
    mut router: axum::Router<AppState>,
    state: &AppState,
    apps: Vec<AppRoutes>,
) -> axum::Router<AppState> {
    for app in apps {
        let guard_state = state.clone();
        let id = app.id.clone();
        let guarded = app.router.layer(axum::middleware::from_fn(move |req: Request, next: Next| {
            let state = guard_state.clone();
            let id = id.clone();
            async move { guard(state, id, req, next).await }
        }));
        router = router.nest_service(&format!("/apps/{}", app.id), guarded);
    }
    // Anything else under `/apps/` is a module that is not here — disabled,
    // or never was — and must not fall through to the interface's index.
    router.route(
        "/apps/{*rest}",
        axum::routing::any(|| async { (StatusCode::NOT_FOUND, "no module serves this path") }),
    )
}

async fn guard(state: AppState, module: String, req: Request, next: Next) -> Response {
    let headers = req.headers();
    if !crate::origin_allowed(headers, &state.config.extra_origins) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    let query = req.uri().query().unwrap_or("").to_string();
    let Some(subject) = subject_of(&state, headers, &query).await else {
        return (StatusCode::UNAUTHORIZED, "no credential").into_response();
    };
    let Some(gate) = state.route_gate.as_ref() else {
        return (StatusCode::NOT_FOUND, "no module routes here").into_response();
    };
    if let Err(e) = gate.decide(&subject, &module, req.uri().path()) {
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
        if let Some(established) = state.sessions.get(&credential).await {
            return Some(established.subject.clone());
        }
    }
    if state.config.admission == Admission::Token {
        let token = state.config.token.as_str();
        let by_token = bearer.as_deref() == Some(token) || param("token").as_deref() == Some(token);
        if by_token || state.config.links.admits(headers) {
            return Some(Subject::Device { id: "local".into() });
        }
    }
    None
}

pub(crate) type SharedGate = Option<Arc<dyn RouteGate>>;
