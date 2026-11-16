//! `ProcTools`: the running packages' tools in every session of an instance this process builds
//! or spawns, each call carried over the package's extension wire.
//!
//! A tool provider (`harnix_ext::engine::ToolProvider`) the component gives every engine this
//! process builds or spawns (`engine_everywhere`). Its tools are offered on the instance's link to
//! the engine — sub-agents, team members and background tasks have them too — and offered again
//! each time a package's process comes or goes: one up now is in the next session built, one that
//! is not is not (a session already built keeps what it was built with until it is rebuilt). Per
//! package:
//!
//! - each tool its manifest declares, as `ext__<id>__<tool>`. A call goes to its process
//!   (`ctl.call_tool`), carrying the turn's chain and the package's call timeout as its deadline;
//!   it has no `/project` (the project is outside the process's sandbox); `proc.progress` goes to
//!   the tool's progress and to the component's `packages.progress` topic, and cancelling the
//!   turn stops it (`ctl.cancel`, and the wire's `$/cancel`). **Every call asks first**: a tool's
//!   `read-only` claim is the package's own word, so it schedules calls and decides nothing about
//!   permission.
//! - its `events` export, as a hook before each of the session's tool calls: offered
//!   `tool.before` with the tool's name and input — for its own tools' calls only, unless its
//!   manifest asks for every one (`[events] tools = "all"`, which the installer is told) and it
//!   was granted `events.all_tools`; none without the export. Decided here, before the call is
//!   sent ([`offered`]) — it may let the call through, refuse it, or add a note, which the model
//!   is handed as a reminder before its next step. A hook is asked before the engine checks the call or asks
//!   about it. A note is about one of its own tools only, and only so long ([`MAX_NOTE`]): a
//!   downloaded package does not get to write into what the model reads of every tool it calls. It
//!   cannot change the call. A hook that fails lets the call through: a package's fault is not the
//!   user's.

use std::sync::Arc;

use harnix_carrier_shared::tools::{cancellable, hook_decision, offered, permission, tool_output, tool_spec, EventScope};
use harnix_carrier_shared::Loaded;
use harnix_ext::engine::{
    HookDecision, Permission, Tool, ToolCallFacts, ToolCx, ToolError, ToolHook, ToolOutput, ToolProvider, ToolSpec,
    ToolsChanged,
};
use harnix_wasm_port::socket::now_ms;
use harnix_wasm_port::{CallMeta, HookCall, HookReply, ToolDef};
use serde_json::{json, Value};

use crate::registry::Registry;

pub use harnix_carrier_shared::tools::{MAX_NOTE, NAME};

/// The running packages' tools and hooks, for an instance's sessions: a provider of the
/// instance's engine.
pub struct ProcTools {
    registry: Arc<Registry>,
}

impl ProcTools {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    /// The tools a session is given now: what the engine registers.
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        for app in self.registry.loaded() {
            if !app.component {
                continue;
            }
            for def in &app.tools {
                let spec = tool_spec(&app, def);
                tools.push(Arc::new(AppTool {
                    spec,
                    def: def.clone(),
                    app: app.clone(),
                    registry: self.registry.clone(),
                }));
            }
        }
        tools
    }

    /// The hooks before every session's tool calls: each running package's `events`, when it is
    /// offered any.
    pub fn hooks(&self) -> Vec<Arc<dyn ToolHook>> {
        self.registry
            .loaded()
            .into_iter()
            .filter(|app| app.component && app.events != EventScope::None)
            .map(|app| {
                Arc::new(AppEvents { app, registry: self.registry.clone() }) as Arc<dyn ToolHook>
            })
            .collect()
    }
}

impl ToolProvider for ProcTools {
    fn name(&self) -> &str {
        NAME
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        ProcTools::tools(self)
    }

    fn hooks(&self) -> Vec<Arc<dyn ToolHook>> {
        ProcTools::hooks(self)
    }

    fn changes(&self) -> Vec<ToolsChanged> {
        vec![self.registry.changed()]
    }
}

/// One package's tool, as a session has it.
struct AppTool {
    spec: ToolSpec,
    def: ToolDef,
    app: Arc<Loaded>,
    registry: Arc<Registry>,
}

#[async_trait::async_trait]
impl Tool for AppTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn permission(&self, _: &Value) -> Permission {
        permission(&self.app, &self.def.name)
    }

    /// A tool wakes a process let go while idle.
    async fn call(&self, cx: ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
        let _held = self.registry.hold(&self.app.id);
        let Some(control) = self.registry.awake(&self.app.id).await else {
            return Err(ToolError::NotFound(format!("`{}` is not running", self.app.id)));
        };
        let call_id = cx.tool_use_id().to_string();
        // A tool call's `host.progress`: to the engine's progress channel, and to the host's
        // listeners (`Registry::progress`), which is what a client sees.
        {
            let registry = self.registry.clone();
            let progress = cx.progress().clone();
            let (app, tool, session_id, tool_use_id) = (
                self.app.id.clone(),
                self.spec.name().to_string(),
                cx.session_id().to_string(),
                call_id.clone(),
            );
            self.registry.listen(
                &call_id,
                Arc::new(move |text: &str| {
                    progress.send(text);
                    registry.progress(json!({
                        "app": app,
                        "tool": tool,
                        "session_id": session_id,
                        "tool_use_id": tool_use_id,
                        "text": text,
                    }));
                }),
            );
        }
        let call = harnix_wasm_port::ToolCall {
            app: self.app.id.clone(),
            tool: self.def.name.clone(),
            input,
            call_id: call_id.clone(),
            project: None,
            progress: true,
        };
        // On behalf of the session's turn: what the tool calls carries the turn's chain, and is
        // given as long as the package gives a call.
        let meta = CallMeta::default()
            .chain(cx.chain().iter().cloned())
            .with_deadline(Some(now_ms() + self.app.timeout.as_millis() as u64));
        let reply = cancellable(&control, &call_id, cx.cancel(), control.call_tool(meta, call)).await;
        self.registry.unlisten(&call_id);
        tool_output(&self.app.id, reply)
    }
}

/// A package's `events`, before every tool call of one session.
struct AppEvents {
    app: Arc<Loaded>,
    registry: Arc<Registry>,
}

#[async_trait::async_trait]
impl ToolHook for AppEvents {
    /// Not for a process let go while idle: a session's every tool call does not wake it.
    async fn before(&self, call: &ToolCallFacts) -> HookDecision {
        // Decided before anything crosses to its process: what it was not granted to see, it is
        // not sent.
        if !offered(&self.app, call.name()) {
            return HookDecision::Proceed;
        }
        let Some(control) = self.registry.control(&self.app.id) else {
            return HookDecision::Proceed;
        };
        let _held = self.registry.hold(&self.app.id);
        let payload = json!({
            "tool": call.name(),
            "input": call.input(),
            "session_id": call.session_id(),
            "depth": call.depth(),
        });
        let call_id = uuid::Uuid::new_v4().simple().to_string();
        let hook = HookCall {
            app: self.app.id.clone(),
            event: "tool.before".into(),
            payload,
            call_id: call_id.clone(),
        };
        let decision = cancellable(&control, &call_id, call.cancel(), control.hook(CallMeta::default(), hook))
            .await
            .unwrap_or_else(|e| HookReply::Failed(e.message));
        hook_decision(&self.app.id, call.name(), decision)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use harnix_ext::engine::Cancel;
    use harnix_package::manifest::Interface;
    use harnix_package::signature::Trust;
    use harnix_wasm_port::{
        Control, MethodCall, Place, Placement, RpcError, Start, Started, Stop, ToolCall, ToolReply, TopicValue,
        TurnChain, TurnRef, Unplace,
    };

    use super::*;

    /// A package's process that only remembers which tool calls its `events` was sent.
    #[derive(Default)]
    struct Heard(Mutex<Vec<String>>);

    fn unused<T>() -> Result<T, RpcError> {
        Err(RpcError::new(harnix_ext::codes::INTERNAL_ERROR, "not in this test"))
    }

    #[async_trait::async_trait]
    impl Control for Heard {
        async fn place(&self, _: CallMeta, _: Place) -> Result<Placement, RpcError> {
            unused()
        }
        async fn unplace(&self, _: CallMeta, _: Unplace) -> Result<Value, RpcError> {
            unused()
        }
        async fn start(&self, _: CallMeta, _: Start) -> Result<Started, RpcError> {
            unused()
        }
        async fn stop(&self, _: CallMeta, _: Stop) -> Result<(), RpcError> {
            unused()
        }
        async fn forget(&self, _: CallMeta, _: String) -> Result<(), RpcError> {
            unused()
        }
        async fn status(&self, _: CallMeta, _: String) -> Result<Value, RpcError> {
            unused()
        }
        async fn call_method(&self, _: CallMeta, _: MethodCall) -> Result<Value, RpcError> {
            unused()
        }
        async fn deliver(&self, _: CallMeta, _: TopicValue) -> Result<(), RpcError> {
            unused()
        }
        async fn call_tool(&self, _: CallMeta, _: ToolCall) -> Result<ToolReply, RpcError> {
            unused()
        }
        async fn hook(&self, _: CallMeta, call: HookCall) -> Result<HookReply, RpcError> {
            let tool = call.payload["tool"].as_str().unwrap_or_default().to_string();
            self.0.lock().unwrap().push(format!("{} {tool}", call.app));
            Ok(HookReply::Proceed)
        }
        async fn cancel(&self, _: CallMeta, _: String) -> Result<(), RpcError> {
            Ok(())
        }
        async fn shutdown(&self, _: CallMeta, _: String) -> Result<(), RpcError> {
            Ok(())
        }
        fn turn_started(&self, _: TurnChain) {}
        fn turn_settled(&self, _: TurnRef) {}
    }

    fn package(id: &str, events: EventScope) -> Arc<Loaded> {
        Arc::new(Loaded {
            id: id.into(),
            version: "1.0.0".into(),
            title: None,
            root: PathBuf::new(),
            component: true,
            events,
            tools: vec![ToolDef {
                name: "search".into(),
                description: "d".into(),
                doc: None,
                input_schema: "{}".into(),
                read_only: true,
                concurrency_safe: true,
            }],
            ui: Vec::new(),
            interface: Interface::default(),
            surfaces: Vec::new(),
            trust: Trust::parse("unsigned"),
            backend_spaces: Vec::new(),
            inline: Value::Null,
            timeout: harnix_carrier_shared::DEFAULT_TIMEOUT,
        })
    }

    /// Each package's process is sent what it was granted to see, and nothing else: its own tools'
    /// calls (`own`, or `all` without `events.all_tools`), every call (`all`, granted), or none
    /// (no `events` export).
    #[tokio::test]
    async fn a_tool_call_crosses_only_to_the_processes_granted_to_see_it() {
        let registry = Arc::new(Registry::default());
        let heard = Arc::new(Heard::default());
        for (id, events) in [("acme.own", EventScope::Own), ("acme.all", EventScope::All), ("acme.none", EventScope::None)] {
            registry.insert(package(id, events), heard.clone());
        }
        let tools = ProcTools::new(registry.clone());
        let hooks = tools.hooks();
        assert_eq!(hooks.len(), 2, "a package offered nothing has no hook");
        let own_tool = harnix_package::tool_name("acme.own", "search");
        let none_tool = harnix_package::tool_name("acme.none", "search");
        for tool in ["Bash", own_tool.as_str(), none_tool.as_str()] {
            let facts = ToolCallFacts::new(tool, json!({"command": "echo secret"}), Cancel::new());
            for hook in &hooks {
                assert!(matches!(hook.before(&facts).await, HookDecision::Proceed));
            }
        }
        let mut heard = heard.0.lock().unwrap().clone();
        heard.sort();
        assert_eq!(
            heard,
            [
                "acme.all Bash".to_string(),
                format!("acme.all {none_tool}"),
                format!("acme.all {own_tool}"),
                format!("acme.own {own_tool}"),
            ]
        );
    }
}
