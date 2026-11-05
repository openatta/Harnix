//! Structural validation: everything a JSON Schema cannot say. A definition with any
//! problem cannot start a run; [`validate`] reports all of them at once.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::def::{Assert, Executor, ExitCond, FlowDef, Station, TaskSpec, Work, MAX_REWORK_LIMIT};
use super::host::{Catalog, Packs, EVENT_KINDS};
use super::refs::{refs_in, refs_in_value, ROOTS};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Problem {
    /// The station it is about; `None` for the flow as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.station {
            Some(s) => write!(f, "station {s}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Parse and validate a definition text.
pub fn validate_yaml(yaml: &str, packs: &dyn Packs) -> Vec<Problem> {
    match FlowDef::parse(yaml) {
        Ok(def) => validate(&def, packs),
        Err(e) => vec![Problem {
            station: None,
            message: format!("not a flow definition: {e}"),
        }],
    }
}

/// A skill's name, as a pack can store it: `[a-z][a-z0-9_-]*`.
pub fn is_skill_name(s: &str) -> bool {
    is_name(s, b"_")
}

fn is_name(s: &str, extra: &[u8]) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || extra.contains(c)
        })
}

struct Ctx<'a> {
    def: &'a FlowDef,
    packs: &'a dyn Packs,
    catalog: Catalog,
    out: Vec<Problem>,
}

impl Ctx<'_> {
    fn flow(&mut self, m: impl Into<String>) {
        self.out.push(Problem {
            station: None,
            message: m.into(),
        });
    }

    fn at(&mut self, st: &str, m: impl Into<String>) {
        self.out.push(Problem {
            station: Some(st.to_string()),
            message: m.into(),
        });
    }
}

/// Every structural problem of a definition; empty = valid.
pub fn validate(def: &FlowDef, packs: &dyn Packs) -> Vec<Problem> {
    let mut c = Ctx {
        def,
        packs,
        catalog: packs.catalog(),
        out: vec![],
    };
    if !is_name(&def.name, &[]) {
        c.flow(format!("name {:?} must match [a-z][a-z0-9-]*", def.name));
    } else if def.name.starts_with(DRY_PREFIX) {
        c.flow(format!(
            "name {:?}: names starting `{DRY_PREFIX}` are kept for dry runs",
            def.name
        ));
    }
    if def.stations.is_empty() {
        c.flow("a flow needs at least one station");
    }
    if let Some(n) = def.rework_limit {
        if n == 0 || n > MAX_REWORK_LIMIT {
            c.flow(format!(
                "rework_limit {n} must be between 1 and {MAX_REWORK_LIMIT}"
            ));
        }
    }
    for (k, i) in &def.inputs {
        if !is_name(k, b"_") {
            c.flow(format!("input name {k:?} must match [a-z][a-z0-9_-]*"));
        }
        schema_problems(&mut c, None, &format!("input {k}"), &i.schema);
        if let Some(d) = &i.default {
            if let Ok(v) = jsonschema::validator_for(&i.schema) {
                if !v.is_valid(d) {
                    c.flow(format!(
                        "input {k}: the default does not satisfy the schema"
                    ));
                }
            }
        }
    }
    for h in &def.hooks {
        hook_problems(&mut c, None, h.on.as_str(), h.action.as_str());
    }

    let mut ids = BTreeSet::new();
    for s in &def.stations {
        if !is_name(&s.id, b"_") {
            c.at(&s.id, "station id must match [a-z][a-z0-9_-]*");
        }
        if !ids.insert(s.id.as_str()) {
            c.at(&s.id, "station id is not unique");
        }
    }
    let cyclic = graph_problems(&mut c);
    for s in &def.stations {
        station_problems(&mut c, s, cyclic);
    }
    shared_session_problems(&mut c);
    c.out
}

/// The prefix of a dry run's id; no flow may be named with it.
pub const DRY_PREFIX: &str = "dry-";

/// What a session is opened with and keeps: everything a task sharing it must agree on.
#[derive(PartialEq)]
struct SessionSettings {
    scene: String,
    output: Option<Value>,
    refused: Vec<String>,
    model: Option<String>,
    provider: Option<String>,
    instance: Option<String>,
    template: Option<String>,
    params: BTreeMap<String, String>,
    space: Option<String>,
}

impl SessionSettings {
    /// The names of the fields that differ.
    fn differ(&self, o: &Self) -> Vec<&'static str> {
        let mut d = vec![];
        if self.scene != o.scene {
            d.push("scene (and so how it hands in)");
        }
        if self.output != o.output {
            d.push("output contract");
        }
        if self.refused != o.refused {
            d.push("writes");
        }
        if self.model != o.model || self.provider != o.provider {
            d.push("model");
        }
        if self.instance != o.instance
            || self.template != o.template
            || self.params != o.params
            || self.space != o.space
        {
            d.push("placement");
        }
        d
    }
}

/// Tasks that may share a session (`per_run`, or the same `{key}` expression) must be opened
/// the same way: the engine takes a session's scene, `output_schema`, permission rules, model
/// and place when it is created and keeps them, so a task arriving later would work under the
/// first one's — another contract, another station's writes.
fn shared_session_problems(c: &mut Ctx) {
    use super::def::SessionBinding;
    let def = c.def;
    // group → the first (station, task, settings) seen
    let mut first: BTreeMap<String, (String, String, SessionSettings)> = BTreeMap::new();
    for s in &def.stations {
        let specs = match s.work() {
            Work::None => vec![],
            Work::One(t) | Work::Foreach { task: t, .. } => vec![t],
            Work::Tasks(ts) => ts,
        };
        for t in specs {
            let Executor::Session {
                scene,
                session,
                model,
                provider,
                instance,
                template,
                params,
                space,
            } = &t.executor
            else {
                continue;
            };
            let group = match session {
                SessionBinding::New => continue,
                SessionBinding::PerRun => "`session: per_run`".to_string(),
                SessionBinding::Key(k) => format!("`session: {{key: {k}}}`"),
            };
            let refused: Vec<String> = if c.catalog.guarded.is_empty() {
                let mut w = s.writes.clone();
                w.sort();
                w.dedup();
                w
            } else {
                let mut r: Vec<String> = super::protocol::refused(&c.catalog.guarded, &s.writes)
                    .into_iter()
                    .map(|(_, r)| r)
                    .collect();
                r.sort();
                r
            };
            let mine = SessionSettings {
                scene: scene.clone(),
                // Kept by the session only when it hands in by structured output.
                output: if c.catalog.submit_scenes.contains(scene) {
                    None
                } else {
                    s.output.clone()
                },
                refused,
                model: model.clone(),
                provider: provider.clone(),
                instance: instance.clone(),
                template: template.clone(),
                params: params.clone(),
                space: space.clone(),
            };
            match first.get(&group) {
                None => {
                    first.insert(group, (s.id.clone(), t.id.clone(), mine));
                }
                Some((st, task, theirs)) => {
                    let d = theirs.differ(&mine);
                    if !d.is_empty() {
                        let other = if *st == s.id {
                            format!("task {task} of this station")
                        } else {
                            format!("station {st}")
                        };
                        let m = format!(
                            "task {} shares its session ({group}) with {other}, but its {} \
                             differ: a session keeps what it was opened with — give them the same, \
                             or a session each",
                            t.id,
                            d.join(", ")
                        );
                        c.at(&s.id, m);
                    }
                }
            }
        }
    }
}

/// Needs exist, are not the station itself, and form no cycle. Returns whether a cycle
/// was found (ancestor checks are skipped then).
fn graph_problems(c: &mut Ctx) -> bool {
    let def = c.def;
    let ids: BTreeSet<&str> = def.stations.iter().map(|s| s.id.as_str()).collect();
    let mut edges: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for s in &def.stations {
        let needs = def.needs_of(&s.id);
        let mut seen = BTreeSet::new();
        for n in &needs {
            if n == &s.id {
                c.at(&s.id, "needs itself");
            } else if !ids.contains(n.as_str()) {
                c.at(&s.id, format!("needs {n}, which is not a station"));
            }
            if !seen.insert(n) {
                c.at(&s.id, format!("needs {n} twice"));
            }
        }
        edges.insert(
            s.id.clone(),
            needs
                .into_iter()
                .filter(|n| ids.contains(n.as_str()) && *n != s.id)
                .collect(),
        );
    }
    // Kahn's algorithm: whatever cannot be ordered is on or behind a cycle.
    let mut done: BTreeSet<String> = BTreeSet::new();
    loop {
        let next: Vec<String> = edges
            .iter()
            .filter(|(id, ns)| !done.contains(*id) && ns.iter().all(|n| done.contains(n)))
            .map(|(id, _)| id.clone())
            .collect();
        if next.is_empty() {
            break;
        }
        done.extend(next);
    }
    let stuck: Vec<String> = edges
        .keys()
        .filter(|k| !done.contains(*k))
        .cloned()
        .collect();
    if !stuck.is_empty() {
        c.flow(format!("needs form a cycle through {}", stuck.join(", ")));
    }
    !stuck.is_empty()
}

fn station_problems(c: &mut Ctx, s: &Station, cyclic: bool) {
    let def = c.def;
    let id = s.id.as_str();

    // work shape
    if s.executor.is_some() && s.tasks.is_some() {
        c.at(id, "write either executor or tasks, not both");
    }
    if s.foreach.is_some() && s.executor.is_none() {
        c.at(id, "foreach needs an executor (and no tasks)");
    }
    if s.max_parallel.is_some() && s.foreach.is_none() {
        c.at(id, "max_parallel only goes with foreach");
    }
    if s.max_parallel == Some(0) {
        c.at(id, "max_parallel must be at least 1");
    }
    if s.tasks.is_some() && (s.skill.is_some() || s.retry.is_some()) {
        c.at(id, "with tasks, skill and retry go on each task");
    }
    let work = s.work();
    if matches!(work, Work::None) && (s.skill.is_some() || s.retry.is_some()) {
        c.at(id, "a checkpoint (no executor) has no skill or retry");
    }
    if let Some(ts) = &s.tasks {
        if ts.is_empty() {
            c.at(id, "tasks is empty");
        }
        let mut seen = BTreeSet::new();
        for t in ts {
            if !is_name(&t.id, b"_") {
                c.at(
                    id,
                    format!("task id {:?} must match [a-z][a-z0-9_-]*", t.id),
                );
            }
            if !seen.insert(t.id.as_str()) {
                c.at(id, format!("task id {} is not unique", t.id));
            }
        }
    }
    let specs: Vec<TaskSpec> = match &work {
        Work::None => vec![],
        Work::One(t) | Work::Foreach { task: t, .. } => vec![t.clone()],
        Work::Tasks(ts) => ts.clone(),
    };
    for t in &specs {
        executor_problems(c, id, t);
    }
    if let Some(items) = &s.foreach {
        let r = refs_in(items);
        if r.len() != 1 || !items.trim().starts_with("${") || !items.trim().ends_with('}') {
            c.at(
                id,
                "foreach must be one reference, like ${stations.<id>.output.<list>}",
            );
        }
    }

    // exits
    if s.exit.is_empty() {
        c.at(id, "no exit condition");
    }
    let has_subflow = specs
        .iter()
        .any(|t| matches!(t.executor, Executor::Subflow { .. }));
    for e in &s.exit {
        match e {
            ExitCond::Output { .. } => {
                if s.output.is_none() {
                    c.at(id, "an output exit needs an output contract");
                }
                if matches!(work, Work::None) {
                    c.at(id, "an output exit needs tasks; a checkpoint has none");
                }
            }
            ExitCond::Children { .. } if !has_subflow => {
                c.at(id, "a children exit only goes with a subflow executor");
            }
            ExitCond::Tool {
                tool, args, assert, ..
            } => {
                if !c.catalog.tools.contains(tool) {
                    c.at(id, format!("tool {tool} is not in the catalog"));
                }
                if assert.is_empty() {
                    c.at(id, format!("the tool exit {tool} has no assertion"));
                }
                for a in assert {
                    assert_problems(c, id, a, false);
                }
                ref_problems(c, s, &refs_in_value(args));
            }
            ExitCond::Gate { gate, .. } if !c.catalog.gates.contains(gate) => {
                c.at(id, format!("gate {gate} is not in the catalog"));
            }
            _ => {}
        }
    }
    if let Some(o) = &s.output {
        schema_problems(c, Some(id), "output", o);
    }

    // writes the host can grant: a guarded group whole or not at all
    for (w, g) in super::protocol::partial_writes(&c.catalog.guarded, &s.writes) {
        c.at(
            id,
            format!(
                "writes {w} opens only part of the guarded group {g}: a session's deny rule \
                 cannot make an exception — write {g} to open the whole group, or have the \
                 application guard those tools one by one"
            ),
        );
    }

    // when
    for a in &s.when {
        assert_problems(c, id, a, true);
    }

    // rework
    if !cyclic {
        for r in &s.rework_to {
            if def.station(r).is_none() {
                c.at(id, format!("rework_to {r} is not a station"));
            } else if !def.is_self_or_ancestor(r, id) {
                c.at(
                    id,
                    format!("rework_to {r} is neither this station nor an ancestor"),
                );
            }
        }
        if let Some(u) = &s.on_unmet {
            if def.station(&u.rework_to).is_none() {
                c.at(
                    id,
                    format!("on_unmet.rework_to {} is not a station", u.rework_to),
                );
            } else if !def.is_self_or_ancestor(&u.rework_to, id) {
                c.at(
                    id,
                    format!(
                        "on_unmet.rework_to {} is neither this station nor an ancestor",
                        u.rework_to
                    ),
                );
            }
        }
    }
    if let Some(u) = &s.on_unmet {
        if u.max == 0 {
            c.at(id, "on_unmet.max must be at least 1");
        }
    }

    for h in &s.hooks {
        hook_problems(c, Some(id), h.on.as_str(), h.action.as_str());
    }
}

fn executor_problems(c: &mut Ctx, id: &str, t: &TaskSpec) {
    match &t.executor {
        Executor::Session {
            scene,
            session,
            model,
            provider,
            instance,
            template,
            params,
            space,
        } => {
            if scene.trim().is_empty() {
                c.at(id, format!("session task {} needs a scene", t.id));
            } else if !c.catalog.scenes.contains(scene) {
                c.at(id, format!("scene {scene} is not in the catalog"));
            }
            if t.skill.is_none() {
                c.at(id, format!("session task {} needs a skill", t.id));
            }
            // The output is what a session hands in, by structured output or by `flow_submit`.
            if c.def.station(id).is_some_and(|s| s.output.is_none()) {
                c.at(
                    id,
                    format!(
                        "session task {} needs an output contract: it is what the session hands in",
                        t.id
                    ),
                );
            }
            if let super::def::SessionBinding::Key(k) = session {
                if refs_in(k).is_empty() {
                    c.at(id, "a session key should contain a reference");
                }
            }
            placement_problems(
                c,
                id,
                &t.id,
                Placed {
                    model,
                    provider,
                    instance,
                    template,
                    params,
                    space,
                },
            );
        }
        Executor::Program { step } => {
            if !c.catalog.steps.contains(step) {
                c.at(id, format!("step {step} is not in the catalog"));
            }
        }
        Executor::External { .. } => {}
        Executor::Subflow { flow, .. } => {
            if c.packs.flow(flow).is_none() {
                c.at(id, format!("subflow {flow} is not a flow in the packs"));
            } else if let Some(path) = subflow_cycle(c.packs, &c.def.name, flow) {
                c.at(
                    id,
                    format!(
                        "subflow {flow} leads back to this flow: {}",
                        path.join(" → ")
                    ),
                );
            }
        }
    }
    if let Some(skill) = &t.skill {
        if c.packs.skill(skill).is_none() {
            c.at(id, format!("skill {skill} is not in the packs"));
        }
    }
}

/// A session executor's placement fields.
struct Placed<'a> {
    model: &'a Option<String>,
    provider: &'a Option<String>,
    instance: &'a Option<String>,
    template: &'a Option<String>,
    params: &'a BTreeMap<String, String>,
    space: &'a Option<String>,
}

/// `instance` or `template`, not both; `params` only with `template`, their references sound;
/// each name in the catalog when the application filled that part of it in.
fn placement_problems(c: &mut Ctx, id: &str, task: &str, p: Placed) {
    let empty = |v: &Option<String>| v.as_ref().is_some_and(|s| s.trim().is_empty());
    for (what, v) in [
        ("model", p.model),
        ("provider", p.provider),
        ("instance", p.instance),
        ("template", p.template),
        ("space", p.space),
    ] {
        if empty(v) {
            c.at(id, format!("session task {task}: {what} is empty"));
        }
    }
    if p.instance.is_some() && p.template.is_some() {
        c.at(
            id,
            format!("session task {task}: name an instance or a template, not both"),
        );
    }
    if !p.params.is_empty() && p.template.is_none() {
        c.at(
            id,
            format!("session task {task}: params only go with a template"),
        );
    }
    let st = c.def.station(id).cloned();
    if let Some(st) = &st {
        let refs: Vec<String> = p
            .params
            .values()
            .flat_map(|v| refs_in(v).into_iter().map(String::from).collect::<Vec<_>>())
            .collect();
        ref_problems(c, st, &refs);
    }
    let check = |set: &BTreeSet<String>, v: &Option<String>| -> Option<String> {
        let v = v.as_ref()?;
        (!set.is_empty() && !set.contains(v)).then(|| v.clone())
    };
    if let Some(t) = check(&c.catalog.templates, p.template) {
        c.at(id, format!("template {t} is not in the catalog"));
    }
    if let Some(i) = check(&c.catalog.instances, p.instance) {
        c.at(id, format!("instance {i} is not in the catalog"));
    }
    if let Some(s) = check(&c.catalog.spaces, p.space) {
        c.at(id, format!("space {s} is not in the catalog"));
    }
    if let (Some(m), false) = (p.model, c.catalog.models.is_empty()) {
        let known = c.catalog.models.contains(m)
            || p.provider
                .as_ref()
                .is_some_and(|pr| c.catalog.models.contains(&format!("{pr}/{m}")))
            || (p.provider.is_none()
                && c.catalog
                    .models
                    .iter()
                    .any(|e| e.split_once('/').is_some_and(|(_, mm)| mm == m)));
        if !known {
            let named = match p.provider {
                Some(pr) => format!("{pr}/{m}"),
                None => m.clone(),
            };
            c.at(id, format!("model {named} is not in the catalog"));
        }
    }
}

/// A chain of subflows from `start` back to `root`, if there is one: `[root, start, …, root]`.
/// The pack's current definitions are followed; a flow already visited is not entered again.
fn subflow_cycle(packs: &dyn Packs, root: &str, start: &str) -> Option<Vec<String>> {
    fn children(def: &FlowDef) -> Vec<String> {
        let mut out = vec![];
        for s in &def.stations {
            let specs = match s.work() {
                super::def::Work::None => vec![],
                super::def::Work::One(t) | super::def::Work::Foreach { task: t, .. } => vec![t],
                super::def::Work::Tasks(ts) => ts,
            };
            for t in specs {
                if let Executor::Subflow { flow, .. } = t.executor {
                    out.push(flow);
                }
            }
        }
        out
    }
    let mut seen = BTreeSet::new();
    let mut stack = vec![vec![root.to_string(), start.to_string()]];
    while let Some(path) = stack.pop() {
        let last = path.last().cloned().unwrap_or_default();
        if last == root {
            return Some(path);
        }
        if !seen.insert(last.clone()) {
            continue;
        }
        let Some((def, _)) = packs.flow(&last) else {
            continue;
        };
        for next in children(&def) {
            let mut p = path.clone();
            p.push(next);
            stack.push(p);
        }
    }
    None
}

fn assert_problems(c: &mut Ctx, id: &str, a: &Assert, against_context: bool) {
    if a.path.trim().is_empty() {
        c.at(id, "an assertion needs a path");
    }
    if a.op_count() != 1 {
        c.at(
            id,
            format!(
                "assertion on {:?} needs exactly one of eq, ne, len, min_len, exists",
                a.path
            ),
        );
    }
    if against_context && !a.path.trim().is_empty() {
        let st = c.def.station(id).cloned();
        if let Some(st) = st {
            ref_problems(c, &st, std::slice::from_ref(&a.path));
        }
    }
}

/// A reference's root exists; `inputs.k` is declared; `stations.x` is an ancestor;
/// `item` only inside foreach.
fn ref_problems(c: &mut Ctx, s: &Station, refs: &[String]) {
    for r in refs {
        let mut parts = r.split(['.', '[']);
        let root = parts.next().unwrap_or("");
        if !ROOTS.contains(&root) {
            c.at(&s.id, format!("reference {r}: unknown root {root:?}"));
            continue;
        }
        match root {
            "inputs" => {
                if let Some(k) = parts.next() {
                    if !c.def.inputs.contains_key(k) {
                        c.at(&s.id, format!("reference {r}: no input {k}"));
                    }
                }
            }
            "stations" => {
                if let Some(x) = parts.next() {
                    if c.def.station(x).is_none() {
                        c.at(&s.id, format!("reference {r}: no station {x}"));
                    } else if x == s.id || !c.def.descendants(x).contains(&s.id) {
                        c.at(
                            &s.id,
                            format!("reference {r}: station {x} is not an ancestor"),
                        );
                    }
                }
            }
            "item" if s.foreach.is_none() => {
                c.at(
                    &s.id,
                    format!("reference {r}: item only exists in a foreach station"),
                );
            }
            _ => {}
        }
    }
}

fn schema_problems(c: &mut Ctx, station: Option<&str>, what: &str, schema: &Value) {
    if let Err(e) = jsonschema::meta::validate(schema) {
        let m = format!("{what} is not a valid JSON Schema: {e}");
        match station {
            Some(s) => c.at(s, m),
            None => c.flow(m),
        }
    }
}

fn hook_problems(c: &mut Ctx, station: Option<&str>, on: &str, action: &str) {
    let mut ms = vec![];
    if !EVENT_KINDS.contains(&on) {
        ms.push(format!("hook on {on:?}: not an event kind"));
    }
    if !c.catalog.actions.contains(action) {
        ms.push(format!("hook action {action} is not in the catalog"));
    }
    for m in ms {
        match station {
            Some(s) => c.at(s, m),
            None => c.flow(m),
        }
    }
}
