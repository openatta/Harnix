//! A tool: what the model may call, and what a call gets to see.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use harnix_contract::s1::{ChainLink, SessionRef};
use serde_json::Value;

/// A tool a session can call.
///
/// Its [`ToolSpec`] is what the model is told; [`Tool::permission`] is asked before every call
/// (the default asks the person), and [`Tool::call`] runs it.
#[async_trait::async_trait]
pub trait Tool: Send + Sync + 'static {
    fn spec(&self) -> &ToolSpec;

    /// Whether a call with this input runs, is asked about, or is refused. Asked by default: a
    /// tool that touches nothing says [`Permission::Allow`] itself.
    ///
    /// The link answers the engine's permission ask for the tool by it: `Allow` is let through
    /// and `Deny` refused without anyone seeing the ask; `Ask` goes to the person, its question
    /// shown beside the engine's line. A call is asked again as it runs: one refused does not run
    /// whatever let the engine make it, and nor does one that would be asked about in a session
    /// nobody answers.
    fn permission(&self, _input: &Value) -> Permission {
        Permission::Ask(format!("Run the tool `{}`?", self.spec().name()))
    }

    async fn call(&self, cx: ToolCx, input: Value) -> Result<ToolOutput, ToolError>;
}

/// What the model is told of a tool, and how the engine may schedule it.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ToolSpec {
    name: String,
    description: String,
    input_schema: Value,
    doc: Option<String>,
    origin: Option<String>,
    read_only: bool,
    concurrency_safe: bool,
    destructive: bool,
}

impl ToolSpec {
    pub fn new(name: &str, description: &str, input_schema: Value) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            input_schema,
            doc: None,
            origin: None,
            read_only: false,
            concurrency_safe: false,
            destructive: false,
        }
    }

    /// A longer guide the model fetches when it asks for the tool by name; the description is
    /// what it sees on every call.
    pub fn doc(mut self, doc: &str) -> Self {
        self.doc = Some(doc.to_string());
        self
    }

    /// Where the tool comes from, as the engine's tool table annotates it (`ext:<id>`); a tool
    /// that says nothing is `builtin` there.
    pub fn source(mut self, origin: &str) -> Self {
        self.origin = Some(origin.to_string());
        self
    }

    /// Reads and changes nothing. Scheduling only: it decides nothing about permission.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// May run beside other calls of the same turn.
    pub fn concurrency_safe(mut self) -> Self {
        self.concurrency_safe = true;
        self
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    pub fn documentation(&self) -> Option<&str> {
        self.doc.as_deref()
    }

    pub fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn is_concurrency_safe(&self) -> bool {
        self.concurrency_safe
    }

    pub fn is_destructive(&self) -> bool {
        self.destructive
    }
}

/// Whether a call runs.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Permission {
    Allow,
    /// Ask the person first, with this question.
    Ask(String),
    /// Refused, with why.
    Deny(String),
}

/// What a tool answered.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    text: String,
    structured: Option<Value>,
    is_error: bool,
}

impl ToolOutput {
    /// An answer: the text the model sees.
    pub fn text(text: impl Into<String>) -> Self {
        Self { text: text.into(), structured: None, is_error: false }
    }

    /// The tool ran and calls its answer a failure; the model sees the text.
    pub fn error(text: impl Into<String>) -> Self {
        Self { text: text.into(), structured: None, is_error: true }
    }

    /// A value: pretty-printed for the model, and kept as the result's structured content.
    pub fn json(value: Value) -> Self {
        let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
        Self { text, structured: Some(value), is_error: false }
    }

    /// The same answer, with structured content beside its text.
    pub fn with_structured(mut self, structured: Option<Value>) -> Self {
        self.structured = structured;
        self
    }

    pub fn content(&self) -> &str {
        &self.text
    }

    pub fn structured(&self) -> Option<&Value> {
        self.structured.as_ref()
    }

    pub fn is_error(&self) -> bool {
        self.is_error
    }
}

/// A call that did not answer.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// Stopped: its turn was interrupted.
    Cancelled,
    /// What it would call is not there (an extension that is not running).
    NotFound(String),
    Failed(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::Cancelled => f.write_str("cancelled"),
            ToolError::NotFound(m) => write!(f, "not found: {m}"),
            ToolError::Failed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ToolError {}

/// What one call sees of the session that made it.
#[non_exhaustive]
#[derive(Clone)]
pub struct ToolCx {
    instance: String,
    session: String,
    project: PathBuf,
    depth: u32,
    tool_use_id: String,
    cancel: Cancel,
    progress: Progress,
    chain: Vec<ChainLink>,
}

impl ToolCx {
    /// A call made by session `session` (its id), as the model's tool use `tool_use_id`.
    pub fn new(session: &str, tool_use_id: &str) -> Self {
        Self {
            instance: String::new(),
            session: session.to_string(),
            project: PathBuf::new(),
            depth: 0,
            tool_use_id: tool_use_id.to_string(),
            cancel: Cancel::new(),
            progress: Progress::none(),
            chain: Vec::new(),
        }
    }

    /// The instance whose engine the session is on.
    pub fn with_instance(mut self, instance: &str) -> Self {
        self.instance = instance.to_string();
        self
    }

    pub fn with_project(mut self, project: impl Into<PathBuf>) -> Self {
        self.project = project.into();
        self
    }

    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    pub fn with_cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = cancel;
        self
    }

    pub fn with_progress(mut self, progress: Progress) -> Self {
        self.progress = progress;
        self
    }

    pub fn with_chain(mut self, chain: Vec<ChainLink>) -> Self {
        self.chain = chain;
        self
    }

    /// The calling session's id.
    pub fn session_id(&self) -> &str {
        &self.session
    }

    /// The calling session: its instance and its id — what a call about it names.
    pub fn session(&self) -> SessionRef {
        SessionRef::new(self.instance.as_str(), self.session.as_str())
    }

    /// The directory the session works in.
    pub fn project(&self) -> &Path {
        &self.project
    }

    /// 0 for a session a host created, one more for each delegate down.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    pub fn tool_use_id(&self) -> &str {
        &self.tool_use_id
    }

    /// Fired when the call should stop: its turn was interrupted, or the engine dropped it.
    pub fn cancel(&self) -> &Cancel {
        &self.cancel
    }

    /// Where a running call says how far it got; the person sees it under the call.
    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// The calls that led to this one's turn, outermost first.
    pub fn chain(&self) -> &[ChainLink] {
        &self.chain
    }
}

/// A call's cancellation: cloned to whoever should notice it, fired once.
#[non_exhaustive]
#[derive(Clone, Default)]
pub struct Cancel(tokio_util::sync::CancellationToken);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }

    /// Resolves once it is fired.
    pub async fn cancelled(&self) {
        self.0.cancelled().await
    }

    /// Fires this one when the guard is dropped, unless [`CancelGuard::disarm`]ed first.
    pub fn guard(&self) -> CancelGuard {
        CancelGuard(Some(self.clone()))
    }
}

/// See [`Cancel::guard`].
#[non_exhaustive]
pub struct CancelGuard(Option<Cancel>);

impl CancelGuard {
    pub fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if let Some(cancel) = self.0.take() {
            cancel.cancel();
        }
    }
}

/// Where a call reports progress.
#[non_exhaustive]
#[derive(Clone)]
pub struct Progress(Option<Report>);

type Report = Arc<dyn Fn(&str) + Send + Sync>;

impl Progress {
    pub fn new(f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(f)))
    }

    /// Reports go nowhere.
    pub fn none() -> Self {
        Self(None)
    }

    pub fn send(&self, text: &str) {
        if let Some(f) = &self.0 {
            f(text);
        }
    }
}
