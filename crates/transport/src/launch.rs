//! One-time launch links, exchanged for a session cookie (IPDrun N-301).
//!
//! # Why the page token is not always enough
//!
//! The default way a browser is admitted on loopback is the per-process token
//! written into the page (`statics.rs`). That keeps other *pages* out — a page
//! cannot read a cross-origin response — but not other *local clients*:
//! anything that can open a TCP connection to the port reads the page and the
//! token with it. Another account on the same machine is one; a DNS-rebound
//! page reading what is, to its browser, a same-origin response is another.
//!
//! # What this adds
//!
//! With launch links on (`BrowserAdmission::LaunchLink`), the page carries no
//! token at all. Whoever holds the token — the file under the state directory,
//! readable by its owner only — asks for a link; the link is single-use and
//! lives two minutes; following it sets an `HttpOnly`, `SameSite=Strict`
//! cookie and moves on to the interface. The cookie then stands in for the
//! token wherever a browser has to present one: the WebSocket upgrade and the
//! split-streams handshake. The Origin check is unchanged and still applies —
//! a cookie is what a browser *carries*, the Origin is what proves which page
//! is carrying it.
//!
//! The cookie lives as long as this process. A restart ends every browser
//! session, the same way it rotates the token.
//!
//! ```text
//!   POST /auth/link     Authorization: Bearer <token>  →  {"code", "path", "expires_in_secs"}
//!   GET  /auth/{code}   once, within two minutes       →  200, Set-Cookie: harnix_session=…,
//!                                                         and a meta refresh to /
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::AppState;

/// The cookie's name.
pub const COOKIE: &str = "harnix_session";

/// How long an unredeemed link is worth keeping: long enough to be pasted
/// from a terminal into a browser, short enough that one left in scrollback
/// is worthless by the time anyone reads it.
pub const LINK_TTL: Duration = Duration::from_secs(120);

/// How a browser comes to hold what admits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BrowserAdmission {
    /// The per-process token is written into the page. The default.
    #[default]
    PageToken,
    /// The page carries no token; a one-time launch link sets a session
    /// cookie that stands in for it.
    LaunchLink,
}

/// Outstanding links and the sessions they opened.
#[derive(Default)]
pub struct LaunchLinks {
    codes: Mutex<HashMap<String, SystemTime>>,
    sessions: Mutex<HashSet<String>>,
}

impl LaunchLinks {
    /// Mint one link code.
    pub fn mint(&self, now: SystemTime) -> String {
        let code = secret();
        let mut codes = self.codes.lock().unwrap();
        codes.retain(|_, expires| now <= *expires);
        codes.insert(code.clone(), now + LINK_TTL);
        code
    }

    /// Spend a code. `None` for unknown, expired and already used alike —
    /// telling them apart tells a prober which guesses were closer.
    pub fn redeem(&self, code: &str, now: SystemTime) -> Option<String> {
        let expires = self.codes.lock().unwrap().remove(code)?;
        if now > expires {
            return None;
        }
        let session = secret();
        self.sessions.lock().unwrap().insert(session.clone());
        Some(session)
    }

    /// Whether this request carries a session cookie this process issued.
    pub fn admits(&self, headers: &HeaderMap) -> bool {
        let sessions = self.sessions.lock().unwrap();
        cookie_values(headers, COOKIE).any(|value| sessions.contains(value))
    }
}

/// 256 bits from the OS generator, hex. Two v4 UUIDs would be 244; the
/// difference does not matter, the dependency is already here.
fn secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Every value of a named cookie across every `Cookie` header.
fn cookie_values<'a>(headers: &'a HeaderMap, name: &'a str) -> impl Iterator<Item = &'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(move |pair| {
            let (k, v) = pair.trim().split_once('=')?;
            (k == name).then_some(v)
        })
}

/// `POST /auth/link` — the token holder asks for a link.
///
/// The token travels as a bearer header, never in the URL: this is called by
/// a program (a launcher, a desktop shell, a CLI), not typed.
pub async fn create(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !crate::origin_allowed(&headers, &state.config.extra_origins) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if bearer != Some(state.config.token.as_str()) {
        return (StatusCode::UNAUTHORIZED, "bad token").into_response();
    }
    let code = state.config.links.mint(SystemTime::now());
    tracing::info!("launch link created");
    (
        [(header::CACHE_CONTROL, "no-store")],
        axum::Json(serde_json::json!({
            "code": code,
            "path": format!("/auth/{code}"),
            "expires_in_secs": LINK_TTL.as_secs(),
        })),
    )
        .into_response()
}

/// `GET /auth/{code}` — follow a link, get a cookie, land on the interface.
pub async fn redeem(State(state): State<AppState>, Path(code): Path<String>) -> Response {
    let Some(session) = state.config.links.redeem(&code, SystemTime::now()) else {
        tracing::warn!("a launch link was refused (unknown, expired, or already used)");
        return refusal(
            StatusCode::UNAUTHORIZED,
            "This launch link is unknown, expired, or already used. Ask for a new one.",
        );
    };
    tracing::info!("launch link redeemed; browser session opened");
    // A page that moves itself on, not a 303. The navigation that follows a
    // link is usually *initiated elsewhere* — a terminal, another app, a
    // desktop shell's own start page on a different origin — and a redirect
    // keeps that initiator, so the browser treats the hop to `/` as
    // cross-site and withholds a `SameSite=Strict` cookie from it. (Measured
    // in WKWebView: the request for `/` arrived with no cookie at all.) A
    // meta refresh is a navigation this origin starts, so the cookie goes
    // with it; it is not script, so the CSP stays as strict as everywhere
    // else.
    //
    // `Secure` when this listener speaks TLS: the cookie then
    // never travels in clear, even to a plaintext listener on the same host.
    let secure = if state.config.tls.is_some() {
        "; Secure"
    } else {
        ""
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
            (
                header::SET_COOKIE,
                format!("{COOKIE}={session}; Path=/; HttpOnly; SameSite=Strict{secure}"),
            ),
            (header::CONTENT_SECURITY_POLICY, crate::CSP.to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
            (header::REFERRER_POLICY, "no-referrer".to_string()),
        ],
        "<!doctype html><meta charset=\"utf-8\"><title>Harnix</title>\
         <meta http-equiv=\"refresh\" content=\"0;url=/\"><p><a href=\"/\">Continue</a></p>",
    )
        .into_response()
}

/// A page saying why, under the same CSP as everything else. No script.
pub(crate) fn refusal(status: StatusCode, message: &str) -> Response {
    let page = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Harnix</title>\
         <p>{message}</p>"
    );
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_SECURITY_POLICY, crate::CSP),
            (header::CACHE_CONTROL, "no-store"),
        ],
        page,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn with_cookie(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn a_link_opens_exactly_one_session() {
        let links = LaunchLinks::default();
        let now = SystemTime::now();
        let code = links.mint(now);
        let session = links.redeem(&code, now).expect("first use");
        assert!(links.redeem(&code, now).is_none(), "second use refused");
        assert!(links.admits(&with_cookie(&format!("{COOKIE}={session}"))));
    }

    #[test]
    fn an_expired_link_opens_nothing() {
        let links = LaunchLinks::default();
        let now = SystemTime::now();
        let code = links.mint(now);
        assert!(links
            .redeem(&code, now + LINK_TTL + Duration::from_secs(1))
            .is_none());
        // And it is gone, not merely refused this once.
        assert!(links.redeem(&code, now).is_none());
    }

    #[test]
    fn unknown_and_foreign_cookies_are_not_admitted() {
        let links = LaunchLinks::default();
        assert!(!links.admits(&HeaderMap::new()));
        assert!(!links.admits(&with_cookie(&format!("{COOKIE}=guess"))));
        let now = SystemTime::now();
        let session = links.redeem(&links.mint(now), now).unwrap();
        // Same value under another name is not ours.
        assert!(!links.admits(&with_cookie(&format!("other={session}"))));
        // Among other cookies, with spacing browsers actually send.
        assert!(links.admits(&with_cookie(&format!(
            "theme=dark; {COOKIE}={session}; lang=zh"
        ))));
    }

    #[test]
    fn codes_and_sessions_are_long_and_distinct() {
        let a = secret();
        let b = secret();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }
}
