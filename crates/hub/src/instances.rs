//! The instance table: what is running, where it came from, who owns it
//! (design §5.4, §5.5).
//!
//! Each instance is a [`Slot`] — its link and what is known about it — and a
//! router task that reads the link's one event stream for as long as the
//! slot exists. The router is started before the slot is visible, so no
//! client can reach an instance the hub is not already listening to.
//!
//! Templates and the ledger live in the [`Registry`]. What was built from a
//! template is recorded, and built again at the next start; one whose
//! template is gone or whose owner is disabled is listed `Down` with the
//! reason, and its data root is left alone.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use harnix_contract::{codes, RpcError, Subject};
use harnix_instance::link::{self, Carrier, DownLink, EngineLink, LinkStatus};
use harnix_instance::{Built, Facts, Factory, InstanceTemplate, Ledger, LedgerEntry, Owner};
use serde_json::{json, Value};

use crate::{Hub, SYSTEM};

pub struct Slot {
    pub id: String,
    pub link: Arc<dyn EngineLink>,
    pub facts: Facts,
    pub template: Option<String>,
    pub params: BTreeMap<String, String>,
    pub owner: Owner,
    pub created_ms: u64,
    tasks: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl Slot {
    pub(crate) async fn stop(&self) {
        for task in self.tasks.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            task.abort();
        }
        self.link.shutdown().await;
    }

    pub fn status(&self) -> LinkStatus {
        self.link.status().borrow().clone()
    }
}

/// Templates, the ledger, and how to build.
pub struct Registry {
    pub templates: BTreeMap<String, (InstanceTemplate, Owner)>,
    pub ledger: Ledger,
    pub factory: Factory,
    /// Where template-built instances keep their data.
    pub instances_dir: PathBuf,
    /// Modules that are disabled: their templates build nothing and their
    /// instances are down.
    pub disabled: HashSet<String>,
    /// The profile's own instances and where their data is, so one can be restarted.
    pub fixed: BTreeMap<String, (harnix_instance::TemplateCarrier, PathBuf)>,
}

impl Registry {
    pub fn add_template(&mut self, template: InstanceTemplate, owner: Owner) -> Result<(), String> {
        if self.templates.contains_key(&template.id) {
            return Err(format!("there is already a template `{}`", template.id));
        }
        self.templates.insert(template.id.clone(), (template, owner));
        Ok(())
    }
}

/// Who a caller builds as, and whether it may use this template. The interface builds as the space
/// it claims.
fn builder_of(subject: &Subject, space: &str, template: &InstanceTemplate, declared_by: &Owner) -> Result<Owner, RpcError> {
    match subject {
        Subject::Kernel => Ok(Owner::Kernel),
        // A module builds from its own templates and the profile's.
        Subject::Module { id } => match declared_by {
            Owner::Module(owner) if owner != id => Err(refused(format!(
                "template `{}` belongs to module `{owner}`",
                template.id
            ))),
            _ => Ok(Owner::Module(id.clone())),
        },
        // The interface builds only from what is opened to it (D18); the system space, the
        // debugging face, from any template.
        Subject::Device { .. } if space == harnix_contract::SYSTEM_SPACE => {
            Ok(Owner::Space(space.to_string()))
        }
        Subject::Device { .. } => {
            if template.ui {
                Ok(Owner::Space(space.to_string()))
            } else {
                Err(refused(format!(
                    "template `{}` is not one the interface may build from",
                    template.id
                )))
            }
        }
    }
}

/// The space a call says it comes from; the session space when it says none.
pub(crate) fn claimed_space(params: &Value) -> String {
    params
        .get("space")
        .and_then(Value::as_str)
        .unwrap_or(harnix_contract::SESSIONS_SPACE)
        .to_string()
}

fn refused(message: String) -> RpcError {
    RpcError::new(codes::REFUSED, message)
}

impl Hub {
    /// Make an instance visible, its router already running.
    pub async fn add_instance(
        self: &Arc<Self>,
        id: &str,
        built: Built,
        template: Option<String>,
        params: BTreeMap<String, String>,
        owner: Owner,
        created_ms: u64,
    ) {
        let slot = Arc::new(Slot {
            id: id.to_string(),
            link: built.link,
            facts: built.facts,
            template,
            params,
            owner,
            created_ms,
            tasks: std::sync::Mutex::new(Vec::new()),
        });
        let mut tasks = Vec::new();
        if let Some(events) = slot.link.take_events() {
            tasks.push(tokio::spawn(self.clone().route(id.to_string(), events)));
        }
        let mut status = slot.link.status();
        let hub = self.clone();
        tasks.push(tokio::spawn(async move {
            while status.changed().await.is_ok() {
                hub.refresh("harnix.instances", json!({}));
            }
        }));
        *slot.tasks.lock().unwrap_or_else(|e| e.into_inner()) = tasks;
        let previous = self
            .instances
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string(), slot);
        if let Some(previous) = previous {
            previous.stop().await;
        }
        self.refresh("harnix.instances", json!({}));
    }

    /// Build the instances the ledger records, at start. One that cannot be
    /// built is listed down, with why.
    pub async fn restore(self: &Arc<Self>) {
        let entries: Vec<LedgerEntry> = self.registry.lock().await.ledger.entries().to_vec();
        for entry in entries {
            if self.slot(&entry.id).is_some() {
                tracing::warn!(instance = %entry.id, "a ledger entry names an instance the profile declares; skipped");
                continue;
            }
            let built = self.build_entry(&entry).await;
            self.add_instance(
                &entry.id,
                built,
                Some(entry.template.clone()),
                entry.params.clone(),
                entry.owner.clone(),
                entry.created_ms,
            )
            .await;
        }
    }

    async fn build_entry(&self, entry: &LedgerEntry) -> Built {
        let registry = self.registry.lock().await;
        let down = |carrier: Carrier, reason: String| Built {
            link: DownLink::new(carrier, reason) as Arc<dyn EngineLink>,
            facts: Facts::default(),
        };
        let Some((template, declared_by)) = registry.templates.get(&entry.template) else {
            return down(
                Carrier::InProcess,
                format!("its template `{}` no longer exists", entry.template),
            );
        };
        for owner in [&entry.owner, declared_by] {
            if let Owner::Module(id) = owner {
                if registry.disabled.contains(id) {
                    return down(template.carrier.carrier(), format!("module `{id}` is disabled"));
                }
            }
        }
        let root = match template.resolve(&entry.id, &entry.params, &registry.instances_dir) {
            Ok(root) => root,
            Err(e) => return down(template.carrier.carrier(), e),
        };
        match registry.factory.build(&entry.id, &template.carrier, root).await {
            Ok(built) => built,
            Err(e) => down(template.carrier.carrier(), format!("{e:#}")),
        }
    }

    /// `harnix.instances.create` / `.ensure { template, name, params? }`.
    ///
    /// `ensure` returns the instance when one by that name was already built
    /// from that template, which makes it the call a module makes at start.
    pub(crate) async fn create_instance(
        self: &Arc<Self>,
        subject: &Subject,
        params: Value,
        ensure: bool,
    ) -> Result<Value, RpcError> {
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

        let space = self.claimed(&params)?;
        let (template, declared_by) = {
            let registry = self.registry.lock().await;
            registry
                .templates
                .get(&template_id)
                .cloned()
                .ok_or_else(|| RpcError::not_found(format!("there is no template `{template_id}`")))?
        };
        let owner = builder_of(subject, &space, &template, &declared_by)?;
        if let Some(existing) = self.slot(&name) {
            if ensure && existing.template.as_deref() == Some(template_id.as_str()) {
                return Ok(self.instance_view(&existing));
            }
        }
        self.build(&template_id, &name, filled, owner).await
    }

    /// Build an instance from a template for its owner — a space's default instance, here.
    pub(crate) async fn build_owned(self: &Arc<Self>, template: &str, name: &str, owner: Owner) -> Result<Value, RpcError> {
        if let Some(existing) = self.slot(name) {
            if existing.template.as_deref() == Some(template) {
                return Ok(self.instance_view(&existing));
            }
        }
        self.build(template, name, BTreeMap::new(), owner).await
    }

    async fn build(
        self: &Arc<Self>,
        template_id: &str,
        name: &str,
        filled: BTreeMap<String, String>,
        owner: Owner,
    ) -> Result<Value, RpcError> {
        let template_id = template_id.to_string();
        let name = name.to_string();
        let mut registry = self.registry.lock().await;
        let Some((template, declared_by)) = registry.templates.get(&template_id).cloned() else {
            return Err(RpcError::not_found(format!("there is no template `{template_id}`")));
        };
        if let Owner::Module(id) = &declared_by {
            if registry.disabled.contains(id) {
                return Err(refused(format!("module `{id}` is disabled")));
            }
        }
        if self.slot(&name).is_some() {
            return Err(RpcError::invalid_params(format!(
                "there is already an instance `{name}`"
            )));
        }
        if let Some(max) = template.max {
            let built = self
                .slots()
                .iter()
                .filter(|s| s.template.as_deref() == Some(template_id.as_str()))
                .count();
            if built >= max {
                return Err(refused(format!(
                    "template `{template_id}` allows {max} instance(s) and has {built}"
                )));
            }
        }
        let root = template
            .resolve(&name, &filled, &registry.instances_dir)
            .map_err(RpcError::invalid_params)?;
        let built = registry
            .factory
            .build(&name, &template.carrier, root)
            .await
            .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, format!("{e:#}")))?;
        let created_ms = link::now_ms();
        registry
            .ledger
            .insert(LedgerEntry {
                id: name.clone(),
                template: template_id.clone(),
                params: filled.clone(),
                owner: owner.clone(),
                created_ms,
            })
            .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, format!("ledger: {e:#}")))?;
        drop(registry);
        tracing::info!(instance = %name, template = %template_id, owner = %owner, "instance built");
        self.add_instance(&name, built, Some(template_id), filled, owner, created_ms)
            .await;
        let slot = self.slot(&name).expect("just added");
        Ok(self.instance_view(&slot))
    }

    /// `harnix.instances.stop { instance }`: stop it and forget it. Its data
    /// root stays; building it again from the same template picks it up.
    pub(crate) async fn stop_instance(
        self: &Arc<Self>,
        subject: &Subject,
        params: Value,
    ) -> Result<Value, RpcError> {
        let instance = self.instance_of(&params)?;
        let slot = self.slot(&instance).expect("checked");
        let space = self.claimed(&params)?;
        let allowed = match (subject, &slot.owner) {
            _ if instance == SYSTEM || slot.owner == Owner::Kernel => false,
            (Subject::Kernel, _) => true,
            (Subject::Module { id }, Owner::Module(owner)) => id == owner,
            (Subject::Device { .. }, _) if space == harnix_contract::SYSTEM_SPACE => true,
            (Subject::Device { .. }, Owner::Space(owner)) => *owner == space,
            _ => false,
        };
        if !allowed {
            return Err(refused(format!(
                "instance `{instance}` belongs to {}, which is not {}",
                slot.owner,
                subject.label()
            )));
        }
        self.remove_instance(&instance).await;
        Ok(json!({"stopped": instance}))
    }

    /// `harnix.instances.restart { instance }`: build it again — a template-built instance from its
    /// ledger entry, a profile one from its declaration, a daemon by connecting (and spawning)
    /// afresh — and resync whoever was watching its sessions.
    pub(crate) async fn restart_instance(
        self: &Arc<Self>,
        subject: &Subject,
        params: Value,
    ) -> Result<Value, RpcError> {
        let instance = self.instance_of(&params)?;
        if instance == SYSTEM {
            return Err(refused(
                "the system instance is this process; restart Harnix to restart it".into(),
            ));
        }
        let slot = self.slot(&instance).expect("checked");
        let space = self.claimed(&params)?;
        let allowed = match (subject, &slot.owner) {
            (Subject::Kernel, _) => true,
            (Subject::Module { id }, _) if id == "system" => true,
            (Subject::Module { id }, Owner::Module(owner)) => id == owner,
            (Subject::Device { .. }, _) if space == harnix_contract::SYSTEM_SPACE => true,
            (Subject::Device { .. }, Owner::Space(owner)) => *owner == space,
            _ => false,
        };
        if !allowed {
            return Err(refused(format!(
                "instance `{instance}` belongs to {}, which is not {}",
                slot.owner,
                subject.label()
            )));
        }
        let built = if let Some(template) = slot.template.clone() {
            let entry = LedgerEntry {
                id: instance.clone(),
                template,
                params: slot.params.clone(),
                owner: slot.owner.clone(),
                created_ms: slot.created_ms,
            };
            slot.stop().await;
            self.build_entry(&entry).await
        } else {
            let fixed = self.registry.lock().await.fixed.get(&instance).cloned();
            let Some((carrier, root)) = fixed else {
                return Err(refused(format!(
                    "instance `{instance}` has nothing to be built again from"
                )));
            };
            slot.stop().await;
            let registry = self.registry.lock().await;
            match registry.factory.build(&instance, &carrier, root).await {
                Ok(built) => built,
                Err(e) => Built {
                    link: DownLink::new(carrier.carrier(), format!("{e:#}")) as Arc<dyn EngineLink>,
                    facts: Facts::default(),
                },
            }
        };
        self.add_instance(
            &instance,
            built,
            slot.template.clone(),
            slot.params.clone(),
            slot.owner.clone(),
            slot.created_ms,
        )
        .await;
        // Every subscription belonged to the old link.
        tokio::spawn(self.clone().resync_instance(instance.clone()));
        tracing::info!(instance, "instance restarted");
        let slot = self.slot(&instance).expect("just added");
        Ok(self.instance_view(&slot))
    }

    async fn remove_instance(self: &Arc<Self>, instance: &str) {
        let removed = self
            .instances
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(instance);
        if let Some(slot) = removed {
            slot.stop().await;
        }
        if let Err(e) = self.registry.lock().await.ledger.remove(instance) {
            tracing::warn!(instance, error = %e, "ledger entry not removed");
        }
        self.inner
            .lock()
            .await
            .sessions
            .retain(|key, _| key.0 != instance);
        self.refresh("harnix.instances", json!({}));
        self.refresh("harnix.sessions.status", json!({}));
    }

    /// A module was disabled after its instances were built: they stop with
    /// it, and stay in the ledger to come back when it does (design §5.5).
    pub async fn module_disabled(self: &Arc<Self>, module: &str) {
        let owned: Vec<Arc<Slot>> = {
            let mut registry = self.registry.lock().await;
            registry.disabled.insert(module.to_string());
            let theirs: HashSet<String> = registry
                .templates
                .iter()
                .filter(|(_, (_, by))| *by == Owner::Module(module.to_string()))
                .map(|(id, _)| id.clone())
                .collect();
            self.slots()
                .into_iter()
                .filter(|s| {
                    s.owner == Owner::Module(module.to_string())
                        || s.template.as_ref().is_some_and(|t| theirs.contains(t))
                })
                .collect()
        };
        for slot in owned {
            slot.stop().await;
            let built = Built {
                link: DownLink::new(slot.link.carrier(), format!("module `{module}` is disabled")),
                facts: slot.facts.clone(),
            };
            self.add_instance(
                &slot.id,
                built,
                slot.template.clone(),
                slot.params.clone(),
                slot.owner.clone(),
                slot.created_ms,
            )
            .await;
        }
    }

    /// Facts about one instance, for a module's read-only handle.
    pub fn facts(&self, instance: &str) -> Option<Facts> {
        self.slot(instance).map(|s| s.facts.clone())
    }

    pub(crate) fn instance_view(&self, slot: &Slot) -> Value {
        let identity = slot.link.identity();
        json!({
            "id": slot.id,
            "carrier": slot.link.carrier(),
            "template": slot.template,
            "params": slot.params,
            "owner": slot.owner,
            "status": slot.status(),
            "version": identity.get("version"),
            "protocol_version": identity.get("protocol_version"),
            "data_root": slot.facts.data_root.display().to_string(),
            "scenes": slot.facts.active_scenes,
            "pid": slot.link.pid(),
            "created_ms": slot.created_ms,
        })
    }

    /// `harnix.instances.templates`: the templates this caller may build
    /// from, with the parameters each opens.
    pub(crate) async fn templates_view(&self, subject: &Subject, params: &Value) -> Value {
        let space = self
            .claimed(params)
            .unwrap_or_else(|_| harnix_contract::SESSIONS_SPACE.to_string());
        let registry = self.registry.lock().await;
        let list: Vec<Value> = registry
            .templates
            .values()
            .filter(|(t, by)| builder_of(subject, &space, t, by).is_ok())
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
