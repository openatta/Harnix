//! The runtime-application host: what lets names into the process after it started.
//!
//! Everything else in this crate puts names in the authorization table once,
//! at assembly. This is the one path that does so later, and it is narrow on
//! purpose (`[apps] enabled`; `harnix_module::apps` for the contract):
//!
//! **Registering** one application is all or nothing, one at a time (a
//! registration or withdrawal holds `serial` throughout). Checked first —
//! nothing is written by a refusal, and every refusal is audited — the id
//! must be well formed and overlap no compiled-in module (by id, whether or
//! not it has names in the table) and no registered application; then the
//! table's own check (namespace, names taken). Then, in this order:
//!
//! 1. its topics are declared in the hub and its spaces added — invisible to
//!    a client so far, since subscribing is decided by the table;
//! 2. its handlers are bound in the gate above the hub — unreachable so far
//!    for the same reason, and binding never replaces a name already bound;
//! 3. its names go in the table, in one write. From here a call is admitted.
//!
//! A failure at any step undoes the earlier ones. So a name the table admits
//! always has its handler and its topic behind it.
//!
//! **Withdrawing** goes the other way: the names leave the table first (one
//! write; a call decided after it is unknown, `-32601`, like a name never
//! declared), then the handlers leave the gate (a call already running holds
//! its own reference and finishes; one admitted in the instant between is
//! answered not-found by the gate, never handed to the hub), then each
//! subscriber of its topics is sent `harnix.topic_ended` and the topics are
//! gone, then its spaces leave `harnix.spaces.list` (pushed on
//! `harnix.spaces`).
//!
//! Every name declared, withdrawn or refused is an audit entry under the
//! subject of the module that hosts the application, with the time.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use harnix_authz::{apps_overlap, topic, valid_app_id, Authorizer, Change};
use harnix_contract::{RpcError, SpaceDecl, Subject};
use harnix_hub::Hub;
use harnix_module::apps::AppRuntime;
use harnix_module::{AppSpec, AppSummary, AppTopic, AppWithdrawn, Principal};
use serde_json::Value;

use crate::gate::AppGate;

/// What an application's topics and spaces are kept in: the hub, or a
/// stand-in in the tests.
#[async_trait::async_trait]
pub(crate) trait Surfaces: Send + Sync {
    /// `false` when the topic exists already.
    async fn declare_topic(&self, name: &str, params: Vec<String>) -> bool;
    /// Subscriptions ended.
    async fn withdraw_topic(&self, name: &str, reason: &str) -> usize;
    async fn publish(&self, name: &str, params: Value, value: Value);
    fn add_space(&self, decl: SpaceDecl) -> Result<(), String>;
    fn remove_space(&self, id: &str) -> bool;
}

pub(crate) struct HubSurfaces(pub Arc<Hub>);

#[async_trait::async_trait]
impl Surfaces for HubSurfaces {
    async fn declare_topic(&self, name: &str, params: Vec<String>) -> bool {
        self.0.declare_runtime_topic(name, params).await
    }
    async fn withdraw_topic(&self, name: &str, reason: &str) -> usize {
        self.0.withdraw_topic(name, reason).await
    }
    async fn publish(&self, name: &str, params: Value, value: Value) {
        self.0.publish(name, params, value).await
    }
    fn add_space(&self, decl: SpaceDecl) -> Result<(), String> {
        self.0.add_space(decl)
    }
    fn remove_space(&self, id: &str) -> bool {
        self.0.remove_space(id)
    }
}

struct Live {
    summary: AppSummary,
    topics: Vec<AppTopic>,
}

pub(crate) struct Apps {
    authorizer: Arc<Authorizer>,
    gate: Arc<AppGate>,
    surfaces: Arc<dyn Surfaces>,
    /// Every compiled-in module's id, enabled or not: no application may
    /// take one, or nest under one, even where the module has no names in
    /// the table.
    reserved: BTreeSet<String>,
    live: Mutex<BTreeMap<String, Live>>,
    serial: tokio::sync::Mutex<()>,
}

/// Refused, audited under `by`, as `REFUSED` with the reason.
fn refuse(authorizer: &Authorizer, by: &Subject, name: &str, why: &'static str, message: String) -> RpcError {
    authorizer.audit().record_change(by, name, Change::Refuse, Some(why));
    RpcError::refused(message)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Keep only the declared parameters, so `{job: 1, extra: 2}` and `{job: 1}`
/// are the same value.
fn normalize(params: Value, declared: &[String]) -> Result<Value, RpcError> {
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

impl Apps {
    pub fn new(
        authorizer: Arc<Authorizer>,
        gate: Arc<AppGate>,
        surfaces: Arc<dyn Surfaces>,
        reserved: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            authorizer,
            gate,
            surfaces,
            reserved: reserved.into_iter().collect(),
            live: Mutex::new(BTreeMap::new()),
            serial: tokio::sync::Mutex::new(()),
        }
    }

    fn live(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Live>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn undo(&self, topics: &[String], spaces: &[String], methods: &[String]) {
        self.gate.unbind(methods);
        for id in spaces {
            self.surfaces.remove_space(id);
        }
        for name in topics {
            self.surfaces.withdraw_topic(name, "the application was not registered").await;
        }
    }
}

#[async_trait::async_trait]
impl AppRuntime for Apps {
    async fn register(&self, by: &str, spec: AppSpec) -> Result<AppSummary, RpcError> {
        let _serial = self.serial.lock().await;
        let host = Subject::Module { id: by.to_string() };
        let id = spec.id.clone();
        let all = format!("app.{id}.*");
        let az = &self.authorizer;
        if !valid_app_id(&id) {
            return Err(refuse(
                az,
                &host,
                &all,
                "not an application id",
                format!("`{id}` is not an application id: dot-separated lowercase segments, each starting with a letter"),
            ));
        }
        if let Some(module) = self.reserved.iter().find(|m| apps_overlap(&id, m)) {
            return Err(refuse(
                az,
                &host,
                &all,
                "the id overlaps a compiled-in module",
                format!("the application id `{id}` overlaps the compiled-in module `{module}`, whose names keep precedence"),
            ));
        }
        if let Some(other) = self.live().keys().find(|other| apps_overlap(&id, other)).cloned() {
            return Err(refuse(
                az,
                &host,
                &all,
                "the id overlaps a registered application",
                format!("the application id `{id}` overlaps `{other}`, which is registered"),
            ));
        }
        for space in &spec.spaces {
            if space.id != id && !space.id.starts_with(&format!("{id}.")) {
                return Err(refuse(
                    az,
                    &host,
                    &format!("space:{}", space.id),
                    "a space outside the application's namespace",
                    format!("the space `{}` is not `{id}` or `{id}.<name>`", space.id),
                ));
            }
        }

        let methods: Vec<String> = spec.methods.iter().map(|m| format!("app.{id}.{}", m.name)).collect();
        let topics: Vec<String> = spec.topics.iter().map(|t| format!("app.{id}.{}", t.name)).collect();
        let rows: Vec<(String, Vec<Principal>)> = spec
            .methods
            .iter()
            .zip(&methods)
            .map(|(m, name)| (name.clone(), m.principals.clone()))
            .chain(spec.topics.iter().zip(&topics).map(|(t, name)| (topic(name), t.principals.clone())))
            .collect();
        if let Err(e) = az.table().check_app(&id, &rows) {
            let name = match &e {
                harnix_authz::TableError::Taken(n) | harnix_authz::TableError::Namespace(n) => n.clone(),
                _ => all.clone(),
            };
            return Err(refuse(az, &host, &name, "the authorization table refused it", e.to_string()));
        }

        // 1. Topics and spaces: nobody can subscribe or claim a session's space by them through
        //    the table yet.
        let mut declared_topics = Vec::new();
        for (t, name) in spec.topics.iter().zip(&topics) {
            if !self.surfaces.declare_topic(name, t.params.clone()).await {
                self.undo(&declared_topics, &[], &[]).await;
                return Err(refuse(az, &host, &topic(name), "the topic exists", format!("there is already a topic `{name}`")));
            }
            declared_topics.push(name.clone());
        }
        let mut added_spaces = Vec::new();
        for space in &spec.spaces {
            if let Err(reason) = self.surfaces.add_space(space.clone()) {
                self.undo(&declared_topics, &added_spaces, &[]).await;
                return Err(refuse(az, &host, &format!("space:{}", space.id), "the space was refused", reason));
            }
            added_spaces.push(space.id.clone());
        }
        // 2. Handlers: unreachable until the table says otherwise.
        let mut bound = Vec::new();
        for (m, name) in spec.methods.iter().zip(&methods) {
            if !self.gate.bind_app(name, m.handler.clone()) {
                self.undo(&declared_topics, &added_spaces, &bound).await;
                return Err(refuse(az, &host, name, "the name is bound already", format!("`{name}` is served already")));
            }
            bound.push(name.clone());
        }
        // 3. The table, in one write. Audited there.
        if let Err(e) = az.declare_app(&host, &id, &rows) {
            self.undo(&declared_topics, &added_spaces, &bound).await;
            return Err(RpcError::refused(e.to_string()));
        }
        for space in &added_spaces {
            az.audit().record_change(&host, &format!("space:{space}"), Change::Declare, Some("declared by a runtime application"));
        }
        tracing::info!(app = %id, by, methods = methods.len(), topics = topics.len(), spaces = added_spaces.len(), "runtime application registered");
        let summary = AppSummary {
            id: id.clone(),
            by: by.to_string(),
            methods,
            topics,
            spaces: added_spaces,
            since_ms: now_ms(),
        };
        self.live().insert(
            id,
            Live {
                summary: summary.clone(),
                topics: spec.topics,
            },
        );
        Ok(summary)
    }

    async fn withdraw(&self, by: &str, id: &str) -> Result<AppWithdrawn, RpcError> {
        let _serial = self.serial.lock().await;
        let host = Subject::Module { id: by.to_string() };
        let summary = match self.live().get(id) {
            None => return Err(RpcError::invalid_params(format!("no application `{id}` is registered"))),
            Some(live) if live.summary.by != by => {
                return Err(RpcError::refused(format!(
                    "the application `{id}` was registered by the module `{}`; only it may withdraw it",
                    live.summary.by
                )))
            }
            Some(live) => live.summary.clone(),
        };
        // The table first: from here a call is decided as unknown.
        self.authorizer.withdraw_app(&host, id);
        self.gate.unbind(&summary.methods);
        let reason = format!("the application `{id}` was withdrawn");
        let mut ended = 0;
        for name in &summary.topics {
            ended += self.surfaces.withdraw_topic(name, &reason).await;
        }
        for space in &summary.spaces {
            self.surfaces.remove_space(space);
            self.authorizer.audit().record_change(
                &host,
                &format!("space:{space}"),
                Change::Withdraw,
                Some("withdrawn with its runtime application"),
            );
        }
        self.live().remove(id);
        tracing::info!(app = %id, by, subscriptions_ended = ended, "runtime application withdrawn");
        Ok(AppWithdrawn {
            id: id.to_string(),
            methods: summary.methods,
            topics: summary.topics,
            spaces: summary.spaces,
            subscriptions_ended: ended,
        })
    }

    async fn publish(&self, by: &str, id: &str, topic: &str, params: Value, value: Value) -> Result<(), RpcError> {
        let (name, params) = {
            let live = self.live();
            let Some(app) = live.get(id).filter(|a| a.summary.by == by) else {
                return Err(RpcError::invalid_params(format!("no application `{id}` is registered by this module")));
            };
            let Some(decl) = app.topics.iter().find(|t| t.name == topic) else {
                return Err(RpcError::not_found(format!("`app.{id}.{topic}` is not a topic this application declared")));
            };
            (format!("app.{id}.{topic}"), normalize(params, &decl.params)?)
        };
        self.surfaces.publish(&name, params, value).await;
        Ok(())
    }

    fn registered(&self) -> Vec<AppSummary> {
        self.live().values().map(|l| l.summary.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::RwLock;

    use harnix_authz::{Audit, MethodTable, Reach};
    use harnix_contract::{codes, Gate};
    use serde_json::json;

    use super::*;
    use crate::gate::Diagnostics;

    /// What sits below the gate. No `app.*` name may ever reach it.
    #[derive(Default)]
    struct Below {
        app_calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Gate for Below {
        async fn call(&self, _s: &Subject, method: &str, _p: Value) -> Result<Value, RpcError> {
            if method.starts_with("app.") {
                self.app_calls.fetch_add(1, Ordering::SeqCst);
            }
            Ok(json!({"below": method}))
        }
    }

    #[derive(Default)]
    struct Stub {
        topics: Mutex<BTreeMap<String, Vec<String>>>,
        /// Subscribers per topic, as if clients had subscribed.
        subscribers: Mutex<BTreeMap<String, usize>>,
        ended: Mutex<Vec<(String, String)>>,
        published: Mutex<Vec<(String, Value, Value)>>,
        spaces: Mutex<BTreeSet<String>>,
    }

    #[async_trait::async_trait]
    impl Surfaces for Stub {
        async fn declare_topic(&self, name: &str, params: Vec<String>) -> bool {
            let mut topics = self.topics.lock().unwrap();
            if topics.contains_key(name) {
                return false;
            }
            topics.insert(name.into(), params);
            true
        }
        async fn withdraw_topic(&self, name: &str, reason: &str) -> usize {
            self.topics.lock().unwrap().remove(name);
            self.ended.lock().unwrap().push((name.into(), reason.into()));
            self.subscribers.lock().unwrap().remove(name).unwrap_or(0)
        }
        async fn publish(&self, name: &str, params: Value, value: Value) {
            if self.topics.lock().unwrap().contains_key(name) {
                self.published.lock().unwrap().push((name.into(), params, value));
            }
        }
        fn add_space(&self, decl: SpaceDecl) -> Result<(), String> {
            if decl.id == "taken" || !self.spaces.lock().unwrap().insert(decl.id.clone()) {
                return Err(format!("there is already a space `{}`", decl.id));
            }
            Ok(())
        }
        fn remove_space(&self, id: &str) -> bool {
            self.spaces.lock().unwrap().remove(id)
        }
    }

    struct World {
        apps: Arc<Apps>,
        az: Arc<Authorizer>,
        below: Arc<Below>,
        stub: Arc<Stub>,
    }

    fn device() -> Subject {
        Subject::Device { id: "d1".into() }
    }

    /// One process: the hub's methods, and a compiled-in module `probe` whose `app.probe.echo`
    /// is in the table and bound.
    fn world() -> World {
        let below = Arc::new(Below::default());
        let audit = Arc::new(Audit::default());
        let diagnostics = Arc::new(Diagnostics {
            versions: json!({}),
            modules: RwLock::new(Vec::new()),
            unknown_sections: Vec::new(),
        });
        let gate = Arc::new(AppGate::new(below.clone(), diagnostics, audit.clone()));
        let table = MethodTable::new().allow("harnix.hello", Reach::Device).allow("app.probe.echo", Reach::Device);
        let compiled: Arc<dyn harnix_module::AppHandler> = Arc::new(Fixed(json!("the compiled-in probe")));
        assert!(gate.bind_app("app.probe.echo", compiled));
        let az = Arc::new(Authorizer::new(table, gate.clone(), audit));
        let stub = Arc::new(Stub::default());
        let apps = Arc::new(Apps::new(
            az.clone(),
            gate,
            stub.clone(),
            ["probe".to_string(), "quiet".to_string()],
        ));
        World { apps, az, below, stub }
    }

    struct Fixed(Value);

    #[async_trait::async_trait]
    impl harnix_module::AppHandler for Fixed {
        async fn call(&self, _caller: &Subject, _params: Value) -> Result<Value, RpcError> {
            Ok(self.0.clone())
        }
    }

    fn fake(id: &str) -> AppSpec {
        AppSpec::new(id)
            .method("echo", &[Principal::Device], |caller, params| async move {
                Ok(json!({"params": params, "caller": caller}))
            })
            .method("internal", &[Principal::Kernel], |_c, _p| async move { Ok(json!("kernel only")) })
            .topic("ticks", &["job"])
            .space(SpaceDecl::new(id))
    }

    #[tokio::test]
    async fn a_registered_method_is_admitted_routed_and_audited_then_withdrawn() {
        let w = world();
        let summary = w.apps.register("host", fake("fake")).await.unwrap();
        assert_eq!(summary.methods, vec!["app.fake.echo", "app.fake.internal"]);
        assert_eq!(summary.topics, vec!["app.fake.ticks"]);
        assert_eq!(summary.spaces, vec!["fake"]);

        let answer = w.az.call(&device(), "app.fake.echo", json!({"x": 1, "__client": 7})).await.unwrap();
        assert_eq!(answer["params"], json!({"x": 1}), "the transport's stamp is taken off");
        assert_eq!(answer["caller"], json!({"kind": "device", "id": "d1"}));
        // Only who it was declared for.
        let refused = w.az.call(&device(), "app.fake.internal", json!({})).await.unwrap_err();
        assert_eq!(refused.code, codes::REFUSED);
        assert_eq!(w.az.call(&Subject::Kernel, "app.fake.internal", json!({})).await.unwrap(), json!("kernel only"));
        assert!(w.az.reachable(&device()).contains(&"app.fake.echo".to_string()));

        // On the record: who declared what, and when.
        let declared: Vec<_> = w.az.audit().recent(50).into_iter().filter(|e| e.decision == "declare").collect();
        let names: BTreeSet<_> = declared.iter().map(|e| e.method.as_str()).collect();
        assert_eq!(names, BTreeSet::from(["app.fake.echo", "app.fake.internal", "topic:app.fake.ticks", "space:fake"]));
        assert!(declared.iter().all(|e| e.subject == "module:host" && e.at_ms.is_some()));
        let called = w.az.audit().recent(50).into_iter().find(|e| e.method == "app.fake.echo" && e.decision == "allow");
        assert_eq!(called.unwrap().subject, "device:d1");

        w.stub.subscribers.lock().unwrap().insert("app.fake.ticks".into(), 2);
        let withdrawn = w.apps.withdraw("host", "fake").await.unwrap();
        assert_eq!(withdrawn.subscriptions_ended, 2);
        assert_eq!(
            w.stub.ended.lock().unwrap().as_slice(),
            [("app.fake.ticks".to_string(), "the application `fake` was withdrawn".to_string())]
        );
        assert!(w.stub.spaces.lock().unwrap().is_empty());

        // Refused afterwards like a name that never existed, and never handed below.
        let gone = w.az.call(&device(), "app.fake.echo", json!({})).await.unwrap_err();
        assert_eq!(gone.code, codes::METHOD_NOT_FOUND);
        assert!(!w.az.reachable(&device()).contains(&"app.fake.echo".to_string()));
        assert_eq!(w.below.app_calls.load(Ordering::SeqCst), 0);
        let withdrew: Vec<_> = w.az.audit().recent(80).into_iter().filter(|e| e.decision == "withdraw").collect();
        assert_eq!(withdrew.len(), 4);
        assert!(w.apps.registered().is_empty());
        // And the id is free again.
        w.apps.register("host", fake("fake")).await.unwrap();
    }

    #[tokio::test]
    async fn collisions_are_refused_and_leave_nothing_behind() {
        let w = world();
        // A compiled-in module's id, with names in the table or not, or nested under one.
        for id in ["probe", "probe.sub", "quiet", "quiet.x"] {
            let e = w.apps.register("host", fake(id)).await.unwrap_err();
            assert_eq!(e.code, codes::REFUSED, "{id}");
            assert!(e.message.contains("compiled-in"), "{id}: {}", e.message);
        }
        // The compiled-in method is still the compiled-in one.
        assert_eq!(w.az.call(&device(), "app.probe.echo", json!({})).await.unwrap(), json!("the compiled-in probe"));

        // Twice, or nested in a registered one.
        w.apps.register("host", fake("acme.notes")).await.unwrap();
        for id in ["acme.notes", "acme", "acme.notes.pro"] {
            assert_eq!(w.apps.register("host", fake(id)).await.unwrap_err().code, codes::REFUSED, "{id}");
        }
        // A bad id, a bad method name, a duplicate name, a space outside the namespace, a space
        // that is taken: refused, and nothing of them was left declared, bound or listed.
        let bad = [
            AppSpec::new("Bad"),
            AppSpec::new("one").method("a b", &[Principal::Device], |_c, p| async move { Ok(p) }),
            AppSpec::new("two")
                .method("x", &[Principal::Device], |_c, p| async move { Ok(p) })
                .method("x", &[Principal::Device], |_c, p| async move { Ok(p) }),
            AppSpec::new("three").topic("t", &[]).space(SpaceDecl::new("elsewhere")),
            AppSpec::new("taken")
                .topic("t", &[])
                .method("m", &[Principal::Device], |_c, p| async move { Ok(p) })
                .space(SpaceDecl::new("taken")),
        ];
        for spec in bad {
            let id = spec.id.clone();
            assert_eq!(w.apps.register("host", spec).await.unwrap_err().code, codes::REFUSED, "{id}");
        }
        assert_eq!(w.stub.topics.lock().unwrap().keys().collect::<Vec<_>>(), ["app.acme.notes.ticks"]);
        assert_eq!(w.stub.spaces.lock().unwrap().iter().collect::<Vec<_>>(), ["acme.notes"]);
        assert_eq!(
            w.az.call(&device(), "app.taken.m", json!({})).await.unwrap_err().code,
            codes::METHOD_NOT_FOUND
        );
        assert_eq!(w.apps.registered().len(), 1);
        let refusals = w.az.audit().recent(100).into_iter().filter(|e| e.decision == "refuse").count();
        assert_eq!(refusals, 12);
        assert_eq!(w.below.app_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn only_the_registering_module_withdraws_or_publishes() {
        let w = world();
        w.apps.register("host", fake("fake")).await.unwrap();
        assert_eq!(w.apps.withdraw("other", "fake").await.unwrap_err().code, codes::REFUSED);
        assert!(w.apps.publish("other", "fake", "ticks", json!({"job": 1}), json!(1)).await.is_err());
        assert_eq!(w.apps.withdraw("host", "nobody").await.unwrap_err().code, codes::INVALID_PARAMS);

        // Its own topics only, with their parameters, normalized.
        assert!(w.apps.publish("host", "fake", "nope", json!({}), json!(1)).await.is_err());
        assert_eq!(
            w.apps.publish("host", "fake", "ticks", json!({}), json!(1)).await.unwrap_err().code,
            codes::INVALID_PARAMS
        );
        w.apps.publish("host", "fake", "ticks", json!({"job": "a", "extra": 1}), json!({"n": 1})).await.unwrap();
        assert_eq!(
            w.stub.published.lock().unwrap().as_slice(),
            [("app.fake.ticks".to_string(), json!({"job": "a"}), json!({"n": 1}))]
        );
        w.apps.withdraw("host", "fake").await.unwrap();
        assert!(w.apps.publish("host", "fake", "ticks", json!({"job": "a"}), json!(2)).await.is_err());
    }

    #[tokio::test]
    async fn a_call_in_flight_finishes_and_the_next_one_is_not_found() {
        let w = world();
        let (started_tx, started) = tokio::sync::oneshot::channel::<()>();
        let started_tx = Arc::new(Mutex::new(Some(started_tx)));
        let release = Arc::new(tokio::sync::Notify::new());
        let spec = {
            let release = release.clone();
            AppSpec::new("slow").method("wait", &[Principal::Device], move |_c, _p| {
                let release = release.clone();
                let started = started_tx.lock().unwrap().take();
                async move {
                    if let Some(tx) = started {
                        let _ = tx.send(());
                    }
                    release.notified().await;
                    Ok(json!("finished"))
                }
            })
        };
        w.apps.register("host", spec).await.unwrap();
        let az = w.az.clone();
        let in_flight = tokio::spawn(async move { az.call(&device(), "app.slow.wait", json!({})).await });
        started.await.unwrap();
        w.apps.withdraw("host", "slow").await.unwrap();
        assert_eq!(
            w.az.call(&device(), "app.slow.wait", json!({})).await.unwrap_err().code,
            codes::METHOD_NOT_FOUND
        );
        release.notify_one();
        assert_eq!(in_flight.await.unwrap().unwrap(), json!("finished"));
    }

    /// Registering, withdrawing and calling at once: every call is answered by the application
    /// or is not found — never refused for the wrong reason, never handed below the gate — and
    /// when it is over the table holds exactly what assembly put in.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn registering_withdrawing_and_calling_at_once() {
        let w = world();
        let rows_before = w.az.table().len();
        let mut tasks = Vec::new();
        for n in 0..4 {
            let apps = w.apps.clone();
            tasks.push(tokio::spawn(async move {
                // Two hosts contend for each id; whichever registers, only it withdraws.
                let id = format!("race{}", n % 2);
                let host = format!("host{n}");
                for _ in 0..200 {
                    if apps.register(&host, fake(&id)).await.is_ok() {
                        tokio::task::yield_now().await;
                        apps.withdraw(&host, &id).await.unwrap();
                    }
                }
            }));
        }
        for n in 0..4 {
            let az = w.az.clone();
            tasks.push(tokio::spawn(async move {
                for _ in 0..2000 {
                    let method = format!("app.race{}.echo", n % 2);
                    match az.call(&device(), &method, json!({"n": n})).await {
                        Ok(v) => assert_eq!(v["params"]["n"], json!(n)),
                        Err(e) => assert_eq!(e.code, codes::METHOD_NOT_FOUND, "{}", e.message),
                    }
                    // The compiled-in method never moves.
                    assert_eq!(az.call(&device(), "app.probe.echo", json!({})).await.unwrap(), json!("the compiled-in probe"));
                }
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(w.below.app_calls.load(Ordering::SeqCst), 0);
        assert_eq!(w.az.table().len(), rows_before);
        assert!(w.apps.registered().is_empty());
        assert!(w.stub.topics.lock().unwrap().is_empty());
        assert!(w.stub.spaces.lock().unwrap().is_empty());
    }
}
