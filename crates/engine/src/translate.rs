//! Agix's JSON-RPC, in the engine interface's terms: the one place that knows what Agix calls its
//! methods and its events.
//!
//! A link speaks Agix's protocol underneath — a call into an engine in this process, or a line
//! on a daemon's socket — and the hub sees only [`EngineOps`] and [`EngineEvent`]. Every name the
//! hub would otherwise have had to spell is a constant here, and `crates/harnix/tests/layering.rs`
//! reads them to check that the hub spells none of them.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use harnix_contract::wire::{ModelChoice, RemovedTurn, RewindParams, RewindResult, RewindScope, RewindTo, RewoundConversation, RewoundFiles};
use harnix_contract::wire::{HookRow, HookSource, SessionHooks, SessionSkills, ShadowedSkill, SkillDirRow, SkillRow, SkillTier};
use harnix_contract::RpcError;
use harnix_engine_link::events::{Completion, DaemonEventKind, EngineEvent, FileWrite, Fragment, Joins, Prompt, SessionEventKind, Waiting};
use harnix_engine_link::ops::{codes, Answer, EngineOps, Entry, HistoryPage, Message, Said, SessionCreated, SessionInfo, SessionSetup, Subscribed, Tasks, TurnInput};
use harnix_engine_api::{Delivery, ToolError, ToolOutput, ToolSpec};
use serde_json::{json, Value};

pub const SESSION_CREATE: &str = "session.create";
pub const SESSION_GET: &str = "session.get";
pub const SESSION_LIST: &str = "session.list";
pub const SESSION_SUBSCRIBE: &str = "session.subscribe";
pub const SESSION_RESUME: &str = "session.resume";
pub const SESSION_HISTORY: &str = "session.history";
pub const SESSION_TASKS: &str = "session.tasks";
pub const SESSION_RUN_TURN: &str = "session.run_turn";
pub const SESSION_INTERRUPT: &str = "session.interrupt";
pub const SESSION_RENAME: &str = "session.rename";
pub const SESSION_SET_MODEL: &str = "session.setModel";
pub const SESSION_SET_PROMPT: &str = "session.setPrompt";
pub const SESSION_SET_SKILL_DIRS: &str = "session.setSkillDirs";
pub const SESSION_CLOSE: &str = "session.close";
pub const SESSION_DELETE: &str = "session.delete";
pub const SESSION_FORK: &str = "session.fork";
pub const SESSION_APPEND_ENTRY: &str = "session.appendEntry";
pub const SESSION_RESPOND: &str = "session.respondToPrompt";
pub const CONFIG_RELOAD: &str = "config.reload";
pub const DAEMON_SUBSCRIBE_EVENTS: &str = "daemon.subscribeEvents";
pub const MCP_STATUS: &str = "mcp.status";
pub const DAEMON_PING: &str = "daemon.ping";
pub const DAEMON_AUTH: &str = "daemon.auth";
pub const DAEMON_SET_HOST_TOOLS: &str = "daemon.setHostTools";
pub const DAEMON_SET_HOOKS: &str = "daemon.setHooks";
pub const SESSION_RESPOND_TOOL_CALL: &str = "session.respondToolCall";
pub const SESSION_REPORT_TOOL_PROGRESS: &str = "session.reportToolProgress";
pub const SESSION_RESPOND_HOOK: &str = "session.respondHook";
pub const SESSION_CALLS: &str = "session.calls";
pub const SESSION_CALL: &str = "session.call";
pub const SESSION_CALL_BLOB: &str = "session.callBlob";
pub const SESSION_SEARCH: &str = "session.search";
pub const SESSION_REWIND: &str = "session.rewind";
pub const SESSION_CHECKPOINTS: &str = "session.checkpoints";
pub const CONFIG_GET: &str = "config.get";
pub const COMMANDS_LIST: &str = "commands.list";
pub const PROJECT_STATUS: &str = "project.status";
pub const PLUGIN_LIST: &str = "plugin.list";

/// What a daemon must say it has (`daemon.ping`'s `capabilities`) for a link to take it: the
/// tools and hooks a connection registers, a session's own `attended`, `skill_dirs`, its recorded
/// calls and search, and the entries a host appends.
pub const REQUIRED_CAPABILITIES: &[&str] = &[
    "host_tool_provider",
    "host_hooks",
    "host_tool_progress",
    "session_attended",
    "session_skill_dirs",
    "session_calls",
    "session_search",
    "extension_entries",
];

/// The two notifications an engine sends.
pub const SESSION_EVENT: &str = "session.event";
pub const DAEMON_EVENT: &str = "daemon.event";

// What a session's events are called.
const TEXT_DELTA: &str = "text_delta";
const THINKING_DELTA: &str = "thinking_delta";
const TOOL_ARGS_DELTA: &str = "tool_args_delta";
const TOOL_USE: &str = "tool_use";
const TOOL_RESULT: &str = "tool_result";
/// What a file-editing tool's result carries of the files it wrote (daemon protocol §7.5).
const FILE_CHANGES: &str = "file_changes";
const PROMPT: &str = "prompt";
const PROMPT_RESOLVED: &str = "prompt_resolved";
const SUBAGENT_PROGRESS: &str = "subagent_progress";
/// The kind of ask a permission is.
const PERMISSION: &str = "permission";
const GOAL_CHANGED: &str = "goal_changed";
const TEAM_PROGRESS: &str = "team_progress";
const TASK_STARTED: &str = "task_started";
const TASK_FINISHED: &str = "task_finished";
const TASK_UPDATED: &str = "task_updated";
const COMPACT: &str = "compact";
const TURN_COMPLETE: &str = "turn_complete";
/// A host rewound the session (daemon protocol §6.3): `seq` is how many messages it has now.
const REWOUND: &str = "rewound";
const SUBSCRIPTION_DROPPED: &str = "subscription_dropped";
const HOST_TOOL_CALL: &str = "host_tool_call";
const HOST_TOOL_CALL_RESOLVED: &str = "host_tool_call_resolved";
const HOOK_CALL: &str = "hook_call";
const HOOK_CALL_RESOLVED: &str = "hook_call_resolved";
/// The two hook events a connection registers for.
const PRE_TOOL_USE: &str = "PreToolUse";
const USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
/// How a hook-call frame, or a host tool call's, says it was taken back.
const WITHDRAWN: &str = "withdrawn";

// What the engine's own events are called.
const TURN_STARTED: &str = "turn_started";
const TURN_COMPLETED: &str = "turn_completed";
const SESSION_CREATED: &str = "session_created";
const SESSION_RESUMED: &str = "session_resumed";
const SESSION_EVICTED: &str = "session_evicted";
const SESSION_CLOSED: &str = "session_closed";
const SESSION_DELETED: &str = "session_deleted";
const EVENTS_LAGGED: &str = "events_lagged";
/// An MCP server the engine could not connect, and the module whose methods report on it.
const MCP_CONNECT_FAILED: &str = "mcp_connect_failed";
const MCP_CONNECTED: &str = "mcp_connected";
const MCP_MODULE: &str = "mcp";

/// A question whose answer is words, not a permission.
const QUESTION: &str = "question";
/// An MCP server's ask of the person, in one of its tool calls.
const MCP_ELICITATION: &str = "mcp_elicitation";
/// Why a question was not answered, when the person said no more than that.
const DECLINED: &str = "declined";
/// What a session names the asks its host answers with (`session.create`'s `options`).
const PROMPT_TYPES: &str = "prompt_types";
/// The tool a plan is handed in by: leaving plan mode is asked as a permission, and the plan waits
/// on a review.
const PLAN_TOOL: &str = "ExitPlanMode";

/// One of Agix's calls, as a link makes it over its stream.
#[async_trait::async_trait]
pub(crate) trait Rpc: Send + Sync + 'static {
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError>;
}

/// [`EngineOps`] over Agix's calls.
pub(crate) struct AgixOps(pub(crate) Arc<dyn Rpc>);

impl AgixOps {
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.0.call(method, params).await
    }
}

#[async_trait::async_trait]
impl EngineOps for AgixOps {
    async fn create(&self, params: Value, setup: SessionSetup) -> Answer<SessionCreated> {
        self.call(SESSION_CREATE, set_up(params, &setup)).await.map(SessionCreated::new)
    }

    async fn get(&self, session: &str) -> Answer<SessionInfo> {
        self.call(SESSION_GET, json!({"session_id": session})).await.map(SessionInfo::new)
    }

    async fn list(&self) -> Answer<Vec<SessionInfo>> {
        let listed = self.call(SESSION_LIST, json!({})).await?;
        let rows = match listed.get("sessions") {
            Some(Value::Array(rows)) => rows.iter().cloned().map(SessionInfo::new).collect(),
            _ => Vec::new(),
        };
        Ok(rows)
    }

    async fn subscribe(&self, session: &str) -> Answer<Option<Subscribed>> {
        match self.call(SESSION_SUBSCRIBE, json!({"session_id": session})).await {
            Ok(answer) => {
                let pending = match answer.get("pending_prompts") {
                    Some(Value::Array(asks)) => asks.iter().cloned().map(prompt).collect(),
                    _ => Vec::new(),
                };
                Ok(Some(Subscribed::new(answer.get("last_seq").and_then(Value::as_u64), pending)))
            }
            // Only live sessions have subscribers.
            Err(e) if e.code == codes::SESSION_NOT_FOUND => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// A session that recorded options of its own keeps them (its scene layer's tools, prompt and
    /// naming among them); one created before sessions had any is attended, as every session is.
    async fn resume(&self, session: &str, from: &SessionInfo) -> Answer<()> {
        let mut params = json!({"session_id": session, "options": {"attended": true}});
        if let Some(scene) = from.scene() {
            params["scene"] = json!(scene);
        }
        if let Some(root) = from.project_root() {
            params["project_root"] = root.clone();
        }
        self.call(SESSION_RESUME, params).await.map(|_| ())
    }

    async fn history_len(&self, session: &str) -> Answer<u64> {
        let page = self.call(SESSION_HISTORY, json!({"session_id": session, "limit": 0})).await?;
        Ok(page.get("total").and_then(Value::as_u64).unwrap_or(0))
    }

    async fn transcript(&self, session: &str, limit: usize) -> Answer<Vec<Said>> {
        let page = self
            .call(SESSION_HISTORY, json!({"session_id": session, "offset": 0, "limit": limit}))
            .await?;
        let said = page
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|m| Said::new(m.get("role").and_then(Value::as_str).unwrap_or("?"), message_text(m)))
            .collect();
        Ok(said)
    }

    async fn history(&self, session: &str, offset: u64, limit: usize) -> Answer<HistoryPage> {
        let page = self
            .call(SESSION_HISTORY, json!({"session_id": session, "offset": offset, "limit": limit}))
            .await?;
        let text = |m: &Value, k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
        let messages = page
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, m)| {
                let seq = m.get("seq").and_then(Value::as_u64).unwrap_or(offset + i as u64);
                let mut message = Message::new(seq, text(m, "role").unwrap_or_default(), message_text(m));
                message.origin = text(m, "origin");
                message.turn_id = text(m, "turn_id");
                message.id = text(m, "id");
                message.ts = text(m, "ts");
                message
            })
            .collect();
        Ok(HistoryPage::new(page.get("total").and_then(Value::as_u64).unwrap_or(0), messages))
    }

    async fn tasks(&self, session: &str) -> Answer<Tasks> {
        let value = match self.call(SESSION_TASKS, json!({"session_id": session})).await {
            Ok(value) => value,
            // Not in memory: nothing of it is running.
            Err(e) if e.code == codes::SESSION_NOT_FOUND => json!({"active": false, "tasks": []}),
            Err(e) => return Err(e),
        };
        let live: BTreeSet<String> = value
            .get("tasks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|t| task_is_live(t))
            .filter_map(|t| t.get("task_id").and_then(Value::as_str).map(str::to_string))
            .collect();
        Ok(Tasks::new(value, live))
    }

    async fn run_turn(&self, session: &str, input: TurnInput) -> Answer<Value> {
        let params = if input.steer {
            json!({"session_id": session, "turn_id": input.turn_id, "message": input.message, "on_busy": "steer"})
        } else {
            json!({"session_id": session, "turn_id": input.turn_id, "message": input.message, "attachments": input.attachments})
        };
        self.call(SESSION_RUN_TURN, params).await
    }

    async fn interrupt(&self, session: &str) -> Answer<Value> {
        self.call(SESSION_INTERRUPT, json!({"session_id": session})).await
    }

    async fn rename(&self, session: &str, title: &str) -> Answer<Value> {
        self.call(SESSION_RENAME, json!({"session_id": session, "name": title})).await
    }

    async fn set_model(&self, session: &str, choice: &ModelChoice) -> Answer<Value> {
        self.call(SESSION_SET_MODEL, set_model_params(session, choice)).await
    }

    async fn set_prompt(&self, session: &str, append: Option<Option<String>>, override_text: Option<Option<String>>) -> Answer<Value> {
        let mut params = json!({"session_id": session});
        if let Some(append) = append {
            params["append"] = json!(append);
        }
        if let Some(override_text) = override_text {
            params["override"] = json!(override_text);
        }
        self.call(SESSION_SET_PROMPT, params).await
    }

    async fn set_skill_dirs(&self, session: &str, dirs: &[std::path::PathBuf]) -> Answer<Value> {
        let listed: Vec<Value> = dirs.iter().map(|d| json!({"path": d})).collect();
        self.call(SESSION_SET_SKILL_DIRS, json!({"session_id": session, "skill_dirs": listed})).await
    }

    async fn close(&self, params: Value, delete: bool) -> Answer<Value> {
        self.call(if delete { SESSION_DELETE } else { SESSION_CLOSE }, params).await
    }

    async fn fork(&self, params: Value) -> Answer<SessionCreated> {
        self.call(SESSION_FORK, params).await.map(SessionCreated::new)
    }

    async fn append_entry(&self, session: &str, entry: Entry) -> Answer<Value> {
        let params = json!({"session_id": session, "ns": entry.ns, "event": entry.event, "payload": entry.payload});
        self.call(SESSION_APPEND_ENTRY, params).await
    }

    async fn respond(&self, params: Value) -> Answer<Value> {
        self.call(SESSION_RESPOND, respond_params(params)).await
    }

    async fn passthrough(&self, method: &str, params: Value) -> Answer<Value> {
        self.call(agix_method(method), params).await
    }

    async fn rewind(&self, session: &str, ask: &RewindParams) -> Answer<RewindResult> {
        let params = rewind_params(session, ask)?;
        let answer = self.call(SESSION_REWIND, params).await?;
        Ok(rewound(session, ask, &answer))
    }

    async fn live_turns(&self, session: &str) -> Answer<Vec<String>> {
        let listed = self.call(SESSION_CHECKPOINTS, json!({"session_id": session})).await?;
        let turns = listed.get("turns").and_then(Value::as_array).into_iter().flatten();
        Ok(turns.filter_map(|t| t.get("turn_id").and_then(Value::as_str).map(str::to_string)).collect())
    }

    async fn session_skills(&self, session: &str) -> Answer<SessionSkills> {
        let info = self.get(session).await?;
        let status = self.project_status(&info).await;
        // The engine lists a session's own commands only while it is in memory.
        let commands = match info.loaded() {
            true => self.call(COMMANDS_LIST, json!({"session_id": session})).await.ok(),
            false => None,
        };
        let plugins = self.call(PLUGIN_LIST, json!({})).await.ok();
        Ok(skills_of(info.value(), commands.as_ref(), status.as_ref(), plugins.as_ref()))
    }

    async fn settings_hooks(&self, session: &str) -> Answer<SessionHooks> {
        let info = self.get(session).await?;
        let trust = self.project_status(&info).await.and_then(|s| s.get("trust").and_then(Value::as_str).map(str::to_string));
        let project = info.value().get("project_root").and_then(Value::as_str);
        let scene = info.value().get("scene").and_then(Value::as_str);
        let mut tiers = Vec::new();
        for (source, tier) in [(HookSource::Global, "global"), (HookSource::Scene, "scene"), (HookSource::Project, "project")] {
            if (source == HookSource::Scene && scene.is_none()) || (source == HookSource::Project && project.is_none()) {
                continue;
            }
            let ask = json!({"scene": scene, "project_root": project, "tier": tier});
            let settings = self.call(CONFIG_GET, ask).await.ok().map(|a| a.get("settings").cloned().unwrap_or(Value::Null));
            tiers.push((source, settings));
        }
        let (hooks, unread) = hooks_of(&tiers, trust.as_deref());
        Ok(SessionHooks { instance: String::new(), session_id: String::new(), trust, hooks, unread })
    }
}

impl AgixOps {
    /// The status of the session's project, when it has one and the engine says it.
    async fn project_status(&self, info: &SessionInfo) -> Option<Value> {
        let root = info.value().get("project_root").and_then(Value::as_str)?;
        let scene = info.value().get("scene").and_then(Value::as_str);
        self.call(PROJECT_STATUS, json!({"project_root": root, "scene": scene})).await.ok()
    }
}

/// A project's own skills, under its root.
const PROJECT_SKILLS: &str = ".agents/skills";

/// The skills a session sees, from what the engine said of it: `session` (`session.get`), its
/// commands (`commands.list`, absent when it is not in memory), its project's status and the
/// plugin packages. Skills are the commands that expand into a prompt; an MCP server's prompts and
/// the engine's local commands are not.
fn skills_of(session: &Value, commands: Option<&Value>, status: Option<&Value>, plugins: Option<&Value>) -> SessionSkills {
    let dirs: Vec<SkillDirRow> = session
        .get("skill_dirs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|d| {
            Some(SkillDirRow {
                path: d.get("path")?.as_str()?.to_string(),
                package: None,
                state: d.get("state").and_then(Value::as_str).map(str::to_string),
                skills: strings(d.get("skills")),
                shadowed: d
                    .get("shadowed")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|s| Some(ShadowedSkill { skill: s.get("skill")?.as_str()?.to_string(), by: s.get("by")?.as_str()?.to_string() }))
                    .collect(),
            })
        })
        .collect();
    let trust = status.and_then(|s| s.get("trust")).and_then(Value::as_str).map(str::to_string);
    let held: BTreeSet<String> = match trust.as_deref() {
        Some("trusted") => BTreeSet::new(),
        _ => strings(status.and_then(|s| s.get("requires_trust")))
            .into_iter()
            .filter_map(|item| item.strip_prefix("skills/").map(str::to_string))
            .collect(),
    };
    let project = session.get("project_root").and_then(Value::as_str);
    let plugins: Vec<&Value> = plugins
        .and_then(|p| p.get("plugins"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| p.get("enabled").and_then(Value::as_bool).unwrap_or(false))
        .collect();
    let mut skills = Vec::new();
    for command in commands.and_then(|c| c.get("commands")).and_then(Value::as_array).into_iter().flatten() {
        if command.get("kind").and_then(Value::as_str) != Some(PROMPT) {
            continue;
        }
        let Some(name) = command.get("name").and_then(Value::as_str) else { continue };
        let tier = match command.get("source").and_then(Value::as_str) {
            Some("host") => SkillTier::Space,
            Some("project") => SkillTier::Project,
            Some("plugin") => SkillTier::Plugin,
            Some("user") => SkillTier::User,
            Some("builtin") => SkillTier::Builtin,
            _ => continue,
        };
        let mut row = SkillRow {
            name: name.to_string(),
            description: command.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
            tier,
            dir: None,
            package: None,
            plugin: None,
            withheld: false,
        };
        match tier {
            SkillTier::Space => row.dir = dirs.iter().find(|d| d.skills.iter().any(|s| s == name)).map(|d| d.path.clone()),
            SkillTier::Project => {
                row.dir = project.map(|root| std::path::Path::new(root).join(PROJECT_SKILLS).display().to_string());
                row.withheld = held.contains(name);
            }
            SkillTier::Plugin => {
                if let Some(plugin) = plugins.iter().find(|p| strings(p.get("skills")).iter().any(|s| s == name)) {
                    row.plugin = plugin.get("name").and_then(Value::as_str).map(str::to_string);
                    row.dir = plugin.get("root").and_then(Value::as_str).map(str::to_string);
                }
            }
            SkillTier::User | SkillTier::Builtin => {}
        }
        skills.push(row);
    }
    SessionSkills { instance: String::new(), session_id: String::new(), listed: commands.is_some(), trust, skills, dirs }
}

/// The hooks each settings tier configures (`tiers`, lowest first; `None` for one that could not
/// be read), and the tiers not read. The highest tier that configures an event replaces the lower
/// ones' list for it; a project's counts only when the engine says the project is trusted. A hook
/// is said by its type, its `if` and its `on_error`, never by what it runs.
fn hooks_of(tiers: &[(HookSource, Option<Value>)], trust: Option<&str>) -> (Vec<HookRow>, Vec<HookSource>) {
    let project_held = trust != Some("trusted");
    let mut rows = Vec::new();
    let mut unread = Vec::new();
    // The tier whose list each event takes.
    let mut winner: std::collections::BTreeMap<String, HookSource> = std::collections::BTreeMap::new();
    for (source, settings) in tiers {
        let Some(settings) = settings else {
            unread.push(*source);
            continue;
        };
        let withheld = *source == HookSource::Project && project_held;
        let Some(config) = settings.get("hooks_config").and_then(Value::as_object) else { continue };
        for (event, entries) in config {
            if !withheld {
                winner.insert(event.clone(), *source);
            }
            let entries: Vec<&Value> = match entries {
                Value::Array(list) => list.iter().collect(),
                one @ Value::Object(_) => vec![one],
                _ => Vec::new(),
            };
            for entry in entries {
                // The types the engine runs; any other is `other`.
                let kind = entry
                    .get("type")
                    .and_then(Value::as_str)
                    .filter(|t| matches!(*t, "command" | "prompt" | "http" | "agent"))
                    .unwrap_or("other");
                let on_error = entry.get("on_error").and_then(Value::as_str).filter(|e| matches!(*e, "allow" | "block")).map(str::to_string);
                let mut row = HookRow::new(event.as_str(), *source, kind, entry.get("if").and_then(Value::as_str), on_error);
                row.withheld = withheld;
                row.in_force = !withheld;
                rows.push(row);
            }
        }
    }
    for row in &mut rows {
        if let Some(&by) = winner.get(&row.event).filter(|&&by| by != row.source && !row.withheld) {
            row.replaced_by = Some(by);
            row.in_force = false;
        }
    }
    (rows, unread)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

/// What a rewind puts back when the ask does not say: the files alone for a tool use (the only
/// scope it takes), both otherwise.
fn rewind_scope(ask: &RewindParams) -> RewindScope {
    match (ask.scope, &ask.to) {
        (Some(scope), _) => scope,
        (None, RewindTo::ToolUse(_)) => RewindScope::Files,
        (None, _) => RewindScope::Both,
    }
}

/// A rewind as the engine takes it: exactly one target, the scope always said.
fn rewind_params(session: &str, ask: &RewindParams) -> Answer<Value> {
    let mut params = json!({
        "session_id": session,
        "scope": rewind_scope(ask),
        "dry_run": ask.dry_run,
        "fork_first": ask.fork_first,
        "force": ask.force,
        "allow_partial": ask.allow_partial,
    });
    match &ask.to {
        RewindTo::Start => params["to_start"] = json!(true),
        RewindTo::Turn(turn) => params["to_turn"] = json!(turn),
        RewindTo::ToolUse(call) => params["to_tool_use"] = json!(call),
        RewindTo::BeforeTurn(_) => return Err(RpcError::invalid_params("a rewind goes to a turn, a tool use or the start")),
    }
    Ok(params)
}

/// The engine's rewind answer in the hub's terms: a removed turn's prompt as the text it was
/// started with (the composer takes text), the rest as the engine said it.
fn rewound(session: &str, ask: &RewindParams, answer: &Value) -> RewindResult {
    let conversation = answer.get("conversation").filter(|c| c.is_object()).map(|c| {
        let count = |k: &str| c.get(k).and_then(Value::as_u64).unwrap_or(0);
        let removed = c.get("removed_turns").and_then(Value::as_array).into_iter().flatten();
        RewoundConversation {
            messages_before: count("messages_before"),
            messages_after: count("messages_after"),
            removed_turns: removed
                .filter_map(|t| {
                    let turn_id = t.get("turn_id").and_then(Value::as_str)?.to_string();
                    let prompt = message_text(&json!({"content": t.get("prompt").cloned().unwrap_or(Value::Null)}));
                    Some(RemovedTurn { turn_id, prompt })
                })
                .collect(),
        }
    });
    let files = answer
        .get("files")
        .filter(|f| f.is_object())
        .map(|f| serde_json::from_value::<RewoundFiles>(f.clone()).unwrap_or_default());
    RewindResult {
        instance: String::new(),
        session_id: session.to_string(),
        rewind_id: answer.get("rewind_id").and_then(Value::as_str).unwrap_or_default().to_string(),
        dry_run: answer.get("dry_run").and_then(Value::as_bool).unwrap_or(ask.dry_run),
        scope: rewind_scope(ask),
        to: ask.to.clone(),
        conversation,
        files,
        fork_session_id: answer.get("fork_session_id").and_then(Value::as_str).map(str::to_string),
    }
}

/// A session's creation, with what the hub set it up with in its options: the scene layer's base
/// as its scene, and what the layer adds — whether someone answers (yes, unless the layer says
/// no), its naming, its prompt, its own tools, allowed outright in a session nobody answers —
/// and the skill directories it sees.
fn set_up(mut params: Value, setup: &SessionSetup) -> Value {
    if !params.is_object() {
        params = json!({});
    }
    let scene = setup.scene.as_ref();
    if let Some(scene) = scene {
        params["scene"] = json!(scene.base);
    }
    let obj = params.as_object_mut().expect("an object just now");
    let options = obj.entry("options").or_insert_with(|| json!({}));
    if !options.is_object() {
        *options = json!({});
    }
    let options = options.as_object_mut().expect("an object just now");
    let attended = scene.and_then(|s| s.attended).unwrap_or(true);
    options.insert("attended".into(), json!(attended));
    // A person answers the model's questions and an MCP server's forms too; without them the
    // session has no `AskUserQuestion` and the server is told no.
    if attended && setup.person {
        options.insert(PROMPT_TYPES.into(), json!([PERMISSION, QUESTION, MCP_ELICITATION]));
    }
    if let Some(scene) = scene {
        if let Some(on) = scene.auto_name {
            options.insert("auto_name".into(), json!(on));
        }
        if let Some(append) = scene.prompt_append() {
            options.insert("prompt".into(), json!({"append": append}));
        }
        if !scene.tools.is_empty() {
            let decls: Vec<Value> = scene.tools.iter().map(|t| host_tool_decl(t.spec())).collect();
            options.insert("host_tools".into(), json!(decls));
            // Nobody answers a session that is unattended: its own tools are let through without
            // an ask, which would be refused. A deny rule the caller gave still wins.
            if !attended {
                let rules = options.entry("permission_rules").or_insert_with(|| json!([]));
                if let Some(rules) = rules.as_array_mut() {
                    rules.extend(scene.tools.iter().map(|t| json!({"source": "session", "behavior": "allow", "tool_name": t.spec().name()})));
                }
            }
        }
    }
    if !setup.skill_dirs.is_empty() {
        let dirs: Vec<Value> = setup.skill_dirs.iter().map(|d| json!({"path": d.display().to_string()})).collect();
        options.insert("skill_dirs".into(), json!(dirs));
    }
    params
}

/// `harnix.session.respond`'s answer as the engine takes it: a question's `decline` gives a
/// reason (the model reads "has not answered: <reason>"), an MCP server's gives none.
/// `session.setModel`'s params: the session, and each key the choice says — `null` for effort or
/// thinking gives the session's own back to the settings'.
fn set_model_params(session: &str, choice: &ModelChoice) -> Value {
    let mut params = json!({"session_id": session});
    if let Some(model) = &choice.model {
        params["model"] = json!(model);
    }
    if let Some(provider) = &choice.provider {
        params["provider"] = json!(provider);
    }
    if let Some(effort) = &choice.effort {
        params["effort"] = json!(effort);
    }
    if let Some(thinking) = &choice.thinking_mode {
        params["thinking_mode"] = json!(thinking);
    }
    params
}

fn respond_params(mut params: Value) -> Value {
    let kind = params.get("prompt_type").and_then(Value::as_str).map(str::to_string);
    if let Some(Value::Object(answer)) = params.get_mut("answer") {
        if answer.get("type").and_then(Value::as_str) == Some("decline") {
            match kind.as_deref() {
                Some(QUESTION) => {
                    answer.entry("reason").or_insert_with(|| json!(DECLINED));
                }
                Some(MCP_ELICITATION) => {
                    answer.remove("reason");
                }
                _ => {}
            }
        }
    }
    params
}

/// The Agix method a row of the hub's passes through to: its name without the `engine.` the
/// kernel's names put in front of the engine's.
fn agix_method(row: &str) -> &str {
    row.strip_prefix("engine.").unwrap_or(row)
}

/// One frame an engine sent unasked, as the hub reads it; `None` for one that is not an event.
pub(crate) fn event(frame: Value) -> Option<EngineEvent> {
    let Value::Object(mut frame) = frame else { return None };
    let params = frame.remove("params").unwrap_or(Value::Null);
    match frame.get("method").and_then(Value::as_str) {
        Some(SESSION_EVENT) => session_event(params),
        Some(DAEMON_EVENT) => Some(daemon_event(params)),
        _ => None,
    }
}

/// Whether a task row is work in progress: a standing member between two messages is `idle`.
fn task_is_live(task: &Value) -> bool {
    matches!(task.get("status").and_then(Value::as_str), Some("running" | "waiting" | "pending"))
}

fn session_event(mut params: Value) -> Option<EngineEvent> {
    let session = params.get("session_id").and_then(Value::as_str)?.to_string();
    let event = params.get_mut("event").map(Value::take).unwrap_or(Value::Null);
    let kind_name = event.get("kind").and_then(Value::as_str).unwrap_or("");
    if kind_name == SUBSCRIPTION_DROPPED {
        return Some(EngineEvent::SubscriptionDropped { session });
    }
    let turn_id = params.get("turn_id").and_then(Value::as_str).unwrap_or("").to_string();
    let field = |k: &str| event.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let kind = match kind_name {
        TEXT_DELTA => SessionEventKind::Fragment(Fragment::new(Joins::Answer, "text", field("text"))),
        THINKING_DELTA => SessionEventKind::Fragment(Fragment::new(Joins::Thinking, "text", field("text"))),
        TOOL_ARGS_DELTA => {
            SessionEventKind::Fragment(Fragment::new(Joins::ToolArgs(field("id")), "partial_json", field("partial_json")))
        }
        TOOL_USE => SessionEventKind::ToolCall { id: field("id"), name: field("name") },
        TOOL_RESULT => SessionEventKind::ToolResult {
            id: field("id"),
            is_error: event.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            wrote: file_writes(&event),
        },
        // A sub-agent's writes are the turn's too.
        SUBAGENT_PROGRESS => match event.get("event") {
            Some(nested) if nested.get("kind").and_then(Value::as_str) == Some(TOOL_RESULT) => {
                let wrote = file_writes(nested);
                if wrote.is_empty() {
                    SessionEventKind::Other
                } else {
                    SessionEventKind::Wrote(wrote)
                }
            }
            _ => SessionEventKind::Other,
        },
        PROMPT => SessionEventKind::Asked(prompt(event.clone())),
        PROMPT_RESOLVED => match event.get("prompt_id").and_then(Value::as_str) {
            Some(id) => SessionEventKind::AskResolved {
                prompt_id: id.to_string(),
                answered: event.get("resolution").and_then(Value::as_str) == Some("answered"),
            },
            None => SessionEventKind::Other,
        },
        GOAL_CHANGED => SessionEventKind::GoalChanged { goal: event.get("goal").cloned().unwrap_or(Value::Null) },
        TEAM_PROGRESS => match event.get("team_id").and_then(Value::as_str) {
            Some(team) => SessionEventKind::TeamProgress { team_id: team.to_string() },
            None => SessionEventKind::Other,
        },
        TASK_STARTED | TASK_FINISHED => {
            let task = event.get("task");
            match task.and_then(|t| t.get("task_id")).and_then(Value::as_str) {
                Some(id) => SessionEventKind::Task {
                    task_id: id.to_string(),
                    started: kind_name == TASK_STARTED,
                    live: kind_name == TASK_STARTED && task.is_some_and(task_is_live),
                    session: task
                        .and_then(|t| t.pointer("/detail/agent_session_id"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                },
                None => SessionEventKind::Other,
            }
        }
        TASK_UPDATED => match event.pointer("/task/task_id").and_then(Value::as_str) {
            Some(id) => SessionEventKind::TaskChanged {
                task_id: id.to_string(),
                live: event.get("task").is_some_and(task_is_live),
            },
            None => SessionEventKind::Other,
        },
        // A trim swaps old tool results for a placeholder and keeps every message: the
        // transcript is not shorter for it.
        COMPACT if event.get("trim").and_then(Value::as_bool) == Some(true) => SessionEventKind::Other,
        COMPACT => SessionEventKind::Compacted,
        REWOUND => SessionEventKind::Rewound { messages: event.get("seq").and_then(Value::as_u64).unwrap_or(0) },
        TURN_COMPLETE => SessionEventKind::TurnComplete(completion(&event)),
        _ => SessionEventKind::Other,
    };
    Some(EngineEvent::Session { session, turn_id, kind, event })
}

/// The files a tool result says its call wrote; none when it says nothing.
fn file_writes(result: &Value) -> Vec<FileWrite> {
    let Some(changes) = result.get(FILE_CHANGES).and_then(Value::as_array) else { return Vec::new() };
    changes
        .iter()
        .filter_map(|c| {
            let path = c.get("path").and_then(Value::as_str)?;
            let text = |k: &str| c.get(k).and_then(Value::as_str).map(str::to_string);
            let count = |k: &str| c.get(k).and_then(Value::as_u64).unwrap_or(0);
            Some(
                FileWrite::new(path, c.get("op").and_then(Value::as_str) == Some("create"))
                    .lines(count("added"), count("removed"))
                    .bytes(c.get("after_bytes").and_then(Value::as_u64))
                    .diff(text("unified_diff"), c.get("truncated").and_then(Value::as_bool).unwrap_or(false), text("omitted")),
            )
        })
        .collect()
}

fn daemon_event(params: Value) -> EngineEvent {
    let field = |k: &str| params.get(k).and_then(Value::as_str).map(str::to_string);
    let session = field("session_id");
    let turn_id = field("turn_id").unwrap_or_default();
    let kind = match (params.get("kind").and_then(Value::as_str).unwrap_or(""), session) {
        (TURN_STARTED, Some(session)) => DaemonEventKind::TurnStarted { session, turn_id },
        (TURN_COMPLETED, Some(session)) => DaemonEventKind::TurnCompleted {
            session,
            turn_id,
            initiator: params.get("initiator").cloned().unwrap_or(Value::Null),
        },
        (SESSION_CREATED | SESSION_RESUMED, session) => DaemonEventKind::SessionsChanged { session, ended: false },
        (SESSION_EVICTED | SESSION_CLOSED | SESSION_DELETED, session) => {
            DaemonEventKind::SessionsChanged { session, ended: true }
        }
        (EVENTS_LAGGED, _) => DaemonEventKind::Lagged,
        (kind @ (MCP_CONNECT_FAILED | MCP_CONNECTED), _) => match field("server") {
            Some(server) => DaemonEventKind::McpServer { server, failed: kind == MCP_CONNECT_FAILED },
            None => DaemonEventKind::Other,
        },
        _ => DaemonEventKind::Other,
    };
    EngineEvent::Daemon { kind, event: params }
}

/// Whether an engine's build has the module that answers `mcp.status`.
pub(crate) fn has_mcp(modules: &[String]) -> bool {
    modules.iter().any(|m| m == MCP_MODULE)
}

/// The server a frame says failed to connect, when it is the engine's own event for that.
pub(crate) fn mcp_failed_server(frame: &Value) -> Option<&str> {
    if frame.get("method").and_then(Value::as_str) != Some(DAEMON_EVENT) {
        return None;
    }
    let params = frame.get("params")?;
    if params.get("kind").and_then(Value::as_str) != Some(MCP_CONNECT_FAILED) {
        return None;
    }
    params.get("server").and_then(Value::as_str)
}

/// The failures an `mcp.status` answer lists, as the events the engine would have sent when they
/// happened: `(server, event)`.
pub(crate) fn mcp_failures(status: &Value) -> Vec<(String, EngineEvent)> {
    let servers = status.get("servers").and_then(Value::as_array).into_iter().flatten();
    servers
        .filter(|s| s.get("state").and_then(Value::as_str) == Some("failed"))
        .filter_map(|s| {
            let name = s.get("name").and_then(Value::as_str)?;
            let error = s.get("error").and_then(Value::as_str).unwrap_or("connect failed");
            let params = json!({"kind": MCP_CONNECT_FAILED, "server": name, "error": error});
            Some((name.to_string(), event(json!({"method": DAEMON_EVENT, "params": params}))?))
        })
        .collect()
}

/// A question as the engine put it, with what it waits for.
fn prompt(value: Value) -> Prompt {
    let field = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_string);
    let waiting = match (field("prompt_type").as_deref(), field("tool_name").as_deref()) {
        (Some(QUESTION | MCP_ELICITATION), _) => Waiting::Answer,
        (_, Some(PLAN_TOOL)) => Waiting::Plan,
        _ => Waiting::Approval,
    };
    let (prompt_id, tool_use_id) = (field("prompt_id"), field("tool_use_id"));
    Prompt::new(value, prompt_id, tool_use_id, waiting)
}

fn completion(event: &Value) -> Completion {
    let present = |k: &str| event.get(k).filter(|v| !v.is_null()).cloned();
    Completion::new(
        event.get("stop_reason").cloned().unwrap_or(Value::Null),
        event.get("usage").cloned().unwrap_or(Value::Null),
    )
    .with_structured_output(present("structured_output"))
    .with_message(present("message"))
}

/// The text of one transcript message, whether its content is a string or blocks.
fn message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The project a session's `session.get` says it works in; empty for one with none.
pub(crate) fn project_root(answer: &Value) -> PathBuf {
    answer.get("project_root").and_then(Value::as_str).map(PathBuf::from).unwrap_or_default()
}

/// One of the frames a connection's own tools and hooks are called by.
#[derive(Debug)]
pub(crate) enum HostFrame {
    ToolCall(HostToolCall),
    ToolCallResolved { call_id: String, withdrawn: bool },
    HookCall(HostHookCall),
    HookCallResolved { call_id: String },
}

/// A call of a tool a connection declared.
#[derive(Debug)]
pub(crate) struct HostToolCall {
    /// The root session: what the answer names.
    pub session: String,
    pub call_id: String,
    pub tool_use_id: String,
    pub name: String,
    pub input: Value,
    /// The delegate's own session, when a delegate made the call.
    pub agent_session: Option<String>,
    /// The calling session's working directory: a delegate in a worktree works in that worktree.
    pub cwd: Option<PathBuf>,
    /// The calling session's depth: 0 for the root, one more for each delegate down.
    pub depth: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HookEvent {
    PreToolUse,
    UserPromptSubmit,
    Other(String),
}

/// A hook a connection registered, called.
#[derive(Debug)]
pub(crate) struct HostHookCall {
    /// The root session: what the answer names.
    pub session: String,
    pub turn_id: Option<String>,
    pub call_id: String,
    pub event: HookEvent,
    pub input: Value,
    pub agent_session: Option<String>,
    /// As in [`HostToolCall`]: the calling session's, the same as the input's own.
    pub cwd: Option<PathBuf>,
    pub depth: Option<u32>,
}

/// What a frame says of one session's activity, for a reader deciding whether to read again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stir {
    TurnStarted,
    TurnEnded,
    Other,
    Reconnect,
}

/// A frame an engine sent unasked, as far as it shows that a session moved: its session and what
/// it did. `events_lagged` says events were lost, so anything may have moved (`Reconnect`, with
/// no session). Read, not taken.
pub(crate) fn stir(frame: &Value) -> Option<(String, Stir)> {
    let params = frame.get("params")?;
    match frame.get("method").and_then(Value::as_str)? {
        DAEMON_EVENT => {
            let kind = params.get("kind").and_then(Value::as_str)?;
            if kind == EVENTS_LAGGED {
                return Some((String::new(), Stir::Reconnect));
            }
            let session = params.get("session_id").and_then(Value::as_str)?.to_string();
            let stir = match kind {
                TURN_STARTED => Stir::TurnStarted,
                TURN_COMPLETED => Stir::TurnEnded,
                _ => Stir::Other,
            };
            Some((session, stir))
        }
        SESSION_EVENT => {
            let session = params.get("session_id").and_then(Value::as_str)?.to_string();
            let stir = match params.pointer("/event/kind").and_then(Value::as_str) {
                Some(TURN_COMPLETE | "error") => Stir::TurnEnded,
                _ => Stir::Other,
            };
            Some((session, stir))
        }
        _ => None,
    }
}

/// The frame a host's tool or hook is called by, or taken back by; `None` for any other. Read,
/// not taken: a frame that is not this link's goes on to [`event`] as it is.
pub(crate) fn host_frame(frame: &Value) -> Option<HostFrame> {
    if frame.get("method").and_then(Value::as_str) != Some(SESSION_EVENT) {
        return None;
    }
    let params = frame.get("params")?;
    let event = params.get("event")?;
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let call_id = text(event, "call_id")?;
    let session = text(params, "session_id").unwrap_or_default();
    let agent_session = text(event, "agent_session_id");
    let cwd = text(event, "cwd").map(PathBuf::from);
    let depth = event.get("depth").and_then(Value::as_u64).map(|d| d as u32);
    match event.get("kind").and_then(Value::as_str)? {
        HOST_TOOL_CALL => Some(HostFrame::ToolCall(HostToolCall {
            session,
            call_id,
            tool_use_id: text(event, "tool_use_id").unwrap_or_default(),
            name: text(event, "name")?,
            input: event.get("input").cloned().unwrap_or(Value::Null),
            agent_session,
            cwd,
            depth,
        })),
        HOST_TOOL_CALL_RESOLVED => Some(HostFrame::ToolCallResolved {
            call_id,
            withdrawn: event.get("resolution").and_then(Value::as_str) == Some(WITHDRAWN),
        }),
        HOOK_CALL => Some(HostFrame::HookCall(HostHookCall {
            session,
            turn_id: text(params, "turn_id"),
            call_id,
            event: match text(event, "hook_event").as_deref() {
                Some(PRE_TOOL_USE) => HookEvent::PreToolUse,
                Some(USER_PROMPT_SUBMIT) => HookEvent::UserPromptSubmit,
                other => HookEvent::Other(other.unwrap_or_default().to_string()),
            },
            input: event.get("input").cloned().unwrap_or(Value::Null),
            agent_session,
            cwd,
            depth,
        })),
        HOOK_CALL_RESOLVED => Some(HostFrame::HookCallResolved { call_id }),
        _ => None,
    }
}

/// A frame a host tool's permission ask is judged from: a call of a tool and its end — a
/// delegate's mirrored in its progress — a permission ask about one, and an ask resolved.
#[derive(Debug, PartialEq)]
pub(crate) enum PermitFrame {
    ToolUse { session: String, id: String, name: String, input: Value },
    ToolResult { session: String, id: String },
    Asked { session: String, tool_name: String, tool_use_id: String },
    Resolved { prompt_id: String },
}

/// The [`PermitFrame`] `frame` is, if any.
pub(crate) fn permit_frame(frame: &Value) -> Option<PermitFrame> {
    if frame.get("method").and_then(Value::as_str) != Some(SESSION_EVENT) {
        return None;
    }
    let params = frame.get("params")?;
    let session = params.get("session_id").and_then(Value::as_str)?.to_string();
    let mut event = params.get("event")?;
    let nested = event.get("kind").and_then(Value::as_str) == Some(SUBAGENT_PROGRESS);
    if nested {
        event = event.get("event")?;
    }
    let text = |k: &str| event.get(k).and_then(Value::as_str).map(str::to_string);
    match event.get("kind").and_then(Value::as_str)? {
        TOOL_USE => Some(PermitFrame::ToolUse {
            session,
            id: text("id")?,
            name: text("name")?,
            input: event.get("input").cloned().unwrap_or(Value::Null),
        }),
        TOOL_RESULT => Some(PermitFrame::ToolResult { session, id: text("id")? }),
        // A delegate's ask is the root session's own frame, not mirrored.
        PROMPT if !nested => {
            if text("prompt_type").is_some_and(|t| t != PERMISSION) {
                return None;
            }
            Some(PermitFrame::Asked { session, tool_name: text("tool_name")?, tool_use_id: text("tool_use_id")? })
        }
        PROMPT_RESOLVED if !nested => Some(PermitFrame::Resolved { prompt_id: text("prompt_id")? }),
        _ => None,
    }
}

/// The ask a frame is.
pub(crate) fn prompt_id(frame: &Value) -> Option<String> {
    frame.pointer("/params/event/prompt_id").and_then(Value::as_str).map(str::to_string)
}

/// An ask, with the tool's own question beside the engine's line.
pub(crate) fn with_tool_question(frame: &mut Value, question: &str) {
    if let Some(Value::Object(event)) = frame.pointer_mut("/params/event") {
        event.insert("tool_question".into(), json!(question));
    }
}

/// The answer to a permission ask: permitted, or refused with why.
pub(crate) fn permit_answer(session: &str, prompt_id: &str, decision: Result<(), String>) -> Value {
    let decision = match decision {
        Ok(()) => json!({"type": "permit"}),
        Err(why) => json!({"type": "deny", "reason": why}),
    };
    json!({"session_id": session, "prompt_id": prompt_id, "prompt_type": PERMISSION, "decision": decision})
}

/// The longest description a declared tool may have, in bytes.
const MAX_DESCRIPTION: usize = 8192;
/// How long the engine waits for a declared tool: its longest. A native tool has no timeout of its
/// own, and a package's has its call's.
const HOST_TOOL_TIMEOUT_MS: u64 = 600_000;
/// How long the engine waits for a registered hook: its longest. A package's hook is a call to
/// another process.
const HOST_HOOK_TIMEOUT_MS: u64 = 30_000;

/// A tool, as a connection declares it: delegates have it too.
pub(crate) fn host_tool_decl(spec: &ToolSpec) -> Value {
    let mut description = match spec.documentation() {
        Some(doc) => format!("{}\n\n{doc}", spec.description()),
        None => spec.description().to_string(),
    };
    if description.len() > MAX_DESCRIPTION {
        let mut end = MAX_DESCRIPTION;
        while !description.is_char_boundary(end) {
            end -= 1;
        }
        description.truncate(end);
    }
    json!({
        "name": spec.name(),
        "description": description,
        "input_schema": spec.input_schema(),
        "read_only": spec.is_read_only(),
        "destructive": spec.is_destructive(),
        "concurrency_safe": spec.is_concurrency_safe(),
        "timeout_ms": HOST_TOOL_TIMEOUT_MS,
        "subagents": true,
    })
}

pub(crate) fn host_tools_params(decls: Vec<Value>) -> Value {
    json!({"host_tools": decls})
}

/// The tool names a `daemon.setHostTools` answer took.
pub(crate) fn adopted_tools(answer: &Value) -> Vec<String> {
    answer
        .get("host_tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|n| n.as_str().map(str::to_string))
        .collect()
}

/// What a `daemon.setHostTools` or `daemon.setHooks` answer refused: the entry's name when it
/// has one, and why.
pub(crate) fn refused(answer: &Value) -> Vec<(Option<String>, String)> {
    answer
        .get("refused")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|r| {
            let name = r.get("name").and_then(Value::as_str).map(str::to_string);
            let error = r.get("error").and_then(Value::as_str).unwrap_or("refused").to_string();
            (name, error)
        })
        .collect()
}

/// The hooks a connection registers: one before every tool call, which lets the call through
/// when it fails, and one on every message, which stops the message when it fails — a gate that
/// did not answer has not let anything through.
pub(crate) fn hooks_params(tool_hooks: bool, gates: bool) -> Value {
    let mut hooks = Vec::new();
    if tool_hooks {
        hooks.push(json!({"events": [PRE_TOOL_USE], "tools": ["*"], "timeout_ms": HOST_HOOK_TIMEOUT_MS, "on_error": "allow"}));
    }
    if gates {
        hooks.push(json!({"events": [USER_PROMPT_SUBMIT], "timeout_ms": HOST_HOOK_TIMEOUT_MS, "on_error": "block"}));
    }
    json!({"hooks": hooks})
}

/// The hooks [`hooks_params`] registers, one per event, as a link reports them.
pub(crate) fn registered_hooks(tool_hooks: bool, gates: bool) -> Vec<harnix_engine_link::HostHook> {
    let params = hooks_params(tool_hooks, gates);
    let mut out = Vec::new();
    for hook in params["hooks"].as_array().into_iter().flatten() {
        let tools: Vec<String> = hook["tools"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        let on_error = hook["on_error"].as_str().unwrap_or("allow");
        for event in hook["events"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            out.push(harnix_engine_link::HostHook::new(event, tools.clone(), on_error));
        }
    }
    out
}

/// A tool's answer, as the engine reads it. A tool that failed, ran: only a call that could not
/// be made is an `error`, which also stops the calls running beside it. Structured content has
/// nowhere to go and is left out: a value's text is in the content already.
pub(crate) fn tool_answer(session: &str, call_id: &str, out: Result<ToolOutput, ToolError>) -> Value {
    let mut answer = json!({"session_id": session, "call_id": call_id});
    let (key, value) = match out {
        Ok(out) => ("result", json!({"content": out.content(), "is_error": out.is_error()})),
        Err(ToolError::Failed(m)) => ("result", json!({"content": m, "is_error": true})),
        Err(ToolError::Cancelled) => ("error", json!({"message": "cancelled"})),
        Err(ToolError::NotFound(m)) => ("error", json!({"message": m})),
        Err(other) => ("result", json!({"content": other.to_string(), "is_error": true})),
    };
    answer[key] = value;
    answer
}

pub(crate) fn tool_progress(session: &str, call_id: &str, message: &str) -> Value {
    json!({"session_id": session, "call_id": call_id, "message": message})
}

/// What a connection's hook decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HookVerdict {
    Proceed { context: Option<String> },
    Block(String),
}

pub(crate) fn hook_answer(session: &str, call_id: &str, verdict: HookVerdict) -> Value {
    let mut answer = json!({"session_id": session, "call_id": call_id});
    match verdict {
        HookVerdict::Block(message) => {
            answer["decision"] = json!("block");
            if !message.is_empty() {
                answer["message"] = json!(message);
            }
        }
        HookVerdict::Proceed { context } => {
            answer["decision"] = json!("proceed");
            if let Some(context) = context {
                answer["additional_context"] = json!(context);
            }
        }
    }
    answer
}

/// A tool hook's input: the tool, its input, the session that called it and where it works.
pub(crate) fn pre_tool_use(input: &Value) -> (String, Value, Option<String>, Option<PathBuf>) {
    let text = |k: &str| input.get(k).and_then(Value::as_str).map(str::to_string);
    (
        text("tool_name").unwrap_or_default(),
        input.get("tool_input").cloned().unwrap_or(Value::Null),
        text("session_id"),
        text("cwd").map(PathBuf::from),
    )
}

/// A prompt hook's input: the message, how it is delivered, and the session it is for.
pub(crate) fn user_prompt(input: &Value) -> (String, Delivery, Option<String>) {
    let text = |k: &str| input.get(k).and_then(Value::as_str).map(str::to_string);
    let delivery = match text("delivery").as_deref() {
        // Spelled here, not as a constant: the hub has the word for its own steering.
        Some("steer") => Delivery::Steer,
        _ => Delivery::Turn,
    };
    (text("user_prompt").unwrap_or_default(), delivery, text("session_id"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_choice_names_only_what_it_changes() {
        let choice = ModelChoice { model: Some("m".into()), effort: Some(Some("high".into())), thinking_mode: Some(None), ..Default::default() };
        assert_eq!(set_model_params("s", &choice), json!({"session_id": "s", "model": "m", "effort": "high", "thinking_mode": null}));
        let provider = ModelChoice { provider: Some("p".into()), ..Default::default() };
        assert_eq!(set_model_params("s", &provider), json!({"session_id": "s", "provider": "p"}));
    }

    #[test]
    fn a_frame_says_which_session_moved_and_whether_a_turn_began_or_ended() {
        let daemon = |kind: &str, session: Option<&str>| json!({"method": DAEMON_EVENT, "params": {"kind": kind, "session_id": session}});
        let session = |kind: &str| json!({"method": SESSION_EVENT, "params": {"session_id": "s", "event": {"kind": kind}}});
        assert_eq!(stir(&daemon("turn_started", Some("s"))), Some(("s".into(), Stir::TurnStarted)));
        assert_eq!(stir(&daemon("turn_completed", Some("s"))), Some(("s".into(), Stir::TurnEnded)));
        assert_eq!(stir(&daemon("session_created", Some("s"))), Some(("s".into(), Stir::Other)));
        assert_eq!(stir(&daemon("events_lagged", None)), Some((String::new(), Stir::Reconnect)));
        assert_eq!(stir(&daemon("session_created", None)), None, "no session named");
        assert_eq!(stir(&session("turn_complete")), Some(("s".into(), Stir::TurnEnded)));
        assert_eq!(stir(&session("error")), Some(("s".into(), Stir::TurnEnded)));
        assert_eq!(stir(&session("text_delta")), Some(("s".into(), Stir::Other)));
        assert_eq!(stir(&json!({"id": 3, "result": {}})), None);
    }

    fn session(event: Value) -> EngineEvent {
        event_of(json!({"jsonrpc": "2.0", "method": SESSION_EVENT, "params": {"session_id": "s", "turn_id": "t", "event": event}}))
    }

    fn event_of(frame: Value) -> EngineEvent {
        super::event(frame).expect("an event")
    }

    fn kind(e: EngineEvent) -> SessionEventKind {
        match e {
            EngineEvent::Session { kind, .. } => kind,
            other => panic!("not a session event: {other:?}"),
        }
    }

    #[test]
    fn a_session_event_keeps_the_engines_value_and_says_what_it_is() {
        let e = session(json!({"kind": "text_delta", "text": "hel"}));
        let EngineEvent::Session { session, turn_id, kind, event } = e else { panic!() };
        assert_eq!((session.as_str(), turn_id.as_str()), ("s", "t"));
        assert_eq!(kind, SessionEventKind::Fragment(Fragment::new(Joins::Answer, "text", "hel")));
        assert_eq!(event, json!({"kind": "text_delta", "text": "hel"}));

        let args = kind_of(json!({"kind": "tool_args_delta", "id": "c", "partial_json": "{"}));
        assert_eq!(args, SessionEventKind::Fragment(Fragment::new(Joins::ToolArgs("c".into()), "partial_json", "{")));
        assert_eq!(
            kind_of(json!({"kind": "task_finished", "task": {"task_id": "k"}})),
            SessionEventKind::Task { task_id: "k".into(), started: false, session: None, live: false }
        );
        assert_eq!(
            kind_of(json!({"kind": "task_started", "task": {"task_id": "k", "detail": {"agent_session_id": "sub"}}})),
            SessionEventKind::Task { task_id: "k".into(), started: true, session: Some("sub".into()), live: false }
        );
        let member = json!({"kind": "task_started", "task": {"task_id": "member-1", "status": "idle"}});
        assert_eq!(
            kind_of(member),
            SessionEventKind::Task { task_id: "member-1".into(), started: true, session: None, live: false }
        );
        let running = json!({"kind": "task_started", "task": {"task_id": "k", "status": "running"}});
        assert!(matches!(kind_of(running), SessionEventKind::Task { live: true, .. }));
        let updated = |status: &str| json!({"kind": "task_updated", "task": {"task_id": "k", "status": status}});
        assert_eq!(kind_of(updated("running")), SessionEventKind::TaskChanged { task_id: "k".into(), live: true });
        assert_eq!(kind_of(updated("idle")), SessionEventKind::TaskChanged { task_id: "k".into(), live: false });
        assert_eq!(kind_of(json!({"kind": "task_updated"})), SessionEventKind::Other);
        assert_eq!(kind_of(json!({"kind": "compact", "strategy": "MicroCompact", "trim": true})), SessionEventKind::Other);
        assert_eq!(kind_of(json!({"kind": "compact", "strategy": "Summary"})), SessionEventKind::Compacted);
        assert_eq!(kind_of(json!({"kind": "assistant_message"})), SessionEventKind::Other);
    }

    fn kind_of(event: Value) -> SessionEventKind {
        kind(session(event))
    }

    #[test]
    fn the_files_a_tool_result_wrote_are_read_off_it_and_off_a_sub_agents() {
        let change = json!({"path": "/w/src/a.rs", "op": "modify", "before_hash": "sha256:1", "after_hash": "sha256:2",
            "before_bytes": 10, "after_bytes": 12, "added": 3, "removed": 1,
            "unified_diff": "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+b\n", "truncated": false, "omitted": null});
        let created = json!({"path": "/w/big.bin", "op": "create", "before_hash": null, "after_hash": null, "before_bytes": null,
            "after_bytes": 99, "added": 0, "removed": 0, "unified_diff": null, "truncated": false, "omitted": "binary"});
        let result = json!({"kind": "tool_result", "id": "u", "name": "Write", "content": "ok", "is_error": false, "file_changes": [change, created]});
        let SessionEventKind::ToolResult { id, is_error, wrote } = kind_of(result.clone()) else { panic!() };
        assert_eq!((id.as_str(), is_error, wrote.len()), ("u", false, 2));
        assert_eq!(
            wrote[0],
            FileWrite::new("/w/src/a.rs", false)
                .lines(3, 1)
                .bytes(Some(12))
                .diff(Some("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+b\n".into()), false, None)
        );
        assert_eq!(wrote[1], FileWrite::new("/w/big.bin", true).bytes(Some(99)).diff(None, false, Some("binary".into())));
        // A tool that writes nothing says nothing.
        let plain = kind_of(json!({"kind": "tool_result", "id": "v", "content": "x"}));
        assert_eq!(plain, SessionEventKind::ToolResult { id: "v".into(), is_error: false, wrote: Vec::new() });
        // A sub-agent's: what it wrote, and nothing for the rest of its stream.
        let nested = kind_of(json!({"kind": "subagent_progress", "agent_session_id": "sub", "event": result}));
        assert!(matches!(nested, SessionEventKind::Wrote(w) if w.len() == 2));
        assert_eq!(kind_of(json!({"kind": "subagent_progress", "event": {"kind": "text_delta", "text": "x"}})), SessionEventKind::Other);
        assert_eq!(kind_of(json!({"kind": "subagent_progress", "event": {"kind": "tool_result", "id": "w"}})), SessionEventKind::Other);
    }

    #[test]
    fn a_question_says_what_it_waits_for() {
        let asked = |e: Value| match kind_of(e) {
            SessionEventKind::Asked(p) => p,
            other => panic!("{other:?}"),
        };
        let p = asked(json!({"kind": "prompt", "prompt_id": "p", "tool_use_id": "u", "tool_name": "Bash"}));
        assert_eq!((p.prompt_id.as_deref(), p.tool_use_id.as_deref(), p.waiting), (Some("p"), Some("u"), Waiting::Approval));
        assert_eq!(asked(json!({"kind": "prompt", "prompt_type": "question"})).waiting, Waiting::Answer);
        assert_eq!(asked(json!({"kind": "prompt", "prompt_type": "mcp_elicitation"})).waiting, Waiting::Answer);
        assert_eq!(asked(json!({"kind": "prompt", "tool_name": "ExitPlanMode"})).waiting, Waiting::Plan);
        assert_eq!(
            kind_of(json!({"kind": "prompt_resolved", "prompt_id": "p", "resolution": "withdrawn", "reason": "timed_out"})),
            SessionEventKind::AskResolved { prompt_id: "p".into(), answered: false }
        );
        assert_eq!(
            kind_of(json!({"kind": "prompt_resolved", "prompt_id": "p", "resolution": "answered"})),
            SessionEventKind::AskResolved { prompt_id: "p".into(), answered: true }
        );
        assert_eq!(kind_of(json!({"kind": "prompt_resolved"})), SessionEventKind::Other);
    }

    #[test]
    fn a_dropped_subscription_and_the_engines_own_events_are_told_apart() {
        assert!(matches!(
            session(json!({"kind": "subscription_dropped"})),
            EngineEvent::SubscriptionDropped { session } if session == "s"
        ));
        let daemon = |p: Value| match event_of(json!({"jsonrpc": "2.0", "method": DAEMON_EVENT, "params": p})) {
            EngineEvent::Daemon { kind, .. } => kind,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            daemon(json!({"kind": "turn_completed", "session_id": "s", "turn_id": "t", "initiator": "schedule"})),
            DaemonEventKind::TurnCompleted { session: "s".into(), turn_id: "t".into(), initiator: json!("schedule") }
        );
        assert_eq!(
            daemon(json!({"kind": "session_deleted", "session_id": "s"})),
            DaemonEventKind::SessionsChanged { session: Some("s".into()), ended: true }
        );
        assert_eq!(daemon(json!({"kind": "events_lagged"})), DaemonEventKind::Lagged);
        assert_eq!(
            daemon(json!({"kind": "mcp_connect_failed", "server": "x", "error": "e"})),
            DaemonEventKind::McpServer { server: "x".into(), failed: true }
        );
        assert_eq!(
            daemon(json!({"kind": "mcp_connected", "server": "x"})),
            DaemonEventKind::McpServer { server: "x".into(), failed: false }
        );
        assert!(super::event(json!({"jsonrpc": "2.0", "id": 3, "result": {}})).is_none(), "a reply is no event");
    }

    #[test]
    fn a_completion_is_the_points_a_caller_acts_on() {
        let SessionEventKind::TurnComplete(c) = kind_of(json!({
            "kind": "turn_complete", "stop_reason": "end_turn", "usage": {"input_tokens": 1},
            "structured_output": null, "message": "said",
        })) else {
            panic!()
        };
        assert_eq!(c.stop_reason, json!("end_turn"));
        assert_eq!(c.structured_output, None, "null is not an output");
        assert_eq!(c.message, Some(json!("said")));
    }

    #[test]
    fn a_host_frame_is_one_of_four_and_anything_else_is_not() {
        let frame = |event: Value| json!({"jsonrpc": "2.0", "method": SESSION_EVENT, "params": {"session_id": "s", "turn_id": "t", "event": event}});
        let Some(HostFrame::ToolCall(call)) = host_frame(&frame(json!({
            "kind": "host_tool_call", "call_id": "c", "tool_use_id": "u", "name": "n", "input": {"a": 1}, "agent_session_id": "sub",
            "cwd": "/w/.harnix/worktrees/x", "depth": 2,
        }))) else {
            panic!()
        };
        assert_eq!((call.session.as_str(), call.call_id.as_str(), call.name.as_str()), ("s", "c", "n"));
        assert_eq!((call.input.clone(), call.agent_session.as_deref()), (json!({"a": 1}), Some("sub")));
        assert_eq!((call.cwd.as_deref(), call.depth), (Some(std::path::Path::new("/w/.harnix/worktrees/x")), Some(2)));
        assert!(matches!(
            host_frame(&frame(json!({"kind": "host_tool_call_resolved", "call_id": "c", "resolution": "withdrawn"}))),
            Some(HostFrame::ToolCallResolved { withdrawn: true, .. })
        ));
        assert!(matches!(
            host_frame(&frame(json!({"kind": "host_tool_call_resolved", "call_id": "c", "resolution": "answered"}))),
            Some(HostFrame::ToolCallResolved { withdrawn: false, .. })
        ));
        let Some(HostFrame::HookCall(hook)) = host_frame(&frame(json!({"kind": "hook_call", "call_id": "h", "hook_event": "UserPromptSubmit", "input": {}}))) else {
            panic!()
        };
        assert_eq!((hook.event, hook.turn_id.as_deref()), (HookEvent::UserPromptSubmit, Some("t")));
        // An older engine's frame says neither.
        assert_eq!((hook.cwd, hook.depth), (None, None));
        let Some(HostFrame::HookCall(hook)) =
            host_frame(&frame(json!({"kind": "hook_call", "call_id": "h", "hook_event": "PreToolUse", "input": {}, "cwd": "/w", "depth": 1})))
        else {
            panic!()
        };
        assert_eq!((hook.cwd, hook.depth), (Some(PathBuf::from("/w")), Some(1)));
        assert!(matches!(host_frame(&frame(json!({"kind": "hook_call_resolved", "call_id": "h"}))), Some(HostFrame::HookCallResolved { .. })));
        assert!(host_frame(&frame(json!({"kind": "text_delta", "text": "x"}))).is_none());
        assert!(host_frame(&json!({"jsonrpc": "2.0", "method": DAEMON_EVENT, "params": {"event": {"kind": "hook_call", "call_id": "h"}}})).is_none());
    }

    struct Quiet(ToolSpec);

    #[async_trait::async_trait]
    impl harnix_engine_api::Tool for Quiet {
        fn spec(&self) -> &ToolSpec {
            &self.0
        }
        async fn call(&self, _: harnix_engine_api::ToolCx, _: Value) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::text(""))
        }
    }

    fn layer(scene: harnix_engine_api::SceneLayer) -> harnix_engine_link::ExpandedScene {
        let id = scene.id().to_string();
        harnix_engine_link::SceneLayers::new(&[scene]).unwrap().get(&id).cloned().unwrap()
    }

    #[test]
    fn a_session_is_created_on_its_layers_base_with_what_the_layer_adds() {
        use harnix_engine_api::{PromptText, SceneBase, SceneLayer};
        let tool = std::sync::Arc::new(Quiet(ToolSpec::new("Probe", "p", json!({"type": "object"}))));
        let probe = layer(
            SceneLayer::new("probe", SceneBase::builtin("harnix"))
                .prompt(PromptText::plain("one"))
                .prompt(PromptText::cached("two"))
                .tool(tool.clone())
                .auto_name(false),
        );
        let setup = SessionSetup::new().scene(Some(probe));
        let sent = set_up(json!({"scene": "probe", "options": {"model": "m"}}), &setup);
        assert_eq!(sent["scene"], "harnix");
        let options = &sent["options"];
        assert_eq!((&options["model"], &options["attended"], &options["auto_name"]), (&json!("m"), &json!(true), &json!(false)));
        assert_eq!(options["prompt"], json!({"append": "one\n\ntwo"}));
        assert_eq!(options["host_tools"][0]["name"], "Probe");
        assert!(options.get("permission_rules").is_none(), "an attended session asks");
        assert!(options.get("skill_dirs").is_none());

        let quiet = layer(SceneLayer::new("quiet", SceneBase::builtin("code")).tool(tool).interactive(false));
        let rules = json!([{"source": "session", "behavior": "deny", "tool_name": "Probe"}]);
        let sent = set_up(json!({"options": {"permission_rules": rules}}), &SessionSetup::new().scene(Some(quiet)));
        assert_eq!((&sent["scene"], &sent["options"]["attended"]), (&json!("code"), &json!(false)));
        let rules = sent["options"]["permission_rules"].as_array().unwrap();
        assert_eq!((rules[0]["behavior"].as_str(), rules[1]["behavior"].as_str()), (Some("deny"), Some("allow")), "the caller's first");
        assert!(sent["options"].get("prompt").is_none() && sent["options"].get("auto_name").is_none());

        let plain = set_up(Value::Null, &SessionSetup::new().skill_dirs(vec![PathBuf::from("/s/a")]));
        assert_eq!(plain, json!({"options": {"attended": true, "skill_dirs": [{"path": "/s/a"}]}}));
    }

    #[test]
    fn a_persons_session_takes_questions_and_forms_unless_nobody_attends_it() {
        use harnix_engine_api::{SceneBase, SceneLayer};
        let sent = set_up(Value::Null, &SessionSetup::new().person(true));
        assert_eq!(sent["options"]["prompt_types"], json!(["permission", "question", "mcp_elicitation"]));
        assert!(set_up(Value::Null, &SessionSetup::new()).pointer("/options/prompt_types").is_none(), "an extension's");
        let quiet = layer(SceneLayer::new("quiet", SceneBase::builtin("code")).interactive(false));
        let sent = set_up(Value::Null, &SessionSetup::new().scene(Some(quiet)).person(true));
        assert!(sent.pointer("/options/prompt_types").is_none(), "an unattended one's");
    }

    #[tokio::test]
    async fn an_answer_reaches_the_engine_in_its_shape() {
        struct Echo;
        #[async_trait::async_trait]
        impl Rpc for Echo {
            async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
                Ok(json!({"method": method, "params": params}))
            }
        }
        let ops = AgixOps(Arc::new(Echo));
        let sent = |params: Value| {
            let ops = &ops;
            async move { ops.respond(params).await.unwrap() }
        };
        let decision = json!({"session_id": "s", "prompt_id": "p", "decision": {"type": "permit"}});
        assert_eq!(sent(decision.clone()).await, json!({"method": "session.respondToPrompt", "params": decision}));
        let declined = sent(json!({"session_id": "s", "prompt_id": "p", "prompt_type": "question", "answer": {"type": "decline"}})).await;
        assert_eq!(declined["params"]["answer"], json!({"type": "decline", "reason": "declined"}));
        let said = sent(json!({"prompt_type": "question", "answer": {"type": "decline", "reason": "later"}})).await;
        assert_eq!(said["params"]["answer"]["reason"], "later");
        let form = sent(json!({"prompt_type": "mcp_elicitation", "answer": {"type": "decline", "reason": "no"}})).await;
        assert_eq!(form["params"]["answer"], json!({"type": "decline"}));
        let accepted = sent(json!({"prompt_type": "mcp_elicitation", "answer": {"type": "accept", "content": {"n": 1}}})).await;
        assert_eq!(accepted["params"]["answer"], json!({"type": "accept", "content": {"n": 1}}));
    }

    #[tokio::test]
    async fn set_prompt_sends_only_what_it_was_given() {
        struct Echo;
        #[async_trait::async_trait]
        impl Rpc for Echo {
            async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
                Ok(json!({"method": method, "params": params}))
            }
        }
        let ops = AgixOps(Arc::new(Echo));
        let sent = ops.set_prompt("s", Some(Some("a".into())), None).await.unwrap();
        assert_eq!(sent, json!({"method": "session.setPrompt", "params": {"session_id": "s", "append": "a"}}));
        let sent = ops.set_prompt("s", Some(None), Some(Some("o".into()))).await.unwrap();
        assert_eq!(sent["params"], json!({"session_id": "s", "append": null, "override": "o"}));
    }

    fn ask(to: RewindTo) -> RewindParams {
        serde_json::from_value(json!({"session_id": "s", "to": to})).unwrap()
    }

    #[test]
    fn a_rewind_names_one_target_and_always_its_scope() {
        let mut dry = ask(RewindTo::Turn("t1".into()));
        dry.dry_run = true;
        let sent = rewind_params("s", &dry).unwrap();
        assert_eq!(
            sent,
            json!({"session_id": "s", "to_turn": "t1", "scope": "both", "dry_run": true, "fork_first": false, "force": false, "allow_partial": false})
        );
        assert_eq!(rewind_params("s", &ask(RewindTo::Start)).unwrap()["to_start"], true);
        let call = rewind_params("s", &ask(RewindTo::ToolUse("u1".into()))).unwrap();
        assert_eq!((&call["to_tool_use"], &call["scope"]), (&json!("u1"), &json!("files")), "a tool use takes the files alone");
        let mut conversation = ask(RewindTo::Start);
        conversation.scope = Some(RewindScope::Conversation);
        assert_eq!(rewind_params("s", &conversation).unwrap()["scope"], "conversation");
        assert!(rewind_params("s", &ask(RewindTo::BeforeTurn("t2".into()))).is_err(), "the hub resolves it first");
    }

    #[test]
    fn a_rewinds_answer_gives_each_removed_turn_its_text() {
        let answer = json!({
            "session_id": "s", "rewind_id": "r1", "dry_run": true, "scope": "both",
            "cut": {"to_turn": "t1", "to_tool_use": null, "to_start": false},
            "conversation": {"messages_before": 6, "messages_after": 2, "removed_turns": [
                {"turn_id": "t2", "prompt": [{"type": "text", "text": "fix a"}, {"type": "image"}, {"type": "text", "text": "and b"}]}
            ]},
            "files": {"restored": [{"path": "a.rs", "to": "file", "sha256": "x", "outside_project": false, "unverified": false}],
                      "skipped": [{"path": "b.rs", "issue": "modified", "detail": "changed", "overridable_by": "force"}],
                      "overwrote_external": []},
            "fork_session_id": null
        });
        let mut dry = ask(RewindTo::Turn("t1".into()));
        dry.dry_run = true;
        let got = rewound("s", &dry, &answer);
        assert_eq!((got.rewind_id.as_str(), got.dry_run, got.scope), ("r1", true, RewindScope::Both));
        let conversation = got.conversation.unwrap();
        assert_eq!((conversation.messages_before, conversation.messages_after), (6, 2));
        assert_eq!(conversation.removed_turns, [RemovedTurn { turn_id: "t2".into(), prompt: "fix a\nand b".into() }]);
        let files = got.files.unwrap();
        assert_eq!((files.restored[0].path.as_str(), files.restored[0].to.as_str()), ("a.rs", "file"));
        assert_eq!((files.skipped[0].issue.as_str(), files.skipped[0].overridable_by.as_str()), ("modified", "force"));
        let files_only = rewound("s", &ask(RewindTo::ToolUse("u".into())), &json!({"rewind_id": "r2", "conversation": null, "files": {"restored": [], "skipped": [], "overwrote_external": []}, "fork_session_id": "s2"}));
        assert!(files_only.conversation.is_none());
        assert_eq!(files_only.fork_session_id.as_deref(), Some("s2"));
    }

    #[test]
    fn a_rewound_frame_says_how_long_the_conversation_is_now() {
        let e = session(json!({"kind": "rewound", "rewind_id": "r", "scope": "both", "seq": 4, "files_restored": [], "files_skipped": [], "overwrote_external": []}));
        assert_eq!(kind(e), SessionEventKind::Rewound { messages: 4 });
    }

    #[test]
    fn the_frames_a_permission_is_judged_from_are_read() {
        let frame = |event: Value| json!({"jsonrpc": "2.0", "method": SESSION_EVENT, "params": {"session_id": "s", "event": event}});
        assert_eq!(
            permit_frame(&frame(json!({"kind": "tool_use", "id": "u", "name": "T", "input": {"a": 1}}))),
            Some(PermitFrame::ToolUse { session: "s".into(), id: "u".into(), name: "T".into(), input: json!({"a": 1}) })
        );
        let nested = frame(json!({"kind": "subagent_progress", "event": {"kind": "tool_result", "id": "v"}}));
        assert_eq!(permit_frame(&nested), Some(PermitFrame::ToolResult { session: "s".into(), id: "v".into() }));
        let ask = frame(json!({"kind": "prompt", "prompt_type": "permission", "prompt_id": "p", "tool_name": "T", "tool_use_id": "u", "message": "Allow?"}));
        assert_eq!(permit_frame(&ask), Some(PermitFrame::Asked { session: "s".into(), tool_name: "T".into(), tool_use_id: "u".into() }));
        assert_eq!(prompt_id(&ask).as_deref(), Some("p"));
        assert!(permit_frame(&frame(json!({"kind": "prompt", "prompt_type": "question", "prompt_id": "p", "tool_use_id": "u"}))).is_none());
        assert!(permit_frame(&frame(json!({"kind": "text_delta", "text": "x"}))).is_none());
        let mut asked = ask.clone();
        with_tool_question(&mut asked, "Really?");
        assert_eq!((&asked["params"]["event"]["tool_question"], &asked["params"]["event"]["message"]), (&json!("Really?"), &json!("Allow?")));
        assert_eq!(permit_answer("s", "p", Err("no".into()))["decision"], json!({"type": "deny", "reason": "no"}));
    }

    #[test]
    fn a_tools_answer_is_a_result_unless_it_could_not_run() {
        let answer = |out| tool_answer("s", "c", out);
        let ok = answer(Ok(ToolOutput::json(json!({"a": 1}))));
        assert_eq!(ok["result"]["is_error"], false);
        assert!(ok["result"]["content"].as_str().unwrap().contains("\"a\""));
        assert_eq!((&ok["session_id"], &ok["call_id"]), (&json!("s"), &json!("c")));
        assert_eq!(answer(Ok(ToolOutput::error("no")))["result"], json!({"content": "no", "is_error": true}));
        assert_eq!(answer(Err(ToolError::Failed("f".into())))["result"], json!({"content": "f", "is_error": true}));
        assert_eq!(answer(Err(ToolError::Cancelled))["error"], json!({"message": "cancelled"}));
        assert_eq!(answer(Err(ToolError::NotFound("gone".into())))["error"], json!({"message": "gone"}));
    }

    #[test]
    fn a_declared_tool_carries_its_doc_cut_at_a_character() {
        let spec = ToolSpec::new("t", "short", json!({"type": "object"})).doc("long").read_only();
        let decl = host_tool_decl(&spec);
        assert_eq!(decl["description"], "short\n\nlong");
        assert_eq!((&decl["read_only"], &decl["concurrency_safe"], &decl["destructive"]), (&json!(true), &json!(false), &json!(false)));
        let wide = ToolSpec::new("t", &"é".repeat(5000), json!({"type": "object"}));
        let cut = host_tool_decl(&wide)["description"].as_str().unwrap().to_string();
        assert!(cut.len() <= 8192 && cut.chars().all(|c| c == 'é'));
    }

    #[test]
    fn the_hooks_are_one_before_tools_and_one_on_messages() {
        assert_eq!(hooks_params(false, false), json!({"hooks": []}));
        let both = hooks_params(true, true);
        assert_eq!(both["hooks"][0]["on_error"], "allow");
        assert_eq!(both["hooks"][1]["events"], json!(["UserPromptSubmit"]));
        assert_eq!(hooks_params(false, true)["hooks"].as_array().unwrap().len(), 1);
        assert_eq!(hook_answer("s", "h", HookVerdict::Block(String::new())), json!({"session_id": "s", "call_id": "h", "decision": "block"}));
    }

    #[test]
    fn the_registered_hooks_are_reported_as_registered() {
        assert!(registered_hooks(false, false).is_empty());
        let both = registered_hooks(true, true);
        assert_eq!(both.len(), 2);
        assert_eq!((both[0].event.as_str(), both[0].tools.clone(), both[0].on_error.as_str()), ("PreToolUse", vec!["*".to_string()], "allow"));
        assert_eq!((both[1].event.as_str(), both[1].tools.is_empty(), both[1].on_error.as_str()), ("UserPromptSubmit", true, "block"));
    }

    #[test]
    fn a_hooks_input_is_read_for_what_it_asks() {
        let (tool, input, session, cwd) = pre_tool_use(&json!({"tool_name": "Bash", "tool_input": {"c": 1}, "session_id": "s", "cwd": "/w"}));
        assert_eq!((tool.as_str(), input, session.as_deref(), cwd), ("Bash", json!({"c": 1}), Some("s"), Some(PathBuf::from("/w"))));
        assert_eq!(user_prompt(&json!({"user_prompt": "hi", "delivery": "steer"})).1, Delivery::Steer);
        assert_eq!(user_prompt(&json!({"user_prompt": "hi"})).1, Delivery::Turn);
    }
}

#[cfg(test)]
mod seen_tests {
    use harnix_contract::wire::HOOK_MATCHER_CHARS;

    use super::*;

    #[test]
    fn each_skill_says_its_tier_and_where_it_was_read() {
        let session = json!({
            "project_root": "/p",
            "skill_dirs": [{"path": "/state/ext/acme.notes/skills", "state": "ok", "skills": ["tour"], "shadowed": [{"skill": "x", "by": "user", "path": "/u"}]}],
        });
        let commands = json!({"commands": [
            {"name": "tour", "description": "a tour", "kind": "prompt", "source": "host"},
            {"name": "deploy", "description": "ship it", "kind": "prompt", "source": "project"},
            {"name": "lint", "description": "", "kind": "prompt", "source": "project"},
            {"name": "board", "description": "", "kind": "prompt", "source": "plugin"},
            {"name": "mine", "description": "", "kind": "prompt", "source": "user"},
            {"name": "clear", "description": "", "kind": "local", "source": "builtin"},
            {"name": "srv:ask", "description": "", "kind": "mcp-prompt", "source": "mcp"},
        ]});
        let status = json!({"trust": "untrusted", "requires_trust": ["skills/deploy", "hooks_config"]});
        let plugins = json!({"plugins": [
            {"name": "off", "enabled": false, "root": "/off", "skills": ["board"]},
            {"name": "boards", "enabled": true, "root": "/plug/boards", "skills": ["board"]},
        ]});
        let out = skills_of(&session, Some(&commands), Some(&status), Some(&plugins));
        assert!(out.listed);
        assert_eq!(out.trust.as_deref(), Some("untrusted"));
        let names: Vec<(&str, SkillTier)> = out.skills.iter().map(|s| (s.name.as_str(), s.tier)).collect();
        assert_eq!(
            names,
            [("tour", SkillTier::Space), ("deploy", SkillTier::Project), ("lint", SkillTier::Project), ("board", SkillTier::Plugin), ("mine", SkillTier::User)]
        );
        assert_eq!((out.skills[0].dir.as_deref(), out.skills[0].package.as_deref()), (Some("/state/ext/acme.notes/skills"), None));
        assert_eq!(out.skills[1].dir.as_deref(), Some("/p/.agents/skills"));
        assert!(out.skills[1].withheld, "a project skill the engine holds back until trust");
        assert!(!out.skills[2].withheld);
        assert_eq!((out.skills[3].plugin.as_deref(), out.skills[3].dir.as_deref()), (Some("boards"), Some("/plug/boards")));
        assert_eq!(out.dirs[0].shadowed, vec![ShadowedSkill { skill: "x".into(), by: "user".into() }]);
    }

    #[test]
    fn a_trusted_project_holds_nothing_back_and_a_session_on_disk_lists_nothing() {
        let session = json!({"project_root": "/p", "skill_dirs": [{"path": "/elsewhere"}]});
        let commands = json!({"commands": [{"name": "deploy", "kind": "prompt", "source": "project"}]});
        let status = json!({"trust": "trusted", "requires_trust": ["skills/deploy"]});
        assert!(!skills_of(&session, Some(&commands), Some(&status), None).skills[0].withheld);
        let on_disk = skills_of(&session, None, None, None);
        assert!(!on_disk.listed && on_disk.skills.is_empty());
        assert_eq!(on_disk.dirs[0].state, None);
    }

    fn tier(source: HookSource, hooks: Value) -> (HookSource, Option<Value>) {
        (source, Some(json!({"hooks_config": hooks})))
    }

    #[test]
    fn a_hook_is_shown_by_its_kind_and_matcher_and_never_by_what_it_runs() {
        let global = tier(
            HookSource::Global,
            json!({"PreToolUse": [
                {"type": "command", "command": "secret-cmd --token=abc", "if": "Bash(git push:*)", "on_error": "block"},
                {"type": "http", "url": "https://user:pw@hooks.example.com/x", "headers": {"authorization": "Bearer xyz"}},
                {"type": "prompt", "prompt": "a private prompt"},
                {"type": "agent", "if": "  "},
                {"type": "teleport"},
            ]}),
        );
        let (rows, unread) = hooks_of(&[global], None);
        assert!(unread.is_empty());
        let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
        assert_eq!(kinds, ["command", "http", "prompt", "agent", "other"]);
        assert_eq!(rows[0].label, "command · Bash(git push:*)");
        assert_eq!(rows[0].on_error.as_deref(), Some("block"));
        assert_eq!((rows[1].label.as_str(), rows[3].matcher.clone()), ("http", None));
        let text = serde_json::to_string(&rows).unwrap();
        for secret in ["secret-cmd", "abc", "user:pw", "hooks.example.com", "Bearer", "xyz", "private prompt"] {
            assert!(!text.contains(secret), "{secret} leaked: {text}");
        }
    }

    #[test]
    fn a_long_matcher_is_cut() {
        let long = "x".repeat(200);
        let (rows, _) = hooks_of(&[tier(HookSource::Global, json!({"Stop": [{"type": "command", "if": long}]}))], None);
        let matcher = rows[0].matcher.clone().unwrap();
        assert_eq!(matcher.chars().count(), HOOK_MATCHER_CHARS + 1);
        assert!(matcher.ends_with('…'));
    }

    #[test]
    fn the_highest_tier_with_an_event_replaces_the_lower_ones_and_an_untrusted_project_counts_for_nothing() {
        let tiers = [
            tier(HookSource::Global, json!({"PreToolUse": [{"type": "command"}], "Stop": [{"type": "command"}]})),
            tier(HookSource::Scene, json!({"PreToolUse": [{"type": "http"}]})),
            tier(HookSource::Project, json!({"Stop": [{"type": "prompt"}], "SessionEnd": [{"type": "command"}]})),
        ];
        let find = |rows: &[HookRow], event: &str, source: HookSource| rows.iter().find(|r| r.event == event && r.source == source).cloned().unwrap();

        let (rows, _) = hooks_of(&tiers, Some("untrusted"));
        let pre = find(&rows, "PreToolUse", HookSource::Global);
        assert_eq!((pre.in_force, pre.replaced_by), (false, Some(HookSource::Scene)));
        assert!(find(&rows, "PreToolUse", HookSource::Scene).in_force);
        // The project's `Stop` does not replace the global one while it is held back.
        let stop = find(&rows, "Stop", HookSource::Global);
        assert_eq!((stop.in_force, stop.replaced_by), (true, None));
        let held = find(&rows, "Stop", HookSource::Project);
        assert_eq!((held.in_force, held.withheld, held.replaced_by), (false, true, None));

        let (rows, _) = hooks_of(&tiers, Some("trusted"));
        let stop = find(&rows, "Stop", HookSource::Global);
        assert_eq!((stop.in_force, stop.replaced_by), (false, Some(HookSource::Project)));
        assert!(find(&rows, "Stop", HookSource::Project).in_force);
        assert!(find(&rows, "SessionEnd", HookSource::Project).in_force);
    }

    #[test]
    fn a_tier_not_read_is_named() {
        let (rows, unread) = hooks_of(&[(HookSource::Global, None), tier(HookSource::Scene, json!({}))], None);
        assert_eq!(unread, [HookSource::Global]);
        assert!(rows.is_empty());
    }
}
