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

    /// Declare a topic that must not exist yet. `false` when one does.
    pub fn declare_new(&mut self, name: &str, params: Vec<String>) -> bool {
        if self.known.contains_key(name) {
            return false;
        }
        self.known.insert(name.to_string(), params);
        true
    }

    /// Take a topic out: it is no longer declared, its values are dropped,
    /// and its subscriptions end. Returns who was subscribed to which
    /// parameter set, so they can be told.
    pub fn withdraw(&mut self, name: &str) -> Vec<(Vec<ClientId>, Value)> {
        if self.known.remove(name).is_none() {
            return Vec::new();
        }
        self.values.retain(|(topic, _), _| topic != name);
        let keys: Vec<(String, String)> = self.subscribers.keys().filter(|(topic, _)| topic == name).cloned().collect();
        let mut told = Vec::new();
        for key in keys {
            let Some(set) = self.subscribers.remove(&key) else { continue };
            if set.is_empty() {
                continue;
            }
            let params = serde_json::from_str(&key.1).unwrap_or(json!({}));
            told.push((set.into_iter().collect(), params));
        }
        told
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

    pub fn has_subscribers(&self, key: &(String, String)) -> bool {
        self.subscribers.get(key).is_some_and(|s| !s.is_empty())
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

/// What a subscriber of a withdrawn topic is told, once per parameter set it
/// was subscribed to. After it, the topic is unknown: subscribing again is
/// refused until something declares it again, and nothing more is pushed.
pub const TOPIC_ENDED: &str = "harnix.topic_ended";

impl crate::Hub {
    /// Declare a topic after assembly — a runtime application's. Refused
    /// (`false`) for a name already declared, the hub's own included.
    pub async fn declare_runtime_topic(&self, name: &str, params: Vec<String>) -> bool {
        self.inner.lock().await.topics.declare_new(name, params)
    }

    /// Withdraw a topic declared at runtime. Each of its subscribers is sent
    /// `harnix.topic_ended {topic, params, reason}` for each parameter set it
    /// held; the topic's values are dropped. The hub's own topics cannot be
    /// withdrawn. Returns how many subscriptions were ended.
    pub async fn withdraw_topic(&self, name: &str, reason: &str) -> usize {
        if crate::HUB_TOPICS.iter().any(|(own, _)| *own == name) || name == crate::SPACES_TOPIC {
            return 0;
        }
        let mut guard = self.inner.lock().await;
        let told = guard.topics.withdraw(name);
        let mut count = 0;
        for (clients, params) in told {
            count += clients.len();
            let frame = Frame::host(notification(
                TOPIC_ENDED,
                json!({"topic": name, "params": params, "reason": reason}),
            ));
            guard.clients.send_many(&clients, &frame).await;
        }
        count
    }

    /// `harnix.subscribe { topic, params? }` → `{ value }`: the current value
    /// now, every later one as `harnix.topic`.
    ///
    /// The hub's own topics are computed now rather than taken from the last
    /// published value: nothing recomputes them while nobody watches.
    pub(crate) async fn subscribe(
        &self,
        subject: &harnix_contract::Subject,
        params: Value,
    ) -> Result<Value, RpcError> {
        let client = crate::session::client_of(subject, &params)?;
        let (topic, args) = topic_params(&params)?;
        let key = self.inner.lock().await.topics.key(&topic, &args)?;
        if crate::HUB_TOPICS.iter().any(|(name, _)| *name == topic) || topic == crate::SPACES_TOPIC {
            if topic == "harnix.sessions.tasks" {
                self.watch_tasks(&args).await;
            }
            let value = self.compute(&topic, &args).await?;
            let mut guard = self.inner.lock().await;
            // A value computed afresh is news to whoever already watches it, as any publish is;
            // the new subscriber has it in its reply.
            let (to, frame) = guard.topics.publish(key.clone(), args, value);
            guard.clients.send_many(&to, &frame).await;
            let value = guard.topics.subscribe(client, key);
            return Ok(json!({"topic": topic, "value": value}));
        }
        let mut guard = self.inner.lock().await;
        let value = guard.topics.subscribe(client, key);
        Ok(json!({"topic": topic, "value": value}))
    }

    /// Task events travel on the session's own stream, so watching a
    /// session's tasks needs the hub subscribed to it — if it is in memory.
    /// A cold session runs no tasks, and is not resumed just to be watched.
    async fn watch_tasks(&self, args: &Value) {
        let Ok(key) = self.key_of(args) else {
            return;
        };
        let subscribed = {
            let guard = self.inner.lock().await;
            guard.sessions.get(&key).is_some_and(|s| s.subscribed)
        };
        if subscribed {
            return;
        }
        if let Ok(resp) = self
            .call(&key.0, "session.subscribe", json!({"session_id": key.1}))
            .await
        {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key).or_default();
            state.subscribed = true;
            state.pending_prompts = resp
                .get("pending_prompts")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_withdrawn_topic_ends_every_subscription_and_forgets_its_values() {
        let mut topics = Topics::default();
        assert!(topics.declare_new("app.fake.ticks", vec!["job".into()]));
        assert!(!topics.declare_new("app.fake.ticks", Vec::new()), "declared twice");
        let a = topics.key("app.fake.ticks", &json!({"job": "a"})).unwrap();
        let b = topics.key("app.fake.ticks", &json!({"job": "b"})).unwrap();
        topics.subscribe(ClientId(1), a.clone());
        topics.subscribe(ClientId(2), a.clone());
        topics.subscribe(ClientId(3), b.clone());
        topics.publish(a.clone(), json!({"job": "a"}), json!(1));

        let mut told = topics.withdraw("app.fake.ticks");
        told.sort_by_key(|(_, p)| p.to_string());
        assert_eq!(told.len(), 2);
        let mut first = told[0].0.clone();
        first.sort_by_key(|c| c.0);
        assert_eq!(first, vec![ClientId(1), ClientId(2)]);
        assert_eq!(told[0].1, json!({"job": "a"}));
        assert_eq!(told[1].0, vec![ClientId(3)]);

        // Unknown from now on; nothing kept.
        assert!(topics.key("app.fake.ticks", &json!({"job": "a"})).is_err());
        assert!(!topics.has_subscribers(&a));
        assert!(topics.values.is_empty());
        assert!(topics.withdraw("app.fake.ticks").is_empty());
        // Declaring it again starts from nothing.
        assert!(topics.declare_new("app.fake.ticks", vec!["job".into()]));
        assert_eq!(topics.subscribe(ClientId(1), a), Value::Null);
    }
}
