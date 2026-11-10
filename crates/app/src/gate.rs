//! What sits behind the authorizer: module methods first, then the hub.
//!
//! The hub knows nothing about modules — it does not depend on the module
//! contract, which is what keeps the dependency graph one-way. So the
//! application answers module methods here, above it, and hands everything
//! else down. A module method cannot shadow a hub method: both are rows in
//! one authorization table, and the table refuses a name already taken.
//!
//! Also here: the module's view of the process ([`ProcessKernel`]), and the
//! route decision transport asks for.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock, Weak};

use harnix_authz::{Authorizer, MethodTable, Rule};
use harnix_contract::sync::{read, write};
use harnix_contract::{Gate, RpcError, Subject};
use harnix_hub::Hub;
use harnix_module::{
    AppHandler, CallContext, Handler, InstanceHandle, Kernel, Listen, ModuleContext, SessionRef, Tap,
};
use serde_json::{json, Value};

use crate::modules::ModuleStatus;

/// What answers one method above the hub.
#[derive(Clone)]
enum Bound {
    /// A compiled-in module's, with that module's context.
    Module(Arc<dyn Handler>, ModuleContext),
    /// A runtime application's (`crate::apps`).
    App(Arc<dyn AppHandler>),
}

pub(crate) struct AppGate {
    hub: Arc<dyn Gate>,
    audit: Arc<harnix_authz::Audit>,
    methods: RwLock<HashMap<String, Bound>>,
    pub(crate) diagnostics: Arc<Diagnostics>,
}

impl AppGate {
    pub fn new(hub: Arc<dyn Gate>, diagnostics: Arc<Diagnostics>, audit: Arc<harnix_authz::Audit>) -> Self {
        Self {
            hub,
            audit,
            methods: RwLock::new(HashMap::new()),
            diagnostics,
        }
    }

    pub fn bind(&self, name: &str, handler: Arc<dyn Handler>, cx: ModuleContext) {
        write(&self.methods)
            .insert(name.to_string(), Bound::Module(handler, cx));
    }

    /// Bind a runtime application's method. Never replaces anything: a name
    /// already bound is refused (`false`), whoever bound it.
    pub fn bind_app(&self, name: &str, handler: Arc<dyn AppHandler>) -> bool {
        let mut methods = write(&self.methods);
        if methods.contains_key(name) {
            return false;
        }
        methods.insert(name.to_string(), Bound::App(handler));
        true
    }

    pub fn unbind(&self, names: &[String]) {
        let mut methods = write(&self.methods);
        for name in names {
            methods.remove(name);
        }
    }

    fn bound(&self, name: &str) -> Option<Bound> {
        read(&self.methods)
            .get(name)
            .cloned()
    }
}

#[async_trait::async_trait]
impl Gate for AppGate {
    async fn call(&self, subject: &Subject, method: &str, mut params: Value) -> Result<Value, RpcError> {
        match method {
            "harnix.contributions" => return Ok(self.diagnostics.contributions()),
            "harnix.diagnostics" => return Ok(self.diagnostics.report()),
            // The decisions the authorizer recorded, newest first: what the system space shows as
            // its audit, and what a test reads instead of transcript files.
            "harnix.audit" => {
                let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(200).min(2048) as usize;
                return Ok(json!({"entries": self.audit.recent(limit)}));
            }
            _ => {}
        }
        if let Some(bound) = self.bound(method) {
            // The transport's stamp is for the hub's watching methods; a
            // module's typed params should not have to know it exists.
            if let Some(obj) = params.as_object_mut() {
                obj.remove("__client");
            }
            // The handler is a clone out of the map: withdrawing the method
            // while this runs does not cut it short.
            return match bound {
                Bound::Module(handler, cx) => handler.call(CallContext::new(subject.clone(), cx), params).await,
                Bound::App(handler) => handler.call(subject, params).await,
            };
        }
        // The hub answers no `app.*` name. One that reached here with nothing
        // bound was withdrawn between admission and now; it goes no further
        // down than a name that never existed.
        if method.starts_with("app.") {
            return Err(RpcError::not_found(format!("method `{method}` is not served here (any more)")));
        }
        self.hub.call(subject, method, params).await
    }
}

/// The process, as a module's context sees it.
pub(crate) struct ProcessKernel {
    pub gate: OnceLock<Arc<dyn Gate>>,
    /// The runtime-application host, set only with `[apps] enabled`.
    pub apps: OnceLock<Arc<dyn harnix_module::apps::AppRuntime>>,
    /// The modules it is handed to: the `apps` built-in, and whatever the
    /// application named with [`crate::AppBuilder::app_host`] (a test
    /// application's fake host).
    pub app_hosts: std::collections::BTreeSet<String>,
    pub hub: Arc<Hub>,
    /// Where the listener is, once it is bound (ipdrun-14).
    pub listen: tokio::sync::watch::Sender<Option<Listen>>,
}

impl ProcessKernel {
    fn gate(&self) -> Result<Arc<dyn Gate>, RpcError> {
        self.gate.get().cloned().ok_or_else(|| {
            RpcError::new(
                harnix_contract::codes::INTERNAL_ERROR,
                "the admission point is not up yet",
            )
        })
    }
}

#[async_trait::async_trait]
impl Kernel for ProcessKernel {
    async fn call(&self, subject: &Subject, method: &str, params: Value) -> Result<Value, RpcError> {
        self.gate()?.call(subject, method, params).await
    }

    async fn publish(&self, topic: &str, params: Value, value: Value) {
        self.hub.publish(topic, params, value).await;
    }

    fn instance(&self, id: &str) -> Option<InstanceHandle> {
        let facts = self.hub.facts(id)?;
        Some(InstanceHandle {
            id: id.to_string(),
            recordings_root: facts
                .recordings_root
                .clone()
                .unwrap_or_else(|| facts.data_root.join("recordings")),
            data_root: facts.data_root,
            history: facts.history,
            active_scenes: facts.active_scenes,
            model: facts.model.unwrap_or_default(),
        })
    }

    /// A tap is a client of the hub like any other, registered as a watcher
    /// by the same `harnix.attach` — through the admission point, as the
    /// module, so the same table decides whether it may.
    async fn tap(&self, subject: &Subject, session: &SessionRef, capacity: usize) -> Result<Tap, RpcError> {
        let gate = self.gate()?;
        let (sink, parts) = Tap::channel(session.clone(), capacity);
        let client = self.hub.add_client(sink).await;
        let attach_params = json!({
            "instance": session.instance,
            "session_id": session.session_id,
            "__client": client.0,
        });
        let attached = match gate.call(subject, "harnix.attach", attach_params.clone()).await {
            Ok(v) => v,
            Err(e) => {
                self.hub.remove_client(client).await;
                return Err(e);
            }
        };
        let reattach = {
            let gate = gate.clone();
            let subject = subject.clone();
            Arc::new(move || {
                let gate = gate.clone();
                let subject = subject.clone();
                let params = attach_params.clone();
                Box::pin(async move { gate.call(&subject, "harnix.attach", params).await })
                    as std::pin::Pin<Box<dyn std::future::Future<Output = _> + Send>>
            })
        };
        let hub = self.hub.clone();
        // A tap can be dropped where there is no runtime — a module's state torn down on a thread
        // of its own, or after the runtime is gone — and `tokio::spawn` there panics. With no
        // runtime there is no hub left to tell either.
        let on_drop = Box::new(move || {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move { hub.remove_client(client).await });
            }
        });
        Ok(Tap::new(parts, attached, reattach, on_drop))
    }

    fn listen(&self) -> Option<tokio::sync::watch::Receiver<Option<Listen>>> {
        Some(self.listen.subscribe())
    }

    fn apps(&self, module: &str) -> Option<Arc<dyn harnix_module::apps::AppRuntime>> {
        if !self.app_hosts.contains(module) {
            return None;
        }
        self.apps.get().cloned()
    }
}

/// Transport asks; the authorization table answers.
pub(crate) struct Routes(pub Arc<Authorizer>);

impl harnix_transport::RouteGate for Routes {
    fn decide(&self, subject: &Subject, module: &str, path: &str) -> Result<(), RpcError> {
        self.0.decide_route(subject, module, path)
    }
}

/// What this process is made of and what did not take: `harnix.diagnostics`.
pub(crate) struct Diagnostics {
    pub versions: Value,
    pub modules: RwLock<Vec<ModuleStatus>>,
    pub unknown_sections: Vec<String>,
    /// The authorizer whose table `table` lists; set once it exists, and weak because the
    /// authorizer holds the gate that holds this.
    pub authorizer: OnceLock<Weak<Authorizer>>,
}

impl Diagnostics {
    fn report(&self) -> Value {
        let modules = read(&self.modules).clone();
        json!({
            "versions": self.versions,
            "modules": modules,
            "unknown_config_sections": self.unknown_sections,
            "table": self.table(),
            // Over the single-duplex in-flight cap a request is answered `BUSY`; a notification has
            // nothing to answer and is dropped, counted here (ipdrun-01).
            "transport": {"dropped_notifications": harnix_transport::dropped_notifications()},
        })
    }

    /// Every row of the authorization table, in the row shape the method-table snapshot pins
    /// (`crates/harnix/tests/method-table.snap.json`): empty before the authorizer is set.
    fn table(&self) -> Value {
        let Some(authorizer) = self.authorizer.get().and_then(Weak::upgrade) else {
            return json!({"rows": [], "aliases": []});
        };
        let modules = read(&self.modules).clone();
        let table = authorizer.table();
        table_report(&table, &modules)
    }

    /// The shape `harnix.contributions` has always had: every method the
    /// modules registered, and what did not take.
    fn contributions(&self) -> Value {
        let modules = read(&self.modules);
        let methods: Vec<&String> = modules
            .iter()
            .filter(|m| m.enabled)
            .flat_map(|m| m.methods.iter().map(|method| &method.name))
            .collect();
        let refused: Vec<Value> = modules
            .iter()
            .filter(|m| !m.enabled)
            .map(|m| json!({"who": m.id, "point": m.phase, "reason": m.reason}))
            .collect();
        json!({"methods": methods, "refused": refused})
    }
}

/// The table's rows as `{name, kind, owner, access}`, sorted by name within each kind, and the
/// `aliases` section (none: this table has one name per row).
///
/// Owners are what the code says today: the hub for what it answers, a module by its id, the
/// engine for what passes through, this layer for the three methods it answers itself.
pub(crate) fn table_report(table: &MethodTable, modules: &[ModuleStatus]) -> Value {
    let mut by_module: HashMap<String, String> = HashMap::new();
    for m in modules.iter().filter(|m| m.enabled) {
        let owner = format!("ext:{}", m.id);
        for method in &m.methods {
            by_module.insert(method.name.clone(), owner.clone());
        }
        for t in &m.topics {
            by_module.insert(harnix_authz::topic(t), owner.clone());
        }
        if m.routes {
            by_module.insert(harnix_authz::route(&m.id), owner.clone());
        }
    }
    let mut rows: Vec<(String, String, Value)> = table
        .rows()
        .map(|(key, rule)| {
            let (kind, name) = match (key.strip_prefix("topic:"), key.strip_prefix("route:")) {
                (Some(name), _) => ("topic", name),
                (_, Some(name)) => ("route", name),
                _ => ("method", key),
            };
            let owner = by_module
                .get(key)
                .cloned()
                .or_else(|| table.runtime_owner(key).map(|app| format!("ext:{app}")))
                .unwrap_or_else(|| native_owner(key, kind, rule));
            let mut extensions: Vec<&String> = rule.modules.iter().collect();
            extensions.sort();
            // Only what a grant opens: a row nothing but the kernel reaches by itself, and that
            // a module's manifest may still name.
            let granted = owner == "engine" || owner.starts_with("kernel:");
            let requires: Vec<&str> =
                if granted && !rule.device && !rule.hub_owned && !rule.modules.contains("*") { vec![key] } else { vec![] };
            let row = json!({
                "name": name,
                "kind": kind,
                "owner": owner,
                "access": {
                    "device": rule.device,
                    "extensions": extensions,
                    "engine": false,
                    "requires": requires,
                    "scope": null,
                    "refused": rule.refused,
                    "hub_owned": rule.hub_owned,
                },
            });
            (kind.to_string(), name.to_string(), row)
        })
        .collect();
    rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    json!({"rows": rows.into_iter().map(|r| r.2).collect::<Vec<_>>(), "aliases": []})
}

/// Who answers a row no module declared.
fn native_owner(key: &str, kind: &str, rule: &Rule) -> String {
    let name = key;
    if rule.hub_owned || kind != "method" {
        "kernel:hub".to_string()
    } else if crate::methods::APP_METHODS.contains(&name) {
        "kernel:app".to_string()
    } else if name == harnix_authz::REACHABLE {
        "kernel:authz".to_string()
    } else if harnix_hub::HUB_METHODS.contains(&name)
        || ["session.interrupt", "session.close", "session.delete"].contains(&name)
    {
        "kernel:hub".to_string()
    } else {
        "engine".to_string()
    }
}

#[cfg(test)]
mod tests {
    use harnix_authz::{Audit, Principal};

    use super::*;

    struct Nothing;

    #[async_trait::async_trait]
    impl Gate for Nothing {
        async fn call(&self, _: &Subject, _: &str, _: Value) -> Result<Value, RpcError> {
            Ok(Value::Null)
        }
    }

    fn snapshot_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../harnix/tests/reachable.snap.json")
    }

    /// What a runtime application's subject reaches, on the kernel's table: its own names, what
    /// its `hub` lists and the table lets through, and nothing else. The table is declared as
    /// `Apps::register` does it, and the answer is `harnix.reachable` asked as the application.
    /// The device's and a compiled-in module's reach are read from a running process by
    /// `tests/live/api/method-table.spec.ts`, into the same file.
    #[tokio::test]
    async fn a_runtime_applications_reach_is_the_snapshots() {
        let az = Authorizer::new(crate::methods::table(), Arc::new(Nothing), Arc::new(Audit::default()));
        let id = "snapshot-app";
        let hub: Vec<String> = ["harnix.sessions", "harnix.send", "session.interrupt", "topic:harnix.sessions.status", "plugin.install"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        for name in &hub {
            if az.table().check_grant(name, id) == Ok(true) {
                let _ = az.grant(&Subject::Kernel, name, id, "its `hub` capability");
            }
        }
        let rows = vec![
            (format!("app.{id}.echo"), vec![Principal::Device, Principal::Module(id.into())]),
            (format!("app.{id}.internal"), vec![Principal::Kernel, Principal::Module(id.into())]),
            (harnix_authz::topic(&format!("app.{id}.ticks")), vec![Principal::Module(id.into())]),
        ];
        az.declare_app(&Subject::Kernel, id, &rows, &hub).unwrap();

        let me = Subject::Module { id: id.into() };
        let reach = az.call(&me, harnix_authz::REACHABLE, json!({})).await.unwrap();
        let mut methods: Vec<String> = serde_json::from_value(reach["methods"].clone()).unwrap();
        let mut topics: Vec<String> = serde_json::from_value(reach["topics"].clone()).unwrap();
        methods.sort();
        topics.sort();
        let actual = json!({"methods": methods, "topics": topics});

        let path = snapshot_path();
        let mut kept: serde_json::Map<String, Value> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if std::env::var("HARNIX_UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
            kept.insert("plugin_app".into(), actual);
            std::fs::write(&path, serde_json::to_string_pretty(&Value::Object(kept)).unwrap() + "\n").unwrap();
            return;
        }
        assert_eq!(
            Some(&actual),
            kept.get("plugin_app"),
            "{} differs; if the change is intended: HARNIX_UPDATE_SNAPSHOTS=1, and say why in the commit",
            path.display()
        );
    }
}
