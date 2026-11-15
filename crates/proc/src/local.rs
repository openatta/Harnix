//! The process's own engine and what its host shares with it: the agent side of the host
//! (`Agent`) over the engine, the tools its package offers in the engine's sessions, the turn
//! chains the product told of, and a tool call's progress to the session that made it.
//!
//! The engine is the process's own, served to the product on the stream it inherited
//! (`harnix_engine::assemble::serve_stream`); it is built from an `EngineProfile` on `run/engine/`,
//! with provider settings the product wrote there beforehand. Nothing of the product's engine
//! policy is in it.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use async_trait::async_trait;
use base64::Engine as _;
use harnix_engine::InProcessLink;
use harnix_engine_api::ops::EngineOps;
use harnix_engine_api::{
    Cancel, ChainLink, ChainSource, EngineControl, EngineLink, HookDecision, Permission, SessionFacts, SessionRef, Tool, ToolCallFacts,
    ToolCx, ToolError, ToolHook, ToolOutput, ToolProvider, ToolSpec,
};
use harnix_package::acquire::{internal, refused};
use harnix_package::{package, tool_name};
use harnix_wasm_host::{Host, HttpFetcher, HttpRequest, HttpResponse};
use harnix_wasm_port::{
    codes, Agent, AgentInstall, AgentRef, AgentSwitch, CallAs, CallMeta, Control, Failure, Fetch, Fetched, HookCall,
    HookReply, Kernel, Offer, Progress, Publish, Registration, RpcError, Skills, Subscription, ToolCall, ToolDef,
    ToolReply, TurnChain, TurnRef,
};
use serde_json::{json, Value};

/// The provider's name in every session.
const NAME: &str = "harnix-extension";

/// The longest note a package's hook may add to a tool's result, in characters.
const MAX_NOTE: usize = 2000;

/// What marks a skill directory as a package's, inside the skills directory.
const SKILL_MARK: &str = ".harnix-app";

/// Where one tool call's progress goes, by the call's id.
type Listener = Arc<dyn Fn(&str) + Send + Sync>;

/// The state the host's agent side, the engine's tools and the product's wire share.
pub(crate) struct Local {
    /// The name its engine goes by: what a turn's chain is told under.
    instance: String,
    skills: PathBuf,
    /// The wire's kernel, which what a package asks of the product goes to.
    wire: Arc<dyn Kernel>,
    host: OnceLock<Weak<Host>>,
    link: OnceLock<Arc<InProcessLink>>,
    offers: Mutex<BTreeMap<String, Offer>>,
    listeners: Mutex<HashMap<String, Listener>>,
    hooks: AtomicU64,
}

impl Local {
    pub(crate) fn new(ext: &str, data_root: &Path, wire: Arc<dyn Kernel>) -> Arc<Self> {
        Arc::new(Self {
            instance: format!("proc.{ext}"),
            skills: data_root.join("skills"),
            wire,
            host: OnceLock::new(),
            link: OnceLock::new(),
            offers: Mutex::new(BTreeMap::new()),
            listeners: Mutex::new(HashMap::new()),
            hooks: AtomicU64::new(0),
        })
    }

    pub(crate) fn instance(&self) -> &str {
        &self.instance
    }

    pub(crate) fn host_is(&self, host: &Arc<Host>) {
        let _ = self.host.set(Arc::downgrade(host));
    }

    pub(crate) fn link_is(&self, link: Arc<InProcessLink>) {
        let _ = self.link.set(link);
    }

    fn control(&self) -> Option<Arc<dyn Control>> {
        Some(self.host.get()?.upgrade()? as Arc<dyn Control>)
    }

    fn ops(&self) -> Result<&dyn EngineOps, RpcError> {
        match self.link.get() {
            Some(link) => Ok(link.ops()),
            None => Err(refused("this extension process's engine is not up")),
        }
    }

    fn listen(&self, call_id: &str, listener: Listener) {
        self.listeners.lock().unwrap_or_else(|e| e.into_inner()).insert(call_id.to_string(), listener);
    }

    fn unlisten(&self, call_id: &str) {
        self.listeners.lock().unwrap_or_else(|e| e.into_inner()).remove(call_id);
    }

    fn offered(&self) -> Vec<Offer> {
        self.offers.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
    }
}

/// What the host asks of the product goes over the wire; a tool call's progress goes to the
/// session that made the call when there is one here, and to the product otherwise.
#[async_trait]
impl Kernel for Local {
    async fn register(&self, meta: CallMeta, registration: Registration) -> Result<(), RpcError> {
        self.wire.register(meta, registration).await
    }

    async fn withdraw(&self, meta: CallMeta, app: String) -> Result<(), RpcError> {
        self.wire.withdraw(meta, app).await
    }

    async fn call(&self, meta: CallMeta, call: CallAs) -> Result<Value, RpcError> {
        self.wire.call(meta, call).await
    }

    async fn publish(&self, meta: CallMeta, publish: Publish) -> Result<(), RpcError> {
        self.wire.publish(meta, publish).await
    }

    async fn subscribe(&self, meta: CallMeta, subscription: Subscription) -> Result<Value, RpcError> {
        self.wire.subscribe(meta, subscription).await
    }

    async fn unsubscribe(&self, meta: CallMeta, subscription: Subscription) -> Result<(), RpcError> {
        self.wire.unsubscribe(meta, subscription).await
    }

    async fn fetch(&self, meta: CallMeta, fetch: Fetch) -> Result<Fetched, RpcError> {
        self.wire.fetch(meta, fetch).await
    }

    async fn turn_chain(&self, meta: CallMeta, turn: TurnRef) -> Result<TurnChain, RpcError> {
        self.wire.turn_chain(meta, turn).await
    }

    fn progress(&self, progress: Progress) {
        let listener = self.listeners.lock().unwrap_or_else(|e| e.into_inner()).get(&progress.call_id).cloned();
        match listener {
            Some(listener) => listener(&progress.text),
            None => self.wire.progress(progress),
        }
    }
}

/// A package's HTTP, which the product sends: `net` is judged where it is sent.
pub(crate) struct WireFetcher(pub(crate) Arc<dyn Kernel>);

#[async_trait]
impl HttpFetcher for WireFetcher {
    async fn fetch(&self, app: &str, request: HttpRequest) -> Result<HttpResponse, String> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let fetch = Fetch {
            app: app.to_string(),
            method: request.method,
            url: request.url,
            headers: request.headers,
            body_b64: request.body.map(|body| b64.encode(body)),
        };
        let fetched = self.0.fetch(CallMeta::default(), fetch).await.map_err(|e| e.message)?;
        let body = b64.decode(fetched.body_b64).map_err(|e| format!("the product's answer: {e}"))?;
        Ok(HttpResponse { status: fetched.status, headers: fetched.headers, body })
    }
}

/// The turns the product told of, for the tool calls of this process's own sessions.
impl ChainSource for Local {
    fn chain(&self, session: &SessionRef) -> Vec<ChainLink> {
        let host = self.host.get().and_then(Weak::upgrade);
        host.and_then(|host| host.turn_chain(&session.instance, &session.session_id)).unwrap_or_default()
    }
}

#[async_trait]
impl Agent for Local {
    async fn has_instance(&self, instance: String) -> Result<bool, RpcError> {
        Ok(instance == self.instance)
    }

    async fn install_agent(&self, a: AgentInstall) -> Result<Value, RpcError> {
        let params = json!({"name": a.name, "version": a.version, "download_url": format!("file://{}", a.archive.display())});
        // Under the kernel's row names, as the hub passes them through: the engine crate says them
        // in the engine's own words.
        let answer = self.ops()?.passthrough("engine.plugin.install", params).await?;
        Ok(answer.get("disclosure").cloned().unwrap_or(answer))
    }

    async fn uninstall_agent(&self, a: AgentRef) -> Result<(), RpcError> {
        self.ops()?.passthrough("engine.plugin.uninstall", json!({"name": a.name, "version": a.version})).await.map(|_| ())
    }

    async fn switch_agent(&self, s: AgentSwitch) -> Result<(), RpcError> {
        let method = if s.on { "engine.plugin.enable" } else { "engine.plugin.disable" };
        self.ops()?.passthrough(method, json!({"name": s.name})).await.map(|_| ())
    }

    /// Copy `<source>/<name>/` into the skills directory, marked as the package's. A directory of
    /// that name that is not the package's is somebody else's skill, and refuses it.
    async fn place_skills(&self, s: Skills) -> Result<(), RpcError> {
        for name in &s.names {
            let target = self.skills.join(name);
            if target.exists() && std::fs::read_to_string(target.join(SKILL_MARK)).ok().as_deref() != Some(s.app.as_str()) {
                return Err(refused(format!("the engine has a skill `{name}` of its own; `{}` would replace it", s.app)));
            }
            let _ = std::fs::remove_dir_all(&target);
            let source = s.source.join(name);
            let mut files = Vec::new();
            package::walk(&source, &source, &mut files).map_err(|e| internal(format!("{}: {e}", source.display())))?;
            for (relative, path) in files {
                let to = target.join(&relative);
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| internal(format!("{}: {e}", parent.display())))?;
                }
                std::fs::copy(&path, &to).map_err(|e| internal(format!("{}: {e}", to.display())))?;
            }
            std::fs::write(target.join(SKILL_MARK), &s.app).map_err(|e| internal(e.to_string()))?;
        }
        Ok(())
    }

    /// Remove the skills marked as the package's, and nothing else.
    async fn remove_skills(&self, s: Skills) -> Result<(), RpcError> {
        for name in &s.names {
            let target = self.skills.join(name);
            if std::fs::read_to_string(target.join(SKILL_MARK)).ok().as_deref() == Some(s.app.as_str()) {
                if let Err(e) = std::fs::remove_dir_all(&target) {
                    tracing::warn!(package = %s.app, skill = %name, error = %e, "a skill was not removed");
                }
            }
        }
        Ok(())
    }

    async fn reload_instance(&self, _instance: String) -> Result<(), RpcError> {
        match self.link.get() {
            Some(link) => link.reload_settings().await,
            None => Err(refused("this extension process's engine is not up")),
        }
    }

    async fn expose_tools(&self, offer: Offer) -> Result<(), RpcError> {
        let mut offers = self.offers.lock().unwrap_or_else(|e| e.into_inner());
        match offer.tools.is_empty() && !offer.component {
            true => offers.remove(&offer.app),
            false => offers.insert(offer.app.clone(), offer),
        };
        Ok(())
    }
}

/// The package's tools and hooks in every session this process's engine builds.
pub(crate) struct LocalTools(pub(crate) Arc<Local>);

impl ToolProvider for LocalTools {
    fn name(&self) -> &str {
        NAME
    }

    fn tools(&self, session: &SessionFacts) -> Vec<Arc<dyn Tool>> {
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        for offer in self.0.offered().into_iter().filter(|o| o.component) {
            for def in &offer.tools {
                let name = tool_name(&offer.app, &def.name);
                let schema = serde_json::from_str(&def.input_schema).unwrap_or_else(|_| json!({"type": "object"}));
                let mut spec = ToolSpec::new(&name, &def.description, schema).source(&format!("ext:{}", offer.app));
                if let Some(doc) = &def.doc {
                    spec = spec.doc(doc);
                }
                if def.read_only {
                    spec = spec.read_only();
                }
                if def.concurrency_safe {
                    spec = spec.concurrency_safe();
                }
                tools.push(Arc::new(AppTool {
                    spec,
                    def: def.clone(),
                    app: offer.app.clone(),
                    title: offer.title.clone(),
                    local: self.0.clone(),
                    project: session.project().to_path_buf(),
                }));
            }
        }
        tools
    }

    fn hooks(&self, session: &SessionFacts) -> Vec<Arc<dyn ToolHook>> {
        self.0
            .offered()
            .into_iter()
            .filter(|o| o.component)
            .map(|offer| {
                Arc::new(AppEvents {
                    app: offer.app,
                    local: self.0.clone(),
                    session_id: session.session_id().to_string(),
                    depth: session.depth(),
                }) as Arc<dyn ToolHook>
            })
            .collect()
    }
}

/// Run `call` (a tool call or a hook named `call_id`), and cancel it in the host if `cancel` fires
/// first. The call is polled first, so the host has its cancellation before it can be asked to.
async fn cancellable<T>(control: &Arc<dyn Control>, call_id: &str, cancel: &Cancel, call: impl std::future::Future<Output = T>) -> T {
    tokio::pin!(call);
    tokio::select! {
        biased;
        out = &mut call => out,
        _ = cancel.cancelled() => {
            let _ = control.cancel(CallMeta::default(), call_id.to_string()).await;
            call.await
        }
    }
}

/// One package's tool, as a session has it.
struct AppTool {
    spec: ToolSpec,
    def: ToolDef,
    app: String,
    title: Option<String>,
    local: Arc<Local>,
    /// The session's project: `/project`, for a manifest that asks for `${project}`.
    project: PathBuf,
}

#[async_trait]
impl Tool for AppTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    /// Every call asks first: a tool's `read-only` claim is the package's own word.
    fn permission(&self, _: &Value) -> Permission {
        Permission::Ask(format!(
            "The package `{}` wants to run its tool `{}`",
            self.title.as_deref().unwrap_or(&self.app),
            self.def.name
        ))
    }

    async fn call(&self, cx: ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
        let Some(control) = self.local.control() else {
            return Err(ToolError::NotFound(format!("`{}` is not running", self.app)));
        };
        let call_id = cx.tool_use_id().to_string();
        {
            let progress = cx.progress().clone();
            self.local.listen(&call_id, Arc::new(move |text: &str| progress.send(text)));
        }
        let call = ToolCall {
            app: self.app.clone(),
            tool: self.def.name.clone(),
            input,
            call_id: call_id.clone(),
            project: Some(self.project.clone()),
            progress: true,
        };
        // On behalf of the session's turn: what the tool calls carries the turn's chain.
        let meta = CallMeta::default().chain(cx.chain().iter().cloned());
        let reply = cancellable(&control, &call_id, cx.cancel(), control.call_tool(meta, call)).await;
        self.local.unlisten(&call_id);
        match reply {
            Ok(ToolReply::Output { content, structured, is_error }) => {
                let out = if is_error { ToolOutput::error(content) } else { ToolOutput::text(content) };
                Ok(out.with_structured(structured))
            }
            Ok(ToolReply::Failed { failure: Failure::Cancelled, .. }) => Err(ToolError::Cancelled),
            Ok(ToolReply::Failed { message, .. }) => Ok(ToolOutput::error(format!("{}: {message}", self.app))),
            Err(e) if e.code == codes::METHOD_NOT_FOUND => Err(ToolError::NotFound(e.message)),
            Err(e) => Ok(ToolOutput::error(format!("{}: {}", self.app, e.message))),
        }
    }
}

/// A package's `events`, before every tool call of one session.
struct AppEvents {
    app: String,
    local: Arc<Local>,
    session_id: String,
    depth: u32,
}

/// What a note of the package `app` adds to the result of `tool`: nothing for a tool not its own,
/// and at most [`MAX_NOTE`] characters, under its name.
fn note_for(app: &str, tool: &str, note: &str) -> Option<String> {
    if !tool.starts_with(&tool_name(app, "")) {
        return None;
    }
    let cut = note.chars().count() > MAX_NOTE;
    let mut note: String = note.chars().take(MAX_NOTE).collect();
    if cut {
        note.push('…');
    }
    Some(format!("\n\n[{app}] {note}"))
}

#[async_trait]
impl ToolHook for AppEvents {
    async fn before(&self, call: &ToolCallFacts) -> HookDecision {
        let Some(control) = self.local.control() else {
            return HookDecision::Proceed;
        };
        let call_id = format!("hook-{}", self.local.hooks.fetch_add(1, Ordering::Relaxed));
        let hook = HookCall {
            app: self.app.clone(),
            event: "tool.before".into(),
            payload: json!({"tool": call.name(), "input": call.input(), "session_id": self.session_id, "depth": self.depth}),
            call_id: call_id.clone(),
        };
        let decision = cancellable(&control, &call_id, call.cancel(), control.hook(CallMeta::default(), hook))
            .await
            .unwrap_or_else(|e| HookReply::Failed(e.message));
        match decision {
            HookReply::Proceed => HookDecision::Proceed,
            HookReply::Block(reason) => {
                tracing::info!(package = %self.app, tool = %call.name(), %reason, "a package refused a tool call");
                HookDecision::Block(format!("refused by the package `{}`: {reason}", self.app))
            }
            HookReply::AddContext(note) => match note_for(&self.app, call.name(), &note) {
                Some(note) => HookDecision::AddContext(note),
                None => HookDecision::Proceed,
            },
            HookReply::Failed(failure) => {
                tracing::warn!(package = %self.app, tool = %call.name(), %failure, "a package's event hook failed; the call goes on");
                HookDecision::Proceed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_goes_on_its_own_tools_results_only_and_only_so_long() {
        assert_eq!(note_for("acme.notes", "ext__acme_notes__search", "seen").as_deref(), Some("\n\n[acme.notes] seen"));
        for tool in ["Bash", "ext__acme_other__search", "ext__acme_notesx__search"] {
            assert_eq!(note_for("acme.notes", tool, "seen"), None, "{tool}");
        }
        let long = "x".repeat(MAX_NOTE + 10);
        let note = note_for("acme.notes", "ext__acme_notes__search", &long).unwrap();
        assert_eq!(note.chars().filter(|c| *c == 'x').count(), MAX_NOTE);
        assert!(note.ends_with('…'));
    }
}
