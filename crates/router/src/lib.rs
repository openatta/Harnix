//! The one route table, and the router that decides and answers by it.
//!
//! Every method, topic and route is a row ([`Row`]): its owner, how far it is open, what answers
//! it. A call is decided once, by its row, and handed to that row's handler; nothing dispatches
//! on what a name starts with, and nothing behind the router decides again.
//!
//! Every change to the table after assembly goes through [`Router`], which puts it on the
//! record: an application's names installed and withdrawn, a grant made and revoked, a failed
//! module's rows withdrawn.
//!
//! Only the assembly depends on this crate: it is the one that installs rows, and everything
//! else is called through [`harnix_kernel::Dispatch`] and asks [`harnix_kernel::RouteDecider`].

pub mod names;
mod snapshot;
mod table;

pub use names::{route, topic};
pub use table::{Row, RowKind, RouteTable, TableError, OUTSIDE_HUB};

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use harnix_authz::{Audit, Change, Decision};
use harnix_contract::sync::{read, write};
use harnix_contract::{RpcError, Subject};
use harnix_kernel::{CallContext, Dispatch, Owner, RouteDecider, Rpc};
use serde_json::Value;

/// The method this layer answers itself.
///
/// "What may I call" is a question about the table, so it is answered where the table is rather
/// than by handing the table to something else. A client uses it to grey out what it cannot
/// reach instead of discovering it by being refused, and a contribution's filtered RPC client is
/// built from it — so the message is the same whichever side catches the call.
pub const REACHABLE: &str = "harnix.reachable";

/// Decides every call by its row, records the decision, and hands the call to the row's handler.
pub struct Router {
    /// Written when an application comes and goes, or a module fails to start; read on every
    /// call.
    table: RwLock<RouteTable>,
    audit: Arc<Audit>,
}

impl Router {
    pub fn new(table: RouteTable, audit: Arc<Audit>) -> Self {
        Self {
            table: RwLock::new(table),
            audit,
        }
    }

    pub fn audit(&self) -> &Arc<Audit> {
        &self.audit
    }

    pub fn table(&self) -> RwLockReadGuard<'_, RouteTable> {
        read(&self.table)
    }

    /// Written only here, where every change is audited.
    fn table_mut(&self) -> RwLockWriteGuard<'_, RouteTable> {
        write(&self.table)
    }

    /// Put a runtime application's rows in the table, all or none, and make its subject a
    /// runtime principal reaching `hub` and its own names ([`RouteTable::reach_only`]) — in one
    /// write. Put on the record who did: one `declare` entry per name, or one `refuse` entry
    /// naming the first name that did not take. See [`RouteTable::install_app`] for what is
    /// refused.
    pub fn install_app(&self, by: &Subject, app: &str, rows: Vec<Row>, hub: &[String]) -> Result<(), TableError> {
        let names: Vec<String> = rows.iter().map(|r| r.name().to_string()).collect();
        let result = {
            let mut table = self.table_mut();
            let installed = table.install_app(app, rows);
            if installed.is_ok() {
                table.reach_only(app, hub.iter().cloned());
            }
            installed
        };
        match &result {
            Ok(()) => {
                for name in &names {
                    self.audit
                        .record_change(by, name, Change::Declare, Some("declared by a runtime application"));
                }
            }
            Err(e) => {
                let name = match e {
                    TableError::Taken(n) | TableError::Namespace(n) | TableError::HubOwned(n) | TableError::Unknown(n) => {
                        n.clone()
                    }
                    TableError::Overlap { app, .. } => names::app_name(app, "*"),
                };
                self.audit.record_change(by, &name, Change::Refuse, Some(change_refusal(e)));
            }
        }
        result
    }

    /// Take an owner's rows back out after assembly — a runtime application withdrawn, a module
    /// that failed to start — each on the record under `by`, with `why`. A call decided after
    /// this returns is decided as unknown, and an application withdrawn reaches nothing as
    /// itself.
    pub fn withdraw(&self, by: &Subject, owner: &Owner, why: &'static str) -> Vec<String> {
        let names = self.table_mut().withdraw(owner);
        for name in &names {
            self.audit.record_change(by, name, Change::Withdraw, Some(why));
        }
        names
    }

    /// Open one existing row to one module ([`RouteTable::grant`]), and put it on the record
    /// under `by`, with `why`.
    pub fn grant(&self, by: &Subject, method: &str, module: &str, why: &'static str) -> Result<(), TableError> {
        let granted = self.table_mut().grant(method, module);
        match &granted {
            Ok(()) => self.audit.record_change(by, method, Change::Grant, Some(why)),
            Err(e) => self.audit.record_change(by, method, Change::Refuse, Some(change_refusal(e))),
        }
        granted
    }

    /// Take back what [`Router::grant`] gave `module` ([`RouteTable::revoke`]), on the record
    /// under `by`, with `why`.
    pub fn revoke(&self, by: &Subject, method: &str, module: &str, why: &'static str) {
        self.table_mut().revoke(method, module);
        self.audit.record_change(by, method, Change::Revoke, Some(why));
    }

    /// Methods this subject may call, for a client that wants to grey out what it cannot use
    /// rather than discover it by being refused.
    pub fn reachable(&self, subject: &Subject) -> Vec<String> {
        self.table()
            .reachable(subject)
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Every row, in the shape the method-table snapshot pins.
    pub fn snapshot(&self) -> Value {
        snapshot::of(&self.table())
    }

    /// Subscribing to a host topic is decided as `topic:<name>`.
    fn decide_topic(table: &RouteTable, subject: &Subject, params: &Value) -> Decision {
        let Some(name) = params.get("topic").and_then(Value::as_str) else {
            return Decision::Allow; // the hub answers a missing topic itself
        };
        table.decide(subject, &topic(name))
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

#[async_trait::async_trait]
impl Dispatch for Router {
    async fn call(&self, cx: CallContext, method: &str, params: Value) -> Rpc<Value> {
        // Read out of the call, not known by this layer. A durable sink needs somewhere to file
        // an entry; deciding whether to allow the call does not.
        let session = params
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        if method == REACHABLE {
            self.audit.record_call(&cx, method, Decision::Allow, None, session);
            let table = self.table();
            let topics: Vec<&str> = table.reachable_topics(&cx.subject);
            return Ok(serde_json::json!({
                "subject": cx.subject,
                "methods": table.reachable(&cx.subject),
                "topics": topics,
            }));
        }
        // One read: the decision and what answers it come from the same row.
        let (decision, handler, subscribing) = {
            let table = self.table();
            let subscribing = names::DECIDED_BY_TOPIC.contains(&table.resolve(method));
            let mut decision = table.decide(&cx.subject, method);
            if decision == Decision::Allow && subscribing {
                decision = Self::decide_topic(&table, &cx.subject, &params);
            }
            (decision, table.row(method).and_then(|row| row.handler().cloned()), subscribing)
        };
        self.audit.record_call(&cx, method, decision, refusal(decision), session);
        match decision {
            // The handler is a clone out of the table: withdrawing the row while this runs does
            // not cut it short.
            Decision::Allow => match handler {
                Some(handler) => handler.call(cx, params).await,
                None => Err(RpcError::not_found(format!("method `{method}` is not served here"))),
            },
            // Refused on purpose, with the reason in the error. Silence here would read as a
            // Harnix bug to whoever hit it.
            Decision::Refuse(reason) => Err(RpcError::refused(reason)),
            Decision::Unknown if subscribing => {
                let name = params.get("topic").and_then(Value::as_str).unwrap_or("");
                Err(RpcError::not_found(format!("there is no topic `{name}`")))
            }
            Decision::Unknown => Err(RpcError::not_found(format!(
                "method `{method}` is not reachable by {}",
                cx.subject.label()
            ))),
        }
    }
}

impl RouteDecider for Router {
    /// Decide a module route, `/apps/<module>/…`, and record it. The transport asks; it does not
    /// know what a module is.
    fn decide_route(&self, subject: &Subject, module: &str, path: &str) -> Result<(), RpcError> {
        let name = route(module);
        let decision = self.table().decide(subject, &name);
        self.audit.record(subject, &format!("{name}{path}"), decision, refusal(decision), None);
        match decision {
            Decision::Allow => Ok(()),
            Decision::Refuse(reason) => Err(RpcError::refused(reason)),
            Decision::Unknown => Err(RpcError::not_found(format!("no module `{module}` serves routes here"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use harnix_authz::{access_for, Principal};
    use harnix_contract::codes;
    use harnix_kernel::{Access, Handler, KernelPart};
    use serde_json::json;

    use super::*;

    /// Answers with what it was handed: the call's subject, parent and params.
    #[derive(Default)]
    struct Seen(AtomicUsize);

    #[async_trait::async_trait]
    impl Handler for Seen {
        async fn call(&self, cx: CallContext, params: Value) -> Rpc<Value> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"subject": cx.subject, "parent": cx.parent.map(|p| p.get()), "params": params}))
        }
    }

    fn device() -> Subject {
        Subject::Device { id: "d".into() }
    }

    fn router() -> (Router, Arc<Seen>) {
        let seen = Arc::new(Seen::default());
        let mut table = RouteTable::new();
        table
            .install(
                &Owner::Kernel(KernelPart::Hub),
                vec![
                    Row::method("harnix.hello", Access::device(), seen.clone()),
                    Row::method("harnix.subscribe", Access::device(), seen.clone()),
                    Row::method("config.set", Access::refused("not for devices"), seen.clone()),
                    Row::closed("session.run_turn", Access::hub_owned("the hub's")),
                    Row::topic("harnix.instances", Access::device()),
                    Row::topic("harnix.secret", Access::kernel()),
                ],
            )
            .unwrap();
        table
            .install(&Owner::Kernel(KernelPart::Router), vec![Row::closed(REACHABLE, Access::device())])
            .unwrap();
        table
            .install(&Owner::extension("w"), vec![Row::route("w", access_for(&[Principal::Kernel]))])
            .unwrap();
        (Router::new(table, Arc::new(Audit::default())), seen)
    }

    #[tokio::test]
    async fn a_call_is_decided_once_audited_and_handed_to_its_row() {
        let (router, seen) = router();
        let root = CallContext::root(device(), None);
        let id = root.call_id.get();
        let answer = router.call(root, "harnix.hello", json!({"x": 1})).await.unwrap();
        assert_eq!(answer["params"], json!({"x": 1}));
        assert_eq!(answer["subject"], json!({"kind": "device", "id": "d"}));
        let entry = &router.audit().recent(1)[0];
        assert_eq!((entry.method.as_str(), entry.decision, entry.call_id), ("harnix.hello", "allow", Some(id)));

        // Refused with the row's reason, never reaching the handler; an unknown name is not found.
        let e = router.call(CallContext::root(device(), None), "config.set", json!({})).await.unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (codes::REFUSED, "not for devices"));
        let e = router.call(CallContext::root(Subject::Kernel, None), "session.run_turn", json!({})).await.unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
        let e = router.call(CallContext::root(device(), None), "harnix.nothing", json!({})).await.unwrap_err();
        assert_eq!(e.code, codes::METHOD_NOT_FOUND);
        assert_eq!(seen.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_call_made_while_handling_another_is_its_child_in_the_audit() {
        let (router, _) = router();
        let outer = CallContext::root(device(), None);
        let inner = outer.child("system", false);
        let answer = router.call(inner, "harnix.hello", json!({})).await.unwrap();
        assert_eq!(answer["parent"], json!(outer.call_id.get()));
        assert_eq!(router.audit().recent(1)[0].parent, Some(outer.call_id.get()));
        assert_eq!(router.audit().recent(1)[0].subject, "module:system");
    }

    #[tokio::test]
    async fn subscribing_is_decided_by_the_topic_too() {
        let (router, _) = router();
        let sub = |topic: &str| json!({"topic": topic});
        assert!(router.call(CallContext::root(device(), None), "harnix.subscribe", sub("harnix.instances")).await.is_ok());
        let e = router.call(CallContext::root(device(), None), "harnix.subscribe", sub("harnix.secret")).await.unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
        let e = router.call(CallContext::root(device(), None), "harnix.subscribe", sub("harnix.none")).await.unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (codes::METHOD_NOT_FOUND, "there is no topic `harnix.none`"));
    }

    #[tokio::test]
    async fn reachable_is_answered_here_from_the_rows() {
        let (router, _) = router();
        let reach = router.call(CallContext::root(device(), None), REACHABLE, json!({})).await.unwrap();
        assert_eq!(reach["methods"], json!(["harnix.hello", "harnix.reachable", "harnix.subscribe"]));
        assert_eq!(reach["topics"], json!(["harnix.instances"]));
    }

    #[test]
    fn a_route_is_decided_and_recorded_with_its_path() {
        let (router, _) = router();
        assert_eq!(router.decide_route(&device(), "w", "/x").unwrap_err().code, codes::REFUSED);
        assert!(router.decide_route(&Subject::Kernel, "w", "/x").is_ok());
        assert_eq!(router.decide_route(&device(), "none", "/").unwrap_err().code, codes::METHOD_NOT_FOUND);
        assert_eq!(router.audit().recent(3)[2].method, "route:w/x");
    }
}
