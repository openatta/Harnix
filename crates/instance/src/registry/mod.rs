//! The instance table: what is running, where it came from, who owns it (concept §3.1.1).
//!
//! Each instance is a [`Slot`] — what is known about it, and what stops it. Its link is handed to
//! the hub, the one holder of sessions, through an [`InstanceObserver`] before the slot is
//! visible, so no caller can reach an instance the hub is not already listening to.
//!
//! Templates and the ledger live here too. What was built from a template is recorded, and built
//! again at the next start; one whose template is gone or whose owner is disabled is listed
//! `Down` with the reason, and its data root is left alone.
//!
//! # Order
//!
//! A name is changed by one operation at a time — built, stopped, restarted, taken down — and
//! the others wait for it ([`InstanceRegistry::claim`]). The registry's lock is held only to read
//! or write the table, never across a build or a call to the observer; the observer is never
//! called holding it, and the hub never calls in holding its own (each asserted in a debug
//! build, [`harnix_kernel::instances`]).

mod adopt;
mod rows;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use harnix_contract::sync::lock;
use harnix_contract::wire::{self, HelloInstance};
use harnix_contract::{codes, RpcError};
use harnix_kernel::instances::{holding, holds, Holding, Side};
use harnix_engine::ReplayLink;
use harnix_engine_link::link::{EngineControl, EngineLink, LinkStatus};
use harnix_engine_link::Readers;
use harnix_kernel::{GrantBook, InstanceBook, InstanceObserver, InstanceTrust, Listed, Rpc};
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::link::{self, DownLink};
use crate::templates::{carrier_of, InstanceTemplate, Ledger, LedgerEntry, Owner, TemplateCarrier};
use crate::{Built, Facts, Factory};

pub use adopt::{instance_of, Adopting, Adoption, EngineNote, Wake, WAKE_WITHIN};
pub use rows::{InstanceRow, RegistryCell, INSTANCES_TOPIC};

/// The instance every process has: the profile's `[engine]`.
pub use harnix_contract::MAIN_INSTANCE as MAIN;

/// What the registry is made with, at assembly: the templates and the ledger, how to build, and
/// the profile's own instances.
pub struct RegistryConfig {
    pub templates: BTreeMap<String, (InstanceTemplate, Owner)>,
    pub ledger: Ledger,
    pub factory: Factory,
    /// Where template-built instances keep their data.
    pub instances_dir: PathBuf,
    /// Modules that are disabled: their templates build nothing and their instances are down.
    pub disabled: HashSet<String>,
    /// The profile's own instances and where their data is, so one can be restarted.
    pub fixed: BTreeMap<String, (TemplateCarrier, PathBuf)>,
    /// What each extension holds: who may act on an instance it does not own.
    pub grants: Arc<dyn GrantBook>,
}

impl RegistryConfig {
    pub fn add_template(&mut self, template: InstanceTemplate, owner: Owner) -> Result<(), String> {
        if self.templates.contains_key(&template.id) {
            return Err(format!("there is already a template `{}`", template.id));
        }
        self.templates.insert(template.id.clone(), (template, owner));
        Ok(())
    }
}

/// What is known about one instance without asking it.
#[derive(Clone)]
pub struct InstanceFacts {
    pub id: String,
    pub owner: Owner,
    /// The template it was built from; `None` for the profile's.
    pub template: Option<String>,
    /// What its engine is: data root, scenes, model, recordings.
    pub engine: Facts,
    /// Whose engine it is: the product's, or an extension's own.
    pub trust: InstanceTrust,
}

/// One instance: what it is, and what stops it. Its link is the hub's to use: the slot keeps only
/// the link's control half, which reaches no session — and the readers of its recordings and
/// transcripts, which read only those, under `recordings.read` and `transcripts.read`.
pub(crate) struct Slot {
    id: String,
    control: Arc<dyn EngineControl>,
    readers: Option<Readers>,
    trust: InstanceTrust,
    facts: Facts,
    template: Option<String>,
    params: BTreeMap<String, String>,
    owner: Owner,
    created_ms: u64,
    /// Republishes the instance list when the link's status changes.
    watching: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Since when its engine has sent more than its trust allows, while it does.
    flooding: Mutex<Option<u64>>,
    /// An extension's own engine, adopted: its carrier's to start, stop and wake.
    adopted: Option<Arc<Adoption>>,
}

impl Slot {
    fn status(&self) -> LinkStatus {
        if let Some(since_ms) = self.adopted.as_ref().and_then(|a| a.asleep()) {
            return LinkStatus::Hibernating { since_ms };
        }
        let status = self.control.status().borrow().clone();
        match (*lock(&self.flooding), &status) {
            (Some(since_ms), LinkStatus::Ready) => LinkStatus::Degraded { since_ms, reason: "flooding".into() },
            _ => status,
        }
    }

    fn unwatch(&self) {
        if let Some(task) = lock(&self.watching).take() {
            task.abort();
        }
    }

    /// What it was built from, to build it again.
    fn record(&self) -> Record {
        Record {
            template: self.template.clone(),
            params: self.params.clone(),
            owner: self.owner.clone(),
            created_ms: self.created_ms,
        }
    }

    fn facts(&self) -> InstanceFacts {
        InstanceFacts {
            id: self.id.clone(),
            owner: self.owner.clone(),
            template: self.template.clone(),
            engine: self.facts.clone(),
            trust: self.trust.clone(),
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.unwatch();
    }
}

/// Where an instance came from and whose it is: what a slot keeps across a restart.
#[derive(Clone)]
struct Record {
    template: Option<String>,
    params: BTreeMap<String, String>,
    owner: Owner,
    created_ms: u64,
}

impl Record {
    fn fixed(owner: Owner) -> Self {
        Self {
            template: None,
            params: BTreeMap::new(),
            owner,
            created_ms: link::now_ms(),
        }
    }
}

/// How an instance is handed to the hub.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Handover {
    Added,
    Restarted,
}

/// The table, under the registry's lock.
struct State {
    templates: BTreeMap<String, (InstanceTemplate, Owner)>,
    ledger: Ledger,
    disabled: HashSet<String>,
    fixed: BTreeMap<String, (TemplateCarrier, PathBuf)>,
    slots: BTreeMap<String, Arc<Slot>>,
    /// Names an operation is changing, and the template one is being built from: whoever else
    /// wants the name waits for the operation to end.
    busy: HashMap<String, Busy>,
    /// Counts `harnix.instances.inheritProviders`: a build that started before one and ends after
    /// it inherits again.
    providers: u64,
}

struct Busy {
    done: watch::Receiver<()>,
    building: Option<String>,
}

/// The registry's lock, marked held for the lock-order assertions.
struct Locked<'a> {
    state: MutexGuard<'a, State>,
    _held: Holding,
}

impl std::ops::Deref for Locked<'_> {
    type Target = State;
    fn deref(&self) -> &State {
        &self.state
    }
}

impl std::ops::DerefMut for Locked<'_> {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

/// A name claimed by one operation, until dropped.
pub(crate) struct Claim<'a> {
    registry: &'a InstanceRegistry,
    name: String,
    _done: watch::Sender<()>,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.registry.state().busy.remove(&self.name);
    }
}

pub struct InstanceRegistry {
    /// The registry itself, for what it starts from a call handed only `&self`.
    this: std::sync::Weak<InstanceRegistry>,
    grants: Arc<dyn GrantBook>,
    factory: Factory,
    instances_dir: PathBuf,
    state: Mutex<State>,
    observer: OnceLock<Arc<dyn InstanceObserver<dyn EngineLink>>>,
}

impl InstanceRegistry {
    pub fn new(config: RegistryConfig) -> Arc<Self> {
        let RegistryConfig {
            templates,
            ledger,
            factory,
            instances_dir,
            disabled,
            fixed,
            grants,
        } = config;
        Arc::new_cyclic(|this| Self {
            this: this.clone(),
            grants,
            factory,
            instances_dir,
            state: Mutex::new(State {
                templates,
                ledger,
                disabled,
                fixed,
                slots: BTreeMap::new(),
                busy: HashMap::new(),
                providers: 0,
            }),
            observer: OnceLock::new(),
        })
    }

    /// Who is told of every instance: the hub, once, at wiring, before any instance is added.
    pub fn observe(&self, observer: Arc<dyn InstanceObserver<dyn EngineLink>>) {
        if self.observer.set(observer).is_err() {
            tracing::warn!("the instance registry is observed already; the second observer is ignored");
        }
    }

    fn state(&self) -> Locked<'_> {
        Locked {
            state: lock(&self.state),
            _held: holding(Side::Registry),
        }
    }

    /// The observer, to be called now: never while the registry's lock is held.
    fn observer(&self) -> &Arc<dyn InstanceObserver<dyn EngineLink>> {
        debug_assert!(!holds(Side::Registry), "the registry calls its observer holding its own lock");
        self.observer.get().expect("the registry is observed before any instance is added")
    }

    /// The instance list changed.
    fn changed(&self) {
        if let Some(observer) = self.observer.get() {
            observer.changed(INSTANCES_TOPIC, json!({}));
        }
    }

    /// Claim `name` for one operation, after whichever has it now.
    async fn claim(&self, name: &str) -> Claim<'_> {
        loop {
            let mut done = {
                let mut state = self.state();
                match state.busy.get(name) {
                    Some(busy) => busy.done.clone(),
                    None => {
                        let (tx, rx) = watch::channel(());
                        state.busy.insert(name.to_string(), Busy { done: rx, building: None });
                        return Claim {
                            registry: self,
                            name: name.to_string(),
                            _done: tx,
                        };
                    }
                }
            };
            // Ends when the operation drops its claim.
            let _ = done.changed().await;
        }
    }

    fn slot(&self, id: &str) -> Option<Arc<Slot>> {
        self.state().slots.get(id).cloned()
    }

    fn slots(&self) -> Vec<Arc<Slot>> {
        self.state().slots.values().cloned().collect()
    }

    /// Where this process plays recordings back from, when it was started in replay.
    pub fn replay_from(&self) -> Option<&std::path::Path> {
        self.factory.recording.replay_from()
    }

    /// Facts about one listed instance. The hub asks this holding none of its locks.
    /// The readers of what `id`'s engine writes down, while it is listed and built.
    pub fn readers(&self, id: &str) -> Option<Readers> {
        self.slot(id)?.readers.clone()
    }

    pub fn facts(&self, id: &str) -> Option<InstanceFacts> {
        debug_assert!(!holds(Side::Hub), "the hub calls the registry holding its own lock");
        self.slot(id).map(|s| s.facts())
    }

    /// Every listed instance.
    pub fn list(&self) -> Vec<InstanceFacts> {
        debug_assert!(!holds(Side::Hub), "the hub calls the registry holding its own lock");
        self.slots().iter().map(|s| s.facts()).collect()
    }

    /// What one instance's engine says it is, when it is listed.
    pub fn identity(&self, id: &str) -> Option<harnix_engine_link::EngineIdentity> {
        self.slot(id).map(|slot| slot.control.identity())
    }

    /// What each instance says it is, for `hello`: carrier, how far its engine is trusted, status,
    /// and the protocol it speaks.
    pub fn identities(&self) -> Vec<HelloInstance> {
        self.slots()
            .iter()
            .map(|slot| {
                let identity = slot.control.identity();
                HelloInstance {
                    id: slot.id.clone(),
                    carrier: slot.control.carrier(),
                    trust: (&slot.trust).into(),
                    status: slot.status(),
                    version: identity.version,
                    protocol_version: identity.protocol_version,
                }
            })
            .collect()
    }

    /// Hand the link to the observer, then list the instance: in that order, so that whoever can
    /// see it, the hub already reads it. A build that overlapped an inheritance of providers
    /// (`providers` is the count it started under) inherits again once it is listed.
    async fn show(&self, id: &str, built: Built, record: Record, handover: Handover, providers: u64) {
        self.show_as(id, built, record, handover, providers, None).await
    }

    /// [`Self::show`], for the product's own engine, or for an extension's adopted one.
    async fn show_as(&self, id: &str, built: Built, record: Record, handover: Handover, providers: u64, adopted: Option<(InstanceTrust, Arc<Adoption>)>) {
        let Built { link, control, facts, readers } = built;
        let (trust, adopted) = match adopted {
            Some((trust, adoption)) => (trust, Some(adoption)),
            None => (InstanceTrust::Product, None),
        };
        let observer = self.observer().clone();
        match handover {
            Handover::Added => observer.added(id, link.clone(), trust.clone()).await,
            Handover::Restarted => observer.restarted(id, link.clone()).await,
        }
        drop(link);
        let slot = Arc::new(Slot {
            id: id.to_string(),
            control,
            readers,
            trust,
            facts,
            template: record.template,
            params: record.params,
            owner: record.owner,
            created_ms: record.created_ms,
            watching: Mutex::new(None),
            flooding: Mutex::new(None),
            adopted,
        });
        *lock(&slot.watching) = Some(watch_status(&slot.control, observer));
        let (previous, inherits) = {
            let mut state = self.state();
            let previous = state.slots.insert(id.to_string(), slot.clone());
            let inherits = state.providers != providers
                && slot
                    .template
                    .as_ref()
                    .and_then(|t| state.templates.get(t))
                    .is_some_and(|(t, _)| t.inherit_providers);
            (previous, inherits)
        };
        if let Some(previous) = previous {
            previous.unwatch();
        }
        if inherits {
            self.inherit_into(&[slot]).await;
        }
        self.changed();
    }

    /// Add one of the profile's instances — `main`, an `[[instances]]` — as built at start.
    pub async fn add(&self, id: &str, built: Built, owner: Owner) {
        let _claim = self.claim(id).await;
        let providers = self.state().providers;
        self.show(id, built, Record::fixed(owner), Handover::Added, providers).await;
    }

    /// Adopt an extension's own engine as `proc.<id>`: its data root made, its own command sandbox
    /// off when its process is in the host's, `main`'s providers written into it when it inherits
    /// them, the hub handed its link and the instance listed — starting until the first run of its
    /// process attaches. Not in the ledger: it is the extension's for as long as the extension
    /// runs, and adopted again when it starts again.
    pub async fn adopt(&self, spec: Adopting) -> Rpc<Arc<Adoption>> {
        let id = instance_of(spec.ext.as_str());
        let _claim = self.claim(&id).await;
        if self.slot(&id).is_some() {
            return Err(refused(format!("there is already an instance `{id}`")));
        }
        std::fs::create_dir_all(&spec.data_root)
            .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, format!("{}: {e}", spec.data_root.display())))?;
        // Before its process first starts, and kept by every merge of providers after.
        if let Err(e) = adopt::command_sandbox(&spec.data_root, spec.sandboxed) {
            tracing::warn!(instance = %id, error = %e, "its engine's command sandbox was not set");
        }
        let facts = Facts { data_root: spec.data_root.clone(), ..Facts::default() };
        if spec.inherit_providers {
            if let Err(e) = self.factory.inherit_into(&facts) {
                tracing::warn!(instance = %id, error = %e, "the inherited providers were not written");
            }
        }
        let adoption = Adoption::new(id.clone(), &spec, self.observer().clone(), self.factory.turns.clone());
        self.factory.turns.watch(&id, adoption.clone());
        // Handed to the hub as any daemon's link is: the hub alone takes its events, and its calls
        // go through the process's recorder, so its sessions record — or, in replay, play back.
        let readers = adoption.link().readers();
        let link = ReplayLink::daemon(adoption.link(), self.factory.recording.of(&id, Some(spec.data_root.join("recordings"))));
        let built = Built { link, control: Arc::new(adopt::AdoptedControl(adoption.clone())), facts, readers: Some(readers) };
        let providers = self.state().providers;
        let record = Record::fixed(Owner::Module(spec.ext.to_string()));
        let trust = InstanceTrust::Extension(spec.ext.clone());
        self.show_as(&id, built, record, Handover::Added, providers, Some((trust, adoption.clone()))).await;
        tracing::info!(instance = %id, "an extension's engine adopted");
        Ok(adoption)
    }

    /// Let go of the engine adopted for `ext`: unlisted, its link shut, the hub told.
    pub async fn release(&self, ext: &str) {
        let id = instance_of(ext);
        let _claim = self.claim(&id).await;
        let removed = {
            let mut state = self.state();
            match state.slots.get(&id) {
                Some(slot) if slot.adopted.is_some() => state.slots.remove(&id),
                _ => None,
            }
        };
        let Some(slot) = removed else { return };
        slot.unwatch();
        slot.control.shutdown().await;
        self.observer().removed(&id).await;
        self.changed();
    }

    /// Build the instances the ledger records, at start. One that cannot be built is listed
    /// down, with why.
    pub async fn restore(&self) {
        let entries: Vec<LedgerEntry> = self.state().ledger.entries().to_vec();
        for entry in entries {
            let _claim = self.claim(&entry.id).await;
            if self.slot(&entry.id).is_some() {
                tracing::warn!(instance = %entry.id, "a ledger entry names an instance the profile declares; skipped");
                continue;
            }
            let providers = self.state().providers;
            let built = self.build_entry(&entry).await;
            let record = Record {
                template: Some(entry.template.clone()),
                params: entry.params.clone(),
                owner: entry.owner.clone(),
                created_ms: entry.created_ms,
            };
            self.show(&entry.id, built, record, Handover::Added, providers).await;
        }
    }

    /// Build what a ledger entry records; down, with why, when it cannot be.
    async fn build_entry(&self, entry: &LedgerEntry) -> Built {
        let down = |link: Arc<DownLink>| Built::new(link, Facts::default());
        let prepared = {
            let state = self.state();
            match state.templates.get(&entry.template) {
                None => Err(DownLink::without_template(format!(
                    "its template `{}` no longer exists",
                    entry.template
                ))),
                Some((template, declared_by)) => {
                    let disabled = [&entry.owner, declared_by].into_iter().find_map(|owner| match owner {
                        Owner::Module(id) if state.disabled.contains(id) => Some(id.clone()),
                        _ => None,
                    });
                    match disabled {
                        Some(id) => Err(DownLink::new(carrier_of(&template.carrier), format!("module `{id}` is disabled"))),
                        None => match template.resolve(&entry.id, &entry.params, &self.instances_dir) {
                            Ok(root) => Ok((template.clone(), root)),
                            Err(e) => Err(DownLink::new(carrier_of(&template.carrier), e)),
                        },
                    }
                }
            }
        };
        let (template, root) = match prepared {
            Ok(prepared) => prepared,
            Err(link) => return down(link),
        };
        match self.factory.build_template(&entry.id, &template, root).await {
            Ok(built) => built,
            Err(e) => down(DownLink::new(carrier_of(&template.carrier), format!("{e:#}"))),
        }
    }

    /// Build an instance from a template for its owner — a space's default instance, which the
    /// hub asks for holding none of its locks.
    pub async fn ensure_owned(self: &Arc<Self>, template: &str, name: &str, owner: Owner) -> Rpc<Value> {
        debug_assert!(!holds(Side::Hub), "the hub calls the registry holding its own lock");
        if let Some(existing) = self.slot(name) {
            if existing.template.as_deref() == Some(template) {
                return Ok(self.instance_view(&existing));
            }
        }
        self.build(template, name, BTreeMap::new(), owner, true).await
    }

    /// Build `name` from a template, with the name claimed until it is listed: two builds of one
    /// name are one after the other, and the second sees the first. `ensure` makes that second
    /// one an answer rather than a refusal. Run to its end whether or not the caller waits for
    /// it: a build abandoned halfway would leave an engine the hub reads and nobody lists.
    async fn build(
        self: &Arc<Self>,
        template_id: &str,
        name: &str,
        filled: BTreeMap<String, String>,
        owner: Owner,
        ensure: bool,
    ) -> Rpc<Value> {
        let this = self.clone();
        let (template_id, name) = (template_id.to_string(), name.to_string());
        tokio::spawn(async move { this.build_now(template_id, name, filled, owner, ensure).await })
            .await
            .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, format!("the build ended: {e}")))?
    }

    async fn build_now(
        &self,
        template_id: String,
        name: String,
        filled: BTreeMap<String, String>,
        owner: Owner,
        ensure: bool,
    ) -> Rpc<Value> {
        let _claim = self.claim(&name).await;
        let (template, root, providers) = {
            let mut state = self.state();
            let Some((template, declared_by)) = state.templates.get(&template_id).cloned() else {
                return Err(RpcError::not_found(format!("there is no template `{template_id}`")));
            };
            if let Owner::Module(id) = &declared_by {
                if state.disabled.contains(id) {
                    return Err(refused(format!("module `{id}` is disabled")));
                }
            }
            if let Some(existing) = state.slots.get(&name).cloned() {
                drop(state);
                if ensure && existing.template.as_deref() == Some(template_id.as_str()) {
                    return Ok(self.instance_view(&existing));
                }
                return Err(RpcError::invalid_params(format!("there is already an instance `{name}`")));
            }
            if let Some(max) = template.max {
                let built = state.slots.values().filter(|s| s.template.as_deref() == Some(template_id.as_str())).count()
                    + state.busy.values().filter(|b| b.building.as_deref() == Some(template_id.as_str())).count();
                if built >= max {
                    return Err(refused(format!(
                        "template `{template_id}` allows {max} instance(s) and has {built}"
                    )));
                }
            }
            let root = template
                .resolve(&name, &filled, &self.instances_dir)
                .map_err(RpcError::invalid_params)?;
            if let Some(busy) = state.busy.get_mut(&name) {
                busy.building = Some(template_id.clone());
            }
            (template, root, state.providers)
        };
        let built = self
            .factory
            .build_template(&name, &template, root)
            .await
            .map_err(|e| RpcError::new(codes::INTERNAL_ERROR, format!("{e:#}")))?;
        let created_ms = link::now_ms();
        let entry = LedgerEntry {
            id: name.clone(),
            template: template_id.clone(),
            params: filled.clone(),
            owner: owner.clone(),
            created_ms,
        };
        let recorded = self.state().ledger.insert(entry);
        if let Err(e) = recorded {
            built.control.shutdown().await;
            return Err(RpcError::new(codes::INTERNAL_ERROR, format!("ledger: {e:#}")));
        }
        tracing::info!(instance = %name, template = %template_id, owner = %owner, "instance built");
        let record = Record {
            template: Some(template_id),
            params: filled,
            owner,
            created_ms,
        };
        self.show(&name, built, record, Handover::Added, providers).await;
        let slot = self.slot(&name).expect("just listed");
        Ok(self.instance_view(&slot))
    }

    /// Stop an instance and forget it: unlisted, its link shut, the hub told, its ledger entry
    /// gone. Its data root stays; building it again from the same template picks it up.
    async fn remove(&self, instance: &str) -> Rpc<()> {
        let _claim = self.claim(instance).await;
        if self.slot(instance).is_some_and(|s| s.adopted.is_some()) {
            return Err(adopted(instance));
        }
        let removed = {
            let mut state = self.state();
            let removed = state.slots.remove(instance);
            if removed.is_some() {
                if let Err(e) = state.ledger.remove(instance) {
                    tracing::warn!(instance, error = %e, "ledger entry not removed");
                }
            }
            removed
        };
        let Some(slot) = removed else {
            return Err(not_found(instance));
        };
        slot.unwatch();
        slot.control.shutdown().await;
        self.observer().removed(instance).await;
        self.changed();
        Ok(())
    }

    /// Build an instance again — a template-built one from its ledger entry, a profile one from
    /// its declaration, a daemon by connecting (and spawning) afresh — and hand the hub the new
    /// link.
    async fn rebuild(&self, instance: &str) -> Rpc<()> {
        let _claim = self.claim(instance).await;
        let Some(slot) = self.slot(instance) else {
            return Err(not_found(instance));
        };
        if slot.adopted.is_some() {
            return Err(adopted(instance));
        }
        let fixed = self.state().fixed.get(instance).cloned();
        let providers = self.state().providers;
        let built = match (&slot.template, fixed) {
            (Some(template), _) => {
                let entry = LedgerEntry {
                    id: instance.to_string(),
                    template: template.clone(),
                    params: slot.params.clone(),
                    owner: slot.owner.clone(),
                    created_ms: slot.created_ms,
                };
                slot.control.shutdown().await;
                self.build_entry(&entry).await
            }
            (None, Some((carrier, root))) => {
                slot.control.shutdown().await;
                match self.factory.build(instance, &carrier, root).await {
                    Ok(built) => built,
                    Err(e) => Built::new(DownLink::new(carrier_of(&carrier), format!("{e:#}")), Facts::default()),
                }
            }
            (None, None) => {
                return Err(refused(format!(
                    "instance `{instance}` has nothing to be built again from"
                )))
            }
        };
        self.show(instance, built, slot.record(), Handover::Restarted, providers).await;
        Ok(())
    }

    /// A module was disabled after its instances were built: they stop with it, listed down,
    /// and stay in the ledger to come back when it does (concept §3.1.1).
    pub async fn module_disabled(&self, module: &str) {
        let owned: Vec<String> = {
            let mut state = self.state();
            state.disabled.insert(module.to_string());
            let theirs: HashSet<String> = state
                .templates
                .iter()
                .filter(|(_, (_, by))| *by == Owner::Module(module.to_string()))
                .map(|(id, _)| id.clone())
                .collect();
            state
                .slots
                .values()
                .filter(|s| {
                    s.owner == Owner::Module(module.to_string())
                        || s.template.as_ref().is_some_and(|t| theirs.contains(t))
                })
                .map(|s| s.id.clone())
                .collect()
        };
        for id in owned {
            let _claim = self.claim(&id).await;
            let Some(slot) = self.slot(&id) else { continue };
            slot.control.shutdown().await;
            let down = Built::new(
                DownLink::new(slot.control.carrier(), format!("module `{module}` is disabled")),
                slot.facts.clone(),
            );
            let providers = self.state().providers;
            self.show(&id, down, slot.record(), Handover::Restarted, providers).await;
        }
    }

    /// `harnix.instances.inheritProviders`: `main`'s providers changed. Merge them into every
    /// instance whose template inherits them, and have each one that changed reload. The kernel's,
    /// and granted to the settings module, which calls it after each provider change (ipdrun-11).
    async fn inherit_providers(&self) -> Rpc<Value> {
        let inheriting: Vec<Arc<Slot>> = {
            let mut state = self.state();
            // A build under way meanwhile inherits again once it is listed.
            state.providers += 1;
            let templates: HashSet<&str> = state
                .templates
                .iter()
                .filter(|(_, (t, _))| t.inherit_providers)
                .map(|(id, _)| id.as_str())
                .collect();
            state
                .slots
                .values()
                .filter(|slot| {
                    slot.template.as_deref().is_some_and(|t| templates.contains(t))
                        || slot.adopted.as_ref().is_some_and(|a| a.inherits)
                })
                .cloned()
                .collect()
        };
        let reloaded = self.inherit_into(&inheriting).await;
        Ok(json!({"reloaded": reloaded}))
    }

    /// Write `main`'s providers into each of `slots`, and reload each one that changed. Its ids.
    async fn inherit_into(&self, slots: &[Arc<Slot>]) -> Vec<String> {
        let changed: Vec<&Arc<Slot>> = slots
            .iter()
            .filter(|slot| match self.factory.inherit_into(&slot.facts) {
                Ok(changed) => changed,
                Err(e) => {
                    tracing::warn!(instance = %slot.id, error = %e, "the inherited providers were not written");
                    false
                }
            })
            .collect();
        let mut reloaded = Vec::new();
        for slot in changed {
            match slot.control.reload_settings().await {
                Ok(_) => reloaded.push(slot.id.clone()),
                // Written already: it reads them when it is back.
                Err(e) => tracing::warn!(instance = %slot.id, error = %e.message, "inherited providers written, not reloaded"),
            }
        }
        reloaded
    }

    /// Wait on the link's own status, not by asking again.
    async fn wait_ready(&self, instance: &str, timeout: std::time::Duration) -> Rpc<Value> {
        let slot = self.slot(instance).ok_or_else(|| not_found(instance))?;
        let mut status = slot.control.status();
        let waited = tokio::time::timeout(timeout, async {
            loop {
                let now = status.borrow_and_update().clone();
                match now {
                    LinkStatus::Ready => return Ok(()),
                    LinkStatus::Down { .. } => return Err(now.unavailable()),
                    _ => {}
                }
                if status.changed().await.is_err() {
                    return Err(status.borrow().unavailable());
                }
            }
        })
        .await;
        match waited {
            Ok(Ok(())) => Ok(self.instance_view(&slot)),
            Ok(Err(mut e)) => {
                e.data = Some(json!({"instance": instance, "status": slot.status()}));
                Err(e)
            }
            Err(_) => Err(RpcError::with_data(
                codes::INSTANCE_UNAVAILABLE,
                format!("the instance `{instance}` was not ready within {} ms", timeout.as_millis()),
                json!({"instance": instance, "status": slot.status()}),
            )),
        }
    }

    fn instance_view(&self, slot: &Slot) -> Value {
        let identity = slot.control.identity();
        wire::params(&wire::InstanceRow {
            id: slot.id.clone(),
            carrier: slot.control.carrier(),
            template: slot.template.clone(),
            params: slot.params.clone(),
            owner: String::from(slot.owner.clone()),
            status: slot.status(),
            version: identity.version,
            protocol_version: identity.protocol_version,
            data_root: slot.facts.root_label(),
            scenes: slot.facts.active_scenes.clone(),
            pid: slot.control.pid(),
            created_ms: slot.created_ms,
            trust: slot.trust.is_extension().then_some(wire::EngineTrust::Extension),
            reap: slot.control.reaps_on_stop(),
        })
    }

    /// `harnix.instances.list`, and the value of the `harnix.instances` topic.
    pub fn view(&self) -> Value {
        let list: Vec<Value> = self.slots().iter().map(|s| self.instance_view(s)).collect();
        json!({"instances": list})
    }

    /// Stop every instance, at the end of the process.
    pub async fn shutdown(&self) {
        for slot in self.slots() {
            slot.unwatch();
            slot.control.shutdown().await;
        }
    }
}

/// How many windows in a row an engine may send more than its trust allows before it is stopped.
const FLOODS_TO_STOP: u32 = 3;

/// What the hub asks back.
#[async_trait::async_trait]
impl InstanceBook for InstanceRegistry {
    fn listed(&self, id: &str) -> Option<Listed> {
        debug_assert!(!holds(Side::Hub), "the hub calls the registry holding its own lock");
        self.slot(id).map(|s| s.listed())
    }

    fn all(&self) -> Vec<Listed> {
        debug_assert!(!holds(Side::Hub), "the hub calls the registry holding its own lock");
        self.slots().iter().map(|s| s.listed()).collect()
    }

    async fn ensure_for_space(&self, template: &str, name: &str, space: &str) -> Rpc<()> {
        let this = self.this.upgrade().ok_or_else(|| RpcError::new(codes::INTERNAL_ERROR, "the registry is gone"))?;
        this.ensure_owned(template, name, Owner::Space(space.to_string())).await.map(|_| ())
    }

    /// Listed degraded while it floods; stopped once it has done so [`FLOODS_TO_STOP`] windows in
    /// a row.
    fn flooding(&self, id: &str, consecutive: u32) {
        let Some(slot) = self.slot(id) else { return };
        let changed = {
            let mut since = lock(&slot.flooding);
            let was = since.is_some();
            *since = match consecutive {
                0 => None,
                _ => Some(since.unwrap_or_else(link::now_ms)),
            };
            was != since.is_some()
        };
        if changed {
            self.changed();
        }
        if consecutive >= FLOODS_TO_STOP {
            // An extension's process is its carrier's to end, and the instance stays: listed
            // degraded while the process is started again.
            if let Some(adoption) = &slot.adopted {
                tracing::warn!(instance = id, consecutive, "the extension's engine keeps flooding; its carrier ends its process");
                adoption.flooding();
                return;
            }
            tracing::warn!(instance = id, consecutive, "the instance's engine keeps flooding; stopping it");
            let Some(this) = self.this.upgrade() else { return };
            let id = id.to_string();
            tokio::spawn(async move {
                if let Err(e) = this.remove(&id).await {
                    tracing::warn!(instance = %id, error = %e.message, "the flooding instance was not stopped");
                }
            });
        }
    }
}

impl Slot {
    fn listed(&self) -> Listed {
        let id = self.id.clone();
        if let Some(adopted) = &self.adopted {
            let (ext, spaces) = adopted.whose();
            return Listed { id, spaces: spaces.to_vec(), extension: Some(ext.clone()) };
        }
        let spaces = match &self.owner {
            Owner::Space(space) => vec![space.clone()],
            _ => Vec::new(),
        };
        Listed { id, spaces, extension: None }
    }
}

/// Republish the instance list whenever `link`'s status changes, for as long as it is listed.
fn watch_status(link: &Arc<dyn EngineControl>, observer: Arc<dyn InstanceObserver<dyn EngineLink>>) -> tokio::task::JoinHandle<()> {
    let mut status = link.status();
    tokio::spawn(async move {
        while status.changed().await.is_ok() {
            observer.changed(INSTANCES_TOPIC, json!({}));
        }
    })
}

/// Asked to stop or restart an extension's adopted engine: its carrier's to do, with the process.
fn adopted(instance: &str) -> RpcError {
    refused(format!("`{instance}` is an extension's own engine: it stops and starts with the extension"))
}

fn refused(message: String) -> RpcError {
    RpcError::new(codes::REFUSED, message)
}

fn not_found(instance: &str) -> RpcError {
    RpcError::new(codes::INSTANCE_NOT_FOUND, format!("there is no instance `{instance}`"))
}

#[cfg(test)]
mod tests;
