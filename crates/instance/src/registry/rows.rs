//! The registry's rows: `harnix.instances.*` and the `harnix.instances` topic.
//!
//! What each answers, and who may build from which template, stop and restart what. The router
//! decides who reaches a row; the owner checks here are about the instance, not the method.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, OnceLock};

use harnix_contract::s1::caps;
use harnix_contract::{codes, RpcError, Subject};
use harnix_kernel::{Access, CallContext, Handler, Rpc, TopicSource};
use serde_json::{json, Value};

use super::{not_found, refused, InstanceRegistry, MAIN};
use crate::templates::{InstanceTemplate, Owner};

/// The instance list, as a topic: pushed when an instance comes, goes, or changes status.
pub const INSTANCES_TOPIC: &str = "harnix.instances";

/// Where the registry's rows find the registry. The rows are installed before the instances are
/// built — with every other name, so that a module cannot take one — and the registry is put
/// here once it is.
pub type RegistryCell = Arc<OnceLock<Arc<InstanceRegistry>>>;

/// One row the registry answers.
pub enum InstanceRow {
    Method {
        name: &'static str,
        access: Access,
        handler: Arc<dyn Handler>,
    },
    Topic { name: &'static str, access: Access },
}

/// What answers one of the registry's rows: a function of the registry, the call and its params.
struct Answer<F> {
    cell: RegistryCell,
    f: F,
}

#[async_trait::async_trait]
impl<F, Fut> Handler for Answer<F>
where
    F: Fn(Arc<InstanceRegistry>, CallContext, Value) -> Fut + Send + Sync,
    Fut: Future<Output = Rpc<Value>> + Send,
{
    async fn call(&self, cx: CallContext, params: Value) -> Rpc<Value> {
        let registry = self
            .cell
            .get()
            .cloned()
            .ok_or_else(|| RpcError::new(codes::INTERNAL_ERROR, "the instances are not up yet"))?;
        (self.f)(registry, cx, params).await
    }
}

fn method<F, Fut>(cell: &RegistryCell, name: &'static str, access: Access, f: F) -> InstanceRow
where
    F: Fn(Arc<InstanceRegistry>, CallContext, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Rpc<Value>> + Send + 'static,
{
    InstanceRow::Method {
        name,
        access,
        handler: Arc::new(Answer { cell: cell.clone(), f }),
    }
}

/// The topic's value, computed when someone watches it.
struct Listed(RegistryCell);

#[async_trait::async_trait]
impl TopicSource for Listed {
    async fn compute(&self, _params: &Value) -> Rpc<Value> {
        let registry = self
            .0
            .get()
            .ok_or_else(|| RpcError::new(codes::INTERNAL_ERROR, "the instances are not up yet"))?;
        Ok(registry.view())
    }
}

impl InstanceRegistry {
    /// Every row the registry answers. `cell` holds the registry once it is built; until then
    /// each answers that it is not up.
    pub fn rows(cell: &RegistryCell) -> Vec<InstanceRow> {
        // An extension starts, waits on and stops instances of its own; who may manage which is
        // the registry's to check.
        let open = || Access::device().with_extensions();
        vec![
            method(cell, "harnix.instances.list", open(), |r, _, _| async move { Ok(r.view()) }),
            method(cell, "harnix.instances.templates", open(), |r, cx, p| async move {
                Ok(r.templates_view(&cx.subject, &p))
            }),
            method(cell, "harnix.instances.create", open(), |r, cx, p| async move {
                r.create_instance(&cx.subject, p, false).await
            }),
            method(cell, "harnix.instances.ensure", open(), |r, cx, p| async move {
                r.create_instance(&cx.subject, p, true).await
            }),
            method(cell, "harnix.instances.stop", open(), |r, cx, p| async move {
                r.stop_instance(&cx.subject, p).await
            }),
            method(cell, "harnix.instances.restart", open(), |r, cx, p| async move {
                r.restart_instance(&cx.subject, p).await
            }),
            method(cell, "harnix.instances.wait", open(), |r, _, p| async move { r.wait_instance(p).await }),
            // After a provider changes, the instances whose template inherits the system
            // instance's get it too (ipdrun-11): whoever may write the engine's providers.
            method(cell, "harnix.instances.inheritProviders", Access::capability(caps::ENGINE_CONFIG_WRITE), |r, _, _| async move {
                r.inherit_providers().await
            }),
            InstanceRow::Topic { name: INSTANCES_TOPIC, access: open() },
        ]
    }

    /// What computes the `harnix.instances` topic, for whoever fans it out.
    pub fn topic(cell: &RegistryCell) -> Arc<dyn TopicSource> {
        Arc::new(Listed(cell.clone()))
    }

    /// Whether `subject` holds `instances.admin`: the kernel does.
    fn is_admin(&self, subject: &Subject) -> bool {
        harnix_kernel::holds(&*self.grants, subject, &caps::INSTANCES_ADMIN)
    }

    /// Whether `subject` may stop or restart an instance `owner` owns: its own — a module's, a
    /// space's from that space — or any, holding `instances.admin`.
    fn may_manage(&self, subject: &Subject, owner: &Owner, space: &str) -> bool {
        if self.is_admin(subject) {
            return true;
        }
        match (subject, owner) {
            (Subject::Extension { id }, Owner::Module(owner)) => id == owner.as_str(),
            (Subject::Device { .. }, Owner::Space(owner)) => owner == space,
            _ => false,
        }
    }

    /// The space a call claims, or the default space when it claims none: one this application
    /// has. Empty when it claims none and there is no default: then it is no space's.
    fn claimed(&self, params: &Value) -> Result<String, RpcError> {
        let claimed = params.get("space").and_then(Value::as_str);
        match (self.observer().space(claimed), claimed) {
            (Some(space), _) => Ok(space),
            (None, None) => Ok(String::new()),
            (None, Some(space)) => {
                Err(RpcError::new(codes::SPACE_MISMATCH, format!("there is no space `{space}` in this application")))
            }
        }
    }

    /// Which instance a call means: the one it names, or `main`. One that is not listed is
    /// refused by name.
    fn instance_of(&self, params: &Value) -> Result<String, RpcError> {
        let name = match params.get("instance") {
            None | Some(Value::Null) => MAIN,
            Some(Value::String(name)) => name.as_str(),
            Some(other) => {
                return Err(RpcError::invalid_params(format!(
                    "`instance` is a name, not {other}"
                )))
            }
        };
        if self.slot(name).is_none() {
            return Err(not_found(name));
        }
        Ok(name.to_string())
    }

    /// `harnix.instances.create` / `.ensure { template, name, params?, wait?, timeout_ms? }`.
    ///
    /// `ensure` returns the instance when one by that name was already built
    /// from that template, which makes it the call a module makes at start —
    /// also when two of them race for the same name (ipdrun-04).
    ///
    /// `wait: "ready"` answers once the instance is ready, rather than as soon as
    /// it exists: a spawned daemon is `starting` for a moment after it is built,
    /// and a session call in that moment is refused (ipdrun-05).
    pub(super) async fn create_instance(self: &Arc<Self>, subject: &Subject, params: Value, ensure: bool) -> Rpc<Value> {
        let wait = Wait::parse(&params)?;
        let view = self.create_instance_now(subject, &params, ensure).await?;
        match wait {
            None => Ok(view),
            Some(timeout) => {
                let name = view.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                self.wait_ready(&name, timeout).await
            }
        }
    }

    /// `harnix.instances.wait { instance, timeout_ms? }`: answer with the instance once it is
    /// ready; at once with `INSTANCE_UNAVAILABLE` if it is down, or when the time runs out.
    async fn wait_instance(&self, params: Value) -> Rpc<Value> {
        let instance = self.instance_of(&params)?;
        let timeout = Wait::timeout(&params)?;
        self.wait_ready(&instance, timeout).await
    }

    async fn create_instance_now(self: &Arc<Self>, subject: &Subject, params: &Value, ensure: bool) -> Rpc<Value> {
        let template_id = params
            .get("template")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid_params("missing template"))?
            .to_string();
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid_params("missing name"))?
            .to_string();
        let filled: BTreeMap<String, String> = match params.get("params") {
            None | Some(Value::Null) => BTreeMap::new(),
            Some(v) => serde_json::from_value(v.clone()).map_err(|_| {
                RpcError::invalid_params("`params` is an object of strings")
            })?,
        };

        let space = self.claimed(params)?;
        let (template, declared_by) = self
            .state()
            .templates
            .get(&template_id)
            .cloned()
            .ok_or_else(|| RpcError::not_found(format!("there is no template `{template_id}`")))?;
        let owner = builder_of(subject, self.is_admin(subject), &space, &template, &declared_by)?;
        // The fast path. The check that decides is the one under the name's claim, in `build`.
        if let Some(existing) = self.slot(&name) {
            if ensure && existing.template.as_deref() == Some(template_id.as_str()) {
                return Ok(self.instance_view(&existing));
            }
        }
        self.build(&template_id, &name, filled, owner, ensure).await
    }

    /// `harnix.instances.stop { instance }`: stop it and forget it. Its data
    /// root stays; building it again from the same template picks it up.
    async fn stop_instance(self: &Arc<Self>, subject: &Subject, params: Value) -> Rpc<Value> {
        let instance = self.instance_of(&params)?;
        let slot = self.slot(&instance).ok_or_else(|| not_found(&instance))?;
        let space = self.claimed(&params)?;
        // The kernel's own instances are this process's: nobody stops them, by owner not by id.
        let allowed = slot.owner != Owner::Kernel && self.may_manage(subject, &slot.owner, &space);
        if !allowed {
            return Err(refused(format!(
                "instance `{instance}` belongs to {}, which is not {}",
                slot.owner,
                subject.label()
            )));
        }
        let this = self.clone();
        let stopping = instance.clone();
        // To its end, whether or not the caller waits: the hub is told of what is unlisted.
        tokio::spawn(async move { this.remove(&stopping).await })
            .await
            .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, format!("the stop ended: {e}")))??;
        Ok(json!({"stopped": instance}))
    }

    /// `harnix.instances.restart { instance }`: build it again, and resync whoever was watching
    /// its sessions.
    async fn restart_instance(self: &Arc<Self>, subject: &Subject, params: Value) -> Rpc<Value> {
        let instance = self.instance_of(&params)?;
        if instance == MAIN {
            return Err(refused(
                "the main instance is this process; restart Harnix to restart it".into(),
            ));
        }
        let slot = self.slot(&instance).ok_or_else(|| not_found(&instance))?;
        let space = self.claimed(&params)?;
        let allowed = self.may_manage(subject, &slot.owner, &space);
        if !allowed {
            return Err(refused(format!(
                "instance `{instance}` belongs to {}, which is not {}",
                slot.owner,
                subject.label()
            )));
        }
        let this = self.clone();
        let restarting = instance.clone();
        tokio::spawn(async move { this.rebuild(&restarting).await })
            .await
            .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, format!("the restart ended: {e}")))??;
        tracing::info!(instance, "instance restarted");
        let slot = self.slot(&instance).ok_or_else(|| not_found(&instance))?;
        Ok(self.instance_view(&slot))
    }

    /// `harnix.instances.templates`: the templates this caller may build
    /// from, with the parameters each opens.
    fn templates_view(&self, subject: &Subject, params: &Value) -> Value {
        let space = self.claimed(params).unwrap_or_default();
        let admin = self.is_admin(subject);
        let state = self.state();
        let list: Vec<Value> = state
            .templates
            .values()
            .filter(|(t, by)| builder_of(subject, admin, &space, t, by).is_ok())
            .map(|(t, by)| {
                json!({
                    "id": t.id,
                    "carrier": t.carrier.kind(),
                    "params": t.params,
                    "ui": t.ui,
                    "max": t.max,
                    "declared_by": by,
                })
            })
            .collect();
        json!({"templates": list})
    }
}

/// Who a caller builds as, and whether it may use this template. The interface builds as the space
/// it claims, and only from what is opened to it (`ui` templates); an extension from its own
/// templates and the profile's; holding `instances.admin`, from any.
fn builder_of(
    subject: &Subject,
    admin: bool,
    space: &str,
    template: &InstanceTemplate,
    declared_by: &Owner,
) -> Result<Owner, RpcError> {
    match subject {
        Subject::Kernel => Ok(Owner::Kernel),
        Subject::Extension { id } => match declared_by {
            Owner::Module(owner) if id != owner.as_str() && !admin => Err(refused(format!(
                "template `{}` belongs to module `{owner}`",
                template.id
            ))),
            _ => Ok(Owner::Module(id.to_string())),
        },
        Subject::Device { .. } if template.ui && space.is_empty() => Err(RpcError::new(
            codes::SPACE_MISMATCH,
            "an instance the interface builds is its space's: claim one, there is no default here",
        )),
        Subject::Device { .. } if template.ui => Ok(Owner::Space(space.to_string())),
        Subject::Device { .. } => Err(refused(format!(
            "template `{}` is not one the interface may build from",
            template.id
        ))),
        _ => Err(refused(format!("template `{}` is not one this caller may build from", template.id))),
    }
}

/// `wait` and `timeout_ms` on `harnix.instances.create` / `.ensure` / `.wait`.
struct Wait;

impl Wait {
    /// How long a wait lasts when the call does not say.
    const DEFAULT: std::time::Duration = std::time::Duration::from_secs(60);

    /// `Some(timeout)` when the call asks to wait until the instance is ready.
    fn parse(params: &Value) -> Result<Option<std::time::Duration>, RpcError> {
        match params.get("wait") {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) if s == "ready" => Self::timeout(params).map(Some),
            Some(other) => Err(RpcError::invalid_params(format!(
                "wait is \"ready\" or absent, not {other}"
            ))),
        }
    }

    fn timeout(params: &Value) -> Result<std::time::Duration, RpcError> {
        match params.get("timeout_ms") {
            None | Some(Value::Null) => Ok(Self::DEFAULT),
            Some(v) => v
                .as_u64()
                .map(std::time::Duration::from_millis)
                .ok_or_else(|| RpcError::invalid_params("timeout_ms is a number of milliseconds")),
        }
    }
}
