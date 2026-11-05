//! Two [`Packs`]: [`MemPacks`] (built in code) and [`DirPacks`] (layered directories).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::RwLock;

use crate::kernel::def::FlowDef;
use crate::kernel::host::{digest, Catalog, Packs};

#[derive(Default, Clone)]
struct Inner {
    /// name → (text, parsed)
    flows: BTreeMap<String, (String, Option<FlowDef>)>,
    skills: BTreeMap<String, String>,
    catalog: Catalog,
}

/// Definitions, skills and a catalog given in code.
///
/// ```
/// # use harnix_flow::MemPacks;
/// let packs = MemPacks::new()
///     .flow("name: demo\nstations: [{id: a, exit: [{kind: confirm}]}]")
///     .skill("analyze", "Analyze the module.")
///     .scene("chat");
/// ```
#[derive(Default)]
pub struct MemPacks {
    inner: RwLock<Inner>,
}

impl MemPacks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add (or replace) a definition. Its name is read from the text; a text that does not
    /// parse is kept under its `name:` line, if any, and never offered as a flow.
    pub fn flow(self, yaml: &str) -> Self {
        self.put_flow(yaml);
        self
    }

    pub fn skill(self, name: &str, body: &str) -> Self {
        self.put_skill(name, body);
        self
    }

    pub fn scene(self, name: &str) -> Self {
        self.edit(|i| i.catalog.scenes.insert(name.into()));
        self
    }

    pub fn step(self, name: &str) -> Self {
        self.edit(|i| i.catalog.steps.insert(name.into()));
        self
    }

    pub fn gate(self, name: &str) -> Self {
        self.edit(|i| i.catalog.gates.insert(name.into()));
        self
    }

    pub fn tool(self, name: &str) -> Self {
        self.edit(|i| i.catalog.tools.insert(name.into()));
        self
    }

    pub fn action(self, name: &str) -> Self {
        self.edit(|i| i.catalog.actions.insert(name.into()));
        self
    }

    /// Replace a definition in place (runs already started keep the one they pinned).
    pub fn put_flow(&self, yaml: &str) {
        let parsed = FlowDef::parse(yaml).ok();
        let name = match &parsed {
            Some(d) => d.name.clone(),
            None => serde_yaml::from_str::<serde_yaml::Value>(yaml)
                .ok()
                .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
                .unwrap_or_default(),
        };
        self.edit(|i| i.flows.insert(name, (yaml.to_string(), parsed)));
    }

    pub fn put_skill(&self, name: &str, body: &str) {
        self.edit(|i| i.skills.insert(name.into(), body.into()));
    }

    /// The text a flow was given as.
    pub fn flow_text(&self, name: &str) -> Option<String> {
        self.read().flows.get(name).map(|(t, _)| t.clone())
    }

    fn edit<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let mut g = self.inner.write().unwrap_or_else(|e| e.into_inner());
        f(&mut g)
    }

    fn read(&self) -> Inner {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Packs for MemPacks {
    fn flows(&self) -> Vec<String> {
        self.read()
            .flows
            .iter()
            .filter(|(_, (_, d))| d.is_some())
            .map(|(n, _)| n.clone())
            .collect()
    }

    fn flow(&self, name: &str) -> Option<(FlowDef, String)> {
        let g = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let (text, def) = g.flows.get(name)?;
        Some((def.clone()?, digest(text)))
    }

    fn skill(&self, name: &str) -> Option<(String, String)> {
        let g = self.inner.read().unwrap_or_else(|e| e.into_inner());
        g.skills.get(name).map(|b| (b.clone(), digest(b)))
    }

    fn catalog(&self) -> Catalog {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .catalog
            .clone()
    }
}

/// Packs read from directories, each laid out as `flows/<name>.yaml` and
/// `skills/<name>/SKILL.md`. A later directory overrides an earlier one, file by file
/// (built-in first, project last). Read on every call: an edit shows up in the next run.
pub struct DirPacks {
    dirs: Vec<PathBuf>,
    catalog: Catalog,
}

impl DirPacks {
    pub fn new(dirs: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        Self {
            dirs: dirs.into_iter().map(Into::into).collect(),
            catalog: Catalog::default(),
        }
    }

    /// The names the application registered (directories cannot say).
    pub fn with_catalog(mut self, catalog: Catalog) -> Self {
        self.catalog = catalog;
        self
    }

    /// The text of a flow, from the last directory that has it.
    pub fn flow_text(&self, name: &str) -> Option<String> {
        if !safe(name) {
            return None;
        }
        self.dirs.iter().rev().find_map(|d| {
            ["yaml", "yml"].iter().find_map(|ext| {
                std::fs::read_to_string(d.join("flows").join(format!("{name}.{ext}"))).ok()
            })
        })
    }
}

fn safe(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && !name.starts_with('.')
}

impl Packs for DirPacks {
    fn flows(&self) -> Vec<String> {
        let mut names = std::collections::BTreeSet::new();
        for d in &self.dirs {
            let Ok(entries) = std::fs::read_dir(d.join("flows")) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if matches!(p.extension().and_then(|x| x.to_str()), Some("yaml" | "yml")) {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        names.insert(stem.to_string());
                    }
                }
            }
        }
        names
            .into_iter()
            .filter(|n| self.flow(n).is_some())
            .collect()
    }

    fn flow(&self, name: &str) -> Option<(FlowDef, String)> {
        let text = self.flow_text(name)?;
        let def = FlowDef::parse(&text).ok()?;
        Some((def, digest(&text)))
    }

    fn skill(&self, name: &str) -> Option<(String, String)> {
        if !safe(name) {
            return None;
        }
        let body = self.dirs.iter().rev().find_map(|d| {
            std::fs::read_to_string(d.join("skills").join(name).join("SKILL.md")).ok()
        })?;
        let digest = digest(&body);
        Some((body, digest))
    }

    fn catalog(&self) -> Catalog {
        self.catalog.clone()
    }
}
