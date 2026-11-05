//! Two [`Packs`]: [`MemPacks`] (built in code) and [`DirPacks`] (layered directories).
//!
//! Both can be made writable (xops-02): a writable layer on top of what was shipped. Saving a
//! flow that a lower layer has stores an override (`Origin::Override`); deleting the override
//! brings the shipped one back. What is saved has been validated by the caller
//! (`Engine::save_flow`); the packs check only that it parses and that its name can be stored.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::kernel::def::FlowDef;
use crate::kernel::host::{digest, Catalog, Origin, Packs};
use crate::kernel::validate::is_skill_name;

/// A flow's name as a pack stores it: `[a-z][a-z0-9-]*`.
fn is_flow_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// The name a definition text declares, checked for storing.
fn name_to_save(yaml: &str) -> Result<String, String> {
    let def = FlowDef::parse(yaml).map_err(|e| format!("not a flow definition: {e}"))?;
    if !is_flow_name(&def.name) {
        return Err(format!("name {:?} must match [a-z][a-z0-9-]*", def.name));
    }
    if def.name.starts_with(crate::kernel::validate::DRY_PREFIX) {
        return Err(format!(
            "name {:?}: names starting `{}` are kept for dry runs",
            def.name,
            crate::kernel::validate::DRY_PREFIX
        ));
    }
    Ok(def.name)
}

#[derive(Default, Clone)]
struct Inner {
    /// name → (text, parsed): what was given in code.
    flows: BTreeMap<String, (String, Option<FlowDef>)>,
    skills: BTreeMap<String, String>,
    /// The writable layer, over the above.
    user_flows: BTreeMap<String, (String, Option<FlowDef>)>,
    user_skills: BTreeMap<String, String>,
    catalog: Catalog,
    writable: bool,
}

impl Inner {
    fn flow_entry(&self, name: &str) -> Option<&(String, Option<FlowDef>)> {
        self.user_flows.get(name).or_else(|| self.flows.get(name))
    }
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

    /// A template session stations may name (the catalog checks templates once one is given).
    pub fn template(self, name: &str) -> Self {
        self.edit(|i| i.catalog.templates.insert(name.into()));
        self
    }

    pub fn instance(self, name: &str) -> Self {
        self.edit(|i| i.catalog.instances.insert(name.into()));
        self
    }

    pub fn space(self, name: &str) -> Self {
        self.edit(|i| i.catalog.spaces.insert(name.into()));
        self
    }

    /// `model`, or `provider/model`.
    pub fn model(self, name: &str) -> Self {
        self.edit(|i| i.catalog.models.insert(name.into()));
        self
    }

    /// A scene whose sessions hand in with `flow_submit` (what the flow module's catalog says of
    /// its `flow-*` scenes): it is in the catalog's `scenes` and `submit_scenes`.
    pub fn submit_scene(self, name: &str) -> Self {
        self.edit(|i| {
            i.catalog.scenes.insert(name.into());
            i.catalog.submit_scenes.insert(name.into())
        });
        self
    }

    /// The scene a run's main session is opened on when a flow's `main` names none (a submit
    /// scene: it is added as one).
    pub fn main_scene(self, name: &str) -> Self {
        self.edit(|i| {
            i.catalog.scenes.insert(name.into());
            i.catalog.submit_scenes.insert(name.into());
            i.catalog.main_scene = Some(name.into());
        });
        self
    }

    /// Accept saved flows and skills, in memory, over the ones given in code.
    pub fn writable(self) -> Self {
        self.edit(|i| i.writable = true);
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
        self.read().flow_entry(name).map(|(t, _)| t.clone())
    }

    fn edit<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let mut g = self.inner.write().unwrap_or_else(|e| e.into_inner());
        f(&mut g)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }
}

impl Packs for MemPacks {
    fn flows(&self) -> Vec<String> {
        let g = self.read();
        let names: BTreeSet<&String> = g.flows.keys().chain(g.user_flows.keys()).collect();
        names
            .into_iter()
            .filter(|n| g.flow_entry(n).is_some_and(|(_, d)| d.is_some()))
            .cloned()
            .collect()
    }

    fn flow(&self, name: &str) -> Option<(FlowDef, String)> {
        let g = self.read();
        let (text, def) = g.flow_entry(name)?;
        Some((def.clone()?, digest(text)))
    }

    fn skill(&self, name: &str) -> Option<(String, String)> {
        let g = self.read();
        g.user_skills
            .get(name)
            .or_else(|| g.skills.get(name))
            .map(|b| (b.clone(), digest(b)))
    }

    fn catalog(&self) -> Catalog {
        self.read().catalog.clone()
    }

    fn flow_text(&self, name: &str) -> Option<String> {
        MemPacks::flow_text(self, name)
    }

    fn skills(&self) -> Vec<String> {
        let g = self.read();
        let names: BTreeSet<&String> = g.skills.keys().chain(g.user_skills.keys()).collect();
        names.into_iter().cloned().collect()
    }

    fn writable(&self) -> bool {
        self.read().writable
    }

    fn origin(&self, flow: &str) -> Origin {
        let g = self.read();
        match (g.user_flows.contains_key(flow), g.flows.contains_key(flow)) {
            (true, true) => Origin::Override,
            (true, false) => Origin::User,
            _ => Origin::Builtin,
        }
    }

    fn save_flow(&self, yaml: &str) -> Result<String, String> {
        if !Packs::writable(self) {
            return Err("read-only".into());
        }
        let name = name_to_save(yaml)?;
        let parsed = FlowDef::parse(yaml).ok();
        self.edit(|i| i.user_flows.insert(name, (yaml.to_string(), parsed)));
        Ok(digest(yaml))
    }

    fn delete_flow(&self, name: &str) -> Result<(), String> {
        if !Packs::writable(self) {
            return Err("read-only".into());
        }
        self.edit(|i| i.user_flows.remove(name))
            .map(|_| ())
            .ok_or_else(|| format!("flow {name} is built in: there is nothing saved to delete"))
    }

    fn save_skill(&self, name: &str, body: &str) -> Result<String, String> {
        if !Packs::writable(self) {
            return Err("read-only".into());
        }
        if !is_skill_name(name) {
            return Err(format!("skill name {name:?} must match [a-z][a-z0-9_-]*"));
        }
        self.edit(|i| i.user_skills.insert(name.into(), body.into()));
        Ok(digest(body))
    }

    fn delete_skill(&self, name: &str) -> Result<(), String> {
        if !Packs::writable(self) {
            return Err("read-only".into());
        }
        self.edit(|i| i.user_skills.remove(name))
            .map(|_| ())
            .ok_or_else(|| format!("skill {name} is built in: there is nothing saved to delete"))
    }
}

/// Packs read from directories, each laid out as `flows/<name>.yaml` and
/// `skills/<name>/SKILL.md`. A later directory overrides an earlier one, file by file
/// (built-in first, project last). Read on every call: an edit shows up in the next run.
///
/// Below the directories there may be packs built into the binary ([`DirPacks::with_builtin`]);
/// above them a writable directory ([`DirPacks::with_writable`], or [`DirPacks::writable`] for
/// one alone), where saving and deleting happen.
pub struct DirPacks {
    dirs: Vec<PathBuf>,
    builtin: Option<MemPacks>,
    top: Option<PathBuf>,
    catalog: Catalog,
}

impl DirPacks {
    pub fn new(dirs: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        Self {
            dirs: dirs.into_iter().map(Into::into).collect(),
            builtin: None,
            top: None,
            catalog: Catalog::default(),
        }
    }

    /// Packs with one writable directory and nothing under it (add some with
    /// [`DirPacks::with_builtin`]).
    pub fn writable(dir: impl Into<PathBuf>) -> Self {
        Self::new(Vec::<PathBuf>::new()).with_writable(dir)
    }

    /// A writable directory on top of every other layer: what is saved goes here.
    pub fn with_writable(mut self, dir: impl Into<PathBuf>) -> Self {
        self.top = Some(dir.into());
        self
    }

    /// Flows and skills built into the binary, under every directory. Its catalog's names are
    /// added to [`DirPacks::with_catalog`]'s.
    pub fn with_builtin(mut self, packs: MemPacks) -> Self {
        self.builtin = Some(packs);
        self
    }

    /// The names the application registered (directories cannot say).
    pub fn with_catalog(mut self, catalog: Catalog) -> Self {
        self.catalog = catalog;
        self
    }

    /// The directory saving writes to.
    pub fn writable_dir(&self) -> Option<&Path> {
        self.top.as_deref()
    }

    /// Read-only directories, lowest first.
    fn read_dirs(&self) -> impl DoubleEndedIterator<Item = &PathBuf> {
        self.dirs.iter()
    }

    fn text_in(dir: &Path, name: &str) -> Option<String> {
        ["yaml", "yml"].iter().find_map(|ext| {
            std::fs::read_to_string(dir.join("flows").join(format!("{name}.{ext}"))).ok()
        })
    }

    /// Whether a layer below the writable one offers `name`.
    fn shipped(&self, name: &str) -> bool {
        self.read_dirs().any(|d| Self::text_in(d, name).is_some())
            || self
                .builtin
                .as_ref()
                .is_some_and(|b| b.flow_text(name).is_some())
    }

    /// The text of a flow, from the highest layer that has it.
    pub fn flow_text(&self, name: &str) -> Option<String> {
        if !safe(name) {
            return None;
        }
        self.top
            .iter()
            .chain(self.read_dirs().rev())
            .find_map(|d| Self::text_in(d, name))
            .or_else(|| self.builtin.as_ref().and_then(|b| b.flow_text(name)))
    }

    fn top_dir(&self) -> Result<&Path, String> {
        self.top.as_deref().ok_or_else(|| "read-only".to_string())
    }
}

fn safe(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\']) && !name.starts_with('.')
}

/// Write a file whole: to a temporary beside it, then renamed over it.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let dir = path.parent().ok_or("no directory")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("x"),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

impl Packs for DirPacks {
    fn flows(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for d in self.read_dirs().chain(self.top.iter()) {
            let Ok(entries) = std::fs::read_dir(d.join("flows")) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if matches!(p.extension().and_then(|x| x.to_str()), Some("yaml" | "yml")) {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        if !stem.starts_with('.') {
                            names.insert(stem.to_string());
                        }
                    }
                }
            }
        }
        if let Some(b) = &self.builtin {
            names.extend(b.flows());
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
        let body = self
            .top
            .iter()
            .chain(self.read_dirs().rev())
            .find_map(|d| {
                std::fs::read_to_string(d.join("skills").join(name).join("SKILL.md")).ok()
            })
            .or_else(|| {
                self.builtin
                    .as_ref()
                    .and_then(|b| b.skill(name).map(|(b, _)| b))
            })?;
        let digest = digest(&body);
        Some((body, digest))
    }

    fn catalog(&self) -> Catalog {
        let mut c = self.catalog.clone();
        if let Some(b) = &self.builtin {
            let bc = b.catalog();
            c.scenes.extend(bc.scenes);
            c.steps.extend(bc.steps);
            c.gates.extend(bc.gates);
            c.tools.extend(bc.tools);
            c.actions.extend(bc.actions);
            c.templates.extend(bc.templates);
            c.instances.extend(bc.instances);
            c.spaces.extend(bc.spaces);
            c.models.extend(bc.models);
        }
        c
    }

    fn flow_text(&self, name: &str) -> Option<String> {
        DirPacks::flow_text(self, name)
    }

    fn skills(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for d in self.read_dirs().chain(self.top.iter()) {
            let Ok(entries) = std::fs::read_dir(d.join("skills")) else {
                continue;
            };
            for e in entries.flatten() {
                if e.path().join("SKILL.md").is_file() {
                    if let Some(n) = e.file_name().to_str() {
                        names.insert(n.to_string());
                    }
                }
            }
        }
        if let Some(b) = &self.builtin {
            names.extend(b.skills());
        }
        names.into_iter().collect()
    }

    fn writable(&self) -> bool {
        self.top.is_some()
    }

    fn origin(&self, flow: &str) -> Origin {
        let saved = safe(flow)
            && self
                .top
                .as_ref()
                .is_some_and(|t| Self::text_in(t, flow).is_some());
        match (saved, self.shipped(flow)) {
            (true, true) => Origin::Override,
            (true, false) => Origin::User,
            _ => Origin::Builtin,
        }
    }

    fn save_flow(&self, yaml: &str) -> Result<String, String> {
        let top = self.top_dir()?;
        let name = name_to_save(yaml)?;
        let flows = top.join("flows");
        write_atomic(&flows.join(format!("{name}.yaml")), yaml)?;
        // One file per flow in the layer: a `.yml` left from before would win the next read.
        let _ = std::fs::remove_file(flows.join(format!("{name}.yml")));
        Ok(digest(yaml))
    }

    fn delete_flow(&self, name: &str) -> Result<(), String> {
        let top = self.top_dir()?;
        if !safe(name) {
            return Err(format!("no flow {name}"));
        }
        let mut removed = false;
        for ext in ["yaml", "yml"] {
            removed |=
                std::fs::remove_file(top.join("flows").join(format!("{name}.{ext}"))).is_ok();
        }
        if removed {
            Ok(())
        } else {
            Err(format!(
                "flow {name} is built in: there is nothing saved to delete"
            ))
        }
    }

    fn save_skill(&self, name: &str, body: &str) -> Result<String, String> {
        let top = self.top_dir()?;
        if !is_skill_name(name) {
            return Err(format!("skill name {name:?} must match [a-z][a-z0-9_-]*"));
        }
        write_atomic(&top.join("skills").join(name).join("SKILL.md"), body)?;
        Ok(digest(body))
    }

    fn delete_skill(&self, name: &str) -> Result<(), String> {
        let top = self.top_dir()?;
        if !is_skill_name(name) {
            return Err(format!("no skill {name}"));
        }
        let dir = top.join("skills").join(name);
        if !dir.join("SKILL.md").is_file() {
            return Err(format!(
                "skill {name} is built in: there is nothing saved to delete"
            ));
        }
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))
    }
}
