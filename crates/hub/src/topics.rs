//! Host topics: named whole-value snapshots, optionally per parameter set.
//!
//! The hub's half of host topics (concept §3.2). Who may subscribe to what is decided by
//! the authorizer before a subscribe reaches here; this keeps the current
//! value of every `(topic, params)` that has one, hands it to a subscriber
//! on arrival, and pushes each new value to the subscribers of exactly that
//! pair. A late subscriber therefore never needs a delta it missed.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use harnix_contract::sync::{lock, read, write};
use harnix_contract::{Frame, RpcError};
use harnix_kernel::TopicSource;
use serde_json::{json, Value};

use crate::clients::ClientId;
use crate::notification;

#[derive(Default)]
pub(crate) struct Topics {
    /// Every topic that exists, and the parameter names it takes.
    known: BTreeMap<String, Vec<String>>,
    values: HashMap<(String, String), Value>,
    subscribers: HashMap<(String, String), HashSet<ClientId>>,
    /// The subscribers held to one space's rows: they claimed it, and a value that lists
    /// sessions is cut down to that space's before they are told it.
    held: HashMap<(String, String), HashMap<ClientId, String>>,
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
        self.held.retain(|(topic, _), _| topic != name);
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

    /// Subscribe `client`, held to `space`'s rows when it claimed one. The value kept: the
    /// caller cuts it down for a held subscriber.
    pub fn subscribe(&mut self, client: ClientId, key: (String, String), space: Option<String>) -> Value {
        let value = self.values.get(&key).cloned().unwrap_or(Value::Null);
        match space {
            Some(space) => {
                self.held.entry(key.clone()).or_default().insert(client, space);
            }
            None => {
                if let Some(held) = self.held.get_mut(&key) {
                    held.remove(&client);
                }
            }
        }
        self.subscribers.entry(key).or_default().insert(client);
        value
    }

    pub fn unsubscribe(&mut self, client: ClientId, key: &(String, String)) {
        if let Some(set) = self.subscribers.get_mut(key) {
            set.remove(&client);
        }
        if let Some(held) = self.held.get_mut(key) {
            held.remove(&client);
        }
    }

    pub fn has_subscribers(&self, key: &(String, String)) -> bool {
        self.subscribers.get(key).is_some_and(|s| !s.is_empty())
    }

    pub fn forget(&mut self, client: ClientId) {
        for set in self.subscribers.values_mut() {
            set.remove(&client);
        }
        for held in self.held.values_mut() {
            held.remove(&client);
        }
    }

    /// Store a new value; who to tell and what to send each. A subscriber held to a space is sent
    /// what `within` makes of the value for that space, once per space.
    pub fn publish(
        &mut self,
        key: (String, String),
        params: Value,
        value: Value,
        within: &dyn Fn(&str, &Value, &str) -> Value,
    ) -> Vec<(Vec<ClientId>, Frame)> {
        let held = self.held.get(&key);
        let mut free: Vec<ClientId> = Vec::new();
        let mut by_space: BTreeMap<&str, Vec<ClientId>> = BTreeMap::new();
        for client in self.subscribers.get(&key).into_iter().flatten().copied() {
            match held.and_then(|h| h.get(&client)) {
                Some(space) => by_space.entry(space.as_str()).or_default().push(client),
                None => free.push(client),
            }
        }
        let frame = |value: &Value| {
            Frame::host(notification("harnix.topic", json!({"topic": key.0, "params": params, "value": value})))
        };
        let mut told = vec![(free, frame(&value))];
        for (space, clients) in by_space {
            told.push((clients, frame(&within(&key.0, &value, space))));
        }
        self.values.insert(key.clone(), value);
        told
    }
}

/// What a subscriber of a withdrawn topic is told, once per parameter set it
/// was subscribed to. After it, the topic is unknown: subscribing again is
/// refused until something declares it again, and nothing more is pushed.
pub const TOPIC_ENDED: &str = "harnix.topic_ended";

impl crate::Hub {
    /// Serve a computed topic: declared, and computed by `source` whenever someone subscribes
    /// and whenever it may have changed. For the kernel's own topics; none can be withdrawn.
    pub async fn serve_topic(&self, name: &str, params: Vec<String>, source: Arc<dyn TopicSource>) {
        write(&self.sources).insert(name.to_string(), source);
        self.inner.lock().await.topics.declare(name, params);
    }

    pub(crate) fn source(&self, topic: &str) -> Option<Arc<dyn TopicSource>> {
        read(&self.sources).get(topic).cloned()
    }

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
        if self.source(name).is_some() {
            return 0;
        }
        let mut guard = self.inner.lock().await;
        let told = guard.topics.withdraw(name);
        let mut count = 0;
        let mut out = Vec::new();
        for (clients, params) in told {
            count += clients.len();
            let frame = Frame::host(notification(
                TOPIC_ENDED,
                json!({"topic": name, "params": params, "reason": reason}),
            ));
            out.push(guard.clients.to_many(&clients, frame));
        }
        crate::send_after(guard, out).await;
        count
    }

    /// `harnix.topics.subscribe { topic, params? }` → `{ value }`: the current value
    /// now, every later one as `harnix.topic`.
    ///
    /// A computed topic is computed now rather than taken from the last
    /// published value: nothing recomputes it while nobody watches.
    pub(crate) async fn subscribe(
        self: &Arc<Self>,
        cx: &harnix_kernel::CallContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        let client = crate::session::client_of(cx)?;
        let (topic, args) = topic_params(&params)?;
        // One session's topic, by a caller claiming a space: the session must be that space's,
        // as for a call about it.
        if args.get("session_id").is_some() {
            self.check_space(&cx.subject, "harnix.topics.subscribe", &args)?;
        }
        let key = self.inner.lock().await.topics.key(&topic, &args)?;
        let held = self.held_space(&args);
        if let Some(source) = self.source(&topic) {
            source.on_subscribe(&args).await;
            if let Some(every) = source.every() {
                self.tick(&topic, &args, every);
            }
            let value = source.compute(&args).await?;
            let mut guard = self.inner.lock().await;
            // A value computed afresh is news to whoever already watches it, as any publish is;
            // the new subscriber has it in its reply.
            let told = guard.topics.publish((topic.clone(), key.1.clone()), args, value, &|t, v, s| self.within(t, v, s));
            let out: Vec<_> = told.into_iter().map(|(to, frame)| guard.clients.to_many(&to, frame)).collect();
            let value = guard.topics.subscribe(client, key, held.clone());
            let value = self.held_to(&topic, value, held.as_deref());
            crate::send_after(guard, out).await;
            return Ok(json!({"topic": topic, "value": value}));
        }
        let mut guard = self.inner.lock().await;
        let value = guard.topics.subscribe(client, key, held.clone());
        let value = self.held_to(&topic, value, held.as_deref());
        Ok(json!({"topic": topic, "value": value}))
    }

    /// The space a subscriber claimed in its topic's parameters, when that holds it to one: a
    /// claim of a space that reaches across spaces holds it to none. A space that does not exist
    /// (yet) holds it all the same, to no sessions: a package claims its own space whether or not
    /// it declared one.
    pub(crate) fn held_space(&self, args: &Value) -> Option<String> {
        let space = args.get("space").and_then(Value::as_str)?;
        (!lock(&self.spaces).crosses(space)).then(|| space.to_string())
    }

    /// `value` of `topic` as a subscriber held to `space` is told it: the sessions of that space.
    /// Only the topics that list sessions say which are whose; the others are told as they are.
    pub(crate) fn within(&self, topic: &str, value: &Value, space: &str) -> Value {
        let mut value = value.clone();
        let in_space = |row: &Value| match topic {
            "harnix.instances.sessions" => row.get("space").and_then(Value::as_str) == Some(space),
            _ => {
                let field = |k: &str| row.get(k).and_then(Value::as_str).map(str::to_string);
                field("instance")
                    .zip(field("session_id"))
                    .is_some_and(|key| self.space_of(&key) == space)
            }
        };
        if matches!(topic, "harnix.instances.sessions" | "harnix.sessions.status") {
            if let Some(Value::Array(rows)) = value.get_mut("sessions") {
                rows.retain(|row| in_space(row));
            }
        }
        value
    }

    fn held_to(&self, topic: &str, value: Value, space: Option<&str>) -> Value {
        match space {
            Some(space) => self.within(topic, &value, space),
            None => value,
        }
    }

    /// Compute a topic value again every `every`, from now on, if anyone watches it then: for a
    /// value that changes without anything to say so. One clock per value.
    fn tick(self: &Arc<Self>, topic: &str, params: &Value, every: std::time::Duration) {
        let marker: crate::views::Marker = (topic.to_string(), params.to_string());
        if !lock(&self.ticking).insert(marker) {
            return;
        }
        let hub = Arc::downgrade(self);
        let (topic, params) = (topic.to_string(), params.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                let Some(hub) = hub.upgrade() else { return };
                hub.refresh(&topic, params.clone());
            }
        });
    }

    /// Task events travel on the session's own stream, so watching a
    /// session's tasks needs the hub subscribed to it — if it is in memory.
    /// A cold session runs no tasks, and is not resumed just to be watched.
    pub(crate) async fn watch_tasks(&self, args: &Value) {
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
        let Ok(linked) = self.engine(&key.0) else { return };
        if let Ok(Some(subscribed)) = linked.ops().subscribe(&key.1).await {
            let mut guard = self.inner.lock().await;
            let state = guard.sessions.entry(key).or_default();
            state.subscribed = true;
            state.pending_prompts = subscribed.pending;
        }
    }

    pub(crate) async fn unsubscribe(
        &self,
        cx: &harnix_kernel::CallContext,
        params: Value,
    ) -> Result<Value, RpcError> {
        let client = crate::session::client_of(cx)?;
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
        assert!(topics.declare_new("fake.ticks", vec!["job".into()]));
        assert!(!topics.declare_new("fake.ticks", Vec::new()), "declared twice");
        let a = topics.key("fake.ticks", &json!({"job": "a"})).unwrap();
        let b = topics.key("fake.ticks", &json!({"job": "b"})).unwrap();
        topics.subscribe(ClientId(1), a.clone(), None);
        topics.subscribe(ClientId(2), a.clone(), None);
        topics.subscribe(ClientId(3), b.clone(), None);
        topics.publish(a.clone(), json!({"job": "a"}), json!(1), &|_, v, _| v.clone());

        let mut told = topics.withdraw("fake.ticks");
        told.sort_by_key(|(_, p)| p.to_string());
        assert_eq!(told.len(), 2);
        let mut first = told[0].0.clone();
        first.sort_by_key(|c| c.0);
        assert_eq!(first, vec![ClientId(1), ClientId(2)]);
        assert_eq!(told[0].1, json!({"job": "a"}));
        assert_eq!(told[1].0, vec![ClientId(3)]);

        // Unknown from now on; nothing kept.
        assert!(topics.key("fake.ticks", &json!({"job": "a"})).is_err());
        assert!(!topics.has_subscribers(&a));
        assert!(topics.values.is_empty());
        assert!(topics.withdraw("fake.ticks").is_empty());
        // Declaring it again starts from nothing.
        assert!(topics.declare_new("fake.ticks", vec!["job".into()]));
        assert_eq!(topics.subscribe(ClientId(1), a, None), Value::Null);
    }

    /// A subscriber held to a space is told the value as that space's rows, and one that is not
    /// is told it whole.
    #[test]
    fn a_held_subscriber_is_told_its_spaces_part_of_a_value() {
        let mut topics = Topics::default();
        topics.declare("harnix.sessions.status", Vec::new());
        let key = topics.key("harnix.sessions.status", &json!({})).unwrap();
        topics.subscribe(ClientId(1), key.clone(), Some("acme".into()));
        topics.subscribe(ClientId(2), key.clone(), None);
        let cut = |_: &str, v: &Value, space: &str| json!({"for": space, "was": v});
        let told = topics.publish(key.clone(), json!({}), json!([1, 2]), &cut);
        assert_eq!(told.len(), 2);
        assert_eq!(told[0].0, vec![ClientId(2)]);
        assert!(told[0].1.payload.contains("[1,2]") && !told[0].1.payload.contains("\"for\""), "{}", told[0].1.payload);
        assert_eq!(told[1].0, vec![ClientId(1)]);
        assert!(told[1].1.payload.contains("\"for\":\"acme\""), "{}", told[1].1.payload);
        // Subscribing again without a claim lifts the hold; unsubscribing drops it.
        topics.subscribe(ClientId(1), key.clone(), None);
        assert_eq!(topics.publish(key.clone(), json!({}), json!(3), &cut).len(), 1);
        topics.subscribe(ClientId(1), key.clone(), Some("acme".into()));
        topics.unsubscribe(ClientId(1), &key);
        assert_eq!(topics.publish(key, json!({}), json!(4), &cut).len(), 1);
    }
}
