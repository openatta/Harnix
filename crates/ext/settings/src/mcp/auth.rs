//! Signing in to a remote MCP server (OAuth): the engine runs the protocol (`agix/docs/mcp_auth.md`);
//! the browser is the person's, so this sends them to the authorization page and hands the way
//! back to the engine.
//!
//! The way back is this extension's own route, `/ext/<id>/mcp/callback`, same origin as the page:
//! the engine is started with it as an `external` redirect, and the browser lands there with the
//! code. The route is admitted like any request — a page opened from a launch link carries the
//! session cookie; a page that came in with a device credential does not, and its opener, which can
//! read where its own window landed, hands the address over with `settings.mcp.authComplete`.
//! Whichever comes first finishes the sign-in, and the other is told how it ended. An origin the
//! engine will not send a browser back to (plain http that is not loopback) gets the engine's own
//! loopback listener instead, which only a browser on the engine's machine reaches.
//!
//! Which sign-in a callback is for is read from its `state`, which the engine put in the
//! authorization URL.

use std::time::{Duration, Instant};

use harnix_ext::http::{full, Body, Request, Response, RouteContext, RouteHandler, StatusCode};
use harnix_ext::{Rpc, RpcError};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::providers::Engine;
use crate::wire::McpAuthDone;

/// Where under the extension's routes the browser comes back to.
pub(crate) const CALLBACK: &str = "/mcp/callback";
/// How long a finished sign-in's outcome is kept for the other of the two ways back to read.
const KEPT_AFTER: Duration = Duration::from_secs(120);

/// Where the engine sends the browser back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Redirect {
    /// This process's route, at this URI.
    External(String),
    /// The engine's own listener on 127.0.0.1.
    Loopback,
}

/// The redirect for a page at `origin` (`scheme://host[:port]`, as the page's `location.origin`
/// says), whose extension routes are under `/ext/<id>`. The engine takes `https`, or `http` on a
/// loopback address; anything else gets its loopback listener.
pub(crate) fn redirect_for(origin: Option<&str>, ext_id: &str) -> Rpc<Redirect> {
    let Some(origin) = origin.map(str::trim).filter(|o| !o.is_empty()) else {
        return Ok(Redirect::Loopback);
    };
    let url = url::Url::parse(origin).map_err(|e| RpcError::invalid_params(format!("origin `{origin}` is not a URL: {e}")))?;
    let bare = matches!(url.path(), "" | "/") && url.query().is_none() && url.fragment().is_none() && url.username().is_empty() && url.password().is_none();
    if !bare || !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(RpcError::invalid_params(format!("origin `{origin}`: scheme, host and port only")));
    }
    let loopback = match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if url.scheme() == "https" || loopback {
        Ok(Redirect::External(format!("{}/ext/{ext_id}{CALLBACK}", origin.trim_end_matches('/'))))
    } else {
        Ok(Redirect::Loopback)
    }
}

/// `engine.mcp.auth.start`'s parameters for this server and redirect, located like every engine
/// call of this extension.
pub(crate) fn start_params(server: &str, redirect: &Redirect, at: &Value, scopes: Option<&Value>) -> Value {
    let mut params = at.clone();
    params["server"] = json!(server);
    match redirect {
        Redirect::External(uri) => {
            params["redirect"] = json!("external");
            params["redirect_uri"] = json!(uri);
        }
        Redirect::Loopback => params["redirect"] = json!("loopback"),
    }
    if let Some(scopes) = scopes.filter(|s| s.as_array().is_some_and(|a| !a.is_empty())) {
        params["scopes"] = scopes.clone();
    }
    params
}

/// A query parameter of a URL, decoded.
fn query_param(url: &str, name: &str) -> Option<String> {
    url::Url::parse(url).ok()?.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned())
}

struct Flow {
    flow_id: String,
    server: String,
    state: Option<String>,
    redirect_uri: String,
    expires: Instant,
    done: Option<(Instant, McpAuthDone)>,
}

/// The sign-ins this extension started and has not seen the end of.
#[derive(Default)]
pub(crate) struct Flows {
    flows: Mutex<Vec<Flow>>,
}

impl Flows {
    /// A sign-in the engine started: `authorize_url` is where the person is sent, and the `state`
    /// in it is what the callback is matched by.
    pub(crate) async fn begin(&self, flow_id: &str, server: &str, authorize_url: &str, redirect_uri: &str, expires_in: u64) {
        let mut flows = self.flows.lock().await;
        let now = Instant::now();
        // Gone: past the engine's own time, or finished long enough ago; and an earlier sign-in to
        // the same server, which the engine cancels when it starts this one.
        flows.retain(|f| f.server != server && f.expires > now && f.done.as_ref().is_none_or(|(at, _)| now.duration_since(*at) < KEPT_AFTER));
        flows.push(Flow {
            flow_id: flow_id.into(),
            server: server.into(),
            state: query_param(authorize_url, "state"),
            redirect_uri: redirect_uri.into(),
            expires: now + Duration::from_secs(expires_in.max(1)) + KEPT_AFTER,
            done: None,
        });
    }

    /// Hand the engine the address the browser came back to (`callback_url`, or the query alone,
    /// put on the redirect URI it was started with), once: a second hand-over of the same sign-in is
    /// told how the first ended.
    pub(crate) async fn complete(&self, engine: &dyn Engine, callback: &str) -> Rpc<McpAuthDone> {
        let query = callback.split_once('?').map_or(callback, |(_, q)| q).to_string();
        let state = query_param(&format!("http://x/?{query}"), "state");
        let mut flows = self.flows.lock().await;
        let flow = match &state {
            Some(state) => flows.iter_mut().find(|f| f.state.as_deref() == Some(state.as_str())),
            None => None,
        }
        .ok_or_else(|| RpcError::invalid_params("no sign-in is waiting for this callback: it ended, or was started again"))?;
        if let Some((_, done)) = &flow.done {
            return Ok(done.clone());
        }
        let callback_url = format!("{}?{query}", flow.redirect_uri);
        let done = match engine.call("engine.mcp.auth.complete", json!({"flow_id": flow.flow_id, "callback_url": callback_url})).await {
            Ok(answer) => McpAuthDone {
                server: answer.get("server").and_then(Value::as_str).unwrap_or(&flow.server).to_string(),
                ok: true,
                scopes: answer
                    .get("scopes")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default(),
                error: None,
            },
            // The authorization server's refusal, a failed exchange, or a sign-in the engine no
            // longer has: an end either way, and its words are for the person.
            Err(e) => McpAuthDone { server: flow.server.clone(), ok: false, scopes: Vec::new(), error: Some(e.message) },
        };
        flow.done = Some((Instant::now(), done.clone()));
        Ok(done)
    }

    /// Forget a sign-in given up on; the server it was for.
    pub(crate) async fn forget(&self, flow_id: &str) -> Option<String> {
        let mut flows = self.flows.lock().await;
        let at = flows.iter().position(|f| f.flow_id == flow_id)?;
        Some(flows.remove(at).server)
    }
}

/// The browser's way back: finish the sign-in, and say how it ended on a page of its own.
pub(crate) struct Callback;

#[async_trait::async_trait]
impl RouteHandler for Callback {
    async fn handle(&self, cx: RouteContext, req: Request<Body>) -> Response<Body> {
        if !req.uri().path().ends_with(CALLBACK) {
            return page(StatusCode::NOT_FOUND, None, "not found");
        }
        let query = req.uri().query().unwrap_or_default().to_string();
        let state = match cx.ext().state::<super::McpState>() {
            Ok(state) => state,
            Err(e) => return page(StatusCode::SERVICE_UNAVAILABLE, None, &e.message),
        };
        let engine = Ext(cx.ext().clone());
        match state.flows.complete(&engine, &query).await {
            Ok(done) => {
                let status = if done.ok { StatusCode::OK } else { StatusCode::BAD_REQUEST };
                let text = match &done.error {
                    None => format!("已登录 MCP 服务器 {0}，可以关掉这个窗口。 Signed in to the MCP server {0}; you can close this window.", done.server),
                    Some(error) => format!("没有登录 MCP 服务器 {0}：{1} Could not sign in to the MCP server {0}: {1}", done.server, error),
                };
                page(status, Some(done.ok), &text)
            }
            Err(e) => page(StatusCode::BAD_REQUEST, Some(false), &e.message),
        }
    }
}

/// An engine call made as the extension itself: the route has no device call to be a child of.
struct Ext(harnix_ext::ExtensionContext);

#[async_trait::async_trait]
impl Engine for Ext {
    async fn call(&self, method: &str, params: Value) -> Rpc<Value> {
        self.0.call(method, params).await
    }
}

/// The page the browser is left on. It runs nothing: the opener reads `data-harnix-mcp-auth` off it.
fn page(status: StatusCode, ok: Option<bool>, text: &str) -> Response<Body> {
    let marker = match ok {
        Some(true) => "done",
        Some(false) => "failed",
        None => "none",
    };
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>MCP</title><main data-harnix-mcp-auth=\"{marker}\" style=\"font:15px system-ui;margin:3em auto;max-width:36em\"><p>{}</p></main>",
        escape(text)
    );
    let mut response = Response::new(full(body));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(::http::header::CONTENT_TYPE, ::http::HeaderValue::from_static("text/html; charset=utf-8"));
    // The code is in the address: not kept, and not sent on.
    headers.insert(::http::header::CACHE_CONTROL, ::http::HeaderValue::from_static("no-store"));
    headers.insert(::http::header::REFERRER_POLICY, ::http::HeaderValue::from_static("no-referrer"));
    response
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::*;

    /// An engine that answers `mcp.auth.complete` as told, and remembers what it was asked.
    struct Fake {
        asked: StdMutex<Vec<(String, Value)>>,
        answer: Rpc<Value>,
    }

    #[async_trait::async_trait]
    impl Engine for Fake {
        async fn call(&self, method: &str, params: Value) -> Rpc<Value> {
            self.asked.lock().unwrap().push((method.into(), params));
            self.answer.clone()
        }
    }

    fn fake(answer: Rpc<Value>) -> Fake {
        Fake { asked: StdMutex::new(Vec::new()), answer }
    }

    #[test]
    fn the_browser_comes_back_here_when_the_engine_takes_this_origin() {
        let here = |o: &str| redirect_for(Some(o), "settings").unwrap();
        assert_eq!(here("https://harnix.example"), Redirect::External("https://harnix.example/ext/settings/mcp/callback".into()));
        assert_eq!(here("http://127.0.0.1:4317"), Redirect::External("http://127.0.0.1:4317/ext/settings/mcp/callback".into()));
        assert_eq!(here("http://localhost:4317/"), Redirect::External("http://localhost:4317/ext/settings/mcp/callback".into()));
        assert_eq!(here("http://[::1]:4317"), Redirect::External("http://[::1]:4317/ext/settings/mcp/callback".into()));
        // Plain http elsewhere: the engine would refuse it; its own listener instead.
        assert_eq!(here("http://192.168.1.5:4317"), Redirect::Loopback);
        assert_eq!(redirect_for(None, "settings").unwrap(), Redirect::Loopback);
        for bad in ["https://x/path", "https://x/?q=1", "https://u:p@x", "ftp://x", "nonsense"] {
            assert!(redirect_for(Some(bad), "settings").is_err(), "{bad}");
        }
    }

    #[test]
    fn the_engine_is_asked_for_an_external_redirect_located_as_the_call_is() {
        let at = json!({"scene": "code", "project_root": "/p"});
        let params = start_params("docs", &Redirect::External("https://h/ext/settings/mcp/callback".into()), &at, Some(&json!(["a"])));
        assert_eq!(params, json!({"scene": "code", "project_root": "/p", "server": "docs", "redirect": "external",
                                  "redirect_uri": "https://h/ext/settings/mcp/callback", "scopes": ["a"]}));
        let params = start_params("docs", &Redirect::Loopback, &json!({}), Some(&json!([])));
        assert_eq!(params, json!({"server": "docs", "redirect": "loopback"}));
    }

    #[tokio::test]
    async fn a_callback_is_matched_by_its_state_handed_over_once_and_told_twice() {
        let flows = Flows::default();
        let redirect = "http://127.0.0.1:4317/ext/settings/mcp/callback";
        flows.begin("F1", "docs", "https://as.example/authorize?client_id=c&state=S%201&code_challenge=x", redirect, 600).await;
        let engine = fake(Ok(json!({"server": "docs", "scopes": ["files:read"]})));
        // The route hands over its query; the opener the whole address. The engine gets the address
        // on the redirect it was started with either way.
        let first = flows.complete(&engine, "code=abc&state=S+1").await.unwrap();
        assert!(first.ok && first.scopes == ["files:read"], "{first:?}");
        let again = flows.complete(&engine, &format!("{redirect}?code=abc&state=S%201")).await.unwrap();
        assert_eq!(again, first);
        let asked = engine.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1, "handed over twice: {asked:?}");
        assert_eq!(asked[0].0, "engine.mcp.auth.complete");
        assert_eq!(asked[0].1, json!({"flow_id": "F1", "callback_url": format!("{redirect}?code=abc&state=S+1")}));
        // A state nobody started is not handed over at all.
        let e = flows.complete(&engine, "code=abc&state=forged").await.unwrap_err();
        assert!(e.message.contains("no sign-in is waiting"), "{}", e.message);
        assert!(flows.complete(&engine, "code=abc").await.is_err());
        assert_eq!(engine.asked.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_refused_sign_in_ends_with_the_engines_words() {
        let flows = Flows::default();
        flows.begin("F1", "docs", "https://as/authorize?state=S", "https://h/ext/settings/mcp/callback", 600).await;
        let engine = fake(Err(RpcError::new(-32021, "the authorization server said: access_denied")));
        let done = flows.complete(&engine, "error=access_denied&state=S").await.unwrap();
        assert_eq!(done, McpAuthDone { server: "docs".into(), ok: false, scopes: vec![], error: Some("the authorization server said: access_denied".into()) });
        // Started again: the earlier one is gone, and its callback is not for the new one.
        flows.begin("F2", "docs", "https://as/authorize?state=T", "https://h/ext/settings/mcp/callback", 600).await;
        assert!(flows.complete(&engine, "code=x&state=S").await.is_err());
        assert_eq!(flows.forget("F2").await.as_deref(), Some("docs"));
        assert!(flows.complete(&engine, "code=x&state=T").await.is_err());
    }

    #[test]
    fn the_page_left_says_how_it_ended_and_runs_nothing() {
        let response = page(StatusCode::OK, Some(true), "<script>alert(1)</script> & done");
        assert_eq!(response.headers()[::http::header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()[::http::header::REFERRER_POLICY], "no-referrer");
        assert_eq!(escape("<a href=\"x\">&</a>"), "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;");
    }
}
