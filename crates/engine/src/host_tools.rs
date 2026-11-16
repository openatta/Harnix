//! One call of a tool a link declared: run by the tool, answered with
//! `session.respondToolCall`, its progress told with `session.reportToolProgress`.
//!
//! A call made by a delegate is the delegate's: its session is the delegate's own. The chain it
//! carries is the root session's turn, which the delegate's call is part of. Its project is the
//! root session's.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use futures::FutureExt as _;

use harnix_contract::sync::lock;
use harnix_engine_api::{Cancel, Permission, Progress, SessionRef, Tool, ToolCx, ToolError, ToolOutput};

use crate::host::{HostSide, Wired};
use crate::translate::{self, HostToolCall};

pub(crate) async fn run(host: Arc<HostSide>, wire: Arc<dyn Wired>, tool: Arc<dyn Tool>, call: HostToolCall, cancel: Cancel) {
    // Asked again of the tool itself: a call it refuses does not run, whatever let the engine
    // make it (a permission mode, an allow rule), and nor does one it would have asked about in
    // a session nobody answers.
    let refused = match permission(&host, &tool, &call.input) {
        Permission::Deny(why) => Some(why),
        Permission::Ask(_) if host.nobody_asks(&call.name) => Some("nobody is there to answer whether it may run".to_string()),
        _ => None,
    };
    if let Some(why) = refused {
        let out = Ok(ToolOutput::error(format!("refused: {why}")));
        let _ = host.answer(&*wire, translate::SESSION_RESPOND_TOOL_CALL, translate::tool_answer(&call.session, &call.call_id, out)).await;
        host.done(&call.call_id);
        return;
    }
    let project = host.project_of(&*wire, &call.session).await;
    let chain = host.chains().chain(&SessionRef::new(host.instance(), &call.session));
    let session = call.agent_session.as_deref().unwrap_or(&call.session);
    let cx = ToolCx::new(session, &call.tool_use_id)
        .with_instance(host.instance())
        .with_chain(chain)
        .with_project(project)
        // The engine says only whether a delegate made the call.
        .with_depth(call.agent_session.is_some() as u32)
        .with_cancel(cancel.clone())
        .with_progress(progress(wire.clone(), &call.session, &call.call_id));
    let out = if cancel.is_cancelled() {
        Err(ToolError::Cancelled)
    } else {
        // A tool that panics fails its call, rather than leave the turn waiting for it.
        AssertUnwindSafe(tool.call(cx, call.input)).catch_unwind().await.unwrap_or_else(|_| {
            tracing::error!(instance = %host.instance(), tool = %call.name, "a tool panicked in a call");
            Err(ToolError::Failed(format!("the tool `{}` failed; the host's log says where", call.name)))
        })
    };
    if let Ok(out) = &out {
        if out.structured().is_some() {
            tracing::debug!(tool = %call.name, "a tool's structured content has nowhere to go in the engine's answer; left out");
        }
    }
    // Answered even once taken back: the engine takes it and does nothing with it. One that does
    // not go is the engine's to end when it times out.
    let _ = host.answer(&*wire, translate::SESSION_RESPOND_TOOL_CALL, translate::tool_answer(&call.session, &call.call_id, out)).await;
    host.done(&call.call_id);
}

/// What `tool` says of a call of `input`. A permission check that panics refuses the call: a tool
/// whose check cannot be run does not run, and is not asked about either — a person's yes would
/// meet the same check when the call comes. It is called on the link's reader too (`permits.rs`),
/// which a panic must not end.
pub(crate) fn permission(host: &HostSide, tool: &Arc<dyn Tool>, input: &serde_json::Value) -> Permission {
    std::panic::catch_unwind(AssertUnwindSafe(|| tool.permission(input))).unwrap_or_else(|_| {
        tracing::error!(instance = %host.instance(), tool = %tool.spec().name(), "a tool's permission check panicked; the call is refused");
        Permission::Deny("its permission check failed; the host's log says where".into())
    })
}

/// A call's progress, to the engine: the latest of what the tool said while one went out, so a
/// tool that says a lot does not queue it up. Sent from a task, never from the tool's own thread.
fn progress(wire: Arc<dyn Wired>, session: &str, call_id: &str) -> Progress {
    let latest: Arc<StdMutex<Option<String>>> = Arc::default();
    let sending = Arc::new(AtomicBool::new(false));
    let (session, call_id) = (session.to_string(), call_id.to_string());
    let runtime = tokio::runtime::Handle::current();
    Progress::new(move |text| {
        *lock(&latest) = Some(text.to_string());
        if sending.swap(true, Ordering::AcqRel) {
            return;
        }
        let (wire, latest, sending) = (wire.clone(), latest.clone(), sending.clone());
        let (session, call_id) = (session.clone(), call_id.clone());
        runtime.spawn(async move {
            loop {
                let next = lock(&latest).take();
                match next {
                    Some(message) => {
                        // Lost progress stays lost: the call's answer is what counts.
                        let _ = wire.send(translate::SESSION_REPORT_TOOL_PROGRESS, translate::tool_progress(&session, &call_id, &message)).await;
                    }
                    None => {
                        sending.store(false, Ordering::Release);
                        // Said between the take and the store: this task sends it, unless another
                        // has started already.
                        if lock(&latest).is_some() && !sending.swap(true, Ordering::AcqRel) {
                            continue;
                        }
                        return;
                    }
                }
            }
        });
    })
}
