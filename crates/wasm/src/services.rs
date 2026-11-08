//! The services a host lends its applications.
//!
//! These traits are the seam between this crate and the rest of Harnix: the
//! hub's admission point behind [`AppHost::call`], its topics behind
//! `publish` and `subscribe`, an HTTP client behind [`HttpFetcher`], storage
//! behind [`crate::KvStore`]. The VM checks a request against the
//! application's capabilities *before* handing it to one of these, so an
//! implementation sees only requests that were allowed — and need not, and
//! must not, re-implement those checks with a second answer.
//!
//! Every method names the application asking (`app`, its id), because one
//! implementation serves every application in the process.

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Harnix, as an application's component sees it.
#[async_trait]
pub trait AppHost: Send + Sync {
    /// `host.call`: call a hub method as `module:<app>`. Admission (the
    /// manifest's `hub` capability) and audit are the implementation's.
    ///
    /// When the backend makes this call it holds its own instance, so a call
    /// that routes back into the same application's backend cannot be
    /// answered; the VM refuses that one itself rather than deadlock, as long
    /// as the implementation awaits the routed call on the same task.
    async fn call(&self, app: &str, method: &str, params: Value) -> Result<Value, String>;

    /// `host.publish`: publish on `app.<app>.<topic>`.
    fn publish(&self, app: &str, topic: &str, value: Value) -> Result<(), String>;

    /// `host.subscribe` from the backend. Values are delivered back with
    /// [`crate::AppVm::deliver_topic`]. The VM records the subscription and
    /// undoes it with [`AppHost::unsubscribe`] when the instance that made it
    /// ends, so the implementation never has to track instance lifetimes.
    async fn subscribe(&self, app: &str, topic: &str, params: &Value) -> Result<(), String>;

    async fn unsubscribe(&self, app: &str, topic: &str, params: &Value);

    /// `host.secret`, for a name the application declared in `env`. Reads
    /// the process environment unless the host keeps secrets elsewhere.
    fn secret(&self, app: &str, key: &str) -> Option<String> {
        let _ = app;
        std::env::var(key).ok()
    }
}

/// An outgoing HTTP request, already checked against `net`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Where an application's HTTP goes out.
///
/// An implementation **must not follow redirects**: the `net` check was made
/// against the URL the component asked for, and a redirect is a second URL
/// nobody checked. A 3xx goes back to the component, which may ask again —
/// and be checked again.
#[async_trait]
pub trait HttpFetcher: Send + Sync {
    async fn fetch(&self, app: &str, request: HttpRequest) -> Result<HttpResponse, String>;
}

/// A fetcher that reaches nothing: for a host that has not wired a client,
/// and for tests.
pub struct NoNetwork;

#[async_trait]
impl HttpFetcher for NoNetwork {
    async fn fetch(&self, _app: &str, _request: HttpRequest) -> Result<HttpResponse, String> {
        Err("this host offers applications no network".into())
    }
}

/// Where a tool call's `host.progress` goes.
pub trait ProgressSink: Send + Sync {
    fn on_progress(&self, call_id: &str, text: &str);
}

/// The services one application's VM uses.
#[derive(Clone)]
pub struct Services {
    pub host: Arc<dyn AppHost>,
    /// This application's own namespace; nothing else should share it.
    pub kv: Arc<dyn crate::KvStore>,
    pub http: Arc<dyn HttpFetcher>,
}
