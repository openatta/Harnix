//! What an application's VM is lent (`harnix_wasm::AppHost`), what answers its methods, and what
//! it is counted by.
//!
//! An application reaches the product, and the product reaches it, through the registration the
//! host makes for it ([`Platform::register`]). That registration is everything the application is
//! to the rest of the product:
//!
//! - **Its methods** (`[[method]]`, `app.<id>.<name>`) are declared with the principals the
//!   manifest names, so a call from the page, a module or another application is admitted and
//!   audited before [`Method::answer`] sees it. Its params are checked against the declared schema
//!   here, then the call queues for the resident backend's `handle`.
//! - **Its topics** (`[[topic]]`) are the only ones `host.publish` reaches, in the order the
//!   backend published them.
//! - **Its `hub` capability** is what `host.call` and `host.subscribe` may reach as
//!   `module:<id>`; the platform decides, grants and audits. Whatever it refuses is kept here too,
//!   for the system space.
//! - **Its spaces** (`[[space]]`) are listed with every other.
//!
//! Who may call an application's methods, then: whoever its manifest names — `device` (the
//! default) is every paired device and so the page, under its token, `module` every module (a
//! compiled-in module, or another application whose `hub` lists the method), `kernel` the product
//! itself — and always the application itself, whose tools reach their backend that way. A session
//! does not call methods: the agent uses the application's tools, and a tool that needs the
//! backend's state asks for it with `host.call`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use harnix_app_port::{CallAs, Failure, Platform, Publish, RpcError, Subject, Subscription};
use harnix_wasm::{AppHost, AppVm, CallFailure, WeakAppVm};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use harnix_app_port::codes;

/// Refused calls kept per application for the system space.
const REFUSALS_KEPT: usize = 20;
/// Method errors kept per application.
const ERRORS_KEPT: usize = 20;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What one application did, as the system space shows it.
#[derive(Default)]
pub struct Stats {
    inner: Mutex<Counted>,
}

#[derive(Default)]
struct Counted {
    /// Per method: calls, and how many of them failed.
    methods: BTreeMap<String, (u64, u64)>,
    /// `host.call`s it made, and how many were refused.
    hub_calls: u64,
    published: u64,
    errors: VecDeque<Value>,
    refused: VecDeque<Value>,
}

impl Stats {
    pub(crate) fn method(&self, name: &str, error: Option<&RpcError>) {
        let mut c = lock(&self.inner);
        let entry = c.methods.entry(name.to_string()).or_default();
        entry.0 += 1;
        if let Some(e) = error {
            entry.1 += 1;
            if c.errors.len() == ERRORS_KEPT {
                c.errors.pop_front();
            }
            c.errors.push_back(json!({"method": name, "code": e.code, "message": e.message, "at_ms": now_ms()}));
        }
    }

    /// A call the application made as itself (`host.call`, `host.subscribe`) and how it ended.
    fn hub(&self, what: &str, outcome: &Result<Value, RpcError>) {
        let mut c = lock(&self.inner);
        c.hub_calls += 1;
        if let Err(e) = outcome {
            if e.code == codes::REFUSED {
                if c.refused.len() == REFUSALS_KEPT {
                    c.refused.pop_front();
                }
                c.refused.push_back(json!({"method": what, "reason": e.message, "at_ms": now_ms()}));
            }
        }
    }

    pub fn report(&self) -> Value {
        let c = lock(&self.inner);
        let calls: u64 = c.methods.values().map(|(n, _)| n).sum();
        let errors: u64 = c.methods.values().map(|(_, e)| e).sum();
        json!({
            "calls": calls,
            "errors": errors,
            "methods": c.methods.iter().map(|(name, (n, e))| json!({"name": name, "calls": n, "errors": e})).collect::<Vec<_>>(),
            "hub_calls": c.hub_calls,
            "published": c.published,
            // Newest first.
            "recent_errors": c.errors.iter().rev().collect::<Vec<_>>(),
            "refused": c.refused.iter().rev().collect::<Vec<_>>(),
        })
    }
}

/// What the application's VM is lent: the platform behind `call`, `publish` and `subscribe`, and
/// the queue its subscriptions' values wait in until the VM has started.
pub struct Bridge {
    app: String,
    platform: Arc<dyn Platform>,
    /// `[[topic]]`: all `host.publish` may reach.
    topics: BTreeSet<String>,
    /// Publishes, in the order the backend made them, to one task that hands them to the platform.
    publish: mpsc::UnboundedSender<(String, Value)>,
    /// Topic values for the backend's `on-topic`, held until its VM has started.
    topic_values: mpsc::UnboundedSender<(String, Value)>,
    started: Mutex<Option<oneshot::Sender<WeakAppVm>>>,
    pub stats: Arc<Stats>,
}

impl Bridge {
    pub fn new(
        app: &str,
        topics: impl IntoIterator<Item = String>,
        platform: Arc<dyn Platform>,
    ) -> Arc<Self> {
        let (publish, mut publish_rx) = mpsc::unbounded_channel::<(String, Value)>();
        let (topic_values, mut values) = mpsc::unbounded_channel::<(String, Value)>();
        let (started, vm) = oneshot::channel::<WeakAppVm>();
        let stats = Arc::new(Stats::default());
        // Values that arrive while `init` is still running wait here; the VM queues them behind it.
        tokio::spawn(async move {
            let Ok(vm) = vm.await else { return };
            while let Some((topic, value)) = values.recv().await {
                let Some(vm) = vm.upgrade() else { break };
                vm.deliver_topic(&topic, &value);
            }
        });
        {
            let (app, platform, stats) = (app.to_string(), platform.clone(), stats.clone());
            tokio::spawn(async move {
                while let Some((topic, value)) = publish_rx.recv().await {
                    let publish = Publish {
                        app: app.clone(),
                        topic: topic.clone(),
                        value,
                    };
                    match platform.publish(publish).await {
                        Ok(()) => lock(&stats.inner).published += 1,
                        // Withdrawn under it: nothing more can be published.
                        Err(e) => tracing::debug!(%app, %topic, error = %e.message, "not published"),
                    }
                }
            });
        }
        Arc::new(Self {
            app: app.to_string(),
            platform,
            topics: topics.into_iter().collect(),
            publish,
            topic_values,
            started: Mutex::new(Some(started)),
            stats,
        })
    }

    /// A value on a topic it subscribed to, from the platform.
    pub fn deliver(&self, topic: String, value: Value) {
        let _ = self.topic_values.send((topic, value));
    }

    /// The VM has started: hand it what arrived meanwhile, and everything after.
    pub fn started(&self, vm: &AppVm) {
        if let Some(tx) = lock(&self.started).take() {
            let _ = tx.send(vm.downgrade());
        }
    }
}

#[async_trait]
impl AppHost for Bridge {
    async fn call(&self, _app: &str, method: &str, params: Value) -> Result<Value, String> {
        // On the same task, so a call that routes back into this backend is recognized by the VM
        // and refused rather than waiting for itself.
        let outcome = self
            .platform
            .call(CallAs {
                app: self.app.clone(),
                method: method.to_string(),
                params,
            })
            .await;
        self.stats.hub(method, &outcome);
        outcome.map_err(|e| e.message)
    }

    fn publish(&self, _app: &str, topic: &str, value: Value) -> Result<(), String> {
        if !self.topics.contains(topic) {
            return Err(format!(
                "`{topic}` is not a topic `{}` declared (`[[topic]]` in its manifest)",
                self.app
            ));
        }
        self.publish
            .send((topic.to_string(), value))
            .map_err(|_| "the application is stopping".to_string())
    }

    async fn subscribe(&self, _app: &str, topic: &str, params: &Value) -> Result<(), String> {
        let outcome = self
            .platform
            .subscribe(Subscription {
                app: self.app.clone(),
                topic: topic.to_string(),
                params: params.clone(),
            })
            .await;
        self.stats.hub(&format!("topic:{topic}"), &outcome);
        let answer = outcome.map_err(|e| e.message)?;
        // The value now, as any subscriber is given it; every later one comes from the platform.
        let value = &answer["value"];
        if !value.is_null() {
            let _ = self.topic_values.send((topic.to_string(), value.clone()));
        }
        Ok(())
    }

    async fn unsubscribe(&self, _app: &str, topic: &str, params: &Value) {
        let _ = self
            .platform
            .unsubscribe(Subscription {
                app: self.app.clone(),
                topic: topic.to_string(),
                params: params.clone(),
            })
            .await;
    }
}

/// One `[[method]]`, answered by the resident backend.
pub struct Method {
    pub app: String,
    pub name: String,
    pub schema: Option<jsonschema::Validator>,
}

/// What the backend's `handle` is told of who called: `{"subject": "device:d1", "kind", "id"}`.
fn caller_json(caller: &Subject) -> Value {
    let mut caller_value = serde_json::to_value(caller).unwrap_or(json!({}));
    if let Some(obj) = caller_value.as_object_mut() {
        obj.insert("subject".into(), json!(caller.label()));
    }
    caller_value
}

pub(crate) fn failure_of(failure: &CallFailure) -> Failure {
    match failure {
        CallFailure::TimedOut => Failure::TimedOut,
        CallFailure::Cancelled => Failure::Cancelled,
        CallFailure::Faulted(_) => Failure::Faulted,
        CallFailure::Unavailable(_) => Failure::Unavailable,
    }
}

impl Method {
    fn full(&self) -> String {
        format!("app.{}.{}", self.app, self.name)
    }

    /// Checked against its schema, then answered by `vm` (none: the application is not running).
    pub async fn answer(
        &self,
        vm: Option<&AppVm>,
        caller: &Subject,
        params: Value,
    ) -> Result<Value, RpcError> {
        let params = if params.is_null() { json!({}) } else { params };
        if let Some(schema) = &self.schema {
            let problems: Vec<String> = schema
                .iter_errors(&params)
                .take(5)
                .map(|e| {
                    let at = e.instance_path().to_string();
                    if at.is_empty() { e.to_string() } else { format!("{at}: {e}") }
                })
                .collect();
            if !problems.is_empty() {
                return Err(RpcError::invalid_params(format!(
                    "`{}`'s params do not match its schema: {}",
                    self.full(),
                    problems.join("; ")
                )));
            }
        }
        let Some(vm) = vm else {
            return Err(unavailable(&self.app, &self.full(), "the application is not running", "stopped"));
        };
        match vm.handle(&self.name, &params, &caller_json(caller)).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(e)) => Err(RpcError {
                code: e.code,
                message: e.message,
                data: e.data,
            }),
            Err(failure) => Err(unavailable(
                &self.app,
                &self.full(),
                &failure.to_string(),
                failure_of(&failure).as_str(),
            )),
        }
    }
}

/// The backend gave no answer: an internal error that says why, and `data.failure` for a client
/// that wants to tell a timeout from a crash from a stopped application.
pub(crate) fn unavailable(app: &str, method: &str, why: &str, failure: &str) -> RpcError {
    let mut e = RpcError::new(codes::INTERNAL_ERROR, format!("`{method}`: {why}"));
    e.data = Some(json!({"app": app, "failure": failure}));
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caller_is_named_for_the_backend() {
        assert_eq!(
            caller_json(&Subject::Device { id: "d1".into() }),
            json!({"kind": "device", "id": "d1", "subject": "device:d1"})
        );
        assert_eq!(caller_json(&Subject::Kernel)["subject"], json!("kernel"));
    }

    #[test]
    fn refusals_and_errors_are_kept_newest_first_and_bounded() {
        let stats = Stats::default();
        for n in 0..(REFUSALS_KEPT + 5) {
            stats.hub(&format!("m{n}"), &Err(RpcError::refused("no")));
        }
        stats.hub("ok", &Ok(json!(1)));
        stats.hub("broken", &Err(RpcError::new(codes::INTERNAL_ERROR, "x")));
        stats.method("notes.add", None);
        stats.method("notes.add", Some(&RpcError::invalid_params("bad")));
        let report = stats.report();
        assert_eq!(report["hub_calls"], json!(REFUSALS_KEPT + 7));
        let refused = report["refused"].as_array().unwrap();
        assert_eq!(refused.len(), REFUSALS_KEPT, "only refusals, and only so many");
        assert_eq!(refused[0]["method"], json!(format!("m{}", REFUSALS_KEPT + 4)));
        assert_eq!((report["calls"].clone(), report["errors"].clone()), (json!(2), json!(1)));
        assert_eq!(report["recent_errors"][0]["code"], json!(codes::INVALID_PARAMS));
    }
}
