//! Topics, from the server side: an extension's own to publish on, and any to watch as itself.

use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::context::ExtensionContext;
use crate::{internal_error, Rpc, RpcError};

/// An extension's topics.
pub struct Topics<'a> {
    cx: &'a ExtensionContext,
}

impl<'a> Topics<'a> {
    pub(crate) fn new(cx: &'a ExtensionContext) -> Self {
        Self { cx }
    }

    /// Publish the whole current value of its topic `<id>.<topic>` for these subscription
    /// parameters (`Value::Null` or `{}` for none). Only parameters it declared count, so
    /// `{job: 1, extra: 2}` is the value of `{job: 1}`.
    pub async fn publish(&self, topic: &str, params: Value, value: impl Serialize) -> Rpc<()> {
        let (name, params) = self.own(topic, params)?;
        let value = serde_json::to_value(value).map_err(|e| internal_error(e.to_string()))?;
        self.cx.ports().publish(&name, params, value).await;
        Ok(())
    }

    /// Say its computed topic `<id>.<topic>` may have changed for these parameters: computed
    /// again if anyone watches it, and pushed if it did.
    pub fn changed(&self, topic: &str, params: Value) -> Rpc<()> {
        let (name, params) = self.own(topic, params)?;
        self.cx.ports().changed(&name, params);
        Ok(())
    }

    /// Watch any topic it may subscribe to, by its full name, as itself: decided as
    /// `topic:<name>` like any subscription. The current value first, then every later one.
    pub async fn subscribe(&self, topic: &str, params: Value) -> Rpc<TopicStream> {
        self.cx.ports().subscribe(self.cx.subject(), topic, params).await
    }

    fn own(&self, topic: &str, params: Value) -> Rpc<(String, Value)> {
        let name = format!("{}.{topic}", self.cx.id());
        let Some(declared) = self.cx.own_topic(&name) else {
            return Err(RpcError::missing(format!("`{name}` is not a topic this extension declared")));
        };
        Ok((name, normalize(params, declared)?))
    }
}

/// Keep only the declared parameters; each must be there.
pub(crate) fn normalize(params: Value, declared: &[String]) -> Rpc<Value> {
    let mut out = serde_json::Map::new();
    let given = params.as_object().cloned().unwrap_or_default();
    for name in declared {
        match given.get(name) {
            Some(v) if !v.is_null() => {
                out.insert(name.clone(), v.clone());
            }
            _ => return Err(RpcError::invalid_params(format!("missing topic parameter `{name}`"))),
        }
    }
    Ok(Value::Object(out))
}

/// What a watched topic says.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum TopicValue {
    /// Its whole value now.
    Value(Value),
    /// The topic was withdrawn; nothing more will come.
    Ended { reason: String },
}

/// One subscription: its value when subscribed, then each later one. Dropped, it unsubscribes.
pub struct TopicStream {
    current: Option<Value>,
    rx: mpsc::Receiver<TopicValue>,
    on_drop: Option<Box<dyn FnOnce() + Send>>,
}

impl TopicStream {
    pub(crate) fn new(current: Value, rx: mpsc::Receiver<TopicValue>, on_drop: Box<dyn FnOnce() + Send>) -> Self {
        Self { current: Some(current), rx, on_drop: Some(on_drop) }
    }

    /// The next thing it says: first the value it had when subscribed. `None` once nothing more
    /// can come.
    pub async fn next(&mut self) -> Option<TopicValue> {
        if let Some(value) = self.current.take() {
            return Some(TopicValue::Value(value));
        }
        self.rx.recv().await
    }
}

impl Drop for TopicStream {
    fn drop(&mut self) {
        if let Some(f) = self.on_drop.take() {
            f();
        }
    }
}
