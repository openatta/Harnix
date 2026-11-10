//! What the route table's rows are answered by, where the answer is the assembly's: a module's
//! methods, a runtime application's, and the three this layer answers itself. Also the module's
//! view of the process ([`ProcessKernel`]), and the diagnostics page.
//!
//! A module method cannot shadow a hub method: both are rows in one table, and the table refuses
//! a name already taken.

use std::sync::{Arc, OnceLock, RwLock, Weak};

use harnix_contract::sync::read;
use harnix_contract::{RpcError, Subject};
use harnix_hub::{Hub, HubCell};
use harnix_kernel::{CallContext, Dispatch, Handler, Rpc};
use harnix_module::{AppHandler, InstanceHandle, Kernel, Listen, ModuleContext, SessionRef, Tap};
use harnix_router::Router;
use serde_json::{json, Value};

use crate::modules::ModuleStatus;

/// What the kernel's rows are answered by, filled in as each is built: the rows go in the table
/// at registration, with every other name, before the hub or the diagnostics exist.
#[derive(Clone, Default)]
pub(crate) struct Behind {
    pub hub: HubCell,
    pub diagnostics: Arc<OnceLock<Arc<Diagnostics>>>,
}

/// A compiled-in module's method: its handler, with the module's context once the module is
/// started. Until then it answers that it is not served — the same answer a name nobody declared
/// gets from below, and what it got before a method was a row.
pub(crate) struct ModuleMethod {
    pub handler: Arc<dyn harnix_module::Handler>,
    pub cx: Arc<OnceLock<ModuleContext>>,
    pub name: String,
}

#[async_trait::async_trait]
impl Handler for ModuleMethod {
    async fn call(&self, cx: CallContext, params: Value) -> Rpc<Value> {
        let Some(module) = self.cx.get().cloned() else {
            return Err(RpcError::not_found(format!("method `{}` is not served here (yet)", self.name)));
        };
        self.handler.call(harnix_module::CallContext::new(cx, module), params).await
    }
}

/// A runtime application's method (`crate::apps`).
pub(crate) struct RuntimeMethod(pub Arc<dyn AppHandler>);

#[async_trait::async_trait]
impl Handler for RuntimeMethod {
    async fn call(&self, cx: CallContext, params: Value) -> Rpc<Value> {
        self.0.call_in(&cx, params).await
    }
}

/// The process, as a module's context sees it.
pub(crate) struct ProcessKernel {
    pub router: OnceLock<Arc<Router>>,
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
    fn router(&self) -> Result<Arc<Router>, RpcError> {
        self.router.get().cloned().ok_or_else(|| {
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
        self.call_in(CallContext::root(subject.clone(), None), method, params).await
    }

    async fn call_in(&self, cx: CallContext, method: &str, params: Value) -> Result<Value, RpcError> {
        self.router()?.call(cx, method, params).await
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
        let router = self.router()?;
        let (sink, parts) = Tap::channel(session.clone(), capacity);
        let client = self.hub.add_client(sink).await;
        let attach_params = json!({
            "instance": session.instance,
            "session_id": session.session_id,
        });
        let attach = {
            let router = router.clone();
            let subject = subject.clone();
            move |params: Value| {
                let router = router.clone();
                let cx = CallContext::root(subject.clone(), Some(client));
                async move { router.call(cx, "harnix.attach", params).await }
            }
        };
        let attached = match attach(attach_params.clone()).await {
            Ok(v) => v,
            Err(e) => {
                self.hub.remove_client(client).await;
                return Err(e);
            }
        };
        let reattach = Arc::new(move || {
            Box::pin(attach(attach_params.clone()))
                as std::pin::Pin<Box<dyn std::future::Future<Output = _> + Send>>
        });
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

/// What this process is made of and what did not take: `harnix.diagnostics`.
pub(crate) struct Diagnostics {
    pub versions: Value,
    pub modules: RwLock<Vec<ModuleStatus>>,
    pub unknown_sections: Vec<String>,
    /// The router whose table `table` lists; set once it exists, and weak because the router
    /// holds the rows that hold this.
    pub router: OnceLock<Weak<Router>>,
}

impl Diagnostics {
    pub(crate) fn router(&self) -> Option<Arc<Router>> {
        self.router.get().and_then(Weak::upgrade)
    }

    pub(crate) fn report(&self) -> Value {
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

    /// Every row of the route table, in the row shape the method-table snapshot pins
    /// (`crates/harnix/tests/method-table.snap.json`): empty before the router is set.
    fn table(&self) -> Value {
        match self.router() {
            Some(router) => router.snapshot(),
            None => json!({"rows": [], "aliases": []}),
        }
    }

    /// The shape `harnix.contributions` has always had: every method the
    /// modules registered, and what did not take.
    pub(crate) fn contributions(&self) -> Value {
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

#[cfg(test)]
mod tests {
    use harnix_authz::{access_for, Audit, Principal};
    use harnix_contract::s1::{PackageCarrier, Trust};
    use harnix_router::Row;

    use super::*;

    fn snapshot_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../harnix/tests/reachable.snap.json")
    }

    /// What a runtime application's subject reaches, on the kernel's table: its own names, what
    /// its `hub` lists and the table lets through, and nothing else. The table is installed as
    /// `Apps::register` does it, and the answer is `harnix.reachable` asked as the application.
    /// The device's and a compiled-in module's reach are read from a running process by
    /// `tests/live/api/method-table.spec.ts`, into the same file.
    #[tokio::test]
    async fn a_runtime_applications_reach_is_the_snapshots() {
        let grants = Arc::new(harnix_kernel::GrantTable::new());
        let router = Router::new(crate::methods::table(&Behind::default()), grants.clone(), Arc::new(Audit::default()));
        let id = "snapshot-app";
        let hub: Vec<String> = ["harnix.sessions", "harnix.send", "session.interrupt", "topic:harnix.sessions.status", "plugin.install"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let ext = harnix_contract::ExtId::parse(id).unwrap();
        grants.set(ext, [], Trust::consent(hub, PackageCarrier::Wasm));
        struct Nothing;
        #[async_trait::async_trait]
        impl Handler for Nothing {
            async fn call(&self, _: CallContext, _: Value) -> Rpc<Value> {
                Ok(Value::Null)
            }
        }
        let own = |principals: &[Principal]| access_for(principals);
        let rows = vec![
            Row::method(format!("app.{id}.echo"), own(&[Principal::Device, Principal::Module(id.into())]), Arc::new(Nothing)),
            Row::method(format!("app.{id}.internal"), own(&[Principal::Kernel, Principal::Module(id.into())]), Arc::new(Nothing)),
            Row::topic(&format!("app.{id}.ticks"), own(&[Principal::Module(id.into())])),
        ];
        router.install_app(&Subject::Kernel, id, rows).unwrap();

        let me = Subject::extension(harnix_contract::ExtId::parse(id).unwrap());
        let reach = router.call(CallContext::root(me, None), harnix_router::REACHABLE, json!({})).await.unwrap();
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
