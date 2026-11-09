//! What the VM's callers see: the world a component implements, its tools and their output, a
//! lifecycle event's answer, a method's error, and why a call produced no answer.

use crate::bindings::harnix::app::types as wit;
use serde::Serialize;
use serde_json::Value;

/// The `harnix:app` world a component implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum World {
    /// Tools and events only.
    Tools,
    /// A resident backend, plus tools and events.
    Backend,
}

/// One tool a component exports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub doc: Option<String>,
    /// JSON Schema, as the component wrote it.
    pub input_schema: String,
    pub read_only: bool,
    pub concurrency_safe: bool,
}

/// A tool's result. `is_error` is the tool answering that it failed; a trap
/// is a [`CallFailure`] instead.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolOutput {
    pub content: String,
    pub structured: Option<Value>,
    pub is_error: bool,
}

/// What a component answers to a lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", content = "text", rename_all = "snake_case")]
pub enum HookDecision {
    Proceed,
    Block(String),
    AddContext(String),
}

/// A method's failure as the application reports it, for its caller.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

/// Why a call produced no answer from the component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallFailure {
    /// The call outlived the application's `timeout_ms`.
    TimedOut,
    /// The caller withdrew (a cancelled turn, a closing session).
    Cancelled,
    /// The component trapped, ran out of memory, or could not be run.
    Faulted(String),
    /// The call was not made: the application is stopped or disabled, its
    /// component does not export what was asked for, or the call would have
    /// deadlocked. Never counted against the application.
    Unavailable(String),
}

impl std::fmt::Display for CallFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimedOut => write!(f, "the application's call exceeded its timeout"),
            Self::Cancelled => write!(f, "the application's call was cancelled"),
            Self::Faulted(e) => write!(f, "the application failed: {e}"),
            Self::Unavailable(e) => write!(f, "the application cannot take this call: {e}"),
        }
    }
}

impl std::error::Error for CallFailure {}

impl From<wit::ToolDef> for ToolDef {
    fn from(t: wit::ToolDef) -> Self {
        Self {
            name: t.name,
            description: t.description,
            doc: t.doc,
            input_schema: t.input_schema,
            read_only: t.read_only,
            concurrency_safe: t.concurrency_safe,
        }
    }
}

impl From<wit::HookDecision> for HookDecision {
    fn from(d: wit::HookDecision) -> Self {
        match d {
            wit::HookDecision::Proceed => Self::Proceed,
            wit::HookDecision::Block(why) => Self::Block(why),
            wit::HookDecision::AddContext(text) => Self::AddContext(text),
        }
    }
}

impl From<wit::AppError> for AppError {
    fn from(e: wit::AppError) -> Self {
        Self {
            code: e.code,
            message: e.message,
            // Detail that is not JSON is still detail; keep it as a string.
            data: e
                .data_json
                .map(|d| serde_json::from_str(&d).unwrap_or(Value::String(d))),
        }
    }
}
