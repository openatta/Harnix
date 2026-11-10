//! The lifecycle, in order: register, migrate, (the instances are built), start; stop on the
//! way out. Each happens to every extension in the order the product listed them.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use harnix_contract::s1::{Capability, ExtId, Trust};
use harnix_engine_api::{EngineContribution, ToolProvider};
use harnix_ext::{ExtensionDescriptor, Manifest, MigrateContext};
use harnix_kernel::seat::Seated;
use harnix_kernel::GrantTable;
use harnix_module::shim;
use harnix_router::{IdKind, Row, RouteTable};
use serde_json::Value;

use super::seat::Seats;
use super::{Added, Entry, ExtensionStatus, Record};

/// Every extension's id and the capabilities each defines, before any row goes in (a row may
/// require what an extension after it defines); then what each was granted, checked against what
/// it asked for. A grant that does not hold refuses the whole start, naming the extension and the
/// capability: the assembly is declared in `main.rs`, and a mistake there is the program's.
fn check_grants(entries: &[(Manifest, Vec<Capability>)], table: &mut RouteTable) -> anyhow::Result<()> {
    let mut problems = Vec::new();
    for (manifest, _) in entries {
        let Ok(id) = ExtId::native(manifest.id()) else { continue };
        if table.register(&id, IdKind::Native).is_err() {
            continue; // a second extension by that id: disabled when it registers
        }
        for spec in manifest.defined() {
            if let Err(e) = table.define(&id, spec.clone()) {
                problems.push(format!("extension `{id}` defines {e}: an extension's own are `{id}.<name>`, once each"));
            }
        }
    }
    for (manifest, granted) in entries {
        let id = manifest.id();
        for cap in manifest.required() {
            if !granted.contains(cap) {
                problems.push(format!("extension `{id}` requires `{cap}`, which the product did not grant it"));
            }
        }
        for cap in granted {
            if !manifest.required().contains(cap) && !manifest.optionals().contains(cap) {
                problems.push(format!("extension `{id}` was granted `{cap}`, which it neither requires nor lists as optional"));
            } else {
                match table.defined(cap) {
                    None => problems.push(format!("extension `{id}` was granted `{cap}`, which nothing here defines")),
                    Some(spec) if !spec.is_native() => {
                        problems.push(format!("extension `{id}` was granted `{cap}`, which only a package may hold"))
                    }
                    Some(_) => {}
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("the product's grants do not hold:\n  {}", problems.join("\n  ")))
    }
}

/// Register every extension: its manifest and descriptor read, its rows put in the table, what
/// it was granted put in `grants`. A conflict or a configuration that does not read disables the
/// extension; a grant that does not hold, or a profile section no extension here owns, refuses
/// the start.
pub(crate) fn register(
    entries: Vec<Entry>,
    table: &mut RouteTable,
    grants: &GrantTable,
    sections: &BTreeMap<String, Value>,
    off: &[String],
    state_root: &Path,
) -> anyhow::Result<Vec<Record>> {
    let manifests: Vec<(Manifest, Vec<Capability>)> =
        entries.iter().map(|e| (e.added.manifest(), e.granted.clone())).collect();
    check_grants(&manifests, table)?;
    let ownerless: Vec<&String> =
        sections.keys().filter(|id| !manifests.iter().any(|(m, _)| m.id() == id.as_str())).collect();
    if !ownerless.is_empty() {
        let list: Vec<String> = ownerless.iter().map(|id| format!("[ext.{id}]")).collect();
        anyhow::bail!("the profile's {} names no extension of this product", list.join(", "));
    }
    let mut records: Vec<Record> = Vec::new();
    for (entry, (manifest, granted)) in entries.into_iter().zip(manifests) {
        let raw = manifest.id().to_string();
        let namespace = match &entry.added {
            Added::Module { namespace, .. } => Some(*namespace),
            Added::Extension(_) => None,
        };
        let data_dir = match namespace {
            Some(_) => state_root.join("modules").join(&raw),
            None => state_root.join("ext").join(&raw).join("data"),
        };
        let mut record = Record {
            added: entry.added,
            status: ExtensionStatus::new(&manifest, namespace, &granted),
            manifest,
            id: None,
            descriptor: None,
            module: None,
            config: sections.get(&raw).cloned().unwrap_or(Value::Null),
            data_dir,
            enabled: Arc::new(AtomicBool::new(true)),
            installed: false,
        };
        let id = match ExtId::native(&raw) {
            Ok(id) => id,
            Err(_) => {
                let reason = format!("`{raw}` is not an extension id: lowercase letters, digits and `-`, starting with a letter");
                record.disabled("registration", reason);
                records.push(record);
                continue;
            }
        };
        if records.iter().any(|r| r.manifest.id() == raw) {
            record.disabled("registration", format!("another extension already has the id `{raw}`"));
            records.push(record);
            continue;
        }
        record.id = Some(id.clone());
        if off.contains(&raw) {
            // What it declares is read, and nothing of it installed: the spaces it would have been
            // the backend of are then nobody's to claim.
            if let Added::Extension(e) = &record.added {
                record.descriptor = e.register(&id, &record.config).ok();
            }
            record.disabled("assembly", "switched off in the profile's [extensions] off".to_string());
            records.push(record);
            continue;
        }
        let descriptor = match &record.added {
            Added::Extension(e) => match e.register(&id, &record.config) {
                Ok(d) => d,
                Err(e) => {
                    record.disabled("configuration", e.to_string());
                    records.push(record);
                    continue;
                }
            },
            Added::Module { module, namespace } => {
                let slot: shim::ModuleSlot = Arc::new(OnceLock::new());
                let d = shim::describe(module.as_ref(), &id, &namespace.prefix(&raw), &slot);
                record.module = Some(slot);
                d
            }
        };
        record.status.declared(&descriptor);
        let refusal = check_native(&descriptor).err().or_else(|| install(&descriptor, table).err());
        match refusal {
            Some(reason) => record.disable("registration", reason, table),
            None => {
                record.installed = true;
                grants.set(id, granted, Trust::Origin);
            }
        }
        record.descriptor = Some(descriptor);
        records.push(record);
    }
    Ok(records)
}

/// What a native extension may declare that the router does not check: what it gives every
/// engine is tools and the names scenes admit — the rest of an engine contribution is the `main`
/// instance's — and its routes share one access, since the router decides them as one row.
fn check_native(d: &ExtensionDescriptor) -> Result<(), String> {
    if let Some(everywhere) = d.engine_everywhere() {
        if !everywhere.scenes().is_empty()
            || !everywhere.prompt_gates().is_empty()
            || everywhere.credential_source().is_some()
            || !everywhere.foreign_hooks().is_empty()
        {
            return Err("what it gives every engine is tool providers and admitted tool names; scenes, gates, \
                        credentials and hooks are the main instance's"
                .to_string());
        }
    }
    if let Some(first) = d.routes().first() {
        if d.routes().iter().any(|r| r.access() != first.access()) {
            return Err("its routes are decided as one: give them all the same access".to_string());
        }
    }
    Ok(())
}

/// One descriptor's methods, topics and routes in the table, all or none.
pub(crate) fn install(d: &ExtensionDescriptor, table: &mut RouteTable) -> Result<(), String> {
    let mut rows: Vec<Row> = d
        .methods()
        .iter()
        .map(|m| {
            Row::method(m.name(), m.access().clone(), Arc::new(m.clone()))
                .schema(m.schema().cloned())
                .describe(m.description())
        })
        .collect();
    rows.extend(d.topics().iter().map(|t| Row::topic(t.name(), t.access().clone())));
    if let Some(route) = d.routes().first() {
        rows.push(Row::route(d.id().as_str(), route.access().clone()));
    }
    table.install(&harnix_kernel::Owner::extension(d.id().clone()), rows).map_err(|e| e.to_string())
}

/// Migrate every enabled extension, in order: its directory made, then its own migration.
pub(crate) async fn migrate(records: &mut [Record], table: &mut RouteTable) {
    for record in records.iter_mut().filter(|r| r.is_enabled()) {
        if let Err(e) = std::fs::create_dir_all(&record.data_dir) {
            let reason = format!("its data directory {}: {e}", record.data_dir.display());
            record.disable("migration", reason, table);
            continue;
        }
        let applied = match &record.added {
            Added::Extension(e) => {
                let cx = MigrateContext::new(record.id.clone().expect("checked"), record.data_dir.clone(), record.config.clone());
                e.migrate(&cx).await.map(|r| r.steps().to_vec())
            }
            Added::Module { module, .. } => {
                let cx = harnix_module::MigrateContext {
                    id: record.manifest.id().to_string(),
                    data_dir: record.data_dir.clone(),
                    config: record.config.clone(),
                };
                module.migrate(&cx).await.map(|r| r.applied)
            }
        };
        match applied {
            Ok(steps) => {
                if !steps.is_empty() {
                    tracing::info!(extension = %record.manifest.id(), ?steps, "migrated");
                }
                record.status.migrations = steps;
            }
            Err(e) => record.disable("migration", format!("{e:#}"), table),
        }
    }
}

/// What the enabled extensions give the `main` instance, in order. Each is switched off with its
/// extension: one that fails to start after the instance was built keeps its scenes there until
/// the next start, and stops reaching sessions built after.
pub(crate) fn main_contribution(records: &[Record]) -> EngineContribution {
    let mut all = EngineContribution::new();
    for record in records.iter().filter(|r| r.is_enabled()) {
        let Some(d) = &record.descriptor else { continue };
        for c in [d.engine(), d.engine_everywhere()].into_iter().flatten() {
            super::contribution::merge(&mut all, &super::contribution::gated(c, record.enabled.clone()));
        }
    }
    all
}

/// What the enabled extensions give every engine built in this process: tool providers, and
/// the tool names every scene admits.
pub(crate) fn everywhere(records: &[Record]) -> (Vec<Arc<dyn ToolProvider>>, Vec<String>) {
    let mut providers = Vec::new();
    let mut admitted = Vec::new();
    for record in records.iter().filter(|r| r.is_enabled()) {
        let Some(c) = record.descriptor.as_ref().and_then(|d| d.engine_everywhere()) else { continue };
        let c = super::contribution::gated(c, record.enabled.clone());
        providers.extend(c.providers().iter().cloned());
        admitted.extend(c.admitted().iter().cloned());
    }
    (providers, admitted)
}

/// Start each enabled extension, in order, seated first. One that fails is disabled with the
/// reason, and takes what it registered with it; the rest start anyway. Answers the ids that
/// failed.
pub(crate) async fn start(
    records: &mut [Record],
    seats: &Seats,
    legacy: &Arc<crate::gate::ProcessKernel>,
    credentials: &Arc<dyn harnix_ext::Credentials>,
) -> Vec<String> {
    let mut failed = Vec::new();
    for record in records.iter_mut().filter(|r| r.is_enabled()) {
        let (Some(id), Some(d)) = (record.id.clone(), record.descriptor.as_ref()) else { continue };
        d.seat(seats.seat(&id, &record.config, &record.data_dir));
        let started = match &record.added {
            Added::Extension(e) => e.start(d.context().expect("seated just now")).await,
            Added::Module { module, namespace } => {
                let topics = d
                    .topics()
                    .iter()
                    .map(|t| harnix_module::TopicDecl {
                        name: t.name().to_string(),
                        params: t.params().to_vec(),
                        principals: Vec::new(),
                    })
                    .collect();
                let cx = harnix_module::ModuleContext::new(
                    id.as_str(),
                    &namespace.prefix(id.as_str()),
                    record.config.clone(),
                    record.data_dir.clone(),
                    credentials.clone(),
                    legacy.clone(),
                    topics,
                );
                // Its methods answer from before `start`, so a module that calls its own while
                // starting reaches them; withdrawn again if it fails.
                if let Some(slot) = &record.module {
                    let _ = slot.set(cx.clone());
                }
                module.start(cx).await
            }
        };
        if let Err(e) = started {
            record.disabled("start", format!("{e:#}"));
            failed.push(id.to_string());
            if !record.status.scenes.is_empty() {
                record.status.note = Some(format!(
                    "its scenes ({}) stay in the main instance until the next start; \
                     its tool providers and gates no longer reach new sessions",
                    record.status.scenes.join(", ")
                ));
            }
        }
    }
    failed
}

/// Stop the enabled extensions in reverse: a later one may depend on an earlier one.
pub(crate) async fn stop(records: &[Record]) {
    for record in records.iter().rev().filter(|r| r.is_enabled()) {
        match &record.added {
            Added::Extension(e) => e.stop().await,
            Added::Module { module, .. } => module.stop().await,
        }
    }
}

/// A spawned daemon's native contributions: the template `id` names, found among the product's
/// extensions in order — their declarations read, nothing started.
pub(crate) fn native_of(entries: &[&Added], id: &str) -> Result<EngineContribution, String> {
    for added in entries {
        let manifest = added.manifest();
        let Ok(ext) = ExtId::native(manifest.id()) else { continue };
        if !id.starts_with(&format!("{ext}.")) {
            continue;
        }
        let d = match added {
            Added::Extension(e) => e.register(&ext, &Value::Null).map_err(|e| e.to_string())?,
            Added::Module { module, namespace } => {
                shim::describe(module.as_ref(), &ext, &namespace.prefix(ext.as_str()), &Arc::new(OnceLock::new()))
            }
        };
        if let Some(template) = d.templates().iter().find(|t| t.id == id) {
            return template
                .contribution()
                .ok_or_else(|| format!("the template `{id}` has no native contributions to build"));
        }
    }
    Err(format!("this binary declares no template `{id}`"))
}

#[cfg(test)]
mod tests {
    use harnix_contract::Subject;
    use harnix_ext::{Access, Extension, Registrar};

    use super::*;

    struct Probe(&'static str);

    #[async_trait::async_trait]
    impl Extension for Probe {
        type Config = ();

        fn manifest(&self) -> Manifest {
            Manifest::new("probe", self.0)
        }

        fn register(&self, r: &mut Registrar<'_, ()>) {
            r.method("echo").access(Access::device()).raw(|_call, params| async move { Ok(params) });
        }
    }

    fn entry(e: impl Extension) -> Entry {
        Entry { added: Added::Extension(Box::new(e)), granted: Vec::new() }
    }

    /// A second extension with an id already taken is refused, and the first keeps its rows: the
    /// refusal withdraws only what the refused one put in, which is nothing.
    #[test]
    fn an_extension_refused_for_its_id_takes_nothing_of_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let mut table = RouteTable::new();
        let grants = GrantTable::new();
        let records =
            register(vec![entry(Probe("0.0.1")), entry(Probe("9.9.9"))], &mut table, &grants, &BTreeMap::new(), &[], dir.path())
                .unwrap();
        assert!(records[0].is_enabled());
        assert!(!records[1].is_enabled());
        let device = Subject::Device { id: "d".into() };
        assert_eq!(table.decide(&device, &grants, "probe.echo"), harnix_authz::Decision::Allow);
    }

    #[derive(Default, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Strict {
        #[allow(dead_code)]
        greeting: String,
    }

    struct Configured;

    #[async_trait::async_trait]
    impl Extension for Configured {
        type Config = Strict;

        fn manifest(&self) -> Manifest {
            Manifest::new("configured", "1")
        }

        fn register(&self, r: &mut Registrar<'_, Strict>) {
            r.method("hi").raw(|_call, _| async move { Ok(Value::Null) });
        }
    }

    /// A section that does not read as its extension's configuration disables that extension
    /// alone; a section nobody owns refuses the start; one switched off is registered and nothing
    /// of it is installed.
    #[test]
    fn the_profile_reaches_each_extension_as_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let grants = GrantTable::new();
        let mut table = RouteTable::new();
        let sections: BTreeMap<String, Value> = [("configured".to_string(), serde_json::json!({"greetin": "x"}))].into();
        let records = register(vec![entry(Configured), entry(Probe("1"))], &mut table, &grants, &sections, &[], dir.path()).unwrap();
        assert_eq!(records[0].status.phase, Some("configuration"));
        assert!(records[1].is_enabled());
        assert!(table.row("configured.hi").is_none());

        let sections: BTreeMap<String, Value> = [("nobody".to_string(), serde_json::json!({}))].into();
        let Err(err) = register(vec![entry(Probe("1"))], &mut RouteTable::new(), &grants, &sections, &[], dir.path()) else {
            panic!("a section nobody owns started");
        };
        assert!(err.to_string().contains("[ext.nobody]"), "{err}");

        let mut table = RouteTable::new();
        let records =
            register(vec![entry(Probe("1"))], &mut table, &grants, &BTreeMap::new(), &["probe".to_string()], dir.path()).unwrap();
        assert_eq!(records[0].status.phase, Some("assembly"));
        assert!(table.row("probe.echo").is_none());
        assert_eq!(table.registered(&ExtId::native("probe").unwrap()), Some(IdKind::Native), "its id stays its own");
    }
}
