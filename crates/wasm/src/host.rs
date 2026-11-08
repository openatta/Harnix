//! The `host` interface a component imports.
//!
//! Every function here is a place a component asks for something it cannot
//! do itself, so every one is a place a capability is checked. The pattern is
//! uniform on purpose: check the declaration, and on a miss give the
//! component an error naming the capability that would have allowed it —
//! silently returning nothing leaves an application author debugging the
//! wrong thing. Only what passes reaches the host's services.

use crate::bindings::harnix::app::host::Host;
use crate::bindings::harnix::app::types::HttpResponse;
use crate::state::{AppState, Role};
use crate::HttpRequest;
use serde_json::Value;
use std::time::Duration;

/// `types` declares only shapes, so its generated trait has no methods;
/// implementing it is how the linker learns they are accounted for.
impl crate::bindings::harnix::app::types::Host for AppState {}

fn parse(what: &str, json: &str) -> Result<Value, String> {
    serde_json::from_str(json).map_err(|e| format!("{what} is not valid JSON: {e}"))
}

impl AppState {
    fn backend_only(&self, function: &str) -> Result<(), String> {
        match self.role {
            Role::Backend => Ok(()),
            Role::Transient => Err(format!(
                "`{function}` is for the backend: a tool call or event instance ends when it \
                 returns, and what it set up would have nowhere to go"
            )),
        }
    }
}

impl Host for AppState {
    async fn log(&mut self, level: String, msg: String) {
        let app = &self.shared.app;
        match level.as_str() {
            "error" => tracing::error!(app = %app, "{msg}"),
            "warn" => tracing::warn!(app = %app, "{msg}"),
            "debug" | "trace" => tracing::debug!(app = %app, "{msg}"),
            _ => tracing::info!(app = %app, "{msg}"),
        }
    }

    async fn now_ms(&mut self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    async fn config(&mut self) -> String {
        self.shared.config_json.clone()
    }

    async fn progress(&mut self, call_id: String, text: String) {
        if let Some(sink) = &self.progress {
            sink.on_progress(&call_id, &text);
        }
    }

    async fn http_request(
        &mut self,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    ) -> Result<HttpResponse, String> {
        if !self.shared.caps.allows_url(&url) {
            // Names the host, never the URL: the component may have built it
            // from a secret, and this string usually travels on into a tool
            // result and the transcript.
            let refused =
                crate::host_of(&url).unwrap_or_else(|| "(not an http(s) URL)".to_string());
            return Err(format!(
                "network access to `{refused}` is not in this application's `net` capability"
            ));
        }
        let request = HttpRequest {
            method,
            url,
            headers,
            body,
        };
        let response = self
            .shared
            .services
            .http
            .fetch(&self.shared.app, request)
            .await?;
        Ok(HttpResponse {
            status: response.status,
            headers: response.headers,
            body: response.body,
        })
    }

    async fn secret(&mut self, key: String) -> Option<String> {
        if !self.shared.caps.allows_env(&key) {
            tracing::warn!(app = %self.shared.app, key = %key,
                "an application asked for an environment variable it did not declare");
            return None;
        }
        self.shared.services.host.secret(&self.shared.app, &key)
    }

    async fn kv_get(&mut self, key: String) -> Option<Vec<u8>> {
        match self.shared.services.kv.get(&key) {
            Ok(value) => value,
            Err(e) => {
                tracing::warn!(app = %self.shared.app, error = %e, "kv-get failed");
                None
            }
        }
    }

    async fn kv_set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        let quota = self.shared.caps.storage_bytes;
        self.shared
            .services
            .kv
            .put(&key, Some(&value), quota)
            .map_err(|e| e.to_string())
    }

    async fn kv_delete(&mut self, key: String) {
        // A delete never grows the namespace, so the quota cannot refuse it.
        if let Err(e) = self.shared.services.kv.put(&key, None, 0) {
            tracing::warn!(app = %self.shared.app, error = %e, "kv-delete failed");
        }
    }

    async fn call(&mut self, method: String, params_json: String) -> Result<String, String> {
        let params = parse("params-json", &params_json)?;
        let result = self
            .shared
            .services
            .host
            .call(&self.shared.app, &method, params)
            .await?;
        Ok(result.to_string())
    }

    async fn publish(&mut self, topic: String, value_json: String) -> Result<(), String> {
        let value = parse("value-json", &value_json)?;
        self.shared
            .services
            .host
            .publish(&self.shared.app, &topic, value)
    }

    async fn subscribe(&mut self, topic: String, params_json: String) -> Result<(), String> {
        self.backend_only("subscribe")?;
        let params = parse("params-json", &params_json)?;
        self.shared.subscribe(&topic, params).await
    }

    async fn unsubscribe(&mut self, topic: String, params_json: String) {
        if self.role != Role::Backend {
            return;
        }
        if let Ok(params) = parse("params-json", &params_json) {
            self.shared.unsubscribe(&topic, params).await;
        }
    }

    async fn set_timer(&mut self, id: String, after_ms: u64) -> Result<(), String> {
        self.backend_only("set-timer")?;
        if id.is_empty() || id.len() > 128 {
            return Err("a timer id is 1 to 128 bytes".into());
        }
        self.shared.arm_timer(id, Duration::from_millis(after_ms))
    }

    async fn cancel_timer(&mut self, id: String) {
        if self.role == Role::Backend {
            self.shared.cancel_timer(&id);
        }
    }
}
