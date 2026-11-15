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

    /// Every tool it gives the instance's sessions now. Asked again each time one of `changes`
    /// fires; a session built after that has the new set, one built before keeps its own.
    fn tools(&self) -> Vec<Arc<dyn Tool>>;

    /// Asked before each tool call of every session, in the order given.
    fn hooks(&self) -> Vec<Arc<dyn ToolHook>> {
        Vec::new()
    }

    /// What tells the engine `tools` or `hooks` would answer something else now. None: never.
    fn changes(&self) -> Vec<ToolsChanged> {
        Vec::new()
    }
}

/// Fired by a provider whose tools or hooks changed (a package started or stopped).
#[non_exhaustive]
#[derive(Clone)]
pub struct ToolsChanged(Arc<tokio::sync::watch::Sender<u64>>);

impl Default for ToolsChanged {
    fn default() -> Self {
        Self(Arc::new(tokio::sync::watch::channel(0).0))
    }
}

impl ToolsChanged {
    pub fn new() -> Self {
        Self::default()
    }

    /// Say the set changed.
    pub fn notify(&self) {
        self.0.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Hear each change after now.
    pub fn watch(&self) -> ToolsWatch {
        ToolsWatch(self.0.subscribe())
    }
}

/// What hears a [`ToolsChanged`].
#[non_exhaustive]
pub struct ToolsWatch(tokio::sync::watch::Receiver<u64>);

impl ToolsWatch {
    /// The next change; `false` once nothing can fire it any more.
    pub async fn changed(&mut self) -> bool {
        self.0.changed().await.is_ok()
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
    session_id: String,
    depth: u32,
    cwd: PathBuf,
}

impl ToolCallFacts {
    /// No session, depth 0, no working directory: `with_session` and `with_cwd` say them.
    pub fn new(name: &str, input: Value, cancel: Cancel) -> Self {
        Self {
            name: name.to_string(),
            input,
            cancel,
            session_id: String::new(),
            depth: 0,
            cwd: PathBuf::new(),
        }
    }

    pub fn with_session(mut self, session_id: &str, depth: u32) -> Self {
        self.session_id = session_id.to_string();
        self.depth = depth;
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = cwd.into();
        self
    }

    /// The session that made the call: a delegate's own id for a delegate's call.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 0 for a session a host created, more for a delegate.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// The calling session's working directory, as the engine has it.
    pub fn cwd(&self) -> &Path {
        &self.cwd
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_calls_facts_say_its_session_depth_and_cwd() {
        let bare = ToolCallFacts::new("t", Value::Null, Cancel::new());
        assert_eq!((bare.session_id(), bare.depth(), bare.cwd()), ("", 0, Path::new("")));
        let facts = bare.with_session("s1", 1).with_cwd("/p");
        assert_eq!((facts.session_id(), facts.depth(), facts.cwd()), ("s1", 1, Path::new("/p")));
    }

    #[tokio::test]
    async fn a_change_is_heard_after_it_is_watched() {
        let changed = ToolsChanged::new();
        changed.notify();
        let mut watch = changed.watch();
        changed.notify();
        assert!(watch.changed().await);
        let mut gone = changed.watch();
        drop(changed);
        assert!(!gone.changed().await);
    }
}
