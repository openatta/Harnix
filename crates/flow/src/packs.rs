//! Three [`Packs`]: [`MemPacks`] (built in code), [`DirPacks`] (layered directories), and
//! [`ProjectPacks`] (a project's own, over either, for runs in that project).
//!
//! Both can be made writable (xops-02): a writable layer on top of what was shipped. Saving a
//! flow that a lower layer has stores an override (`Origin::Override`); deleting the override
//! brings the shipped one back. What is saved has been validated by the caller
//! (`Engine::save_flow`); the packs check only that it parses and that its name can be stored.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::kernel::def::FlowDef;
use crate::kernel::host::{digest, Catalog, Origin, Packs, Withheld};
use crate::kernel::layout::ProjectLayout;
use crate::kernel::parts::PartKind;
use crate::kernel::validate::is_part_name;

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
    parts: BTreeMap<(PartKind, String), String>,
    /// The writable layer, over the above.
    user_flows: BTreeMap<String, (String, Option<FlowDef>)>,
    user_parts: BTreeMap<(PartKind, String), String>,
    catalog: Catalog,
    writable: bool,
}

impl Inner {
    fn flow_entry(&self, name: &str) -> Option<&(String, Option<FlowDef>)> {
        self.user_flows.get(name).or_else(|| self.flows.get(name))
    }

    fn part_entry(&self, kind: PartKind, name: &str) -> Option<&String> {
        let k = (kind, name.to_string());
        self.user_parts.get(&k).or_else(|| self.parts.get(&k))
    }
}

/// Definitions, parts (skills, rules, policies, gates, hooks) and a catalog given in code.
///
/// ```
/// # use harnix_flow::MemPacks;
/// let packs = MemPacks::new()
///     .flow("name: demo\nstations: [{id: a, exit: [{kind: confirm}]}]")
///     .skill("analyze", "Analyze the module.")
///     .scene("harnix");
/// ```
#[derive(Default)]
pub struct MemPacks {
    inner: RwLock<Inner>,
}

impl MemPacks {
    pub fn new() -> Self {
        Self::default()
    }

    /// The example pack the library carries (`packs/examples`), built in: the flow
    /// `example-review` (the run's main Agent reviews the subject, a person approves), the skill
    /// `review`, the rule `review-honest` and the policy `read-only`. Whatever is in that directory
    /// is here (`every_file_of_the_example_pack_is_in_it`).
    pub fn examples() -> Self {
        Self::new()
            .flow(include_str!("../packs/examples/flows/example-review.yaml"))
            .skill("review", include_str!("../packs/examples/skills/review/SKILL.md"))
            .rule("review-honest", include_str!("../packs/examples/rules/review-honest.md"))
            .policy(include_str!("../packs/examples/policies/read-only.yaml"))
    }

    /// Add (or replace) a definition. Its name is read from the text; a text that does not
    /// parse is kept under its `name:` line, if any, and never offered as a flow.
    pub fn flow(self, yaml: &str) -> Self {
        self.put_flow(yaml);
        self
    }

    pub fn skill(self, name: &str, body: &str) -> Self {
        self.part(PartKind::Skill, name, body)
    }

    /// Add (or replace) a part of any kind.
    pub fn part(self, kind: PartKind, name: &str, text: &str) -> Self {
        self.put_part(kind, name, text);
        self
    }

    pub fn rule(self, name: &str, text: &str) -> Self {
        self.part(PartKind::Rule, name, text)
    }

    /// A policy; its name is read from the text.
    pub fn policy(self, yaml: &str) -> Self {
        let name = yaml_name(yaml);
        self.part(PartKind::Policy, &name, yaml)
    }

    /// A gate of the pack; its name is read from the text. (A name the application's own checks
    /// answer to, for `host` gates, is [`MemPacks::check`].)
    pub fn gate_def(self, yaml: &str) -> Self {
        let name = yaml_name(yaml);
        self.part(PartKind::Gate, &name, yaml)
    }

    /// A hook; its name is read from the text.
    pub fn hook(self, yaml: &str) -> Self {
        let name = yaml_name(yaml);
        self.part(PartKind::Hook, &name, yaml)
    }

    pub fn scene(self, name: &str) -> Self {
        self.edit(|i| i.catalog.scenes.insert(name.into()));
        self
    }

    pub fn step(self, name: &str) -> Self {
        self.edit(|i| i.catalog.steps.insert(name.into()));
        self
    }

    /// A check of the application's own (`Gates`) that `host` gates may name.
    pub fn check(self, name: &str) -> Self {
        self.edit(|i| i.catalog.gates.insert(name.into()));
        self
    }

    pub fn tool(self, name: &str) -> Self {
        self.edit(|i| i.catalog.tools.insert(name.into()));
        self
    }

    /// Let `command` gates run.
    pub fn command_gates(self) -> Self {
        self.edit(|i| i.catalog.command_gates = true);
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

    pub fn put_part(&self, kind: PartKind, name: &str, text: &str) {
        self.edit(|i| i.parts.insert((kind, name.into()), text.into()));
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

    fn part(&self, kind: PartKind, name: &str) -> Option<(String, String)> {
        let g = self.read();
        g.part_entry(kind, name).map(|b| (b.clone(), digest(b)))
    }

    fn parts(&self, kind: PartKind) -> Vec<String> {
        let g = self.read();
        let names: BTreeSet<&String> = g
            .parts
            .keys()
            .chain(g.user_parts.keys())
            .filter(|(k, _)| *k == kind)
            .map(|(_, n)| n)
            .collect();
        names.into_iter().cloned().collect()
    }

    fn catalog(&self) -> Catalog {
        self.read().catalog.clone()
    }

    fn flow_text(&self, name: &str) -> Option<String> {
        MemPacks::flow_text(self, name)
    }

    fn writable(&self) -> bool {
        self.read().writable
    }

    fn part_origin(&self, kind: PartKind, name: &str) -> Origin {
        let g = self.read();
        let k = (kind, name.to_string());
        match (g.user_parts.contains_key(&k), g.parts.contains_key(&k)) {
            (true, true) => Origin::Override,
            (true, false) => Origin::User,
            _ => Origin::Builtin,
        }
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

    fn save_part(&self, kind: PartKind, name: &str, text: &str) -> Result<String, String> {
        if !Packs::writable(self) {
            return Err("read-only".into());
        }
        if !is_part_name(kind, name) {
            return Err(format!("{kind} name {name:?} must match [a-z][a-z0-9-]*"));
        }
        self.edit(|i| i.user_parts.insert((kind, name.into()), text.into()));
        Ok(digest(text))
    }

    fn delete_part(&self, kind: PartKind, name: &str) -> Result<(), String> {
        if !Packs::writable(self) {
            return Err("read-only".into());
        }
        self.edit(|i| i.user_parts.remove(&(kind, name.to_string())))
            .map(|_| ())
            .ok_or_else(|| format!("{kind} {name} is built in: there is nothing saved to delete"))
    }
}

/// The `name:` a YAML text declares, if any.
fn yaml_name(yaml: &str) -> String {
    serde_yaml::from_str::<serde_yaml::Value>(yaml)
        .ok()
        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
        .unwrap_or_default()
}

/// Packs read from directories, each laid out as `flows/<name>.yaml`,
/// `skills/<name>/SKILL.md`, `rules/<name>.md`, `policies/<name>.yaml`, `gates/<name>.yaml` and
/// `hooks/<name>.yaml`. A later directory overrides an earlier one, file by file
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

    /// Flows and parts built into the binary, under every directory. Its catalog's names are
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

    fn part_in(dir: &Path, kind: PartKind, name: &str) -> Option<String> {
        std::fs::read_to_string(dir.join(kind.dir()).join(kind.file(name))).ok()
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

    fn part(&self, kind: PartKind, name: &str) -> Option<(String, String)> {
        if !safe(name) {
            return None;
        }
        let body = self
            .top
            .iter()
            .chain(self.read_dirs().rev())
            .find_map(|d| Self::part_in(d, kind, name))
            .or_else(|| {
                self.builtin
                    .as_ref()
                    .and_then(|b| b.part(kind, name).map(|(b, _)| b))
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
            c.command_gates |= bc.command_gates;
            c.templates.extend(bc.templates);
            c.instances.extend(bc.instances);
            c.spaces.extend(bc.spaces);
            c.models.extend(bc.models);
            c.submit_scenes.extend(bc.submit_scenes);
            if c.main_scene.is_none() {
                c.main_scene = bc.main_scene;
            }
        }
        c
    }

    fn flow_text(&self, name: &str) -> Option<String> {
        DirPacks::flow_text(self, name)
    }

    fn parts(&self, kind: PartKind) -> Vec<String> {
        let mut names = BTreeSet::new();
        for d in self.read_dirs().chain(self.top.iter()) {
            let Ok(entries) = std::fs::read_dir(d.join(kind.dir())) else {
                continue;
            };
            for e in entries.flatten() {
                let Some(n) = e.file_name().to_str().map(String::from) else {
                    continue;
                };
                let name = match kind {
                    PartKind::Skill => e.path().join("SKILL.md").is_file().then_some(n),
                    _ => {
                        let ext = if kind == PartKind::Rule { ".md" } else { ".yaml" };
                        n.strip_suffix(ext).filter(|s| !s.starts_with('.')).map(String::from)
                    }
                };
                names.extend(name);
            }
        }
        if let Some(b) = &self.builtin {
            names.extend(b.parts(kind));
        }
        names.into_iter().collect()
    }

    fn part_origin(&self, kind: PartKind, name: &str) -> Origin {
        let saved = safe(name)
            && self
                .top
                .as_ref()
                .is_some_and(|t| Self::part_in(t, kind, name).is_some());
        let shipped = self.read_dirs().any(|d| Self::part_in(d, kind, name).is_some())
            || self
                .builtin
                .as_ref()
                .is_some_and(|b| b.part(kind, name).is_some());
        match (saved, shipped) {
            (true, true) => Origin::Override,
            (true, false) => Origin::User,
            _ => Origin::Builtin,
        }
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

    fn save_part(&self, kind: PartKind, name: &str, text: &str) -> Result<String, String> {
        let top = self.top_dir()?;
        if !is_part_name(kind, name) {
            return Err(format!("{kind} name {name:?} must match [a-z][a-z0-9-]*"));
        }
        write_atomic(&top.join(kind.dir()).join(kind.file(name)), text)?;
        Ok(digest(text))
    }

    fn delete_part(&self, kind: PartKind, name: &str) -> Result<(), String> {
        let top = self.top_dir()?;
        if !is_part_name(kind, name) {
            return Err(format!("no {kind} {name}"));
        }
        let path = top.join(kind.dir()).join(kind.file(name));
        if !path.is_file() {
            return Err(format!(
                "{kind} {name} is built in: there is nothing saved to delete"
            ));
        }
        match kind {
            PartKind::Skill => {
                let dir = top.join(kind.dir()).join(name);
                std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))
            }
            _ => std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display())),
        }
    }
}

// ── a project's own packs ────────────────────────────────────────────────────

/// Where a project keeps its own packs, committed with it: `<project>/<dir>/flows/` (Harnix:
/// `.harnix`), the flows as `<name>.yaml` and the parts under `skills/`, `rules/`, `policies/`,
/// `gates/`, `hooks/`.
pub fn project_pack_dir(root: &Path, layout: &ProjectLayout) -> PathBuf {
    layout.pack_dir(root)
}

/// A project root as trust is kept by it: its real path when it has one, else as written without
/// a trailing `/` (the engine reports a trusted root resolved).
fn root_key(root: &str) -> PathBuf {
    std::fs::canonicalize(root).unwrap_or_else(|_| PathBuf::from(root.trim_end_matches('/')))
}

/// Whether projects are trusted, as last heard from the engine (`project.status`), by root. A
/// project not heard of is not trusted.
#[derive(Default)]
pub struct ProjectTrust(RwLock<HashMap<PathBuf, bool>>);

impl ProjectTrust {
    pub fn set(&self, root: &str, trusted: bool) {
        let mut g = self.0.write().unwrap_or_else(|e| e.into_inner());
        g.insert(root_key(root), trusted);
    }

    pub fn trusted(&self, root: &str) -> bool {
        let g = self.0.read().unwrap_or_else(|e| e.into_inner());
        g.get(&root_key(root)).copied().unwrap_or(false)
    }

    /// Whether any project is trusted: else no project's own packs are read anywhere.
    pub fn any_trusted(&self) -> bool {
        let g = self.0.read().unwrap_or_else(|e| e.into_inner());
        g.values().any(|t| *t)
    }
}

/// A project's own packs ([`project_pack_dir`]) over the application's: what a run in that
/// project sees. One more layer on top, flows and parts alike — a name resolves to the project's,
/// then to whatever the application offers under it — so a project's flow may use the built-in
/// and saved parts, and every flow run in the project uses the project's part of a name it has.
/// Outside the project none of it is there.
///
/// While the project is not trusted nothing of its own is read: its flows are listed by file
/// name as withheld ([`Withheld::Untrusted`]), each hiding a flow of the same name below it, so
/// a name always means the project's in the project; its parts are not there. Saving writes in
/// the project, and only while it is trusted. Read on every call, as [`DirPacks`]: an edit, a
/// checkout or a pull shows up in the next listing and the next run.
pub struct ProjectPacks {
    base: Arc<dyn Packs>,
    root: PathBuf,
    dir: PathBuf,
    trusted: bool,
    layout: ProjectLayout,
}

impl ProjectPacks {
    pub fn new(base: Arc<dyn Packs>, root: impl Into<PathBuf>, layout: &ProjectLayout, trusted: bool) -> Self {
        let root = root.into();
        let dir = project_pack_dir(&root, layout);
        Self { base, root, dir, trusted, layout: layout.clone() }
    }

    /// The directory the project's packs are read from and saved to.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn trusted(&self) -> bool {
        self.trusted
    }

    fn flow_file(&self, name: &str) -> Option<PathBuf> {
        if !safe(name) {
            return None;
        }
        ["yaml", "yml"]
            .iter()
            .map(|ext| self.dir.join(format!("{name}.{ext}")))
            .find(|p| p.is_file())
    }

    /// The names of the project's flow files, unread.
    fn own_names(&self) -> BTreeSet<String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return BTreeSet::new();
        };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("yaml" | "yml")))
            .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
            .filter(|n| safe(n))
            .collect()
    }

    /// The project's definition of `name`, when trusted and it is one, of that name.
    fn own_flow(&self, name: &str) -> Option<(FlowDef, String)> {
        let text = self.own_text(name)?;
        let def = FlowDef::parse(&text).ok().filter(|d| d.name == name)?;
        Some((def, digest(&text)))
    }

    fn own_text(&self, name: &str) -> Option<String> {
        if !self.trusted {
            return None;
        }
        std::fs::read_to_string(self.flow_file(name)?).ok()
    }

    fn own_part(&self, kind: PartKind, name: &str) -> Option<String> {
        if !self.trusted || !safe(name) {
            return None;
        }
        DirPacks::part_in(&self.dir, kind, name)
    }

    fn own_parts(&self, kind: PartKind) -> BTreeSet<String> {
        if !self.trusted {
            return BTreeSet::new();
        }
        let Ok(entries) = std::fs::read_dir(self.dir.join(kind.dir())) else {
            return BTreeSet::new();
        };
        entries
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_str().map(String::from)?;
                match kind {
                    PartKind::Skill => e.path().join("SKILL.md").is_file().then_some(n),
                    _ => {
                        let ext = if kind == PartKind::Rule { ".md" } else { ".yaml" };
                        n.strip_suffix(ext).filter(|s| !s.starts_with('.')).map(String::from)
                    }
                }
            })
            .collect()
    }

    /// Why nothing is written: the project is not trusted.
    fn must_trust(&self) -> Result<(), String> {
        if self.trusted {
            Ok(())
        } else {
            Err(format!(
                "the project {} is not trusted: trust it first, its own flows are not read or written until then",
                self.root.display()
            ))
        }
    }

    /// Write `path` (under the project's pack directory) whole, refusing a directory that
    /// resolves elsewhere — a data directory or a part directory that links out of the project.
    fn write(&self, path: &Path, text: &str) -> Result<(), String> {
        let parent = path.parent().ok_or("no directory")?;
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        let root = std::fs::canonicalize(&self.root).map_err(|e| format!("{}: {e}", self.root.display()))?;
        let at = std::fs::canonicalize(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        if !at.starts_with(project_pack_dir(&root, &self.layout)) {
            return Err(format!(
                "{} is not inside {}: nothing is written outside the project's packs",
                parent.display(),
                project_pack_dir(&root, &self.layout).display()
            ));
        }
        write_atomic(path, text)
    }
}

impl Packs for ProjectPacks {
    fn flows(&self) -> Vec<String> {
        let own = self.own_names();
        let mut names: BTreeSet<String> =
            self.base.flows().into_iter().filter(|n| !own.contains(n)).collect();
        names.extend(own.into_iter().filter(|n| self.own_flow(n).is_some()));
        names.into_iter().collect()
    }

    fn flow(&self, name: &str) -> Option<(FlowDef, String)> {
        if self.flow_file(name).is_some() {
            return self.own_flow(name);
        }
        self.base.flow(name)
    }

    fn catalog(&self) -> Catalog {
        self.base.catalog()
    }

    fn flow_text(&self, name: &str) -> Option<String> {
        if self.flow_file(name).is_some() {
            return self.own_text(name);
        }
        self.base.flow_text(name)
    }

    fn part(&self, kind: PartKind, name: &str) -> Option<(String, String)> {
        match self.own_part(kind, name) {
            Some(text) => {
                let d = digest(&text);
                Some((text, d))
            }
            None => self.base.part(kind, name),
        }
    }

    fn parts(&self, kind: PartKind) -> Vec<String> {
        let mut names: BTreeSet<String> = self.base.parts(kind).into_iter().collect();
        names.extend(self.own_parts(kind));
        names.into_iter().collect()
    }

    fn writable(&self) -> bool {
        self.trusted
    }

    fn origin(&self, flow: &str) -> Origin {
        if self.flow_file(flow).is_some() {
            Origin::Project
        } else {
            self.base.origin(flow)
        }
    }

    fn part_origin(&self, kind: PartKind, name: &str) -> Origin {
        if self.own_part(kind, name).is_some() {
            Origin::Project
        } else {
            self.base.part_origin(kind, name)
        }
    }

    fn save_flow(&self, yaml: &str) -> Result<String, String> {
        self.must_trust()?;
        let name = name_to_save(yaml)?;
        self.write(&self.dir.join(format!("{name}.yaml")), yaml)?;
        let _ = std::fs::remove_file(self.dir.join(format!("{name}.yml")));
        Ok(digest(yaml))
    }

    fn delete_flow(&self, name: &str) -> Result<(), String> {
        self.must_trust()?;
        let mut removed = false;
        if safe(name) {
            for ext in ["yaml", "yml"] {
                removed |= std::fs::remove_file(self.dir.join(format!("{name}.{ext}"))).is_ok();
            }
        }
        if removed {
            Ok(())
        } else {
            Err(format!("flow {name} is not the project's: there is nothing of the project's to delete"))
        }
    }

    fn save_part(&self, kind: PartKind, name: &str, text: &str) -> Result<String, String> {
        self.must_trust()?;
        if !is_part_name(kind, name) {
            return Err(format!("{kind} name {name:?} must match [a-z][a-z0-9-]*"));
        }
        self.write(&self.dir.join(kind.dir()).join(kind.file(name)), text)?;
        Ok(digest(text))
    }

    fn delete_part(&self, kind: PartKind, name: &str) -> Result<(), String> {
        self.must_trust()?;
        if !is_part_name(kind, name) || self.own_part(kind, name).is_none() {
            return Err(format!("{kind} {name} is not the project's: there is nothing of the project's to delete"));
        }
        let path = match kind {
            PartKind::Skill => self.dir.join(kind.dir()).join(name),
            _ => self.dir.join(kind.dir()).join(kind.file(name)),
        };
        let r = if kind == PartKind::Skill {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        r.map_err(|e| format!("{}: {e}", path.display()))
    }

    fn withheld_flows(&self) -> Vec<(String, Withheld)> {
        self.own_names()
            .into_iter()
            .filter_map(|n| {
                if !self.trusted {
                    Some((n, Withheld::Untrusted))
                } else {
                    self.own_flow(&n).is_none().then_some((n, Withheld::Unreadable))
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod example_tests {
    use super::*;

    /// Every file under `packs/examples` is a flow or a part of `MemPacks::examples`, with the same
    /// text, and it holds nothing else: a file added or moved there is not silently left out.
    #[test]
    fn every_file_of_the_example_pack_is_in_it() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("packs/examples");
        let packs = MemPacks::examples();
        let mut on_disk: Vec<String> = vec![];
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap() {
                let path = e.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    on_disk.push(path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        on_disk.sort();
        let mut held: Vec<(String, String)> = packs
            .flows()
            .into_iter()
            .map(|n| (format!("flows/{n}.yaml"), packs.flow_text(&n).unwrap()))
            .collect();
        for kind in PartKind::ALL {
            for n in Packs::parts(&packs, kind) {
                held.push((format!("{}/{}", kind.dir(), kind.file(&n)), Packs::part(&packs, kind, &n).unwrap().0));
            }
        }
        held.sort();
        let names: Vec<&str> = held.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, on_disk, "MemPacks::examples holds exactly the files of packs/examples");
        for (path, text) in &held {
            assert_eq!(&std::fs::read_to_string(root.join(path)).unwrap(), text, "{path}");
        }
        assert!(crate::kernel::validate::validate_yaml(
            &packs.flow_text("example-review").unwrap(),
            &MemPacks::examples().main_scene("flow-harnix") as &dyn Packs
        )
        .is_empty());
    }
}

#[cfg(test)]
mod project_tests {
    use super::*;

    /// A project's data directory as Harnix names it.
    fn layout() -> ProjectLayout {
        ProjectLayout::new(".harnix", "/state/worktrees", "harnix/flow")
    }

    const BASE: &str = "name: shared\ntitle: Built in\nstations: [{id: a, exit: [{kind: confirm}]}]";
    const OWN: &str = "name: shared\ntitle: The project's\nstations: [{id: a, exit: [{kind: confirm}]}]";
    const MINE: &str = "name: mine\nstations: [{id: a, skill: look, exit: [{kind: confirm}]}]";

    fn base() -> Arc<dyn Packs> {
        Arc::new(
            MemPacks::new()
                .flow(BASE)
                .flow("name: plain\nstations: [{id: a, skill: look, exit: [{kind: confirm}]}]")
                .skill("look", "Look as the application says.")
                .rule("tone", "Be terse."),
        )
    }

    /// A project with `shared` and `mine` of its own, a skill and a rule.
    fn project() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let dir = project_pack_dir(root.path(), &layout());
        std::fs::create_dir_all(dir.join("skills/look")).unwrap();
        std::fs::create_dir_all(dir.join("rules")).unwrap();
        std::fs::write(dir.join("shared.yaml"), OWN).unwrap();
        std::fs::write(dir.join("mine.yaml"), MINE).unwrap();
        std::fs::write(dir.join("skills/look/SKILL.md"), "Look as the project says.").unwrap();
        std::fs::write(dir.join("rules/house.md"), "# House\nKeep the house style.").unwrap();
        root
    }

    #[test]
    fn a_trusted_projects_own_flows_and_parts_are_over_the_rest() {
        let root = project();
        let p = ProjectPacks::new(base(), root.path(), &layout(), true);
        assert_eq!(p.flows(), ["mine", "plain", "shared"]);
        // The same name: the project's, and said so.
        assert_eq!(p.flow("shared").unwrap().0.title, "The project's");
        assert_eq!(p.origin("shared"), Origin::Project);
        assert_eq!(p.origin("plain"), Origin::Builtin);
        // Its parts are over the application's, for every flow run in the project; the
        // application's are there for its own flows.
        assert_eq!(p.skill("look").unwrap().0, "Look as the project says.");
        assert_eq!(p.part_origin(PartKind::Skill, "look"), Origin::Project);
        assert_eq!(p.rule("tone").unwrap().0, "Be terse.");
        assert!(p.rule("house").is_some());
        assert_eq!(p.parts(PartKind::Rule), ["house", "tone"]);
        assert!(p.withheld_flows().is_empty());
        assert!(p.writable());
        // Outside the project none of it is there.
        assert_eq!(base().flow("shared").unwrap().0.title, "Built in");
        assert!(base().flow("mine").is_none());
    }

    #[test]
    fn an_untrusted_projects_own_are_listed_unread_and_never_offered() {
        let root = project();
        let p = ProjectPacks::new(base(), root.path(), &layout(), false);
        // Not offered, and the same name below is hidden: in the project the name is the project's.
        assert_eq!(p.flows(), ["plain"]);
        assert!(p.flow("shared").is_none() && p.flow("mine").is_none());
        assert!(p.flow_text("shared").is_none());
        assert_eq!(p.origin("shared"), Origin::Project);
        assert_eq!(
            p.withheld_flows(),
            [("mine".to_string(), Withheld::Untrusted), ("shared".to_string(), Withheld::Untrusted)]
        );
        // Its parts are not read either.
        assert_eq!(p.skill("look").unwrap().0, "Look as the application says.");
        assert!(p.rule("house").is_none());
        // Nothing is written in it.
        assert!(!p.writable());
        let e = p.save_flow("name: new\nstations: [{id: a, exit: [{kind: confirm}]}]").unwrap_err();
        assert!(e.contains("not trusted"), "{e}");
        assert!(p.save_part(PartKind::Rule, "x", "# X").is_err());
        assert!(!project_pack_dir(root.path(), &layout()).join("new.yaml").exists());
    }

    #[test]
    fn a_project_file_that_is_not_its_flow_is_reported_and_hides_the_name() {
        let root = project();
        let dir = project_pack_dir(root.path(), &layout());
        std::fs::write(dir.join("broken.yaml"), "name: [not a flow").unwrap();
        // A definition of another name than its file's.
        std::fs::write(dir.join("plain.yaml"), "name: other\nstations: [{id: a, exit: [{kind: confirm}]}]").unwrap();
        let p = ProjectPacks::new(base(), root.path(), &layout(), true);
        assert_eq!(
            p.withheld_flows(),
            [("broken".to_string(), Withheld::Unreadable), ("plain".to_string(), Withheld::Unreadable)]
        );
        assert!(p.flow("broken").is_none() && p.flow("plain").is_none() && p.flow("other").is_none());
        assert_eq!(p.flows(), ["mine", "shared"]);
    }

    #[test]
    fn saving_writes_in_the_projects_pack_directory_alone() {
        let root = tempfile::tempdir().unwrap();
        let p = ProjectPacks::new(base(), root.path(), &layout(), true);
        p.save_flow("name: fresh\nstations: [{id: a, exit: [{kind: confirm}]}]").unwrap();
        p.save_part(PartKind::Skill, "probe", "Probe it.").unwrap();
        p.save_part(PartKind::Rule, "house", "# House").unwrap();
        let dir = project_pack_dir(root.path(), &layout());
        assert!(dir.join("fresh.yaml").is_file());
        assert!(dir.join("skills/probe/SKILL.md").is_file());
        assert!(dir.join("rules/house.md").is_file());
        assert_eq!(p.origin("fresh"), Origin::Project);
        // Names that would leave the directory are refused before anything is written.
        assert!(p.save_part(PartKind::Rule, "../escape", "x").is_err());
        assert!(p.save_flow("name: ../escape\nstations: [{id: a, exit: [{kind: confirm}]}]").is_err());
        assert!(!root.path().join(".harnix/escape.md").exists());
        // Deleting takes away the project's own only; the application's is back.
        p.delete_flow("fresh").unwrap();
        assert!(p.flow("fresh").is_none());
        assert!(p.delete_flow("plain").is_err(), "the application's flow is not the project's to delete");
        p.delete_part(PartKind::Skill, "probe").unwrap();
        assert!(!dir.join("skills/probe").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_pack_directory_that_links_out_of_the_project_is_not_written() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".harnix")).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), project_pack_dir(root.path(), &layout())).unwrap();
        let p = ProjectPacks::new(base(), root.path(), &layout(), true);
        let e = p.save_flow("name: fresh\nstations: [{id: a, exit: [{kind: confirm}]}]").unwrap_err();
        assert!(e.contains("not inside"), "{e}");
        assert!(std::fs::read_dir(elsewhere.path()).unwrap().next().is_none());
    }

    #[test]
    fn trust_is_kept_by_the_projects_real_root() {
        let root = tempfile::tempdir().unwrap();
        let trust = ProjectTrust::default();
        let at = root.path().to_str().unwrap().to_string();
        assert!(!trust.trusted(&at), "a project not heard of is not trusted");
        trust.set(&format!("{at}/"), true);
        assert!(trust.trusted(&at));
        trust.set(&at, false);
        assert!(!trust.trusted(&format!("{at}/")));
    }
}
