//! The module lifecycle, in the order concept §3.1 gives it.
//!
//! Registration, migration and start each happen to every module in the
//! order the application listed them. A module that fails at any of them is
//! disabled — its methods, topics and route taken back out of the
//! route table, its customizer no longer applied — and the rest
//! carry on. Why it was disabled is kept for the diagnostics page: "my space
//! does nothing" needs an answer, not a guess.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use harnix_authz::{access_for, Principal};
use harnix_kernel::Owner;
use harnix_router::{Row, RouteTable};
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
    /// The context its methods are answered with, once it is started.
    pub cx: Arc<OnceLock<ModuleContext>>,
    /// Whether its rows went in the table. One refused before they did — a second module with an
    /// id already taken — has nothing to withdraw, and the rows under that id are the other's.
    pub installed: bool,
    pub config: Value,
    pub data_dir: PathBuf,
    pub status: ModuleStatus,
    pub context: Option<ModuleContext>,
}

impl Record {
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Its rows' owner in the route table.
    pub fn owner(&self) -> Owner {
        Owner::extension(self.manifest.id.clone())
    }

    pub fn disable(&mut self, phase: &'static str, reason: String, table: &mut RouteTable) {
        if self.installed {
            table.withdraw(&self.owner());
        }
        self.disabled(phase, reason);
    }

    /// [`Record::disable`] once the table is the router's, which withdraws its rows itself.
    pub fn disabled(&mut self, phase: &'static str, reason: String) {
        tracing::error!(module = %self.manifest.id, phase, %reason, "module disabled");
        self.enabled.store(false, Ordering::Release);
        self.status.enabled = false;
        self.status.phase = Some(phase);
        self.status.reason = Some(reason);
    }
}

/// A spawned daemon's native contributions (ipdrun-10): the template `id` names, found by calling
/// `templates()` on each module in the order they were added, and nothing else of them — no
/// `start`, no methods. Only the application's own modules: the built-in ones declare no templates.
pub(crate) fn native_of(
    modules: &[Box<dyn harnix_module::ErasedModule>],
    id: &str,
) -> Result<EngineContrib, String> {
    for module in modules {
        let owner = module.manifest().id;
        let Some(name) = id.strip_prefix(&format!("{owner}.")) else { continue };
        let mut templates = InstanceTemplates::default();
        module.templates(&mut templates);
        if let Some(template) = templates.into_list().into_iter().find(|t| t.id == name) {
            return template
                .native_contrib()
                .ok_or_else(|| format!("the template `{id}` has no native contributions to build"));
        }
    }
    Err(format!("this binary declares no template `{id}`"))
}

/// Call every module's registration functions and put what they declare in
/// the table. A conflict disables the module that came second.
pub(crate) fn register(
    entries: Vec<Entry>,
    table: &mut RouteTable,
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
            cx: Arc::new(OnceLock::new()),
            installed: false,
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

/// Put one module's methods, topics and route in the table, all or none, and then its grants.
/// On a conflict the caller disables the module, which withdraws whatever of it went in.
fn declare(record: &mut Record, table: &mut RouteTable) -> Result<(), String> {
    let mut rows: Vec<Row> = record
        .methods
        .iter()
        .map(|method| {
            let handler = Arc::new(crate::gate::ModuleMethod {
                handler: method.handler.clone(),
                cx: record.cx.clone(),
                name: method.name.clone(),
            });
            Row::method(method.name.clone(), access_for(&method.principals), handler)
                .schema(method.schema.clone())
                .describe(method.description.clone())
        })
        .collect();
    rows.extend(record.topics.iter().map(|t| Row::topic(&t.name, access_for(&t.principals))));
    if let Some(r) = &record.route {
        rows.push(Row::route(&record.manifest.id, access_for(&r.principals)));
    }
    table.install(&record.owner(), rows).map_err(|e| e.to_string())?;
    record.installed = true;
    for grant in &record.manifest.grants {
        table.grant(grant, &record.manifest.id).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Migrate every enabled module, in order.
pub(crate) async fn migrate(records: &mut [Record], table: &mut RouteTable) {
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

#[cfg(test)]
mod tests {
    use harnix_contract::Subject;
    use harnix_module::{Module, RpcError};

    use super::*;

    struct Probe(&'static str);

    #[async_trait::async_trait]
    impl Module for Probe {
        const API_VERSION: u32 = harnix_module::MODULE_API_VERSION;

        fn manifest(&self) -> Manifest {
            Manifest::new("probe", self.0)
        }

        fn methods(&self, methods: &mut MethodRegistry) {
            methods
                .method("echo", Principal::Device)
                .raw(|_cx, params| async move { Ok::<_, RpcError>(params) });
        }
    }

    /// A second module with an id already taken is refused, and the first keeps its rows: the
    /// refusal withdraws only what the refused one put in, which is nothing.
    #[test]
    fn a_module_refused_for_its_id_takes_nothing_of_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let mut table = RouteTable::new();
        let entries = ["0.0.1", "9.9.9"]
            .into_iter()
            .map(|v| Entry { module: Box::new(Probe(v)), namespace: Namespace::App })
            .collect();
        let records = register(entries, &mut table, &BTreeMap::new(), dir.path());
        assert!(records[0].is_enabled());
        assert!(!records[1].is_enabled());
        let device = Subject::Device { id: "d".into() };
        assert_eq!(table.decide(&device, "app.probe.echo"), harnix_authz::Decision::Allow);
    }
}
