//! The system space: looking into and acting on any instance's sessions, stopping, restarting and
//! building any instance, and this process's figures.
//!
//! Built from the extension interface alone. Every method is one call made as this extension, with
//! the capabilities the product granted it, through the same admission point a device goes
//! through — decided by the route table, on the audit trail as a child of the device's call, never
//! a private channel. What the engine does not offer is not offered here either. A product without
//! it (not linked, or `[extensions] off = ["system"]`) has no space reaching across spaces, and
//! nothing else changes.

use std::time::Duration;

use harnix_ext::{
    catalog, caps, Access, Call, Extension, ExtensionContext, Manifest, Registrar, RowScope, Rpc, RpcError, SpaceDecl,
    TopicCompute,
};
use serde_json::{json, Value};

/// The extension's id: its methods are `system.*`.
pub const ID: &str = "system";

/// Its space: a device's call claiming it may watch and act on a session of any space.
pub const SPACE: &str = "system";

/// How often `system.process` is computed again while watched: memory, threads and connections
/// change without an event to say so.
const PROCESS_EVERY: Duration = Duration::from_secs(5);

/// What each session method asks the engine, and which parameters it passes on besides the
/// instance and the session.
const ENGINE: &[(&str, &str, &[&str])] = &[
    ("tasks.stop", "engine.session.stopTask", &["task_id"]),
    ("tasks.message", "engine.session.messageTask", &["task_id", "message"]),
    ("tasks.output", "engine.session.taskOutput", &["task_id", "tail_bytes", "from_byte"]),
    ("messages.drop", "engine.session.dropMessages", &["at", "total"]),
    ("messages.edit", "engine.session.editMessage", &["at", "text", "total"]),
    ("compact", "engine.session.compact", &[]),
    ("clear", "engine.session.clear", &[]),
    ("model", "engine.session.setModel", &["model", "provider"]),
    ("rename", "engine.session.rename", &["name"]),
    ("goal", "engine.session.goal", &[]),
    ("goal.set", "engine.session.setGoal", &["objective", "max_rounds"]),
    ("goal.update", "engine.session.updateGoal", &["action", "objective", "max_rounds"]),
];

/// What each instance method asks the instance registry, params as given. The panel lists every
/// template, where a device sees only the interface's.
const INSTANCES: &[(&str, &str)] = &[
    ("instances.stop", "harnix.instances.stop"),
    ("instances.restart", "harnix.instances.restart"),
    ("instances.create", "harnix.instances.create"),
    ("instances.templates", "harnix.instances.templates"),
];

/// The system space's backend.
#[derive(Debug, Default)]
pub struct System;

#[async_trait::async_trait]
impl Extension for System {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new(ID, env!("CARGO_PKG_VERSION"))
            .title("System")
            .disclosure(
                "系统 space：看任何实例的会话与任务、读上下文、任务的输出；给在跑的子 Agent 发话；设定、暂停、继续、\
                 编辑、清除会话的目标；压缩、清空、换模型、改名、改写或删除消息、停止任务；停、重起、建任何实例；\
                 读本进程的版本、内存与连接。每次操作写审计",
            )
            .requires([caps::INSTANCES_ADMIN, caps::ENGINE_INSPECT, caps::ENGINE_CONTROL, caps::PROCESS_OBSERVE])
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.space(SpaceDecl::new(SPACE).cross_space());
        // A person's, not an extension's: a package never borrows these powers, as itself or
        // through its frames.
        let session = || Access::kernel().with_device().scoped(RowScope::Session);
        for &(name, method, keys) in ENGINE {
            r.method(name).access(session()).raw(move |call, params| async move {
                call.call(method, engine_params(&params, keys)?).await
            });
        }
        r.method("context").access(session()).raw(inspect_context);
        for &(name, method) in INSTANCES {
            r.method(name)
                .access(Access::kernel().with_device())
                .raw(move |call, params| async move { call.call(method, params).await });
        }
        r.method("process")
            .access(Access::kernel().with_device())
            .describe("this process's versions, uptime, memory, threads and who is connected")
            .raw(|call: Call, _| async move { Ok(call.ext().process()?.to_json()) });
        r.topic("process").computed(Some(PROCESS_EVERY), Process);
        // Its UI package defines the console's tabs and the inspector's pages for others to fill.
        for point in catalog::points_of(ID) {
            r.ui().point(point.decl());
        }
    }
}

/// `system.process`, the topic: the same value as the method.
struct Process;

#[async_trait::async_trait]
impl TopicCompute for Process {
    async fn compute(&self, ext: &ExtensionContext, _params: &Value) -> Rpc<Value> {
        Ok(ext.process()?.to_json())
    }
}

/// The session a call names — its instance when it names one — and the parameters `keys` passes
/// on, claiming this space: an instance built for another space is used only by a call reaching
/// across spaces.
fn engine_params(params: &Value, keys: &[&str]) -> Rpc<Value> {
    let mut out = target(params)?;
    for key in keys {
        if let Some(value) = params.get(*key) {
            out[*key] = value.clone();
        }
    }
    Ok(out)
}

fn target(params: &Value) -> Rpc<Value> {
    let session_id = params
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params("missing session_id"))?;
    let mut out = json!({"session_id": session_id, "space": SPACE});
    if let Some(instance) = params.get("instance").filter(|i| !i.is_null()) {
        out["instance"] = instance.clone();
    }
    Ok(out)
}

/// `system.context {instance?, session_id}`: the engine's `engine.session.inspectContext` — what the next
/// call will carry, per block, its token counts estimated — with `measured`: what the provider
/// counted for the last call that was made (`engine.session.get`'s `context`), absent before the first
/// one. The interface sizes the bar by the measured figure and only divides it by the estimates,
/// so an estimate never passes for a count.
async fn inspect_context(call: Call, params: Value) -> Rpc<Value> {
    let target = target(&params)?;
    let mut context = call.call("engine.session.inspectContext", target.clone()).await?;
    let measured = call
        .call("engine.session.get", target)
        .await
        .ok()
        .and_then(|info| info.get("context").cloned())
        .filter(|c| c.get("tokens").and_then(Value::as_u64).is_some_and(|n| n > 0));
    if let (Some(measured), Some(obj)) = (measured, context.as_object_mut()) {
        obj.insert("measured".into(), measured);
    }
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session method passes on the session, the instance when named, and its own parameters —
    /// nothing else the caller sent — claiming the system space, whatever the caller claimed.
    #[test]
    fn a_session_method_passes_on_only_what_it_names() {
        let params = json!({"session_id": "S1", "instance": "side", "space": "acme", "task_id": "T", "extra": 1});
        assert_eq!(
            engine_params(&params, &["task_id"]).unwrap(),
            json!({"session_id": "S1", "instance": "side", "space": SPACE, "task_id": "T"})
        );
        assert_eq!(engine_params(&json!({"session_id": "S1"}), &[]).unwrap(), json!({"session_id": "S1", "space": SPACE}));
        assert!(engine_params(&json!({"instance": "side"}), &[]).is_err());
    }

    /// Every method has a name of its own.
    #[test]
    fn every_method_is_named_once() {
        let mut names: Vec<&str> = ENGINE.iter().map(|(n, _, _)| *n).chain(INSTANCES.iter().map(|(n, _)| *n)).collect();
        names.extend(["context", "process"]);
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), count);
    }
}
