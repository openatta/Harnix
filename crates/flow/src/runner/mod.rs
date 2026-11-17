//! The runner: carrying out what the kernel hands back, in sessions a host provides.
//!
//! The kernel decides what a run needs next and says so in an `Outcome`'s work items; the runner
//! does it — opens a session where a station says, tells it the protocol layer, the rules and the
//! skill, takes what it hands in, answers its permission questions from the task's permissions,
//! runs gates, posts hooks, projects the run into its project — and hands the result back to the
//! kernel. It knows sessions only through [`SessionHost`] (`host.rs`): the host opens them,
//! sends to them, streams their frames and answers their questions in the runner's words.
//!
//! ```text
//! host.rs      SessionHost and what goes over it: OpenSpec, Frame, TurnEnd, Decision, HostError
//! testing.rs   ScriptedHost: a host played from a script, for tests and flowctl
//! ```

pub mod host;
pub mod testing;

pub use host::{
    Decision, Frame, HostError, Note, OpenSpec, Opened, SessionHost, SessionId, SessionInfo, ToolSpec, TurnEnd, Watch,
};
pub use testing::{ScriptedHost, Turn};
