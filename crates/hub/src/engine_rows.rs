//! The engine's methods that pass through the hub: one explicit row each, under the engine's own
//! name, with how far it is open and what the hub does around it.
//!
//! An allow-list, not a deny-list: the engine trusts whoever reaches its dispatch, and in this
//! process that is the hub. A name that is not a row here is not reachable at all.

use std::sync::Arc;

use harnix_contract::sync::lock;
use harnix_contract::{RpcError, Subject};
use harnix_kernel::{Access, Owner, Rpc};
use serde_json::{json, Value};

use crate::rows::{answer, HubCell, HubRow};
use crate::{without_instance, Hub, ANSWERERS_KEPT};

/// What the hub does once the engine has answered.
#[derive(Clone, Copy)]
enum After {
    Nothing,
    /// The answer carries the session's goal: what the hub tells its watchers.
    Goal,
    /// What the session list shows changed, and the engine sends no event for it.
    Relisted,
}

/// Passed through for any device.
const DEVICE: &[&str] = &[
    "session.list",
    "session.get",
    "session.history",
    "session.entries",
    "session.resume",
    "scene.list",
    "scene.describe",
    "scene.activate",
    "commands.list",
    "mcp.status",
    "plugin.list",
    "daemon.status",
    "daemon.ping",
    "daemon.doctor",
];

/// The kernel's, and a module's by grant — what a built-in calls on a person's behalf and no
/// device calls directly: looking into a session and changing its context (a device reaches
/// those through the system space's own methods, which say what they are doing, concept §5.5),
/// reading settings, managing packages. Reaching Agix's own `plugin.install` and friends is not
/// opened to devices, because changing what is installed changes what this host serves, and
/// going around the host would leave a disabled package's interface module still being served
/// with nothing to notice it.
const BY_GRANT: &[(&str, After)] = &[
    ("session.tasks", After::Nothing),
    ("session.taskOutput", After::Nothing),
    ("session.inspectContext", After::Nothing),
    ("session.stopTask", After::Nothing),
    ("session.messageTask", After::Nothing),
    ("session.dropMessages", After::Relisted),
    ("session.editMessage", After::Relisted),
    ("session.compact", After::Relisted),
    ("session.clear", After::Relisted),
    ("session.rename", After::Relisted),
    ("session.setModel", After::Nothing),
    ("session.goal", After::Goal),
    ("session.setGoal", After::Goal),
    ("session.updateGoal", After::Goal),
    ("session.export", After::Nothing),
    ("config.get", After::Nothing),
    ("config.getProvider", After::Nothing),
    ("config.reload", After::Nothing),
    ("plugin.install", After::Nothing),
    ("plugin.uninstall", After::Nothing),
    ("plugin.enable", After::Nothing),
    ("plugin.disable", After::Nothing),
    ("plugin.reload", After::Nothing),
];

/// The engine methods that change credentials, repoint model endpoints, or hand the caller the
/// process itself. Refused to devices always, with the reason written out — the engine trusts
/// whoever reaches its dispatch, and in this process that is the hub (concept §3.4). A module may
/// be granted one by name in its manifest; the grant is on the diagnostics page.
///
/// `plugin.*` is deliberately **not** in this list. Installing a plugin runs code, but one
/// process serves one user and a device is that person: the question this layer really answers
/// is "may installed code do this on their behalf", not "may this person install something". The
/// enforcement that matters is the engine's — its capability gate, its sandbox, and the
/// disclosure it returns from the install itself.
///
/// `mcp.addServer` stays refused, and the difference is the point: it configures a
/// subprocess-spawning tool with no manifest, no capability declaration and no disclosure at all.
pub const ENGINE_REFUSALS: &[(&str, &str)] = &[
    (
        "config.setProvider",
        "changing provider credentials or base_url would point model traffic elsewhere",
    ),
    (
        "config.set",
        "changing provider credentials or base_url would point model traffic elsewhere",
    ),
    (
        "config.update",
        "changing provider credentials or base_url would point model traffic elsewhere",
    ),
    (
        "mcp.addServer",
        "adding an MCP server is equivalent to installing a tool that runs subprocesses",
    ),
    ("import.run", "config import runs outside the disclosure this layer is for"),
    ("import.list", "config import runs outside the disclosure this layer is for"),
];

/// What the hub owns. Refused to everyone, the kernel and every module included, and never
/// grantable: each is a way around the hub, and the hub being the only holder of sessions is the
/// invariant everything else rests on.
pub const HUB_OWNED: &[(&str, &str)] = &[
    (
        "daemon.shutdown",
        "the process is stopped by whoever started it, not over its own protocol",
    ),
    (
        "session.run_turn",
        "use harnix.send — the hub owns turns, so closing a tab cannot lose one",
    ),
    (
        "session.subscribe",
        "use harnix.attach — the hub subscribes before any client does",
    ),
    ("session.unsubscribe", "use harnix.detach — the hub owns subscriptions"),
    ("daemon.subscribeEvents", "host events are the hub's to fan out"),
];

fn engine(hub: &HubCell, name: &'static str, access: Access, after: After) -> HubRow {
    HubRow::Method {
        name,
        owner: Owner::Engine,
        access,
        handler: Some(answer(hub, name, move |h, _, p| async move { h.pass_through(name, p, after).await })),
    }
}

pub(crate) fn rows(hub: &HubCell) -> Vec<HubRow> {
    let mut rows = vec![
        // Passed through with the hub's semantics around them: where a new session lands and who
        // started it; who answered a permission ask.
        HubRow::Method {
            name: "session.create",
            owner: Owner::Engine,
            access: Access::device(),
            handler: Some(answer(hub, "session.create", |h, cx, p| async move { h.create(&cx.subject, p).await })),
        },
        HubRow::Method {
            name: "session.fork",
            owner: Owner::Engine,
            access: Access::device(),
            handler: Some(answer(hub, "session.fork", |h, cx, p| async move { h.fork(&cx.subject, p).await })),
        },
        HubRow::Method {
            name: "session.respondToPrompt",
            owner: Owner::Engine,
            access: Access::device(),
            handler: Some(answer(hub, "session.respondToPrompt", |h, cx, p| async move { h.respond(&cx.subject, p).await })),
        },
    ];
    rows.extend(DEVICE.iter().map(|name| engine(hub, name, Access::device(), After::Nothing)));
    rows.extend(BY_GRANT.iter().map(|(name, after)| engine(hub, name, Access::kernel(), *after)));
    rows.extend(ENGINE_REFUSALS.iter().map(|(name, why)| engine(hub, name, Access::refused(why), After::Nothing)));
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
    async fn pass_through(self: &Arc<Self>, method: &str, params: Value, after: After) -> Rpc<Value> {
        let instance = self.instance_of(&params)?;
        let session = params.get("session_id").and_then(Value::as_str).map(str::to_string);
        let answer = self.call(&instance, method, without_instance(params)).await?;
        match after {
            After::Nothing => {}
            After::Goal => {
                if let (Some(goal), Some(session)) = (answer.get("goal"), session) {
                    self.goal_heard(&(instance.clone(), session), goal.clone()).await;
                }
            }
            After::Relisted => self.refresh("harnix.instances.sessions", json!({"instance": instance})),
        }
        Ok(answer)
    }

    /// `session.fork`: the new session lands in the space the call claims, or its parent's, and
    /// is the caller's.
    async fn fork(self: &Arc<Self>, subject: &Subject, params: Value) -> Rpc<Value> {
        let key = self.key_of(&params)?;
        let space = params
            .get("space")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| self.space_of(&key));
        let forked = self.call(&key.0, "session.fork", without_instance(params)).await?;
        if let Some(sid) = forked.get("session_id").and_then(Value::as_str) {
            let new = (key.0.clone(), sid.to_string());
            self.bind_space(&new, &space);
            self.note_starter(&new, subject);
        }
        self.refresh("harnix.instances.sessions", json!({"instance": key.0}));
        Ok(forked)
    }

    /// `session.respondToPrompt`: the engine does not take who answered, so the hub keeps it for
    /// the tool's audit (`harnix.prompts.answerer`), and tells the session's watchers the ask is
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
        let answer = self
            .call(&instance, "session.respondToPrompt", without_instance(params))
            .await?;
        if let Some((session, prompt_id)) = prompt {
            self.answered(&(instance.clone(), session), |p| {
                p.get("prompt_id").and_then(Value::as_str) == Some(prompt_id.as_str())
            })
            .await;
        }
        Ok(answer)
    }
}
