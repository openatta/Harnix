//! The network an application's VM is lent by this process: `harnix_wasm::HttpFetcher`.
//!
//! The VM has already checked every request against the application's declared `net` before it
//! gets here; this is what serves the requests that passed. (`host.call`, `publish` and
//! `subscribe` go through the kernel: `bridge.rs`.)

use std::time::Duration;

use async_trait::async_trait;
use harnix_wasm::{HttpFetcher, HttpRequest, HttpResponse};

/// The network, for the hosts an application's `net` lists. Redirects are not followed: the
/// check was made against the URL the component asked for, and a redirect is a URL nobody
/// checked.
///
/// The process's proxy settings apply, except to this machine's own addresses: a proxy elsewhere
/// cannot reach this machine's loopback, and one here would only hand the request back.
pub struct Http {
    client: reqwest::Client,
    direct: reqwest::Client,
}

/// Bodies past this are cut off rather than buffered whole.
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

impl Http {
    pub fn new() -> Self {
        let builder = || {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(60))
        };
        Self {
            client: builder().build().unwrap_or_default(),
            direct: builder().no_proxy().build().unwrap_or_default(),
        }
    }

    fn client_for(&self, url: &str) -> &reqwest::Client {
        let loopback = harnix_wasm::host_of(url).is_some_and(|host| {
            let bare = host.trim_start_matches('[').trim_end_matches(']');
            host == "localhost"
                || bare
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if loopback {
            &self.direct
        } else {
            &self.client
        }
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
        let mut builder = self.client_for(&request.url).request(method, &request.url);
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
