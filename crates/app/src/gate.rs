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
use std::sync::{Arc, OnceLock, RwLock};

use harnix_authz::Authorizer;
use harnix_contract::{Gate, RpcError, Subject};
use harnix_hub::Hub;
use harnix_module::{CallContext, Handler, InstanceHandle, Kernel, ModuleContext, SessionRef, Tap};
use serde_json::{json, Value};

use crate::modules::ModuleStatus;

type Bound = (Arc<dyn Handler>, ModuleContext);

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
        self.methods
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name.to_string(), (handler, cx));
    }

    pub fn unbind(&self, names: &[String]) {
        let mut methods = self.methods.write().unwrap_or_else(|e| e.into_inner());
        for name in names {
            methods.remove(name);
        }
    }

    fn bound(&self, name: &str) -> Option<Bound> {
        self.methods
            .read()
            .unwrap_or_else(|e| e.into_inner())
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
        if let Some((handler, cx)) = self.bound(method) {
            // The transport's stamp is for the hub's watching methods; a
            // module's typed params should not have to know it exists.
            if let Some(obj) = params.as_object_mut() {
                obj.remove("__client");
            }
            return handler.call(CallContext::new(subject.clone(), cx), params).await;
        }
        self.hub.call(subject, method, params).await
    }
}

/// The process, as a module's context sees it.
pub(crate) struct ProcessKernel {
    pub gate: OnceLock<Arc<dyn Gate>>,
    pub hub: Arc<Hub>,
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
        let on_drop = Box::new(move || {
            tokio::spawn(async move { hub.remove_client(client).await });
        });
        Ok(Tap::new(parts, attached, reattach, on_drop))
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
}

impl Diagnostics {
    fn report(&self) -> Value {
        let modules = self.modules.read().unwrap_or_else(|e| e.into_inner()).clone();
        json!({
            "versions": self.versions,
            "modules": modules,
            "unknown_config_sections": self.unknown_sections,
            // Over the single-duplex in-flight cap a request is answered `BUSY`; a notification has
            // nothing to answer and is dropped, counted here (ipdrun-01).
            "transport": {"dropped_notifications": harnix_transport::dropped_notifications()},
        })
    }

    /// The shape `harnix.contributions` has always had: every method the
    /// modules registered, and what did not take.
    fn contributions(&self) -> Value {
        let modules = self.modules.read().unwrap_or_else(|e| e.into_inner());
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
