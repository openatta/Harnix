//! The flow definition: a YAML file shipped in a pack.
//!
//! The types mirror the YAML surface one to one, so the JSON Schema derived
//! from them ([`definition_schema`]) is the schema of the file an author
//! writes. A station states its work as one of `executor` (one task), `tasks`
//! (several, each with an `id`) or `foreach` + `executor` (one task per item);
//! none of them makes it a pure checkpoint. [`Station::work`] gives the
//! normalised form. Combinations the types cannot rule out are structural
//! problems reported by [`crate::validate`].

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Rounds a station may run when the definition does not say.
pub const DEFAULT_REWORK_LIMIT: u32 = 10;
/// The most `rework_limit` may be.
pub const MAX_REWORK_LIMIT: u32 = 20;

/// A flow: a directed acyclic graph of stations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowDef {
    /// `[a-z][a-z0-9-]*`; run ids are `<name>-<n>`.
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Parameters a run is started with, by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, InputDef>,
    /// How many rounds one station may run (first entry included). Default 10, at most 20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rework_limit: Option<u32>,
    /// Notifications fired on run-level and station-level events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HookRef>,
    pub stations: Vec<Station>,
}

/// A run parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputDef {
    /// A JSON Schema the value must satisfy.
    #[serde(default = "empty_object")]
    pub schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

/// A station: the unit of judgement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Station {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Stations that must have finished (passed or skipped) first. Absent = the previous
    /// station in the list; `[]` = none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs: Option<Vec<String>>,
    /// Checked on entry; any assertion failing skips the station.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<Assert>,
    /// One task, or the task template of `foreach`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<Executor>,
    /// Several tasks, each with its own id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Vec<TaskSpec>>,
    /// A reference to a list; one task per item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreach: Option<String>,
    /// With `foreach`: how many tasks may be in flight at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallel: Option<u32>,
    /// With `executor`: the skill a session task works from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    /// With `executor`: how many times a failed task is dispatched again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<Retry>,
    /// JSON Schema every task's output must satisfy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    /// All must hold for the station to pass.
    #[serde(default)]
    pub exit: Vec<ExitCond>,
    /// Stations (this one or an ancestor) a rework from here may go back to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rework_to: Vec<String>,
    /// Rework automatically when the exit conditions fail, at most `max` times.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_unmet: Option<OnUnmet>,
    /// Stop on entry until someone releases the station.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hold: bool,
    /// Write tools a session may call while working here (`prefix*` matches a group).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HookRef>,
}

/// One task of a static `tasks` list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    pub id: String,
    pub executor: Executor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<Retry>,
}

impl TaskSpec {
    /// Attempts a task gets before it counts as failed.
    pub fn attempts(&self) -> u32 {
        1 + self.retry.as_ref().map_or(0, |r| r.max)
    }
}

/// Who does a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Executor {
    /// A headless session the module opens on `scene`, fed the protocol layer and the skill.
    /// Where it runs and on which model are optional: absent, the module's space and that
    /// space's instance, and the instance's model.
    Session {
        scene: String,
        #[serde(default, skip_serializing_if = "SessionBinding::is_new")]
        session: SessionBinding,
        /// The model the session talks to (`SessionSpec.model`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// The provider of that model (`SessionSpec.provider`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        /// An instance that exists already, by name. Not with `template`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance: Option<String>,
        /// A template's full name (`<module>.<template>`, or a profile's): the module makes sure
        /// an instance of it is ready, then opens the session there. Not with `instance`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<String>,
        /// With `template`: its parameters. Values may hold references (`${run}`,
        /// `${inputs.k}`, …); `name` is the instance's name (`flow-${run}` when absent).
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, String>,
        /// The space the session belongs to; the module's when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        space: Option<String>,
    },
    /// The application's own code (`Programs`).
    Program { step: String },
    /// Waits to be claimed from outside: a person or an external agent. `who` is an opaque
    /// label handed to the authorizer.
    External {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<String>,
    },
    /// A child run; finishing it submits the task.
    Subflow {
        flow: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        inputs: Value,
    },
}

impl Executor {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Session { .. } => "session",
            Self::Program { .. } => "program",
            Self::External { .. } => "external",
            Self::Subflow { .. } => "subflow",
        }
    }

    /// Session and program tasks are dispatched (auto-claimed) by the kernel.
    pub fn is_push(&self) -> bool {
        matches!(self, Self::Session { .. } | Self::Program { .. })
    }
}

/// Which session a session task runs in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionBinding {
    /// A new session per task.
    #[default]
    New,
    /// One session for every task of the run.
    PerRun,
    /// Tasks whose key resolves to the same string share a session.
    Key(String),
}

impl SessionBinding {
    pub fn is_new(&self) -> bool {
        matches!(self, Self::New)
    }
}

/// Retry policy of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Retry {
    pub max: u32,
}

/// Automatic rework when a station's exit conditions fail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OnUnmet {
    pub rework_to: String,
    pub max: u32,
}

/// A condition for leaving a station.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExitCond {
    /// Every task has submitted an output satisfying the station's `output` contract.
    Output {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Call an application tool with `args` (references resolved) and assert on the result.
    Tool {
        tool: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        args: Value,
        assert: Vec<Assert>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A check the host runs asynchronously; its result comes back through `gate_result`.
    Gate {
        gate: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Someone confirms. `who` is an opaque label handed to the authorizer.
    Confirm {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Every child run of the station finished.
    Children {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

impl ExitCond {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Output { .. } => "output",
            Self::Tool { .. } => "tool",
            Self::Gate { .. } => "gate",
            Self::Confirm { .. } => "confirm",
            Self::Children { .. } => "children",
        }
    }

    /// A name for people: the `label`, or one made from the kind.
    pub fn label(&self) -> String {
        let (label, made) = match self {
            Self::Output { label } => (label, "output".to_string()),
            Self::Tool { label, tool, .. } => (label, format!("tool {tool}")),
            Self::Gate { label, gate } => (label, format!("gate {gate}")),
            Self::Confirm { label, who } => (
                label,
                who.as_ref()
                    .map_or("confirm".to_string(), |w| format!("confirm by {w}")),
            ),
            Self::Children { label } => (label, "children finished".to_string()),
        };
        label.clone().unwrap_or(made)
    }
}

/// An assertion on a JSON value: `path` selects, exactly one of the operators compares.
///
/// Shared by `when` (evaluated against the run context: `inputs.mode`,
/// `stations.collect.output.modules`, …) and `tool` exits (against the tool's result).
/// A path that selects nothing reads as `null`. `eq: null` cannot be written — use
/// `exists: false`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Assert {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ne: Option<Value>,
    /// Element count (array, object; a non-empty string or scalar counts 1, null 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub len: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_len: Option<usize>,
    /// `true`: present and not null; `false`: absent or null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Assert {
    /// How many operators are set; a well-formed assertion has exactly one.
    pub fn op_count(&self) -> usize {
        [
            self.eq.is_some(),
            self.ne.is_some(),
            self.len.is_some(),
            self.min_len.is_some(),
            self.exists.is_some(),
        ]
        .iter()
        .filter(|b| **b)
        .count()
    }
}

/// A notification: when event `on` happens, the host runs `action`. It never blocks a station.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HookRef {
    pub on: String,
    pub action: String,
}

/// A station's work, normalised.
#[derive(Debug, Clone, PartialEq)]
pub enum Work {
    /// A pure checkpoint: no tasks, the exit is judged on entry.
    None,
    /// One task, whose id is the station's id.
    One(TaskSpec),
    Tasks(Vec<TaskSpec>),
    Foreach {
        items: String,
        max_parallel: Option<u32>,
        task: TaskSpec,
    },
}

impl Station {
    /// The work in normalised form. When the fields are combined in a way validation
    /// rejects, `tasks` wins over `executor`.
    pub fn work(&self) -> Work {
        let one = |e: &Executor| TaskSpec {
            id: self.id.clone(),
            executor: e.clone(),
            skill: self.skill.clone(),
            retry: self.retry,
        };
        match (&self.tasks, &self.executor, &self.foreach) {
            (Some(t), _, _) => Work::Tasks(t.clone()),
            (None, Some(e), Some(items)) => Work::Foreach {
                items: items.clone(),
                max_parallel: self.max_parallel,
                task: one(e),
            },
            (None, Some(e), None) => Work::One(one(e)),
            (None, None, _) => Work::None,
        }
    }

    /// The task template a task id belongs to (foreach tasks share one).
    pub fn task_spec(&self, task: &str) -> Option<TaskSpec> {
        match self.work() {
            Work::None => None,
            Work::One(t) => Some(t),
            Work::Tasks(ts) => ts.into_iter().find(|t| t.id == task),
            Work::Foreach { task: t, .. } => Some(t),
        }
    }

    /// Whether a session working here may call write tool `tool`.
    pub fn allows(&self, tool: &str) -> bool {
        self.writes.iter().any(|w| match w.strip_suffix('*') {
            Some(prefix) => tool.starts_with(prefix),
            None => w == tool,
        })
    }

    /// The `who` label of the first `confirm` exit, if any.
    pub fn confirm_who(&self) -> Option<String> {
        self.exit.iter().find_map(|c| match c {
            ExitCond::Confirm { who, .. } => who.clone(),
            _ => None,
        })
    }
}

impl FlowDef {
    /// Parse a definition from YAML (shape only; see [`crate::validate`] for the rest).
    pub fn parse(yaml: &str) -> Result<Self, String> {
        serde_yaml::from_str(yaml).map_err(|e| e.to_string())
    }

    pub fn rework_limit(&self) -> u32 {
        self.rework_limit.unwrap_or(DEFAULT_REWORK_LIMIT)
    }

    pub fn station(&self, id: &str) -> Option<&Station> {
        self.stations.iter().find(|s| s.id == id)
    }

    /// A station's needs with the default applied (the previous station in the list).
    pub fn needs_of(&self, id: &str) -> Vec<String> {
        let Some(i) = self.stations.iter().position(|s| s.id == id) else {
            return vec![];
        };
        match &self.stations[i].needs {
            Some(n) => n.clone(),
            None if i == 0 => vec![],
            None => vec![self.stations[i - 1].id.clone()],
        }
    }

    /// Every station reachable from `id` along `needs` edges downstream, not including `id`,
    /// in definition order.
    pub fn descendants(&self, id: &str) -> Vec<String> {
        let mut found: Vec<String> = vec![id.to_string()];
        let mut changed = true;
        while changed {
            changed = false;
            for s in &self.stations {
                if found.contains(&s.id) {
                    continue;
                }
                if self.needs_of(&s.id).iter().any(|n| found.contains(n)) {
                    found.push(s.id.clone());
                    changed = true;
                }
            }
        }
        self.stations
            .iter()
            .map(|s| s.id.clone())
            .filter(|s| s != id && found.contains(s))
            .collect()
    }

    /// Whether `anc` is `id` itself or one of its ancestors.
    pub fn is_self_or_ancestor(&self, anc: &str, id: &str) -> bool {
        anc == id || self.descendants(anc).iter().any(|d| d == id)
    }
}

/// The JSON Schema of a flow definition file.
pub fn definition_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(FlowDef)).unwrap_or(Value::Null)
}
