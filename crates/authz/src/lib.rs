//! The one place a call is admitted.
//!
//! Adding a method defaults to refusing it: [`MethodTable`] is an allow-list,
//! and a name that is not in it is refused by falling off the end rather than
//! by anyone remembering to write a rule.
//!
//! Three kinds of subject, judged differently (concept_and_architecture.md
//! §3.4). A device is a person, and in a single-user process the
//! answer to "may this person do it" is nearly always yes. A module is code
//! the application author linked in: trusted like the kernel, and still a
//! subject of its own, so the table says what each one may call and the
//! audit says which one did. The kernel is this binary.
//!
//! A runtime application acting as itself is a module subject too,
//! `module:<id>`, with one difference the table holds rather than anyone in
//! front of it: it is a runtime principal, which reaches only what its `hub`
//! capability lists, and its own names ([`MethodTable::reach_only`]). Trust
//! by origin: code that came in a package does not reach what every module
//! may just because a rule opens it to every module.
//!
//! Every change to the table after assembly goes through [`Authorizer`],
//! which puts it on the record: an application's names declared and
//! withdrawn, a grant made and revoked, a failed module's names retracted.
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

pub use audit::{Audit, AuditEntry, AuditSink, Change, Decision};
pub use devices::challenge as devices_challenge;
pub use devices::{tokens_equal, Device, Devices, PairError, Persist, PAIRING_TTL};
pub use table::{
    apps_overlap, route, topic, valid_app_id, MethodTable, Principal, Reach, Rule, TableError,
    ENGINE_REFUSALS, HUB_OWNED, OUTSIDE_HUB,
};

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use harnix_contract::sync::{read, write};
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
        read(&self.table)
    }

    /// Written only here, where every change is audited.
    pub(crate) fn table_mut(&self) -> RwLockWriteGuard<'_, MethodTable> {
        write(&self.table)
    }

    /// Put a runtime application's names in the table, all or none, and make
    /// its subject a runtime principal reaching `hub` and its own names
    /// ([`MethodTable::reach_only`]) — in one write. Put on the record who did:
    /// one `declare` entry per name, or one `refuse` entry naming the first
    /// name that did not take. See [`MethodTable::declare_app`] for what is
    /// refused.
    pub fn declare_app(
        &self,
        by: &Subject,
        app: &str,
        rows: &[(String, Vec<Principal>)],
        hub: &[String],
    ) -> Result<(), TableError> {
        let result = {
            let mut table = self.table_mut();
            let declared = table.declare_app(app, rows);
            if declared.is_ok() {
                table.reach_only(app, hub.iter().cloned());
            }
            declared
        };
        match &result {
            Ok(()) => {
                for (name, _) in rows {
                    self.audit
                        .record_change(by, name, Change::Declare, Some("declared by a runtime application"));
                }
            }
            Err(e) => {
                let name = match e {
                    TableError::Taken(n) | TableError::Namespace(n) | TableError::HubOwned(n) | TableError::Unknown(n) => {
                        n.clone()
                    }
                    TableError::Overlap { app, .. } => format!("app.{app}.*"),
                };
                self.audit.record_change(by, &name, Change::Refuse, Some(change_refusal(e)));
            }
        }
        result
    }

    /// Take a runtime application's names back out, and put each on the
    /// record. A call decided after this returns is decided as unknown, and the
    /// application itself reaches nothing.
    pub fn withdraw_app(&self, by: &Subject, app: &str) -> Vec<String> {
        let names = self.table_mut().withdraw_app(app);
        for name in &names {
            self.audit
                .record_change(by, name, Change::Withdraw, Some("withdrawn with its runtime application"));
        }
        names
    }

    /// Open one existing entry to one module ([`MethodTable::grant`]), and put
    /// it on the record under `by`, with `why`.
    pub fn grant(&self, by: &Subject, method: &str, module: &str, why: &'static str) -> Result<(), TableError> {
        let granted = self.table_mut().grant(method, module);
        match &granted {
            Ok(()) => self.audit.record_change(by, method, Change::Grant, Some(why)),
            Err(e) => self.audit.record_change(by, method, Change::Refuse, Some(change_refusal(e))),
        }
        granted
    }

    /// Take back what [`Authorizer::grant`] gave `module` ([`MethodTable::revoke`]),
    /// on the record under `by`, with `why`.
    pub fn revoke(&self, by: &Subject, method: &str, module: &str, why: &'static str) {
        self.table_mut().revoke(method, module);
        self.audit.record_change(by, method, Change::Revoke, Some(why));
    }

    /// Take a module's names back out after assembly — it failed to start
    /// ([`MethodTable::retract`]) — each on the record under `by`, with `why`.
    pub fn retract(&self, by: &Subject, names: &[String], why: &'static str) {
        self.table_mut().retract(names);
        for name in names {
            self.audit.record_change(by, name, Change::Withdraw, Some(why));
        }
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

fn change_refusal(e: &TableError) -> &'static str {
    match e {
        TableError::Taken(_) => "the name is already in the authorization table",
        TableError::Namespace(_) => "outside the application's own namespace",
        TableError::Overlap { .. } => "the application id overlaps names already in the authorization table",
        TableError::HubOwned(_) | TableError::Unknown(_) => "not a name that may be declared or granted",
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
const SUBSCRIBE: &[&str] = table::DECIDED_BY_TOPIC;

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
