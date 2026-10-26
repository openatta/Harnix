//! The one place a call is admitted.
//!
//! Adding a method defaults to refusing it: [`MethodTable`] is an allow-list,
//! and a name that is not in it is refused by falling off the end rather than
//! by anyone remembering to write a rule.
//!
//! Three kinds of subject, judged differently (concept_and_architecture.md
//! §3.4, design D27). A device is a person, and in a single-user process the
//! answer to "may this person do it" is nearly always yes. A module is code
//! the application author linked in: trusted like the kernel, and still a
//! subject of its own, so the table says what each one may call and the
//! audit says which one did. The kernel is this binary.
//!
//! Module routes (`/apps/<id>/`) and host topics are decided here too, as
//! entries in the same table — one admission point for every surface.
//!
//! Authorization happens at the method, not at the connection. A multi-channel
//! topology gives one subject several connections; hanging the decision on a
//! connection would give several answers to one question.
//!
//! This layer does not know what a frame looks like or which channel carried
//! it — it depends on `harnix-contract` and nothing else.

mod audit;
mod devices;
mod table;

pub use audit::{Audit, AuditEntry, AuditSink, Decision};
pub use devices::challenge as devices_challenge;
pub use devices::{Device, Devices, PairError, PAIRING_TTL};
pub use table::{
    route, topic, MethodTable, Principal, Reach, Rule, TableError, ENGINE_REFUSALS, HUB_OWNED,
};

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use harnix_contract::{Gate, RpcError, Subject};
use serde_json::Value;

/// Wraps the thing that actually answers, and lets nothing past that the
/// table did not name.
pub struct Authorizer {
    /// Written during assembly — modules declare, and a module that fails to
    /// migrate or start has its entries retracted — and read on every call.
    table: RwLock<MethodTable>,
    inner: Arc<dyn Gate>,
    audit: Arc<Audit>,
}

impl Authorizer {
    pub fn new(table: MethodTable, inner: Arc<dyn Gate>, audit: Arc<Audit>) -> Self {
        Self {
            table: RwLock::new(table),
            inner,
            audit,
        }
    }

    pub fn audit(&self) -> &Arc<Audit> {
        &self.audit
    }

    pub fn table(&self) -> RwLockReadGuard<'_, MethodTable> {
        self.table.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn table_mut(&self) -> RwLockWriteGuard<'_, MethodTable> {
        self.table.write().unwrap_or_else(|e| e.into_inner())
    }

    /// Methods this subject may call, for a client that wants to grey out
    /// what it cannot use rather than discover it by being refused.
    pub fn reachable(&self, subject: &Subject) -> Vec<String> {
        self.table()
            .reachable(subject)
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Decide a module route, `/apps/<module>/…`, and record it. The
    /// transport asks; it does not know what a module is.
    pub fn decide_route(&self, subject: &Subject, module: &str, path: &str) -> Result<(), RpcError> {
        let name = route(module);
        let decision = self.table().decide(subject, &name);
        self.audit.record(subject, &format!("{name}{path}"), decision, refusal(decision), None);
        match decision {
            Decision::Allow => Ok(()),
            Decision::Refuse(reason) => Err(RpcError::refused(reason)),
            Decision::Unknown => Err(RpcError::not_found(format!("no module `{module}` serves routes here"))),
        }
    }

    /// Subscribing to a host topic is decided as `topic:<name>`.
    fn decide_topic(&self, subject: &Subject, params: &Value) -> Decision {
        let Some(name) = params.get("topic").and_then(Value::as_str) else {
            return Decision::Allow; // the hub answers a missing topic itself
        };
        self.table().decide(subject, &topic(name))
    }
}

fn refusal(decision: Decision) -> Option<&'static str> {
    match decision {
        Decision::Refuse(reason) => Some(reason),
        _ => None,
    }
}

/// The method this layer answers itself.
///
/// "What may I call" is an authorization question, so it is answered where
/// the answer lives rather than by handing the table to something else. A
/// client uses it to grey out what it cannot reach instead of discovering it
/// by being refused, and a contribution's filtered RPC client is built from
/// it — so the message is the same whichever side catches the call.
pub const REACHABLE: &str = "harnix.reachable";

/// The two methods whose decision depends on what they name: a topic is
/// subscribed to only if the subject reaches `topic:<name>`.
const SUBSCRIBE: &[&str] = &["harnix.subscribe", "harnix.unsubscribe"];

#[async_trait::async_trait]
impl Gate for Authorizer {
    async fn call(
        &self,
        subject: &Subject,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        // Read out of the call, not known by this layer. A durable sink needs
        // somewhere to file an entry; deciding whether to allow the call does
        // not.
        let session = params
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        if method == REACHABLE {
            self.audit
                .record(subject, method, Decision::Allow, None, session);
            let topics: Vec<String> = self
                .table()
                .reachable_topics(subject)
                .into_iter()
                .map(str::to_string)
                .collect();
            return Ok(serde_json::json!({
                "subject": subject,
                "methods": self.reachable(subject),
                "topics": topics,
            }));
        }
        let mut decision = self.table().decide(subject, method);
        if decision == Decision::Allow && SUBSCRIBE.contains(&method) {
            decision = self.decide_topic(subject, &params);
        }
        match decision {
            Decision::Allow => {
                self.audit
                    .record(subject, method, Decision::Allow, None, session);
                self.inner.call(subject, method, params).await
            }
            // Refused on purpose, with the reason in the error. Silence here
            // would read as a Harnix bug to whoever hit it.
            Decision::Refuse(reason) => {
                self.audit.record(
                    subject,
                    method,
                    Decision::Refuse(reason),
                    Some(reason),
                    session,
                );
                Err(RpcError::refused(reason))
            }
            Decision::Unknown => {
                self.audit
                    .record(subject, method, Decision::Unknown, None, session);
                if SUBSCRIBE.contains(&method) {
                    let name = params.get("topic").and_then(Value::as_str).unwrap_or("");
                    return Err(RpcError::not_found(format!("there is no topic `{name}`")));
                }
                Err(RpcError::not_found(format!(
                    "method `{method}` is not reachable by {}",
                    subject.label()
                )))
            }
        }
    }
}
