//! The network a package's VM is lent by this process when its product lends none of its
//! own: `harnix_wasm::HttpFetcher` over [`harnix_package::egress::Egress`].
//!
//! The VM has already checked every request against the package's declared `net` before it
//! gets here. A product that decides `net` on its side as well hands the host a fetcher of its
//! own ([`crate::HostOptions::http`]). (`host.call`, `publish` and `subscribe` go through the
//! kernel: `bridge.rs`.)

use async_trait::async_trait;
use harnix_package::egress::{Egress, Request};
use harnix_wasm::{HttpFetcher, HttpRequest, HttpResponse};

/// The network, for the hosts a package's `net` lists.
#[derive(Default)]
pub struct Http(Egress);

impl Http {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl HttpFetcher for Http {
    async fn fetch(&self, _app: &str, request: HttpRequest) -> Result<HttpResponse, String> {
        let request = Request { method: request.method, url: request.url, headers: request.headers, body: request.body };
        let response = self.0.send(request).await?;
        Ok(HttpResponse { status: response.status, headers: response.headers, body: response.body })
    }
}
