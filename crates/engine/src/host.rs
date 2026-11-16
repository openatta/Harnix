//! What a link offers on its connection: the tools its providers give every session, the hooks
//! before each tool call, and the gates on each message — declared and registered with the daemon
//! (`daemon.setHostTools`, `daemon.setHooks`), and answered here when the daemon calls them. Its
//! scene layers' tools are a session's own, declared as the session is created in one, and
//! answered here too: by name, the session's own first, as the engine looks them up.
//!
//! What a connection registers goes with it, so a link registers everything again each time it
//! takes one, before it says it is ready; and again whenever a provider says its set changed.
//! Only sessions built after that have the new tools: one built before keeps its own. Hooks are
//! read at every call, so a change reaches every session at once.
//!
//! The daemon's calls come off the link's reader, which must never wait on them: a reply or a
//! call behind one would wait too, and the hub's event stream behind both. Each is taken here
//! without waiting and answered on a task of its own.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::Duration;

use harnix_contract::sync::lock;
use harnix_contract::RpcError;
use harnix_engine_api::{Cancel, ChainSource, EngineContribution, NoChains, PromptGate, SceneLayers, Tool, ToolError, ToolProvider, ToolsWatch};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::daemon_link::Refusal;
use crate::translate::{self, HostFrame};

/// The most tools one connection may declare.
const MAX_TOOLS: usize = 64;
/// How long a change waits for the ones that come with it: starting or stopping a package often
/// changes several providers at once.
const SETTLE: Duration = Duration::from_millis(100);
/// How often, and after how long, an answer the daemon turned away for too many requests in
/// flight on the connection is sent again.
const RETRIES: [Duration; 3] = [Duration::from_millis(50), Duration::from_millis(200), Duration::from_millis(500)];

/// Where an engine's tool calls read their turn's chain from ([`ChainSource`]); by default, no
/// turn waits on any call.
#[derive(Clone)]
pub struct Chains(pub Arc<dyn ChainSource>);

impl Default for Chains {
    fn default() -> Self {
        Self(Arc::new(NoChains))
    }
}

impl std::fmt::Debug for Chains {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Chains")
    }
}

/// The connection a [`HostSide`] registers on and answers through: the link's own.
#[async_trait::async_trait]
pub(crate) trait Wired: Send + Sync + 'static {
    /// A call on the connection, whether or not the link says it is ready yet.
    async fn send(&self, method: &str, params: Value) -> Result<Value, RpcError>;

    fn is_ready(&self) -> bool;
}

/// What one link offers its daemon, and its calls in flight.
pub struct HostSide {
    instance: Arc<str>,
    /// Whether the link is the host of its daemon's sessions: a call of a tool it does not have
    /// is answered as not there. One that offers nothing ([`HostSide::none`]: an attached daemon,
    /// an adopted instance) only looks on, and such a call is another host's to answer (Agix
    /// §7.4: only a host that implements a tool runs it).
    hosts: bool,
    chains: Arc<dyn ChainSource>,
    providers: Vec<Arc<dyn ToolProvider>>,
    gates: Vec<Arc<dyn PromptGate>>,
    /// The scene layers its sessions may be created in, and their tools.
    layers: SceneLayers,
    /// The layers' tools nobody is asked about: those of an unattended layer.
    unattended: HashSet<String>,
    /// Its tools' permission asks, answered for them (`permits.rs`).
    pub(crate) permits: crate::permits::Permits,
    /// The tools the daemon took at the last registration, by name.
    declared: StdMutex<Arc<HashMap<String, Arc<dyn Tool>>>>,
    /// The hooks registered last: before tool calls, and on messages.
    hooks_on: StdMutex<(bool, bool)>,
    calls: StdMutex<HashMap<String, InFlight>>,
    /// Each root session's project, once asked: it never changes.
    projects: StdMutex<HashMap<String, PathBuf>>,
    /// A registration as a connection is taken and one after a change are one at a time.
    registering: tokio::sync::Mutex<()>,
    /// Every provider's changes as of the last registration: one pending is a set of tools the
    /// daemon has not been told yet (`catch_up`).
    unheard: StdMutex<Vec<ToolsWatch>>,
    tasks: StdMutex<Vec<AbortHandle>>,
}

struct InFlight {
    cancel: Cancel,
    task: Option<AbortHandle>,
}

impl HostSide {
    /// What `contrib`'s providers, gates and scene layers offer the sessions of `instance`, whose
    /// turns' chains `chains` says; refused when its scene layers do not expand.
    pub fn new(instance: &str, chains: Chains, contrib: &EngineContribution) -> Result<Arc<Self>, String> {
        Self::build(instance, chains, contrib, true)
    }

    /// Nothing offered: a link that registers nothing, and only looks on at the calls of the
    /// tools on its daemon.
    pub fn none() -> Arc<Self> {
        Self::build("", Chains::default(), &EngineContribution::new(), false).expect("no scene layers expand")
    }

    fn build(instance: &str, chains: Chains, contrib: &EngineContribution, hosts: bool) -> Result<Arc<Self>, String> {
        let layers = SceneLayers::new(contrib.scenes())?;
        let unattended = layers
            .ids()
            .filter_map(|id| layers.get(id))
            .filter(|l| l.attended == Some(false))
            .flat_map(|l| l.tools.iter().map(|t| t.spec().name().to_string()))
            .collect();
        Ok(Arc::new(Self {
            instance: instance.into(),
            hosts,
            chains: chains.0,
            providers: contrib.providers().to_vec(),
            gates: contrib.prompt_gates().to_vec(),
            layers,
            unattended,
            permits: Default::default(),
            declared: StdMutex::default(),
            hooks_on: StdMutex::default(),
            calls: StdMutex::default(),
            projects: StdMutex::default(),
            registering: tokio::sync::Mutex::new(()),
            unheard: StdMutex::new(contrib.providers().iter().flat_map(|p| p.changes()).map(|c| c.watch()).collect()),
            tasks: StdMutex::default(),
        }))
    }

    /// The scene layers its sessions may be created in.
    pub fn layers(&self) -> &SceneLayers {
        &self.layers
    }

    /// Whether a call of `name` that would be asked about has nobody to ask: a tool of an
    /// unattended scene layer.
    pub(crate) fn nobody_asks(&self, name: &str) -> bool {
        self.unattended.contains(name)
    }

    /// A task of the link's own, stopped with it.
    pub(crate) fn keep(&self, task: AbortHandle) {
        let mut tasks = lock(&self.tasks);
        tasks.retain(|t| !t.is_finished());
        tasks.push(task);
    }

    /// The tool a call by `name` reaches: a session's own, from a scene layer, before one the
    /// connection declared.
    pub(crate) fn tool(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.layers.tool(name).cloned().or_else(|| lock(&self.declared).get(name).cloned())
    }

    pub(crate) fn instance(&self) -> &str {
        &self.instance
    }

    pub(crate) fn chains(&self) -> &Arc<dyn ChainSource> {
        &self.chains
    }

    pub(crate) fn providers(&self) -> &[Arc<dyn ToolProvider>] {
        &self.providers
    }

    pub(crate) fn gates(&self) -> &[Arc<dyn PromptGate>] {
        &self.gates
    }

    /// Declare the tools and register the hooks on `wire`. Taking a connection (`connecting`), a
    /// registration that fails refuses it: a gate left off would let through what it is there to
    /// stop. After a change, a failure is logged and the last registration stands.
    pub(crate) async fn register(&self, wire: &dyn Wired, connecting: bool) -> Result<(), Refusal> {
        let _one = self.registering.lock().await;
        for watch in lock(&self.unheard).iter_mut() {
            watch.hear();
        }
        let failed = |what: String| -> Result<(), Refusal> {
            if connecting {
                Err(Refusal::Retry(what))
            } else {
                tracing::error!(instance = %self.instance, "{what}");
                Ok(())
            }
        };

        let tools = self.tools();
        let had = !lock(&self.declared).is_empty();
        if !tools.is_empty() || had {
            let decls = tools.iter().map(|t| translate::host_tool_decl(t.spec())).collect();
            match wire.send(translate::DAEMON_SET_HOST_TOOLS, translate::host_tools_params(decls)).await {
                Ok(answer) => {
                    for (name, error) in translate::refused(&answer) {
                        tracing::warn!(instance = %self.instance, tool = name.as_deref().unwrap_or("?"), %error, "a tool the engine refused");
                    }
                    let adopted: HashSet<String> = translate::adopted_tools(&answer).into_iter().collect();
                    let declared = tools.into_iter().filter(|t| adopted.contains(t.spec().name())).map(|t| (t.spec().name().to_string(), t));
                    *lock(&self.declared) = Arc::new(declared.collect());
                }
                Err(e) => failed(format!("the tools were not declared: {}", e.message))?,
            }
        }

        let want = (self.providers.iter().any(|p| !p.hooks().is_empty()), !self.gates.is_empty());
        let had = *lock(&self.hooks_on);
        if want != (false, false) || had != (false, false) {
            match wire.send(translate::DAEMON_SET_HOOKS, translate::hooks_params(want.0, want.1)).await {
                Ok(answer) => {
                    let refused = translate::refused(&answer);
                    for (_, error) in &refused {
                        tracing::warn!(instance = %self.instance, %error, "a hook the engine refused");
                    }
                    if refused.is_empty() {
                        *lock(&self.hooks_on) = want;
                    } else if want.1 {
                        failed("the prompt gates' hook was not registered".into())?;
                    }
                }
                Err(e) if want.1 => failed(format!("the prompt gates' hook was not registered: {}", e.message))?,
                Err(e) => tracing::error!(instance = %self.instance, error = %e.message, "the tool hooks were not registered"),
            }
        }
        Ok(())
    }

    /// Every provider's tools now: the first by a name is kept, a name the engine would refuse is
    /// left out, and so is everything past the most a connection may declare.
    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        let mut seen = HashSet::new();
        let mut tools = Vec::new();
        for provider in &self.providers {
            for tool in provider.tools() {
                let name = tool.spec().name().to_string();
                if !engine_tool_name(&name) {
                    tracing::warn!(provider = provider.name(), tool = %name, "not a name the engine takes for a tool; left out");
                } else if !seen.insert(name.clone()) {
                    tracing::warn!(provider = provider.name(), tool = %name, "a second tool by that name; left out");
                } else {
                    tools.push(tool);
                }
            }
        }
        if tools.len() > MAX_TOOLS {
            let left: Vec<String> = tools[MAX_TOOLS..].iter().map(|t| t.spec().name().to_string()).collect();
            tracing::error!(count = tools.len(), "a connection offers at most {MAX_TOOLS} host tools; these are left out: {}", left.join(", "));
            tools.truncate(MAX_TOOLS);
        }
        tools
    }

    /// Take `frame` when it is this link's: a call of a tool it declared or a hook it
    /// registered, or one of those taken back, and on a link that hosts its daemon's sessions,
    /// any tool's call. Never waits: the call is answered on a task of its own. `false` hands the
    /// frame on to the hub.
    pub(crate) fn offer(self: &Arc<Self>, frame: HostFrame, wire: &Arc<dyn Wired>) -> bool {
        match frame {
            HostFrame::ToolCall(call) => {
                let Some(tool) = self.tool(&call.name) else {
                    if !self.hosts {
                        return false;
                    }
                    self.not_here(call, wire);
                    return true;
                };
                let cancel = self.begin(&call.call_id);
                let call_id = call.call_id.clone();
                let task = tokio::spawn(crate::host_tools::run(self.clone(), wire.clone(), tool, call, cancel));
                self.running(&call_id, task.abort_handle());
                true
            }
            HostFrame::ToolCallResolved { call_id, withdrawn } => {
                let Some(cancel) = lock(&self.calls).get(&call_id).map(|c| c.cancel.clone()) else { return false };
                if withdrawn {
                    cancel.cancel();
                }
                true
            }
            HostFrame::HookCall(call) => {
                let cancel = self.begin(&call.call_id);
                let call_id = call.call_id.clone();
                let task = tokio::spawn(crate::host_hooks::run(self.clone(), wire.clone(), call, cancel));
                self.running(&call_id, task.abort_handle());
                true
            }
            HostFrame::HookCallResolved { call_id } => {
                if let Some(cancel) = lock(&self.calls).get(&call_id).map(|c| c.cancel.clone()) {
                    cancel.cancel();
                }
                true
            }
        }
    }

    /// A call of a tool the link does not have, answered on the spot: one a session was created
    /// with that this build no longer offers (its scene layer renamed or gone, its extension
    /// stopped). Left to the engine, the turn would wait for it until it times out.
    fn not_here(self: &Arc<Self>, call: translate::HostToolCall, wire: &Arc<dyn Wired>) {
        tracing::warn!(instance = %self.instance, tool = %call.name, session = %call.session, "a call of a tool this host does not offer");
        let missing = Err(ToolError::NotFound(format!("this host offers no tool `{}`", call.name)));
        let answer = translate::tool_answer(&call.session, &call.call_id, missing);
        let (host, wire) = (self.clone(), wire.clone());
        let task = tokio::spawn(async move { host.answer(&*wire, translate::SESSION_RESPOND_TOOL_CALL, answer).await });
        self.keep(task.abort_handle());
    }

    fn begin(&self, call_id: &str) -> Cancel {
        let cancel = Cancel::new();
        lock(&self.calls).insert(call_id.to_string(), InFlight { cancel: cancel.clone(), task: None });
        cancel
    }

    fn running(&self, call_id: &str, task: AbortHandle) {
        if let Some(call) = lock(&self.calls).get_mut(call_id) {
            call.task = Some(task);
        }
    }

    /// A call answered: no longer in flight.
    pub(crate) fn done(&self, call_id: &str) {
        lock(&self.calls).remove(call_id);
    }

    /// The project root session `session` works in, asked of the engine once.
    pub(crate) async fn project_of(&self, wire: &dyn Wired, session: &str) -> PathBuf {
        if let Some(root) = lock(&self.projects).get(session).cloned() {
            return root;
        }
        match wire.send(translate::SESSION_GET, serde_json::json!({"session_id": session})).await {
            Ok(answer) => {
                let root = translate::project_root(&answer);
                lock(&self.projects).insert(session.to_string(), root.clone());
                root
            }
            Err(_) => PathBuf::new(),
        }
    }

    /// Send an answer, again when the connection had too many requests in flight to take it.
    /// One that still does not go is left to the engine, which ends the call when it times out.
    pub(crate) async fn answer(&self, wire: &dyn Wired, method: &str, params: Value) {
        let mut retries = RETRIES.iter();
        loop {
            match wire.send(method, params.clone()).await {
                Err(e) if e.code == crate::embed::TOO_MANY_IN_FLIGHT => match retries.next() {
                    Some(wait) => tokio::time::sleep(*wait).await,
                    None => {
                        tracing::error!(instance = %self.instance, method, "an answer the engine had no room for; it ends the call when it times out");
                        return;
                    }
                },
                Err(e) => {
                    tracing::warn!(instance = %self.instance, method, error = %e.message, "an answer the engine did not take");
                    return;
                }
                Ok(_) => return,
            }
        }
    }

    /// Declare again now if a provider changed since the last registration, rather than after the
    /// change settles: a session made just after a package started has its tools
    /// (harnix-ext-package.md §5).
    pub(crate) async fn catch_up(&self, wire: &dyn Wired) {
        if lock(&self.unheard).iter().any(ToolsWatch::pending) {
            let _ = self.register(wire, false).await;
        }
    }

    /// Hear every provider's changes, and declare again — on `wire`, while it is ready — after
    /// each settles. A link that is not ready then registers everything when it next connects.
    pub(crate) fn watch(self: &Arc<Self>, wire: Weak<dyn Wired>) {
        let changes: Vec<_> = self.providers.iter().flat_map(|p| p.changes()).collect();
        if changes.is_empty() {
            return;
        }
        let (tx, mut rx) = mpsc::channel::<()>(1);
        let mut tasks = Vec::new();
        for change in changes {
            let mut heard = change.watch();
            let tx = tx.clone();
            tasks.push(tokio::spawn(async move {
                while heard.changed().await {
                    let _ = tx.try_send(());
                }
            }));
        }
        drop(tx);
        let host = Arc::downgrade(self);
        tasks.push(tokio::spawn(async move {
            while rx.recv().await.is_some() {
                tokio::time::sleep(SETTLE).await;
                while rx.try_recv().is_ok() {}
                let (Some(host), Some(wire)) = (host.upgrade(), wire.upgrade()) else { break };
                if !wire.is_ready() {
                    continue;
                }
                let _ = host.register(&*wire, false).await;
            }
        }));
        lock(&self.tasks).extend(tasks.iter().map(|t| t.abort_handle()));
    }

    /// The connection is gone, and what it registered with it: the calls in flight are stopped,
    /// and the next connection registers from nothing.
    pub(crate) fn lost(&self) {
        for call in lock(&self.calls).values() {
            call.cancel.cancel();
        }
        *lock(&self.declared) = Arc::default();
        *lock(&self.hooks_on) = (false, false);
        self.permits.clear();
        lock(&self.projects).clear();
    }

    /// The link is shut down: nothing more is heard or answered.
    pub(crate) fn shutdown(&self) {
        for task in lock(&self.tasks).drain(..) {
            task.abort();
        }
        for (_, call) in lock(&self.calls).drain() {
            call.cancel.cancel();
            if let Some(task) = call.task {
                task.abort();
            }
        }
    }
}

/// Whether the engine takes `name` for a tool: `^[A-Za-z][A-Za-z0-9_]{0,63}$`, and not one of an
/// MCP server's.
fn engine_tool_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with("mcp__")
}

#[cfg(test)]
mod tests {
    use super::engine_tool_name;

    #[test]
    fn a_tool_name_is_what_the_engine_takes() {
        assert!(engine_tool_name("ext__acme_notes__search"));
        assert!(!engine_tool_name("ext__acme-notes__search"));
        assert!(!engine_tool_name("1x") && !engine_tool_name("") && !engine_tool_name(&"a".repeat(65)));
        assert!(!engine_tool_name("mcp__github__x"));
    }
}
