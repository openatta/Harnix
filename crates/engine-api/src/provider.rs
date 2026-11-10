//! Tools for every session, and hooks around a call.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use crate::tool::{Cancel, Tool};

/// What hands each session of an instance its tools — sub-agents, team members and background
/// tasks included. Asked each time a session is built, so what it gives is what is there then.
pub trait ToolProvider: Send + Sync + 'static {
    /// Unique among an instance's providers.
    fn name(&self) -> &str;

    fn tools(&self, session: &SessionFacts) -> Vec<Arc<dyn Tool>>;

    /// Asked before each tool call of the session, in the order given.
    fn hooks(&self, _session: &SessionFacts) -> Vec<Arc<dyn ToolHook>> {
        Vec::new()
    }
}

/// The session a provider is giving tools to.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct SessionFacts {
    session_id: String,
    project: PathBuf,
    scene: String,
    depth: u32,
}

impl SessionFacts {
    pub fn new(session_id: &str, project: impl Into<PathBuf>, scene: &str, depth: u32) -> Self {
        Self { session_id: session_id.to_string(), project: project.into(), scene: scene.to_string(), depth }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The session's project root.
    pub fn project(&self) -> &Path {
        &self.project
    }

    pub fn scene(&self) -> &str {
        &self.scene
    }

    /// 0 for a session a host created, one more for each delegate down.
    pub fn depth(&self) -> u32 {
        self.depth
    }
}

/// Asked before a tool call runs.
#[async_trait::async_trait]
pub trait ToolHook: Send + Sync {
    async fn before(&self, call: &ToolCallFacts) -> HookDecision;
}

/// The call a hook is asked about.
#[non_exhaustive]
#[derive(Clone)]
pub struct ToolCallFacts {
    name: String,
    input: Value,
    cancel: Cancel,
}

impl ToolCallFacts {
    pub fn new(name: &str, input: Value, cancel: Cancel) -> Self {
        Self { name: name.to_string(), input, cancel }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn input(&self) -> &Value {
        &self.input
    }

    /// Fired when the call is cancelled, while the hook is still deciding.
    pub fn cancel(&self) -> &Cancel {
        &self.cancel
    }
}

/// What a hook decided.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookDecision {
    /// The call runs.
    Proceed,
    /// It does not run; the model is told this instead, as a failed call.
    Block(String),
    /// It runs, and this is appended to the text of its answer.
    AddContext(String),
}
