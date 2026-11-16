//! The engine's methods that pass through the hub: one explicit row each, named `engine.` and the
//! engine's own name, with how far it is open and what the hub does around it.
//!
//! An allow-list, not a deny-list: the engine trusts whoever reaches its dispatch, and in this
//! process that is the hub. A name that is not a row here is not reachable at all.
//!
//! The hub hands each its own row's name; the engine's link takes the `engine.` off.

use std::sync::Arc;

use harnix_contract::sync::lock;
use harnix_contract::wire::{self, AskAnswered, RespondParams};
use harnix_contract::{codes, RpcError, Subject};
use harnix_engine_link::ops::SessionInfo;
use harnix_contract::s1::{caps, Capability, RowScope};
use harnix_kernel::{Access, CallContext, Owner, Rpc};
use serde_json::{json, Value};

use crate::rows::{answer, HubCell, HubRow};
use crate::{without_instance, Hub, ANSWERERS_KEPT};

/// What the hub does once the engine has answered.
#[derive(Clone, Copy, PartialEq, Eq)]
enum After {
    Nothing,
    /// The answer carries the session's goal: what the hub tells its watchers.
    Goal,
    /// What the session list shows changed, and the engine sends no event for it.
    Relisted,
    /// The answer is one session: held to what a session can say, from an extension's engine.
    OneSession,
    /// The answer lists sessions: each held so, from an extension's engine.
    Sessions,
    /// The answer lists the engine's scenes: the instance's scene layers are added to it.
    SceneList,
    /// The call is about one scene: a scene layer is asked of as its base, and answered as itself.
    OneScene,
    /// The answer lists the session's entries: the changes of turns a rewind took out are left out.
    Entries,
}

/// The engine's scene list, as the hub's row names it.
pub(crate) const SCENE_LIST: &str = "engine.scene.list";

/// Passed through for any device and any extension, with what each acts on, and whether it reaches the engine's
/// settings or plugins — which nobody reaches on an extension's engine: its carrier wrote them.
const DEVICE: &[(&str, RowScope, After, bool)] = &[
    ("engine.session.list", RowScope::Space, After::Sessions, false),
    ("engine.session.get", RowScope::Session, After::OneSession, false),
    ("engine.session.history", RowScope::Session, After::Nothing, false),
    ("engine.session.entries", RowScope::Session, After::Entries, false),
    ("engine.session.resume", RowScope::Session, After::Nothing, false),
    (SCENE_LIST, RowScope::Open, After::SceneList, false),
    ("engine.scene.describe", RowScope::Open, After::OneScene, false),
    ("engine.scene.activate", RowScope::Open, After::OneScene, false),
    ("engine.commands.list", RowScope::Open, After::Nothing, false),
    ("engine.mcp.status", RowScope::Open, After::Nothing, false),
    ("engine.plugin.list", RowScope::Open, After::Nothing, true),
    ("engine.daemon.status", RowScope::Open, After::Nothing, false),
    ("engine.daemon.ping", RowScope::Open, After::Nothing, false),
    ("engine.daemon.doctor", RowScope::Open, After::Nothing, false),
];

/// The capabilities over an engine's settings, plugins and MCP servers: what no row reaches on an
/// extension's engine.
const SETTINGS: &[Capability] = &[caps::ENGINE_CONFIG_READ, caps::ENGINE_CONFIG_WRITE, caps::ENGINE_PLUGINS, caps::ENGINE_MCP];

/// What no device calls directly, open to the kernel and an extension holding the capability:
/// looking into a session and changing its context (a device reaches those through the system
/// space's own methods, which say what they are doing), reading settings, managing packages.
/// Reaching Agix's own `engine.plugin.install` and friends is not opened to devices, because changing
/// what is installed changes what this host serves, and going around the host would leave a
/// disabled package's interface module still being served with nothing to notice it.
const BY_CAPABILITY: &[(&str, Capability, After)] = &[
    ("engine.session.tasks", caps::ENGINE_INSPECT, After::Nothing),
    ("engine.session.taskOutput", caps::ENGINE_INSPECT, After::Nothing),
    ("engine.session.inspectContext", caps::ENGINE_INSPECT, After::Nothing),
    ("engine.session.goal", caps::ENGINE_INSPECT, After::Goal),
    ("engine.session.stopTask", caps::ENGINE_CONTROL, After::Nothing),
    ("engine.session.messageTask", caps::ENGINE_CONTROL, After::Nothing),
    ("engine.session.dropMessages", caps::ENGINE_CONTROL, After::Relisted),
    ("engine.session.editMessage", caps::ENGINE_CONTROL, After::Relisted),
    ("engine.session.compact", caps::ENGINE_CONTROL, After::Relisted),
    ("engine.session.clear", caps::ENGINE_CONTROL, After::Relisted),
    ("engine.session.rename", caps::ENGINE_CONTROL, After::Relisted),
    ("engine.session.setModel", caps::ENGINE_CONTROL, After::Nothing),
    ("engine.session.setGoal", caps::ENGINE_CONTROL, After::Goal),
    ("engine.session.updateGoal", caps::ENGINE_CONTROL, After::Goal),
    ("engine.session.export", caps::ENGINE_EXPORT, After::Nothing),
    ("engine.config.get", caps::ENGINE_CONFIG_READ, After::Nothing),
    ("engine.config.getProvider", caps::ENGINE_CONFIG_READ, After::Nothing),
    ("engine.config.reload", caps::ENGINE_CONFIG_WRITE, After::Nothing),
    ("engine.plugin.install", caps::ENGINE_PLUGINS, After::Nothing),
    ("engine.plugin.uninstall", caps::ENGINE_PLUGINS, After::Nothing),
    ("engine.plugin.enable", caps::ENGINE_PLUGINS, After::Nothing),
    ("engine.plugin.disable", caps::ENGINE_PLUGINS, After::Nothing),
    ("engine.plugin.reload", caps::ENGINE_PLUGINS, After::Nothing),
];

/// A project's trust: whether its own hooks, MCP servers, looser settings, agent types and skills
/// are in force (`docs/harnix-app.md` §3). Read and written by an extension holding
/// `engine.control` — a device decides through the projects space's own methods, which say what
/// they are doing. Trust is kept in the engine's global settings, so an extension's engine refuses
/// it as it refuses its settings.
const PROJECT_TRUST: &[(&str, Capability)] = &[
    ("engine.project.status", caps::ENGINE_CONTROL),
    ("engine.project.trust", caps::ENGINE_CONTROL),
];

/// The engine's own schedule: jobs a model made with its cron tools, each a turn in its session,
/// in UTC (`docs/harnix-app.md` §9). Read by an extension holding `engine.inspect`, a job deleted by
/// one holding `engine.control`; a device sees and deletes them through the extension that lists
/// them beside its own schedules. Nobody makes one through here: making them is the model's.
const ENGINE_SCHEDULE: &[(&str, Capability)] = &[
    ("engine.schedule.list", caps::ENGINE_INSPECT),
    ("engine.schedule.delete", caps::ENGINE_CONTROL),
];

/// The engine methods that change credentials, repoint model endpoints, or hand the caller the
/// process itself. Refused to devices always, with the reason written out — the engine trusts
/// whoever reaches its dispatch, and in this process that is the hub (concept §3.4). The kernel may
/// call one, and an extension holding the capability beside it; `None` is the kernel alone.
///
/// `plugin.*` is deliberately **not** in this list. Installing a plugin runs code, but one
/// process serves one user and a device is that person: the question this layer really answers
/// is "may installed code do this on their behalf", not "may this person install something". The
/// enforcement that matters is the engine's — its capability gate, its sandbox, and the
/// disclosure it returns from the install itself.
///
/// `engine.mcp.addServer` stays refused, and the difference is the point: it configures a
/// subprocess-spawning tool with no manifest, no capability declaration and no disclosure at all.
/// A device adds one through the `settings` extension, which holds `engine.mcp` and says what it
/// does; the same goes for changing one and for signing in to one.
pub const ENGINE_REFUSALS: &[(&str, &str, Option<Capability>)] = &[
    (
        "engine.config.setProvider",
        "changing provider credentials or base_url would point model traffic elsewhere",
        Some(caps::ENGINE_CONFIG_WRITE),
    ),
    (
        "engine.config.set",
        "changing provider credentials or base_url would point model traffic elsewhere",
        Some(caps::ENGINE_CONFIG_WRITE),
    ),
    (
        "engine.config.update",
        "changing provider credentials or base_url would point model traffic elsewhere",
        Some(caps::ENGINE_CONFIG_WRITE),
    ),
    (
        "engine.mcp.addServer",
        "adding an MCP server is equivalent to installing a tool that runs subprocesses",
        Some(caps::ENGINE_MCP),
    ),
    (
        "engine.mcp.updateServer",
        "changing an MCP server is equivalent to installing a tool that runs subprocesses",
        Some(caps::ENGINE_MCP),
    ),
    ("engine.mcp.auth.start", MCP_SIGN_IN, Some(caps::ENGINE_MCP)),
    ("engine.mcp.auth.complete", MCP_SIGN_IN, Some(caps::ENGINE_MCP)),
    ("engine.mcp.auth.cancel", MCP_SIGN_IN, Some(caps::ENGINE_MCP)),
    ("engine.mcp.auth.logout", MCP_SIGN_IN, Some(caps::ENGINE_MCP)),
    ("engine.import.run", "config import runs outside the disclosure this layer is for", None),
    ("engine.import.list", "config import runs outside the disclosure this layer is for", None),
];

/// Why a device does not sign in to an MCP server on the engine directly.
const MCP_SIGN_IN: &str = "an MCP server's sign-in hands its tokens to the engine: it goes through the settings page, which says which server";

/// What the hub owns. Refused to everyone, the kernel and every module included, and never
/// grantable: each is a way around the hub, and the hub being the only holder of sessions is the
/// invariant everything else rests on.
pub const HUB_OWNED: &[(&str, &str)] = &[
    (
        "engine.daemon.shutdown",
        "the process is stopped by whoever started it, not over its own protocol",
    ),
    (
        "engine.session.run_turn",
        "use harnix.session.send — the hub owns turns, so closing a tab cannot lose one",
    ),
    (
        "engine.session.subscribe",
        "use harnix.session.attach — the hub subscribes before any client does",
    ),
    ("engine.session.unsubscribe", "use harnix.session.detach — the hub owns subscriptions"),
    ("engine.daemon.subscribeEvents", "host events are the hub's to fan out"),
    (
        "engine.session.setPrompt",
        "use harnix.session.setPrompt — the hub keeps the scene's text in front of yours",
    ),
];

/// How one engine row is passed through.
#[derive(Clone, Copy)]
struct Through {
    after: After,
    /// About one session a device names: the space a call claims is checked on it.
    checked: bool,
    /// It reaches the engine's settings or plugins.
    settings: bool,
}

fn engine(hub: &HubCell, name: &'static str, access: Access, through: Through) -> HubRow {
    HubRow::Method {
        name,
        owner: Owner::Engine,
        access,
        handler: Some(answer(hub, name, through.checked, move |h, cx, p| async move { h.pass_through(&cx, name, p, through).await })),
    }
}

pub(crate) fn rows(hub: &HubCell) -> Vec<HubRow> {
    let mut rows = vec![
        // The engine's, with the hub's semantics around them, under the hub's own names: where a
        // new session lands and who started it; who answered a permission ask.
        HubRow::Method {
            name: "harnix.session.create",
            owner: Owner::Kernel(harnix_kernel::KernelPart::Hub),
            access: Access::device().with_extensions().scoped(RowScope::Space),
            handler: Some(answer(hub, "harnix.session.create", false, |h, cx, p| async move { h.create(&cx, p).await })),
        },
        HubRow::Method {
            name: "harnix.session.fork",
            owner: Owner::Kernel(harnix_kernel::KernelPart::Hub),
            access: Access::device().with_extensions().scoped(RowScope::Session),
            handler: Some(answer(hub, "harnix.session.fork", true, |h, cx, p| async move { h.fork(&cx.subject, p).await })),
        },
        // A session bundle installed as new sessions in the space the call claims. An extension
        // holding `engine.export` acts for a device: the bundle's path is what the extension
        // checked, not what a device said.
        HubRow::Method {
            name: "harnix.session.import",
            owner: Owner::Kernel(harnix_kernel::KernelPart::Hub),
            access: Access::capability(caps::ENGINE_EXPORT).scoped(RowScope::Space),
            handler: Some(answer(hub, "harnix.session.import", false, |h, cx, p| async move { h.import(&cx, p).await })),
        },
        // A session's own prompt text, behind its scene layer's: changed as its context is, by
        // whoever holds `engine.control` (a device through the system space).
        HubRow::Method {
            name: "harnix.session.setPrompt",
            owner: Owner::Kernel(harnix_kernel::KernelPart::Hub),
            access: Access::capability(caps::ENGINE_CONTROL).scoped(RowScope::Session),
            handler: Some(answer(hub, "harnix.session.setPrompt", true, |h, _, p| async move { h.set_prompt(p).await })),
        },
        // Files, conversation or both back to an earlier point: a person decides it about a session
        // of the space they are in; an extension acts for one holding `engine.control`.
        HubRow::Method {
            name: "harnix.session.rewind",
            owner: Owner::Kernel(harnix_kernel::KernelPart::Hub),
            access: Access::device_or_capability(caps::ENGINE_CONTROL).scoped(RowScope::Session),
            handler: Some(answer(hub, "harnix.session.rewind", true, |h, cx, p| async move { h.rewind(&cx, p).await })),
        },
        HubRow::Method {
            name: "harnix.session.respond",
            owner: Owner::Kernel(harnix_kernel::KernelPart::Hub),
            access: Access::device().with_extensions().scoped(RowScope::Session),
            handler: Some(answer(hub, "harnix.session.respond", true, |h, cx, p| async move { h.respond(&cx.subject, p).await })),
        },
    ];
    rows.extend(DEVICE.iter().map(|(name, scope, after, settings)| {
        let through = Through { after: *after, checked: *scope == RowScope::Session, settings: *settings };
        engine(hub, name, Access::device().with_extensions().scoped(*scope), through)
    }));
    rows.extend(BY_CAPABILITY.iter().map(|(name, cap, after)| {
        // A session's inside, read, changed or exported: one session each.
        let on_a_session = [caps::ENGINE_INSPECT, caps::ENGINE_CONTROL, caps::ENGINE_EXPORT].contains(cap);
        let scope = if on_a_session { RowScope::Session } else { RowScope::Open };
        let through = Through { after: *after, checked: false, settings: SETTINGS.contains(cap) };
        engine(hub, name, Access::capability(cap.clone()).scoped(scope), through)
    }));
    rows.extend(PROJECT_TRUST.iter().map(|(name, cap)| {
        let through = Through { after: After::Nothing, checked: false, settings: true };
        engine(hub, name, Access::capability(cap.clone()), through)
    }));
    rows.extend(ENGINE_SCHEDULE.iter().map(|(name, cap)| {
        let through = Through { after: After::Nothing, checked: false, settings: false };
        engine(hub, name, Access::capability(cap.clone()), through)
    }));
    rows.extend(ENGINE_REFUSALS.iter().map(|(name, why, cap)| {
        let access = match cap {
            Some(cap) => Access::capability(cap.clone()).refusing(why),
            None => Access::refused(why),
        };
        let settings = cap.as_ref().is_some_and(|cap| SETTINGS.contains(cap));
        engine(hub, name, access, Through { after: After::Nothing, checked: false, settings })
    }));
    rows.extend(HUB_OWNED.iter().map(|(name, why)| HubRow::Method {
        name,
        owner: Owner::Kernel(harnix_kernel::KernelPart::Hub),
        access: Access::hub_owned(why),
        handler: None,
    }));
    rows
}

impl Hub {
    /// One engine method into the instance the call names (or the main instance), with the
    /// instance and space names taken off: the engine has no notion of either.
    async fn pass_through(self: &Arc<Self>, cx: &CallContext, method: &str, params: Value, through: Through) -> Rpc<Value> {
        crate::refuse_hub_owned_options(&params)?;
        let instance = self.instance_of(&params)?;
        let linked = self.engine(&instance)?;
        let extension = linked.is_extension();
        if extension && through.settings {
            return Err(RpcError::new(
                codes::REFUSED,
                format!("instance `{instance}` runs an extension's engine: its settings and plugins are its carrier's, and nobody's to change"),
            ));
        }
        // An instance that is some spaces' is theirs and the system space's, whatever the row —
        // one open to any space included: before anything reaches it, or wakes it.
        if let Some(slot) = self.slot(&instance) {
            let (space, crosses) = self.claim_of(&params);
            crate::spaces::may_use(&slot, cx, &space, crosses)?;
        }
        let session = params.get("session_id").and_then(Value::as_str).map(str::to_string);
        // A caller that claimed a space is told of that space's sessions only: the engine lists
        // every one, whoever's it is.
        let held = (through.after == After::Sessions).then(|| self.held_space(&params)).flatten();
        let layers = linked.link.scene_layers();
        let (params, layer) = match through.after {
            After::OneScene => crate::scenes::about_one(&layers, params),
            _ => (params, None),
        };
        let mut answer = linked.ops().passthrough(method, without_instance(params)).await?;
        if extension {
            harden(&mut answer, through.after);
        }
        match (through.after, &layer) {
            (After::OneScene, Some(layer)) => answer = crate::scenes::one_described(layer, answer),
            (After::SceneList, _) => answer = crate::scenes::listed(&layers, answer),
            (After::Entries, _) => crate::changes::drop_rewound(&mut answer),
            (After::OneSession, _) => {
                if let Some(sid) = answer.get("session_id").and_then(Value::as_str).map(str::to_string) {
                    lock(&self.spaces).decorate(&(instance.clone(), sid), &mut answer);
                }
            }
            _ => {}
        }
        if let (Some(space), Some(Value::Array(rows))) = (held, answer.get_mut("sessions")) {
            self.home_of(&instance);
            rows.retain(|row| {
                let sid = row.get("session_id").and_then(Value::as_str).unwrap_or_default();
                self.space_known(&(instance.clone(), sid.to_string())) == space
            });
        }
        if let (After::Sessions, Some(Value::Array(rows))) = (through.after, answer.get_mut("sessions")) {
            let spaces = lock(&self.spaces);
            for row in rows {
                if let Some(sid) = row.get("session_id").and_then(Value::as_str).map(str::to_string) {
                    spaces.decorate(&(instance.clone(), sid), row);
                }
            }
        }
        match through.after {
            After::Nothing | After::OneSession | After::Sessions | After::SceneList | After::OneScene | After::Entries => {}
            After::Goal => {
                if let (Some(goal), Some(session)) = (answer.get("goal"), session) {
                    self.goal_heard(&(instance.clone(), session), goal.clone()).await;
                }
            }
            After::Relisted => self.refresh("harnix.instances.sessions", json!({"instance": instance})),
        }
        Ok(answer)
    }

    /// `harnix.session.import {path, space?, project_root?}`: the bundle at `path` becomes new
    /// sessions, the root and its sub-agents', all in the space the call claims, started by a
    /// person (not by Harnix). `project_root` is passed on only when given: left out, the engine
    /// uses the bundle's own, and a bundle whose project is not on this machine is imported with
    /// no project — what the engine cannot honestly place is not placed in a guess.
    pub(crate) async fn import(self: &Arc<Self>, cx: &CallContext, mut params: Value) -> Rpc<Value> {
        crate::refuse_hub_owned_options(&params)?;
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid_params("missing path"))?
            .to_string();
        let space = self.place(cx, &mut params).await?;
        let instance = self.instance_of(&params)?;
        let linked = self.engine(&instance)?;
        let mut ask = json!({"path": path});
        if let Some(root) = params.get("project_root") {
            ask["project_root"] = root.clone();
        }
        let mut warnings = Vec::new();
        let mut answer = match linked.ops().passthrough("engine.session.import", ask.clone()).await {
            Err(e) if params.get("project_root").is_none() && e.code == codes::INVALID_PARAMS && e.message.contains("project_root") => {
                ask["project_root"] = Value::Null;
                warnings.push(json!("the bundle's project is not on this machine; imported with no project"));
                linked.ops().passthrough("engine.session.import", ask).await?
            }
            other => other?,
        };
        let ids: Vec<String> = answer
            .get("sessions")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(|r| r.get("session_id").and_then(Value::as_str).map(str::to_string)).collect())
            .unwrap_or_default();
        if !space.is_empty() {
            for sid in &ids {
                self.bind_space(&(instance.clone(), sid.clone()), &space);
            }
        }
        if let Some(obj) = answer.as_object_mut() {
            if !warnings.is_empty() {
                let all = obj.entry("warnings").or_insert_with(|| json!([]));
                if let Some(all) = all.as_array_mut() {
                    all.extend(warnings);
                }
            }
            obj.insert("instance".into(), json!(instance));
            obj.insert("space".into(), json!(space));
        }
        self.refresh("harnix.instances.sessions", json!({"instance": instance}));
        Ok(answer)
    }

    /// `harnix.session.fork`: the new session lands in the space the call claims, or its parent's, and
    /// is the caller's.
    async fn fork(self: &Arc<Self>, subject: &Subject, params: Value) -> Rpc<Value> {
        crate::refuse_hub_owned_options(&params)?;
        let key = self.key_of(&params)?;
        let space = params
            .get("space")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| self.space_of(&key));
        let forked = self.engine(&key.0)?.ops().fork(without_instance(params)).await?;
        let mut value = forked.value;
        if let Some(sid) = &forked.session_id {
            let new = (key.0.clone(), sid.clone());
            self.bind_space(&new, &space);
            self.note_starter(&new, subject);
            // The engine copies the session's options and prompt: its layer and its caller's
            // text are the fork's too.
            let mut spaces = lock(&self.spaces);
            spaces.copy_session(&key, &new);
            spaces.decorate(&new, &mut value);
        }
        self.refresh("harnix.instances.sessions", json!({"instance": key.0}));
        Ok(value)
    }

    /// `harnix.session.respond`: the answer is checked against the kind of ask it is for. The
    /// engine does not take who answered, so the hub keeps it for the tool's audit
    /// (`harnix.session.answerer`), tells the session's watchers the ask is answered, and — for the
    /// first answer — what the answer was.
    async fn respond(self: &Arc<Self>, subject: &Subject, mut params: Value) -> Result<Value, RpcError> {
        let given: RespondParams =
            serde_json::from_value(params.clone()).map_err(|e| RpcError::invalid_params(format!("harnix.session.respond: {e}")))?;
        let kind = given.check()?;
        let tool_use = params.as_object_mut().and_then(|o| o.remove("tool_use_id"));
        if let (Subject::Device { id }, Some(Value::String(tool_use))) = (subject, tool_use) {
            let mut kept = lock(&self.answerers);
            if kept.len() == ANSWERERS_KEPT {
                kept.pop_front();
            }
            kept.push_back(((given.session_id.clone(), tool_use), format!("device:{id}")));
        }
        let instance = self.instance_of(&params)?;
        let engine = self.engine(&instance)?;
        let key = (instance.clone(), given.session_id.clone());
        let first = crate::session::claim_ask(&self.inner, &key, &given.prompt_id).await;
        let answer = engine.ops().respond(without_instance(params)).await;
        if first {
            crate::session::release_ask(&self.inner, &key, &given.prompt_id).await;
        }
        let answer = answer?;
        if first {
            let by = match subject {
                Subject::Extension { id } => Some(id.to_string()),
                _ => None,
            };
            let told = wire::params(&AskAnswered::new(&given, kind, by));
            crate::session::tell_answered(&self.inner, &key, told, self.is_extension(&instance)).await;
        }
        self.answered(&key, |p| p.prompt_id.as_deref() == Some(given.prompt_id.as_str())).await;
        Ok(answer)
    }
}

/// What an extension's engine said of a session, or of several, held to what a session can say.
fn harden(answer: &mut Value, after: After) {
    let one = |value: &mut Value| {
        let mut info = SessionInfo::new(value.take());
        info.harden();
        *value = info.into_value();
    };
    match after {
        After::OneSession => one(answer),
        After::Sessions => {
            if let Some(Value::Array(rows)) = answer.get_mut("sessions") {
                rows.iter_mut().for_each(one);
            }
        }
        _ => {}
    }
}
