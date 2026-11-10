//! The engine's methods that pass through the hub: one explicit row each, named `engine.` and the
//! engine's own name, with how far it is open and what the hub does around it.
//!
//! An allow-list, not a deny-list: the engine trusts whoever reaches its dispatch, and in this
//! process that is the hub. A name that is not a row here is not reachable at all.
//!
//! The hub hands each its own row's name; the engine's link takes the `engine.` off.

use std::sync::Arc;

use harnix_contract::sync::lock;
use harnix_contract::{codes, RpcError, Subject};
use harnix_engine_api::ops::SessionInfo;
use harnix_contract::s1::{caps, Capability, RowScope};
use harnix_kernel::{Access, Owner, Rpc};
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
}

/// Passed through for any device, with what each acts on, and whether it reaches the engine's
/// settings or plugins — which nobody reaches on an extension's engine: its carrier wrote them.
const DEVICE: &[(&str, RowScope, After, bool)] = &[
    ("engine.session.list", RowScope::Open, After::Sessions, false),
    ("engine.session.get", RowScope::Session, After::OneSession, false),
    ("engine.session.history", RowScope::Session, After::Nothing, false),
    ("engine.session.entries", RowScope::Session, After::Nothing, false),
    ("engine.session.resume", RowScope::Session, After::Nothing, false),
    ("engine.scene.list", RowScope::Open, After::Nothing, false),
    ("engine.scene.describe", RowScope::Open, After::Nothing, false),
    ("engine.scene.activate", RowScope::Open, After::Nothing, false),
    ("engine.commands.list", RowScope::Open, After::Nothing, false),
    ("engine.mcp.status", RowScope::Open, After::Nothing, false),
    ("engine.plugin.list", RowScope::Open, After::Nothing, true),
    ("engine.daemon.status", RowScope::Open, After::Nothing, false),
    ("engine.daemon.ping", RowScope::Open, After::Nothing, false),
    ("engine.daemon.doctor", RowScope::Open, After::Nothing, false),
];

/// The capabilities over an engine's settings and plugins: what no row reaches on an extension's
/// engine.
const SETTINGS: &[Capability] = &[caps::ENGINE_CONFIG_READ, caps::ENGINE_CONFIG_WRITE, caps::ENGINE_PLUGINS];

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
    ("engine.import.run", "config import runs outside the disclosure this layer is for", None),
    ("engine.import.list", "config import runs outside the disclosure this layer is for", None),
];

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
        handler: Some(answer(hub, name, through.checked, move |h, _, p| async move { h.pass_through(name, p, through).await })),
    }
}

pub(crate) fn rows(hub: &HubCell) -> Vec<HubRow> {
    let mut rows = vec![
        // The engine's, with the hub's semantics around them, under the hub's own names: where a
        // new session lands and who started it; who answered a permission ask.
        HubRow::Method {
            name: "harnix.session.create",
            owner: Owner::Kernel(harnix_kernel::KernelPart::Hub),
            access: Access::device().scoped(RowScope::Space),
            handler: Some(answer(hub, "harnix.session.create", false, |h, cx, p| async move { h.create(&cx.subject, p).await })),
        },
        HubRow::Method {
            name: "harnix.session.fork",
            owner: Owner::Kernel(harnix_kernel::KernelPart::Hub),
            access: Access::device().scoped(RowScope::Session),
            handler: Some(answer(hub, "harnix.session.fork", true, |h, cx, p| async move { h.fork(&cx.subject, p).await })),
        },
        HubRow::Method {
            name: "harnix.session.respond",
            owner: Owner::Kernel(harnix_kernel::KernelPart::Hub),
            access: Access::device().scoped(RowScope::Session),
            handler: Some(answer(hub, "harnix.session.respond", true, |h, cx, p| async move { h.respond(&cx.subject, p).await })),
        },
    ];
    rows.extend(DEVICE.iter().map(|(name, scope, after, settings)| {
        let through = Through { after: *after, checked: *scope == RowScope::Session, settings: *settings };
        engine(hub, name, Access::device().scoped(*scope), through)
    }));
    rows.extend(BY_CAPABILITY.iter().map(|(name, cap, after)| {
        // A session's inside, read, changed or exported: one session each.
        let on_a_session = [caps::ENGINE_INSPECT, caps::ENGINE_CONTROL, caps::ENGINE_EXPORT].contains(cap);
        let scope = if on_a_session { RowScope::Session } else { RowScope::Open };
        let through = Through { after: *after, checked: false, settings: SETTINGS.contains(cap) };
        engine(hub, name, Access::capability(cap.clone()).scoped(scope), through)
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
    /// One engine method into the instance the call names (or the system instance), with the
    /// instance and space names taken off: the engine has no notion of either.
    async fn pass_through(self: &Arc<Self>, method: &str, params: Value, through: Through) -> Rpc<Value> {
        let instance = self.instance_of(&params)?;
        let linked = self.engine(&instance)?;
        let extension = linked.is_extension();
        if extension && through.settings {
            return Err(RpcError::new(
                codes::REFUSED,
                format!("instance `{instance}` runs an extension's engine: its settings and plugins are its carrier's, and nobody's to change"),
            ));
        }
        let session = params.get("session_id").and_then(Value::as_str).map(str::to_string);
        let mut answer = linked.ops().passthrough(method, without_instance(params)).await?;
        if extension {
            harden(&mut answer, through.after);
        }
        match through.after {
            After::Nothing | After::OneSession | After::Sessions => {}
            After::Goal => {
                if let (Some(goal), Some(session)) = (answer.get("goal"), session) {
                    self.goal_heard(&(instance.clone(), session), goal.clone()).await;
                }
            }
            After::Relisted => self.refresh("harnix.instances.sessions", json!({"instance": instance})),
        }
        Ok(answer)
    }

    /// `harnix.session.fork`: the new session lands in the space the call claims, or its parent's, and
    /// is the caller's.
    async fn fork(self: &Arc<Self>, subject: &Subject, params: Value) -> Rpc<Value> {
        let key = self.key_of(&params)?;
        let space = params
            .get("space")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| self.space_of(&key));
        let forked = self.engine(&key.0)?.ops().fork(without_instance(params)).await?;
        if let Some(sid) = &forked.session_id {
            let new = (key.0.clone(), sid.clone());
            self.bind_space(&new, &space);
            self.note_starter(&new, subject);
        }
        self.refresh("harnix.instances.sessions", json!({"instance": key.0}));
        Ok(forked.value)
    }

    /// `harnix.session.respond`: the engine does not take who answered, so the hub keeps it for
    /// the tool's audit (`harnix.session.answerer`), and tells the session's watchers the ask is
    /// answered.
    async fn respond(self: &Arc<Self>, subject: &Subject, mut params: Value) -> Result<Value, RpcError> {
        let tool_use = params.as_object_mut().and_then(|o| o.remove("tool_use_id"));
        if let (Subject::Device { id }, Some(Value::String(tool_use)), Some(session)) =
            (subject, tool_use, params.get("session_id").and_then(Value::as_str))
        {
            let mut kept = lock(&self.answerers);
            if kept.len() == ANSWERERS_KEPT {
                kept.pop_front();
            }
            kept.push_back(((session.to_string(), tool_use), format!("device:{id}")));
        }
        let instance = self.instance_of(&params)?;
        let field = |k: &str| params.get(k).and_then(Value::as_str).map(str::to_string);
        let prompt = field("session_id").zip(field("prompt_id"));
        let answer = self.engine(&instance)?.ops().respond(without_instance(params)).await?;
        if let Some((session, prompt_id)) = prompt {
            self.answered(&(instance.clone(), session), |p| p.prompt_id.as_deref() == Some(prompt_id.as_str()))
                .await;
        }
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
