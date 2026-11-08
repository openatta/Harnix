//! What an application's VM is lent by this process: `harnix_wasm::AppHost` and `HttpFetcher`.
//!
//! The VM has already checked every request against the application's declared capabilities
//! (`net`, `env`, `storage_mb`) before it gets here; this is what serves the requests that passed.
//!
//! `host.call`, `publish` and `subscribe` are a backend's: the hub's admission point behind them,
//! with the subject `module:<app id>`, arrives with backend components. Until then they answer
//! that, rather than something that looks like a refusal by the hub.

use std::time::Duration;

use async_trait::async_trait;
use harnix_wasm::{AppHost, HttpFetcher, HttpRequest, HttpResponse};
use serde_json::Value;

const NOT_YET: &str =
    "this Harnix does not route an application's hub calls, topics or subscriptions yet \
                       (they come with backend components)";

/// The process, as a tool component sees it.
pub struct Host;

#[async_trait]
impl AppHost for Host {
    async fn call(&self, _app: &str, _method: &str, _params: Value) -> Result<Value, String> {
        Err(NOT_YET.into())
    }

    fn publish(&self, _app: &str, _topic: &str, _value: Value) -> Result<(), String> {
        Err(NOT_YET.into())
    }

    async fn subscribe(&self, _app: &str, _topic: &str, _params: &Value) -> Result<(), String> {
        Err(NOT_YET.into())
    }

    async fn unsubscribe(&self, _app: &str, _topic: &str, _params: &Value) {}
}

/// The network, for the hosts an application's `net` lists. Redirects are not followed: the
/// check was made against the URL the component asked for, and a redirect is a URL nobody
/// checked.
pub struct Http {
    client: reqwest::Client,
}

/// Bodies past this are cut off rather than buffered whole.
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

impl Http {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self { client }
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpFetcher for Http {
    async fn fetch(&self, _app: &str, request: HttpRequest) -> Result<HttpResponse, String> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| format!("`{}` is not an HTTP method", request.method))?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let response = builder
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
            .collect();
        let mut body = Vec::new();
        let mut response = response;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| e.without_url().to_string())?
        {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(format!(
                    "the response is larger than {} MiB",
                    MAX_RESPONSE_BYTES >> 20
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}
