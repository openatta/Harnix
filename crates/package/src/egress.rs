//! Where a package's HTTP may go, decided on the product's side: the hosts its `net` capability
//! lists, and nothing else.
//!
//! The VM checks a request against `net` before it leaves; this is the same rule held where the
//! request is sent, so what reaches the network does not rest on the VM's word — in an extension
//! process it is the only check that counts. [`Egress`] is what sends the requests that pass.

use std::time::Duration;

/// reqwest's URL: what [`allowed`] parses is what is sent.
pub use url::Url;

/// An outgoing request, as a package's backend asked for it, its URL let out already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub url: Allowed,
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

/// A URL a package's `net` lets out, as [`allowed`] parsed it. It is this parse that is sent, not
/// the text again: a second parser reading the same text another way (`\` for `/`, userinfo, an
/// odd port) is how a host nobody checked gets reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allowed(Url);

impl Allowed {
    /// The host it was let out to: lowercase, without its port; an IPv6 address in brackets.
    pub fn host(&self) -> &str {
        self.0.host_str().unwrap_or_default()
    }

    pub fn url(&self) -> &Url {
        &self.0
    }

    /// One of this machine's own addresses.
    fn is_loopback(&self) -> bool {
        match self.0.host() {
            Some(url::Host::Domain(name)) => name == "localhost",
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        }
    }
}

/// Whether a package whose `net` is `net` may send to `url`: an `http(s)` URL, parsed as the
/// network will read it (WHATWG), whose host is one `net` lists, exactly and in any case. The
/// port is not judged; the userinfo is not the host.
///
/// The one rule for a package's HTTP, wherever it is held — before the VM's request leaves its
/// host and again where the product sends it. A refusal names the host at most, never the URL: a
/// component may have built it from a secret, and the refusal usually travels on into a tool
/// result and the transcript.
pub fn allowed(net: &[String], url: &str) -> Result<Allowed, String> {
    let parsed = Url::parse(url).map_err(|_| "the request's URL does not parse".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("only http(s) URLs may be fetched, not `{}:`", parsed.scheme()));
    }
    let allowed = Allowed(parsed);
    let host = allowed.host();
    if host.is_empty() {
        return Err("the request's URL names no host".into());
    }
    if net.iter().any(|h| h.eq_ignore_ascii_case(host)) {
        Ok(allowed)
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

    /// Send a request whose URL was let out ([`allowed`]): to what that check parsed.
    pub async fn send(&self, request: Request) -> Result<Response, String> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| format!("`{}` is not an HTTP method", request.method))?;
        let client = if request.url.is_loopback() { &self.direct } else { &self.client };
        let mut builder = client.request(method, request.url.0);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net() -> Vec<String> {
        vec!["api.example.com".to_string(), "127.0.0.1".to_string()]
    }

    fn host(url: &str) -> Result<String, String> {
        allowed(&net(), url).map(|a| a.host().to_string())
    }

    #[test]
    fn a_listed_host_is_let_out_in_any_case_and_on_any_port() {
        assert_eq!(host("https://api.example.com/v1?x=1").as_deref(), Ok("api.example.com"));
        assert_eq!(host("https://API.Example.COM/v1").as_deref(), Ok("api.example.com"));
        assert_eq!(host("HTTP://api.example.com:8080/x").as_deref(), Ok("api.example.com"));
        assert_eq!(host("http://127.0.0.1:4000/x").as_deref(), Ok("127.0.0.1"));
        // Listed in any case too: the manifest's own spelling is not normalized for it.
        assert!(allowed(&["API.example.com".to_string()], "https://api.example.com/").is_ok());
    }

    #[test]
    fn the_host_judged_is_the_host_sent_to() {
        // `\` is `/` to an http(s) URL: what precedes it is the whole authority.
        let e = host("https://evil.example\\@api.example.com/").unwrap_err();
        assert!(e.contains("`evil.example`"), "{e}");
        // Userinfo is not the host, whichever side the listed name is on.
        let e = host("https://api.example.com@evil.example/").unwrap_err();
        assert!(e.contains("`evil.example`"), "{e}");
        let e = host("https://api.example.com:443@evil.example/").unwrap_err();
        assert!(e.contains("`evil.example`"), "{e}");
        assert_eq!(host("https://user:pw@api.example.com/x").as_deref(), Ok("api.example.com"));
        let ok = allowed(&net(), "https://user:pw@api.example.com/x").unwrap();
        assert_eq!(ok.url().host_str(), Some("api.example.com"), "what is sent is what was judged");
        // Neither a suffix nor a prefix of a listed name is it.
        assert!(host("https://sub.api.example.com/").is_err());
        assert!(host("https://api.example.com.evil.test/").is_err());
        // Another spelling of a listed address is the address.
        assert_eq!(host("http://0x7f.0.0.1/").as_deref(), Ok("127.0.0.1"));
    }

    #[test]
    fn ipv6_is_judged_in_brackets() {
        let v6 = vec!["[::1]".to_string()];
        let ok = allowed(&v6, "http://[0:0::1]:4000/x").unwrap();
        assert_eq!(ok.host(), "[::1]");
        assert!(ok.is_loopback());
        assert!(allowed(&net(), "http://[::1]/").unwrap_err().contains("`[::1]`"));
    }

    #[test]
    fn only_http_and_https_with_a_host_go_out_and_the_refusal_keeps_the_url_to_itself() {
        for url in ["file:///etc/passwd", "ftp://api.example.com/", "ws://api.example.com/", "data:text/plain,api.example.com"] {
            assert!(host(url).unwrap_err().contains("only http(s)"), "{url}");
        }
        for url in ["api.example.com/x", "https://", "https://[::1/", "//api.example.com/"] {
            assert!(host(url).is_err(), "{url}");
        }
        assert!(allowed(&[], "https://api.example.com/").is_err());
        let e = host("https://evil.example/path?token=t0ken").unwrap_err();
        assert!(!e.contains("t0ken") && !e.contains("path"), "{e}");
        let e = host("ftp://user:s3cret@api.example.com/").unwrap_err();
        assert!(!e.contains("s3cret"), "{e}");
    }

    #[test]
    fn this_machine_s_own_addresses_go_direct() {
        let local = vec!["localhost".to_string(), "127.0.0.1".to_string(), "api.example.com".to_string()];
        assert!(allowed(&local, "http://localhost:3000/").unwrap().is_loopback());
        assert!(allowed(&local, "http://127.0.0.1/").unwrap().is_loopback());
        assert!(!allowed(&local, "https://api.example.com/").unwrap().is_loopback());
    }
}
