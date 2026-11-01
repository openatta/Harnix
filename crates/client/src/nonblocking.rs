//! [`Client`] for async code: the same calls, each on tokio's blocking pool so a slow answer
//! never holds a runtime thread.

use std::time::Duration;

use serde_json::Value;

use crate::{Client, ClientError};

#[derive(Debug, Clone)]
pub struct AsyncClient(Client);

fn joined<T>(r: Result<Result<T, ClientError>, tokio::task::JoinError>) -> Result<T, ClientError> {
    r.unwrap_or_else(|e| Err(ClientError::transport(e.to_string())))
}

impl AsyncClient {
    pub async fn connect(url: &str, token: &str) -> Result<Self, ClientError> {
        let (url, token) = (url.to_string(), token.to_string());
        joined(tokio::task::spawn_blocking(move || Client::connect(&url, &token)).await).map(Self)
    }

    pub fn with_timeout(self, timeout: Duration) -> Self {
        Self(self.0.with_timeout(timeout))
    }

    pub fn blocking(&self) -> &Client {
        &self.0
    }

    pub fn url(&self) -> String {
        self.0.url()
    }

    pub fn token(&self) -> &str {
        self.0.token()
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        let (client, method) = (self.0.clone(), method.to_string());
        joined(tokio::task::spawn_blocking(move || client.call(&method, params)).await)
    }

    pub async fn call_within(&self, method: &str, params: Value, wait: Duration) -> Result<Value, ClientError> {
        let (client, method) = (self.0.clone(), method.to_string());
        joined(tokio::task::spawn_blocking(move || client.call_within(&method, params, wait)).await)
    }
}

/// [`crate::launch_link`] from async code.
pub async fn launch_link(url: &str, token: &str) -> Result<String, ClientError> {
    let (url, token) = (url.to_string(), token.to_string());
    joined(tokio::task::spawn_blocking(move || crate::launch_link(&url, &token)).await)
}
