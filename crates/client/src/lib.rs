//! A light client of a running Harnix process (ipdrun-42): what a command-line tool, a test or a
//! desktop shell needs to talk to the process it started, without linking the server.
//!
//! The split-streams topology over plain HTTP: [`Client::connect`] holds the handshake (the
//! process token in, a credential out, the versions this crate was built against), then
//! [`Client::call`] is one `/rpc` request and [`Client::events`] one of the two download-only
//! event streams. [`launch_link`] mints a one-time link that opens the interface in a browser.
//!
//! Blocking, on `std::net`: no runtime, no TLS, one dependency besides the contract. It is for a
//! process on this machine or the local network; a client across the internet goes through a
//! proxy with TLS and wants a real HTTP stack. With the `tokio` feature, [`AsyncClient`] makes
//! the same calls from async code.

mod http;
mod sse;

#[cfg(feature = "tokio")]
mod nonblocking;

use std::time::Duration;

use serde_json::{json, Value};

pub use harnix_contract::handshake::{CONTRIB_API_VERSION, PROTOCOL_VERSION};
pub use http::Endpoint;
#[cfg(feature = "tokio")]
pub use nonblocking::{launch_link as launch_link_async, AsyncClient};
pub use sse::{EventStream, Face, StreamItem};

/// How long a call waits for its answer unless told otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// What went wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientError {
    pub kind: ErrorKind,
    /// The JSON-RPC error code for [`ErrorKind::Rpc`] and a refused handshake; the HTTP status
    /// when there was one and nothing better; 0 otherwise.
    pub code: i64,
    pub message: String,
    /// The JSON-RPC error's `data` (an application's own code, for one), or null.
    pub data: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// No answer: not listening, timed out, not HTTP.
    Transport,
    /// The handshake was refused: a wrong token, versions this process does not speak.
    Handshake,
    /// The credential is no longer known (the process restarted): connect again.
    Unauthenticated,
    /// The method answered with an error.
    Rpc,
}

impl ClientError {
    pub(crate) fn transport(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Transport,
            code: 0,
            message: message.into(),
            data: Value::Null,
        }
    }

    fn of(kind: ErrorKind, code: i64, message: impl Into<String>) -> Self {
        Self {
            kind,
            code,
            message: message.into(),
            data: Value::Null,
        }
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            ErrorKind::Rpc => write!(f, "{} ({})", self.message, self.code)?,
            _ => write!(f, "{:?}: {}", self.kind, self.message)?,
        }
        if let Some(code) = self.data.get("code").and_then(Value::as_str) {
            write!(f, " [{code}]")?;
        }
        Ok(())
    }
}

impl std::error::Error for ClientError {}

/// One handshake's worth of calls.
#[derive(Debug, Clone)]
pub struct Client {
    endpoint: Endpoint,
    token: String,
    credential: String,
    timeout: Duration,
}

impl Client {
    /// Hand the process token over and get a credential. The token is what the process wrote
    /// into its state directory (`<state>/token`), which makes this client the device `local`.
    pub fn connect(url: &str, token: &str) -> Result<Self, ClientError> {
        let endpoint = Endpoint::parse(url)?;
        let body = json!({
            "token": token,
            "protocol_version": PROTOCOL_VERSION,
            "contrib_api_version": CONTRIB_API_VERSION,
            "topology": "split_streams",
        });
        let response = http::post(
            &endpoint,
            "/handshake",
            None,
            body.to_string().as_bytes(),
            Duration::from_secs(10),
        )?;
        let text = String::from_utf8_lossy(&response.body);
        let answer: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(credential) = answer.get("credential").and_then(Value::as_str) {
            return Ok(Self {
                endpoint,
                token: token.to_string(),
                credential: credential.to_string(),
                timeout: DEFAULT_TIMEOUT,
            });
        }
        let (code, message) = match answer.get("error") {
            Some(e) => (
                e.get("code").and_then(Value::as_i64).unwrap_or(0),
                e.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
            ),
            None => (i64::from(response.status), text.trim().to_string()),
        };
        Err(ClientError::of(ErrorKind::Handshake, code, message))
    }

    /// How long [`Client::call`] waits from now on.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn url(&self) -> String {
        self.endpoint.url()
    }

    /// The process token this client connected with.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// One method; its `result`.
    pub fn call(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        self.call_within(method, params, self.timeout)
    }

    /// [`Client::call`] with a wait of its own, for a method known to be slow.
    pub fn call_within(&self, method: &str, params: Value, wait: Duration) -> Result<Value, ClientError> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let response = http::post(
            &self.endpoint,
            "/rpc",
            Some(&self.credential),
            body.to_string().as_bytes(),
            wait,
        )?;
        let answer: Value = serde_json::from_slice(&response.body).map_err(|e| {
            ClientError::of(
                ErrorKind::Transport,
                i64::from(response.status),
                format!("{method}: the answer is not JSON ({e})"),
            )
        })?;
        if let Some(error) = answer.get("error") {
            let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
            let message = error.get("message").and_then(Value::as_str).unwrap_or("").to_string();
            let kind = if code == i64::from(harnix_contract::codes::HANDSHAKE_REFUSED) {
                ErrorKind::Unauthenticated
            } else {
                ErrorKind::Rpc
            };
            return Err(ClientError {
                kind,
                code,
                message,
                data: error.get("data").cloned().unwrap_or(Value::Null),
            });
        }
        Ok(answer.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Open one of the two event streams. Opening it is the subscription; each face has one
    /// reader per credential. `idle` bounds how long one [`EventStream::next`] waits before it
    /// answers [`StreamItem::Idle`], so the caller can do other work between frames.
    pub fn events(&self, face: Face, idle: Duration) -> Result<EventStream, ClientError> {
        EventStream::open(&self.endpoint, &self.credential, face, idle)
    }
}

/// A one-time link that opens the interface signed in (`POST /auth/link`, with the process
/// token): the full URL.
pub fn launch_link(url: &str, token: &str) -> Result<String, ClientError> {
    let endpoint = Endpoint::parse(url)?;
    let response = http::post(&endpoint, "/auth/link", Some(token), b"", Duration::from_secs(10))?;
    let answer: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
    match answer.get("path").and_then(Value::as_str) {
        Some(path) => Ok(format!("{}{path}", endpoint.url())),
        None => Err(ClientError::of(
            if response.status == 401 { ErrorKind::Handshake } else { ErrorKind::Transport },
            i64::from(response.status),
            format!("/auth/link: {}", String::from_utf8_lossy(&response.body).trim()),
        )),
    }
}

#[cfg(test)]
mod tests;
