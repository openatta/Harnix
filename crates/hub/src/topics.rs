//! Host topics: named whole-value snapshots, optionally per parameter set.
//!
//! The hub's half of design §7.5. Who may subscribe to what is decided by
//! the authorizer before a subscribe reaches here; this keeps the current
//! value of every `(topic, params)` that has one, hands it to a subscriber
//! on arrival, and pushes each new value to the subscribers of exactly that
//! pair. A late subscriber therefore never needs a delta it missed.

use std::collections::{BTreeMap, HashMap, HashSet};

use harnix_contract::{Frame, RpcError};
use serde_json::{json, Value};

use crate::clients::ClientId;
use crate::notification;

#[derive(Default)]
pub(crate) struct Topics {
    /// Every topic that exists, and the parameter names it takes.
    known: BTreeMap<String, Vec<String>>,
    values: HashMap<(String, String), Value>,
    subscribers: HashMap<(String, String), HashSet<ClientId>>,
}

impl Topics {
    pub fn declare(&mut self, name: &str, params: Vec<String>) {
        self.known.insert(name.to_string(), params);
    }

    pub fn names(&self) -> Vec<String> {
        self.known.keys().cloned().collect()
    }

    /// The canonical key of one subscription: only the declared parameters,
    /// in a fixed order.
    pub fn key(&self, topic: &str, params: &Value) -> Result<(String, String), RpcError> {
        let Some(declared) = self.known.get(topic) else {
            return Err(RpcError::not_found(format!("there is no topic `{topic}`")));
        };
        let mut picked = BTreeMap::new();
        for name in declared {
            match params.get(name) {
                Some(v) if !v.is_null() => {
                    picked.insert(name.clone(), v.clone());
                }
                _ => {
                    return Err(RpcError::invalid_params(format!(
                        "topic `{topic}` needs the parameter `{name}`"
                    )))
                }
            }
        }
        Ok((topic.to_string(), serde_json::to_string(&picked).unwrap_or_default()))
    }

    pub fn subscribe(&mut self, client: ClientId, key: (String, String)) -> Value {
        let value = self.values.get(&key).cloned().unwrap_or(Value::Null);
        self.subscribers.entry(key).or_default().insert(client);
        value
    }

    pub fn unsubscribe(&mut self, client: ClientId, key: &(String, String)) {
        if let Some(set) = self.subscribers.get_mut(key) {
            set.remove(&client);
        }
    }

    pub fn forget(&mut self, client: ClientId) {
        for set in self.subscribers.values_mut() {
            set.remove(&client);
        }
    }

    /// Store a new value; who to tell and what to send them.
    pub fn publish(&mut self, key: (String, String), params: Value, value: Value) -> (Vec<ClientId>, Frame) {
        let frame = Frame::host(notification(
            "harnix.topic",
            json!({"topic": key.0, "params": params, "value": value}),
        ));
        let to = self
            .subscribers
            .get(&key)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        self.values.insert(key, value);
        (to, frame)
    }
}

impl crate::Hub {
    /// `harnix.subscribe { topic, params? }` → `{ value }`: the current value
    /// now, every later one as `harnix.topic`.
    pub(crate) async fn subscribe(
        &self,
        subject: &harnix_contract::Subject,
        params: Value,
    ) -> Result<Value, RpcError> {
        let client = crate::session::client_of(subject, &params)?;
        let (topic, args) = topic_params(&params)?;
        let mut guard = self.inner.lock().await;
        let key = guard.topics.key(&topic, &args)?;
        let value = guard.topics.subscribe(client, key);
        Ok(json!({"topic": topic, "value": value}))
    }

    pub(crate) async fn unsubscribe(
        &self,
        subject: &harnix_contract::Subject,
        params: Value,
    ) -> Result<Value, RpcError> {
        let client = crate::session::client_of(subject, &params)?;
        let (topic, args) = topic_params(&params)?;
        let mut guard = self.inner.lock().await;
        let key = guard.topics.key(&topic, &args)?;
        guard.topics.unsubscribe(client, &key);
        Ok(json!({"unsubscribed": true}))
    }

    /// Every declared topic, for the diagnostics page.
    pub async fn topic_names(&self) -> Vec<String> {
        self.inner.lock().await.topics.names()
    }
}

fn topic_params(params: &Value) -> Result<(String, Value), RpcError> {
    let topic = params
        .get("topic")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params("missing topic"))?
        .to_string();
    let args = params.get("params").cloned().unwrap_or(json!({}));
    Ok((topic, args))
}
