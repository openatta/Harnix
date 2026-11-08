//! An application wired into the process: what its VM is lent (`harnix_wasm::AppHost`), what
//! answers its methods, and what it is counted by.
//!
//! An application reaches the process, and the process reaches it, through the registration the
//! `apps` module makes for it with the kernel (`harnix_module::AppHost`). That one registration is
//! everything the application is to the rest of Harnix:
//!
//! - **Its methods** (`[[method]]`, `app.<id>.<name>`) are rows in the authorization table with the
//!   principals the manifest declared, so a call from the page, a module or another application
//!   is admitted and audited before [`Method`] sees it. Its params are checked against the
//!   declared schema here, then the call queues for the resident backend's `handle`.
//! - **Its topics** (`[[topic]]`) are the only ones `host.publish` reaches, in the order the
//!   backend published them.
//! - **Its `hub` capability** is what `host.call` and `host.subscribe` may reach as
//!   `module:<id>`; the kernel decides, grants and audits ([`harnix_module::AppHandle::call`]).
//!   Whatever it refuses is kept here too, for the system space.
//! - **Its spaces** (`[[space]]`) are listed with every other.
//!
//! Who may call an application's methods, then: whoever its manifest names — `device` (the
//! default) is every paired device and so the page, under its token, `module` every module (a
//! compiled-in module, or another application whose `hub` lists the method), `kernel` this binary —
//! and always the application itself, whose tools reach their backend that way. A session does
//! not call methods: the agent uses the application's tools, and a tool that needs the backend's
//! state asks for it with `host.call`.
//!
//! Nothing here holds the VM strongly: the registry's entry does, so stopping the application is
//! what ends it, however many handlers and forwarders the kernel is still holding on to.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use harnix_contract::{codes, RpcError, Subject};
use harnix_module::{AppHandle, AppHandler, AppTopicSink};
use harnix_wasm::{AppHost, CallFailure, WeakAppVm};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

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

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
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
    fn method(&self, name: &str, error: Option<&RpcError>) {
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

/// The VM, once it has started: what the kernel's handlers and the topic forwarder reach it by.
#[derive(Default)]
pub struct VmSlot(OnceLock<WeakAppVm>);

impl VmSlot {
    pub fn set(&self, vm: WeakAppVm) {
        let _ = self.0.set(vm);
    }
}

/// What the application's VM is lent: the kernel registration behind `call`, `publish` and
/// `subscribe`.
pub struct Bridge {
    app: String,
    handle: OnceLock<AppHandle>,
    /// `[[topic]]`: all `host.publish` may reach.
    topics: BTreeSet<String>,
    /// Publishes, in the order the backend made them, to one task that hands them to the kernel.
    publish: mpsc::UnboundedSender<(String, Value)>,
    publish_rx: Mutex<Option<mpsc::UnboundedReceiver<(String, Value)>>>,
    /// Topic values for the backend's `on-topic`, held until its VM has started.
    topic_values: mpsc::UnboundedSender<(String, Value)>,
    started: Mutex<Option<oneshot::Sender<WeakAppVm>>>,
    pub stats: Arc<Stats>,
}

impl Bridge {
    pub fn new(app: &str, topics: impl IntoIterator<Item = String>) -> Arc<Self> {
        let (publish, publish_rx) = mpsc::unbounded_channel();
        let (topic_values, mut values) = mpsc::unbounded_channel::<(String, Value)>();
        let (started, vm) = oneshot::channel::<WeakAppVm>();
        // Values that arrive while `init` is still running wait here; the VM queues them behind it.
        tokio::spawn(async move {
            let Ok(vm) = vm.await else { return };
            while let Some((topic, value)) = values.recv().await {
                let Some(vm) = vm.upgrade() else { break };
                vm.deliver_topic(&topic, &value);
            }
        });
        Arc::new(Self {
            app: app.to_string(),
            handle: OnceLock::new(),
            topics: topics.into_iter().collect(),
            publish,
            publish_rx: Mutex::new(Some(publish_rx)),
            topic_values,
            started: Mutex::new(Some(started)),
            stats: Arc::new(Stats::default()),
        })
    }

    /// Where the hub's values of what it subscribed to go.
    pub fn sink(&self) -> Arc<dyn AppTopicSink> {
        Arc::new(TopicValues(self.topic_values.clone()))
    }

    /// Registered: `call`, `publish` and `subscribe` go through this from now on.
    pub fn bind(&self, handle: AppHandle) {
        if let Some(mut rx) = lock(&self.publish_rx).take() {
            let handle = handle.clone();
            let stats = self.stats.clone();
            tokio::spawn(async move {
                while let Some((topic, value)) = rx.recv().await {
                    match handle.publish(&topic, json!({}), value).await {
                        Ok(()) => lock(&stats.inner).published += 1,
                        // Withdrawn under it: nothing more can be published.
                        Err(e) => tracing::debug!(app = %handle.id(), %topic, error = %e.message, "not published"),
                    }
                }
            });
        }
        let _ = self.handle.set(handle);
    }

    /// The VM has started: hand it what arrived meanwhile, and everything after.
    pub fn started(&self, vm: WeakAppVm) {
        if let Some(tx) = lock(&self.started).take() {
            let _ = tx.send(vm);
        }
    }

    fn handle(&self) -> Result<&AppHandle, String> {
        self.handle.get().ok_or_else(|| {
            format!("`{}` is not registered with this process's application host", self.app)
        })
    }
}

/// The hub's frames for an application's subscriptions, into its backend.
struct TopicValues(mpsc::UnboundedSender<(String, Value)>);

impl AppTopicSink for TopicValues {
    fn deliver(&self, topic: &str, _params: &Value, value: &Value) {
        let _ = self.0.send((topic.to_string(), value.clone()));
    }

    fn ended(&self, topic: &str, _params: &Value, reason: &str) {
        tracing::info!(%topic, %reason, "a topic an application subscribed to ended");
    }
}

#[async_trait]
impl AppHost for Bridge {
    async fn call(&self, _app: &str, method: &str, params: Value) -> Result<Value, String> {
        let handle = self.handle()?;
        // On the same task, so a call that routes back into this backend is recognized by the VM
        // and refused rather than waiting for itself.
        let outcome = handle.call(method, params).await;
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
        let handle = self.handle()?;
        let outcome = handle.subscribe(topic, params.clone()).await;
        self.stats.hub(&harnix_authz_topic(topic), &outcome);
        let answer = outcome.map_err(|e| e.message)?;
        // The value now, as any subscriber is given it; every later one comes from the hub.
        let value = &answer["value"];
        if !value.is_null() {
            let _ = self.topic_values.send((topic.to_string(), value.clone()));
        }
        Ok(())
    }

    async fn unsubscribe(&self, _app: &str, topic: &str, params: &Value) {
        if let Ok(handle) = self.handle() {
            let _ = handle.unsubscribe(topic, params.clone()).await;
        }
    }
}

/// How a topic is named in the authorization table and in its refusals.
fn harnix_authz_topic(topic: &str) -> String {
    format!("topic:{topic}")
}

/// One `[[method]]`, answered by the resident backend.
pub struct Method {
    pub app: String,
    pub name: String,
    pub schema: Option<jsonschema::Validator>,
    pub vm: Arc<VmSlot>,
    pub stats: Arc<Stats>,
}

/// What the backend's `handle` is told of who called: `{"subject": "device:d1", "kind", "id"}`.
fn caller_json(caller: &Subject) -> Value {
    let mut caller_value = serde_json::to_value(caller).unwrap_or(json!({}));
    if let Some(obj) = caller_value.as_object_mut() {
        obj.insert("subject".into(), json!(caller.label()));
    }
    caller_value
}

impl Method {
    fn full(&self) -> String {
        format!("app.{}.{}", self.app, self.name)
    }

    async fn answer(&self, caller: &Subject, params: Value) -> Result<Value, RpcError> {
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
        let Some(vm) = self.vm.0.get().and_then(WeakAppVm::upgrade) else {
            return Err(unavailable(&self.app, &self.full(), "the application is not running", "stopped"));
        };
        match vm.handle(&self.name, &params, &caller_json(caller)).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(e)) => Err(RpcError {
                code: e.code,
                message: e.message,
                data: e.data,
            }),
            Err(failure) => {
                let kind = match &failure {
                    CallFailure::TimedOut => "timed_out",
                    CallFailure::Cancelled => "cancelled",
                    CallFailure::Faulted(_) => "faulted",
                    CallFailure::Unavailable(_) => "unavailable",
                };
                Err(unavailable(&self.app, &self.full(), &failure.to_string(), kind))
            }
        }
    }
}

/// The backend gave no answer: an internal error that says why, and `data.failure` for a client
/// that wants to tell a timeout from a crash from a stopped application.
fn unavailable(app: &str, method: &str, why: &str, failure: &str) -> RpcError {
    let mut e = RpcError::new(codes::INTERNAL_ERROR, format!("`{method}`: {why}"));
    e.data = Some(json!({"app": app, "failure": failure}));
    e
}

#[async_trait]
impl AppHandler for Method {
    async fn call(&self, caller: &Subject, params: Value) -> Result<Value, RpcError> {
        let outcome = self.answer(caller, params).await;
        self.stats.method(&self.name, outcome.as_ref().err());
        outcome
    }
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
