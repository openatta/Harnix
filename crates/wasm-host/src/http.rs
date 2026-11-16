//! A package's VM's network, as this host lends it: its `net` judged here first, with
//! [`harnix_package::egress::allowed`], then handed to what the product gave the host to send
//! with ([`crate::HostOptions::http`]) — which judges it again by the same rule, against what it
//! granted, where it is sent.
//!
//! Held here and not in the VM, so that the rule is one function: the VM knows no URL, and every
//! side that judges one calls this. (`host.call`, `publish` and `subscribe` go through the
//! kernel: `bridge.rs`.)

use std::sync::Arc;

use async_trait::async_trait;
use harnix_package::egress;
use harnix_wasm::{HttpFetcher, HttpRequest, HttpResponse};

/// One package's network: the hosts its manifest's `net` lists, through `send`.
pub(crate) struct Net {
    net: Vec<String>,
    send: Arc<dyn HttpFetcher>,
}

impl Net {
    pub(crate) fn new(net: Vec<String>, send: Arc<dyn HttpFetcher>) -> Self {
        Self { net, send }
    }
}

#[async_trait]
impl HttpFetcher for Net {
    async fn fetch(&self, app: &str, request: HttpRequest) -> Result<HttpResponse, String> {
        egress::allowed(&self.net, &request.url)?;
        self.send.fetch(app, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Sent(Mutex<Vec<String>>);

    #[async_trait]
    impl HttpFetcher for Sent {
        async fn fetch(&self, _app: &str, request: HttpRequest) -> Result<HttpResponse, String> {
            self.0.lock().unwrap().push(request.url);
            Ok(HttpResponse::default())
        }
    }

    fn get(url: &str) -> HttpRequest {
        HttpRequest { method: "GET".into(), url: url.into(), headers: Vec::new(), body: None }
    }

    #[tokio::test]
    async fn only_what_net_lists_is_handed_on() {
        let sent = Arc::new(Sent::default());
        let net = Net::new(vec!["api.example.com".into()], sent.clone());
        assert!(net.fetch("acme.probe", get("https://API.example.com/v1")).await.is_ok());
        for url in ["https://evil.example\\@api.example.com/", "https://api.example.com@evil.example/", "file:///etc/passwd"] {
            assert!(net.fetch("acme.probe", get(url)).await.is_err(), "{url}");
        }
        assert_eq!(*sent.0.lock().unwrap(), ["https://API.example.com/v1"]);
    }
}
