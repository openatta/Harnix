//! The MCP methods' answers, as structures: the one place their shape is stated. The interface's
//! declaration is generated from these (`@harnix/core/protocol`, by
//! `crates/harnix/tests/ts_export.rs`), so the settings page and the right column's context tab read
//! the names the server sends.
//!
//! No value a server is given in secret is in any of them: an environment variable or a header
//! written as a literal is `***` and its last four characters at most ([`McpValue`]), and the
//! engine's sign-in status never carries a token.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// `settings.mcp.list`'s answer, and what every change answers with: the servers as they stand
/// after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct McpList {
    /// Every server of the tiers there are for this (project, scene), the ones switched off
    /// included, in tier order (global, scene, project) and by name within a tier.
    pub servers: Vec<McpServer>,
    /// The settings.json each tier's servers are declared in; a tier with none here is not one.
    pub paths: McpPaths,
    /// Whether the engine said what its servers are doing. `false`: `engine.mcp.status` could not be
    /// read, and no server has a status.
    pub status_read: bool,
    /// The file a change wrote; only on a change's answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub written: Option<String>,
    /// Whether this (project, scene)'s servers have been connected. `false`: none is started until
    /// its first session, and each status says only what the settings make of it (`not_started`,
    /// `disabled`, `untrusted`, `denied`). Absent when the engine was not asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub loaded: Option<bool>,
    /// The (project, scene) the statuses are of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub scene: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub project_root: Option<String>,
}

/// The settings.json of each tier.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct McpPaths {
    pub global: Option<String>,
    pub scene: Option<String>,
    pub project: Option<String>,
}

/// One server as one tier declares it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct McpServer {
    pub name: String,
    /// `global`, `scene` or `project`.
    pub tier: String,
    /// `false`: switched off for this (project, scene) — `mcp_enabled` in a tier says so — and the
    /// engine does not start it.
    pub enabled: bool,
    /// `stdio`, `http`, `sse` or `websocket`; a type the engine does not have, as written.
    pub transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub command: Option<String>,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub url: Option<String>,
    /// The process's environment (stdio).
    pub env: BTreeMap<String, McpValue>,
    /// The request headers (http, sse).
    pub headers: BTreeMap<String, McpValue>,
    /// The `oauth` block, without anything secret in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub oauth: Option<McpOAuth>,
    /// A higher tier declares a server of the same name, and that one is the engine's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub overridden_by: Option<String>,
    /// What it asks for that this build does not have: `private_key_jwt` (Harnix's engine is built
    /// without the `mcp-oauth-jwt` feature).
    pub unsupported: Vec<String>,
    /// What the engine says of it for this (project, scene); absent when the engine was not asked,
    /// or does not list it, or a higher tier declares it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub status: Option<McpStatus>,
}

/// An environment variable's or a header's value as the page sees it: the variable of the engine's
/// environment it is read from, or that a value was given, shown as its last four characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct McpValue {
    /// `env`: read from the engine's environment variable `var` when the server starts. `secret`:
    /// written into the tier's `settings.local.json`, never shown.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub var: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub masked: Option<String>,
}

/// A remote server's `oauth` block, as far as the page shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct McpOAuth {
    /// `authorization_code` (a person signs in), `client_credentials` or `enterprise`.
    pub grant: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub issuer: Option<String>,
    pub scopes: Vec<String>,
}

/// What the engine says a server is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct McpStatus {
    /// `connected`, `disconnected`, `connecting`, `auth_required`, `failed`, `denied`, `untrusted`,
    /// `disabled` (switched off) or `not_started` (its pair has not connected yet).
    pub state: String,
    pub tool_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub error: Option<String>,
    /// What the engine cut from its tool list to keep to the limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub limited: Option<String>,
    pub auth: McpAuthStatus,
}

/// A server's sign-in. Never a token.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct McpAuthStatus {
    /// `none`, `required`, `authorizing`, `authorized` or `failed`.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub issuer: Option<String>,
    pub scopes: Vec<String>,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub error: Option<String>,
}

/// `settings.mcp.authorize`'s answer: where to send the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct McpAuthStarted {
    pub flow_id: String,
    pub authorize_url: String,
    /// `external`: the browser comes back to this process (`callback`), which finishes the sign-in.
    /// `loopback`: it comes back to a port the engine listens on, on the engine's machine.
    pub redirect: String,
    /// Where the browser comes back to, when it is here: the path of this extension's route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub callback: Option<String>,
    /// Seconds the engine waits for the browser.
    pub expires_in: u64,
}

/// How a sign-in ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct McpAuthDone {
    pub server: String,
    pub ok: bool,
    /// The scopes granted.
    pub scopes: Vec<String>,
    /// Why it did not, in words a person can read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub error: Option<String>,
}

/// `settings.mcp.context`'s answer: the MCP part of what a session's next model call carries — each
/// server's tools and the instructions it gave, as the model sees them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct McpContext {
    /// Every server with a tool or instructions in the request, and every server the engine lists,
    /// by name.
    pub servers: Vec<McpContextServer>,
    /// The engine's estimate of the instructions block's size, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub instructions_tokens: Option<u64>,
}

/// One server in a session's request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct McpContextServer {
    pub name: String,
    /// Its tools the model is offered now (`mcp__<server>__<tool>`); a tool the engine defers until
    /// `ToolSearch` loads it is not among them.
    pub tools: Vec<McpContextTool>,
    /// The server's own instructions, as the model reads them under its heading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub status: Option<McpStatus>,
}

/// One tool in the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct McpContextTool {
    pub name: String,
    /// The engine's estimate of its description and schema.
    pub tokens: u64,
    pub schema_bytes: u64,
}
