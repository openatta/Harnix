//! The module lifecycle, in the order design §7.3 gives it.
//!
//! Registration, migration and start each happen to every module in the
//! order the application listed them. A module that fails at any of them is
//! disabled — its methods, topics and route taken back out of the
//! authorization table, its customizer no longer applied — and the rest
//! carry on. Why it was disabled is kept for the diagnostics page: "my space
//! does nothing" needs an answer, not a guess.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use harnix_authz::{route, topic, MethodTable, Principal, Rule};
use harnix_module::manifest::check_id;
use harnix_module::{
    EngineContrib, ErasedModule, EventRegistry, InstanceTemplate, InstanceTemplates, Manifest,
    MethodDecl, MethodRegistry, MigrateContext, ModuleContext, RouteDecl, RouteRegistry,
    TopicDecl,
};
use serde::Serialize;
use serde_json::Value;

/// Where a module's names live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Namespace {
    /// A built-in: `harnix.*`.
    Kernel,
    /// An application's module: `app.<id>.*`.
    App,
}

pub(crate) struct Entry {
    pub module: Box<dyn ErasedModule>,
    pub namespace: Namespace,
}

/// What the diagnostics page says about one module.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleStatus {
    pub id: String,
    pub version: String,
    pub namespace: Namespace,
    pub enabled: bool,
    /// Where it was disabled: `registration`, `migration` or `start`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub disclosure: String,
    pub grants: Vec<String>,
    pub methods: Vec<MethodStatus>,
    pub topics: Vec<String>,
    pub routes: bool,
    pub scenes: Vec<String>,
    /// Instance templates, by their full ids.
    pub templates: Vec<String>,
    /// Spaces this module is the backend of.
    pub spaces: Vec<String>,
    pub migrations: Vec<String>,
    /// After a start failure: what stays until the next start, because the
    /// instance was already built with it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One method, as the diagnostics page lists it.
#[derive(Debug, Clone, Serialize)]
pub struct MethodStatus {
    pub name: String,
    /// Who may call it: `device`, `kernel`, `module:<id>`, `module:*`.
    pub principals: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
}

pub(crate) struct Record {
    pub module: Box<dyn ErasedModule>,
    pub manifest: Manifest,
    pub prefix: String,
    pub methods: Vec<MethodDecl>,
    pub topics: Vec<TopicDecl>,
    pub route: Option<RouteDecl>,
    pub contrib: EngineContrib,
    pub templates: Vec<InstanceTemplate>,
    pub spaces: Vec<harnix_module::SpaceDecl>,
    pub enabled: Arc<AtomicBool>,
    /// Every name this module put in the authorization table.
    pub declared: Vec<String>,
    pub config: Value,
    pub data_dir: PathBuf,
    pub status: ModuleStatus,
    pub context: Option<ModuleContext>,
}

impl Record {
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    pub fn disable(&mut self, phase: &'static str, reason: String, table: &mut MethodTable) {
        tracing::error!(module = %self.manifest.id, phase, %reason, "module disabled");
        self.enabled.store(false, Ordering::Release);
        table.retract(&self.declared);
        self.status.enabled = false;
        self.status.phase = Some(phase);
        self.status.reason = Some(reason);
    }
}

/// Call every module's registration functions and put what they declare in
/// the table. A conflict disables the module that came second.
pub(crate) fn register(
    entries: Vec<Entry>,
    table: &mut MethodTable,
    sections: &BTreeMap<String, toml::Value>,
    state_root: &Path,
) -> Vec<Record> {
    let mut records: Vec<Record> = Vec::new();
    for entry in entries {
        let manifest = entry.module.manifest();
        let id = manifest.id.clone();
        let prefix = match entry.namespace {
            Namespace::Kernel => "harnix.".to_string(),
            Namespace::App => format!("app.{id}."),
        };

        let mut methods = MethodRegistry::new(prefix.clone());
        let mut events = EventRegistry::new(prefix.clone());
        let mut routes = RouteRegistry::new();
        let mut contrib = EngineContrib::new();
        let mut templates = InstanceTemplates::default();
        entry.module.templates(&mut templates);
        let templates: Vec<InstanceTemplate> = templates
            .into_list()
            .into_iter()
            .map(|mut t| {
                t.id = format!("{id}.{}", t.id);
                t
            })
            .collect();
        let mut spaces = harnix_module::SpaceRegistry::default();
        entry.module.spaces(&mut spaces);
        let spaces = spaces.into_list();
        entry.module.methods(&mut methods);
        entry.module.events(&mut events);
        entry.module.routes(&mut routes);
        entry.module.engine(&mut contrib);
        let methods = methods.into_decls();
        let topics = events.into_decls();
        let route = routes.into_decl();

        let config = sections
            .get(&id)
            .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
            .unwrap_or(Value::Null);
        let enabled = Arc::new(AtomicBool::new(true));
        let status = ModuleStatus {
            id: id.clone(),
            version: manifest.version.clone(),
            namespace: entry.namespace,
            enabled: true,
            phase: None,
            reason: None,
            disclosure: manifest.disclosure.clone(),
            grants: manifest.grants.clone(),
            methods: methods
                .iter()
                .map(|m| MethodStatus {
                    name: m.name.clone(),
                    principals: principals_label(&m.principals),
                    schema: m.schema.clone(),
                    description: m.description.clone(),
                })
                .collect(),
            topics: topics.iter().map(|t| t.name.clone()).collect(),
            routes: route.is_some(),
            scenes: contrib.scene_ids(),
            templates: templates.iter().map(|t| t.id.clone()).collect(),
            spaces: spaces.iter().map(|s| s.id.clone()).collect(),
            migrations: Vec::new(),
            note: None,
        };
        let mut record = Record {
            module: entry.module,
            manifest,
            prefix,
            methods,
            topics,
            route,
            contrib: contrib.gated(enabled.clone()),
            templates,
            spaces,
            enabled,
            declared: Vec::new(),
            config,
            data_dir: state_root.join("modules").join(&id),
            status,
            context: None,
        };

        let refusal = if let Err(e) = check_id(&id) {
            Some(e)
        } else if records.iter().any(|r| r.manifest.id == id) {
            Some(format!("another module already has the id `{id}`"))
        } else {
            declare(&mut record, table).err()
        };
        if let Some(reason) = refusal {
            record.disable("registration", reason, table);
        }
        records.push(record);
    }
    for section in sections.keys() {
        if !records.iter().any(|r| &r.manifest.id == section) {
            tracing::warn!(section = %section, "[modules.{section}] names no module this application has");
        }
    }
    records
}

/// Put one module's methods, topics, route and grants in the table. On the
/// first conflict, stop: the caller disables the module and retracts what
/// was already declared.
fn declare(record: &mut Record, table: &mut MethodTable) -> Result<(), String> {
    for method in &record.methods {
        table
            .declare(&method.name, Rule::for_principals(&method.principals))
            .map_err(|e| e.to_string())?;
        record.declared.push(method.name.clone());
    }
    for t in &record.topics {
        let name = topic(&t.name);
        table
            .declare(&name, Rule::for_principals(&t.principals))
            .map_err(|e| e.to_string())?;
        record.declared.push(name);
    }
    if let Some(r) = &record.route {
        let name = route(&record.manifest.id);
        table
            .declare(&name, Rule::for_principals(&r.principals))
            .map_err(|e| e.to_string())?;
        record.declared.push(name);
    }
    for grant in &record.manifest.grants {
        table.grant(grant, &record.manifest.id).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Migrate every enabled module, in order.
pub(crate) async fn migrate(records: &mut [Record], table: &mut MethodTable) {
    for record in records.iter_mut().filter(|r| r.is_enabled()) {
        if let Err(e) = std::fs::create_dir_all(&record.data_dir) {
            let reason = format!("its data directory {}: {e}", record.data_dir.display());
            record.disable("migration", reason, table);
            continue;
        }
        let cx = MigrateContext {
            id: record.manifest.id.clone(),
            data_dir: record.data_dir.clone(),
            config: record.config.clone(),
        };
        match record.module.migrate(&cx).await {
            Ok(report) => {
                if !report.applied.is_empty() {
                    tracing::info!(module = %record.manifest.id, steps = ?report.applied, "migrated");
                }
                record.status.migrations = report.applied;
            }
            Err(e) => record.disable("migration", format!("{e:#}"), table),
        }
    }
}

/// What the enabled modules give the `system` instance, in order.
pub(crate) fn engine_contrib(records: &[Record]) -> EngineContrib {
    let mut all = EngineContrib::new();
    for record in records.iter().filter(|r| r.is_enabled()) {
        all.extend(record.contrib.clone());
    }
    all
}

/// Who may call a module method or reach a module route, for a status line.
fn principals_label(principals: &[Principal]) -> Vec<String> {
    principals
        .iter()
        .map(|p| match p {
            Principal::Device => "device".to_string(),
            Principal::Kernel => "kernel".to_string(),
            Principal::AnyModule => "module:*".to_string(),
            Principal::Module(id) => format!("module:{id}"),
        })
        .collect()
}
