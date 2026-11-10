//! Where a package's HTTP may go, decided on the product's side: the hosts its `net` capability
//! lists, and nothing else.
//!
//! The VM checks a request against `net` before it leaves; this is the same rule held where the
//! request is sent, so what reaches the network does not rest on the VM's word — in an extension
//! process it is the only check that counts. [`Egress`] is what sends the requests that pass.

use std::time::Duration;

/// An outgoing request, as a package's backend asked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// What came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Bodies past this are cut off rather than buffered whole.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// The host a URL names, lowercase, without its port; `None` for anything but `http(s)://`. The
/// userinfo is dropped (`https://allowed.example@evil.example/` names `evil.example`).
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let host = match authority.strip_prefix('[') {
        Some(v6) => format!("[{}]", v6.split(']').next()?),
        None => authority.split(':').next()?.to_string(),
    };
    (!host.is_empty() && host != "[]").then(|| host.to_ascii_lowercase())
}

/// Whether a package whose `net` is `net` may send `url`: its host is one of them.
pub fn allowed(net: &[String], url: &str) -> Result<(), String> {
    let Some(host) = host_of(url) else {
        return Err(format!("`{url}` is not an http(s) URL"));
    };
    if net.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
        Ok(())
    } else {
        Err(format!("network access to `{host}` is not in this package's `net` capability"))
    }
}

/// The network, for requests already decided. Redirects are not followed: the check was made
/// against the URL asked for, and a redirect is a URL nobody checked.
///
/// The process's proxy settings apply, except to this machine's own addresses: a proxy elsewhere
/// cannot reach this machine's loopback, and one here would only hand the request back.
pub struct Egress {
    client: reqwest::Client,
    direct: reqwest::Client,
}

impl Default for Egress {
    fn default() -> Self {
        Self::new()
    }
}

impl Egress {
    pub fn new() -> Self {
        let builder = || reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(60));
        Self {
            client: builder().build().unwrap_or_default(),
            direct: builder().no_proxy().build().unwrap_or_default(),
        }
    }

    /// Send `request` for a package whose `net` is `net`, refused unless its host is listed.
    pub async fn send_for(&self, net: &[String], request: Request) -> Result<Response, String> {
        allowed(net, &request.url)?;
        self.send(request).await
    }

    /// Send a request whose host was checked already.
    pub async fn send(&self, request: Request) -> Result<Response, String> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| format!("`{}` is not an HTTP method", request.method))?;
        let mut builder = self.client_for(&request.url).request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let mut response = builder.send().await.map_err(|e| e.without_url().to_string())?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_string())))
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url().to_string())? {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(format!("the response is larger than {} MiB", MAX_RESPONSE_BYTES >> 20));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Response { status, headers, body })
    }

    fn client_for(&self, url: &str) -> &reqwest::Client {
        let loopback = host_of(url).is_some_and(|host| {
            let bare = host.trim_start_matches('[').trim_end_matches(']');
            host == "localhost" || bare.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
        });
        if loopback {
            &self.direct
        } else {
            &self.client
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_what_the_url_names_and_nothing_it_disguises() {
        assert_eq!(host_of("https://API.example.com/v1?x=1").as_deref(), Some("api.example.com"));
        assert_eq!(host_of("http://example.com:8080").as_deref(), Some("example.com"));
        assert_eq!(host_of("https://allowed.example@evil.example/").as_deref(), Some("evil.example"));
        assert_eq!(host_of("http://[::1]:4000/x").as_deref(), Some("[::1]"));
        assert_eq!(host_of("ftp://example.com"), None);
        assert_eq!(host_of("https://"), None);
    }

    #[test]
    fn only_a_listed_host_is_let_out() {
        let net = vec!["api.example.com".to_string()];
        assert!(allowed(&net, "https://api.example.com/v1").is_ok());
        assert!(allowed(&net, "https://API.EXAMPLE.COM/v1").is_ok());
        let e = allowed(&net, "https://api.example.com@evil.example/").unwrap_err();
        assert!(e.contains("`evil.example`"), "{e}");
        assert!(allowed(&net, "https://sub.api.example.com/").is_err());
        assert!(allowed(&[], "https://api.example.com/").is_err());
        assert!(allowed(&net, "file:///etc/passwd").is_err());
    }
}
