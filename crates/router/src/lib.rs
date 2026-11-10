//! The one route table, and the router that decides and answers by it.
//!
//! Every method, topic and route is a row ([`Row`]): its owner, how far it is open, what answers
//! it. A call is decided once, by its row, and handed to that row's handler; nothing dispatches
//! on what a name starts with, and nothing behind the router decides again.
//!
//! What a caller holds is not in the table: [`Router`] decides every call by the row and by a
//! [`harnix_kernel::GrantBook`] — capabilities, and how far an extension is trusted — and narrows
//! a package's call, and a page's in a frame, to its own space. Every change to the table after
//! assembly goes through [`Router`], which puts it on the record: an application's names
//! installed and withdrawn, a failed module's rows withdrawn.
//!
//! Only the assembly depends on this crate: it is the one that installs rows, and everything
//! else is called through [`harnix_kernel::Dispatch`] and asks [`harnix_kernel::RouteDecider`].

pub mod names;
mod snapshot;
mod table;

pub use names::{route, topic};
pub use table::{IdKind, Row, RowKind, RouteTable, TableError, OUTSIDE_HUB, UNREGISTERED};

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use harnix_authz::{Audit, Change, Decision};
use harnix_contract::sync::{read, write};
use harnix_contract::{RpcError, Subject};
use harnix_kernel::{CallContext, Dispatch, GrantBook, Owner, RouteDecider, Rpc};
use serde_json::{json, Value};

/// The method this layer answers itself.
///
/// "What may I call" is a question about the table, so it is answered where the table is rather
/// than by handing the table to something else. A client uses it to grey out what it cannot
/// reach instead of discovering it by being refused, and a contribution's filtered RPC client is
/// built from it — so the message is the same whichever side catches the call.
pub const REACHABLE: &str = "harnix.reachable";

/// Why a page in a frame is refused a name its interface does not list.
pub const OUTSIDE_FRAME: &str = "not in the extension interface's `hub`";
/// Why a call claiming a page's extension is refused when that extension has no framed interface.
pub const NO_FRAME: &str = "this extension has no interface in a frame";

/// Decides every call by its row and what the caller holds, records the decision, and hands the
/// call to the row's handler.
pub struct Router {
    /// Written when an application comes and goes, or a module fails to start; read on every
    /// call.
    table: RwLock<RouteTable>,
    grants: Arc<dyn GrantBook>,
    audit: Arc<Audit>,
}

impl Router {
    pub fn new(table: RouteTable, grants: Arc<dyn GrantBook>, audit: Arc<Audit>) -> Self {
        Self {
            table: RwLock::new(table),
            grants,
            audit,
        }
    }

    pub fn audit(&self) -> &Arc<Audit> {
        &self.audit
    }

    pub fn grants(&self) -> &Arc<dyn GrantBook> {
        &self.grants
    }

    pub fn table(&self) -> RwLockReadGuard<'_, RouteTable> {
        read(&self.table)
    }

    /// Written only here, where every change is audited.
    fn table_mut(&self) -> RwLockWriteGuard<'_, RouteTable> {
        write(&self.table)
    }

    /// Put a runtime application's rows in the table, all or none, its id registered as a
    /// package's ([`RouteTable::install_app`]). Put on the record who did: one `declare` entry
    /// per name, or one `refuse` entry naming the first name that did not take. What it may reach
    /// as itself is its consent, in the grants.
    pub fn install_app(&self, by: &Subject, app: &str, rows: Vec<Row>) -> Result<(), TableError> {
        let names: Vec<String> = rows.iter().map(|r| r.name().to_string()).collect();
        let result = self.table_mut().install_app(app, rows);
        match &result {
            Ok(()) => {
                for name in &names {
                    self.audit
                        .record_change(by, name, Change::Declare, Some("declared by a runtime application"));
                }
            }
            Err(e) => {
                let name = match e {
                    TableError::Taken(n)
                    | TableError::Namespace(n)
                    | TableError::HubOwned(n)
                    | TableError::Unknown(n)
                    | TableError::Capability(n)
                    | TableError::NotOwner { name: n, .. } => n.clone(),
                    TableError::NamespaceTaken { id, .. } => names::app_name(id, "*"),
                };
                self.audit.record_change(by, &name, Change::Refuse, Some(change_refusal(e)));
            }
        }
        result
    }

    /// Take an owner's rows back out after assembly — a runtime application withdrawn, a module
    /// that failed to start — each on the record under `by`, with `why`. A call decided after
    /// this returns is decided as unknown.
    pub fn withdraw(&self, by: &Subject, owner: &Owner, why: &'static str) -> Vec<String> {
        let names = self.table_mut().withdraw(owner);
        for name in &names {
            self.audit.record_change(by, name, Change::Withdraw, Some(why));
        }
        names
    }

    /// Methods this subject may call, for a client that wants to grey out what it cannot use
    /// rather than discover it by being refused. Not narrowed by space: that depends on the call.
    pub fn reachable(&self, subject: &Subject) -> Vec<String> {
        self.table()
            .reachable(subject, &*self.grants)
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Every row, in the shape the method-table snapshot pins.
    pub fn snapshot(&self) -> Value {
        snapshot::of(&self.table())
    }

    /// Subscribing to a host topic is decided as `topic:<name>`.
    fn decide_topic(&self, table: &RouteTable, subject: &Subject, params: &Value) -> Decision {
        let Some(name) = params.get("topic").and_then(Value::as_str) else {
            return Decision::Allow; // the hub answers a missing topic itself
        };
        table.decide(subject, &*self.grants, &topic(name))
    }

    /// A device's call made by an extension's page in a frame: only to that extension's own names
    /// and what its interface lists. Its subscription is decided by the topic, as `topic:<name>`,
    /// the way its interface lists one.
    fn decide_frame(&self, table: &RouteTable, cx: &CallContext, method: &str, params: &Value) -> Decision {
        let Some(ext) = &cx.on_behalf_of else {
            return Decision::Allow;
        };
        let Some(calls) = self.grants.frame_calls(ext) else {
            return Decision::Refuse(NO_FRAME);
        };
        let listed = |name: &str| calls.contains(name);
        let allowed = match subscribed_topic(method, params) {
            Some(name) => listed(&topic(name)) || table.owns(ext, name),
            None => listed(method) || table.owns(ext, method),
        };
        match allowed {
            true => Decision::Allow,
            false => Decision::Refuse(OUTSIDE_FRAME),
        }
    }

    /// A package acting as itself, and a page in a frame, reach only their own space and its
    /// sessions ([`harnix_authz::narrow`]). A subscription by its topic's row, claiming the space
    /// in the topic's params.
    fn narrow(&self, table: &RouteTable, cx: &CallContext, method: &str, params: &Value) -> Decision {
        let (name, params) = match subscribed_topic(method, params) {
            Some(name) => (topic(name), params.get("params").unwrap_or(&Value::Null)),
            None => (method.to_string(), params),
        };
        let method = name.as_str();
        let Some(row) = table.row(method) else {
            return Decision::Allow;
        };
        let claimed = params.get("space").and_then(Value::as_str);
        let package = cx
            .subject
            .extension_id()
            .filter(|id| self.grants.trust(id).is_some_and(|t| t.is_consent()));
        for ext in package.into_iter().chain(cx.on_behalf_of.as_ref()) {
            if table.owns(ext, method) {
                continue;
            }
            let narrowed = harnix_authz::narrow(ext, row.access().scope(), claimed);
            if narrowed != Decision::Allow {
                return narrowed;
            }
        }
        Decision::Allow
    }
}

/// The topic a subscribe or unsubscribe is about; `None` for any other call.
fn subscribed_topic<'a>(method: &str, params: &'a Value) -> Option<&'a str> {
    names::DECIDED_BY_TOPIC
        .contains(&method)
        .then(|| params.get("topic").and_then(Value::as_str).unwrap_or(""))
}

fn change_refusal(e: &TableError) -> &'static str {
    match e {
        TableError::Taken(_) => "the name is already in the authorization table",
        TableError::Namespace(_) => "outside the application's own namespace",
        TableError::NamespaceTaken { .. } => "namespace_taken",
        TableError::NotOwner { .. } => "the name is in another extension's namespace",
        TableError::HubOwned(_) | TableError::Unknown(_) => "not a name that may be declared or listed",
        TableError::Capability(_) => "a capability nobody defined",
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
            let topics: Vec<&str> = table.reachable_topics(&cx.subject, &*self.grants);
            return Ok(json!({
                "subject": cx.subject,
                "methods": table.reachable(&cx.subject, &*self.grants),
                "topics": topics,
            }));
        }
        // One read: the decision and what answers it come from the same row.
        let (decision, handler, subscribing, missing) = {
            let table = self.table();
            let subscribing = names::DECIDED_BY_TOPIC.contains(&method);
            let mut decision = self.decide_frame(&table, &cx, method, &params);
            if decision == Decision::Allow {
                decision = table.decide(&cx.subject, &*self.grants, method);
            }
            if decision == Decision::Allow {
                decision = self.narrow(&table, &cx, method, &params);
            }
            if decision == Decision::Allow && subscribing {
                decision = self.decide_topic(&table, &cx.subject, &params);
            }
            let row = table.row(method);
            let missing = match decision {
                Decision::Refuse(_) => row
                    .and_then(|row| harnix_authz::missing_capability(&cx.subject, &*self.grants, row.access()))
                    .cloned(),
                _ => None,
            };
            (decision, row.and_then(|row| row.handler().cloned()), subscribing, missing)
        };
        self.audit.record_call(&cx, method, decision, refusal(decision), session);
        match decision {
            // The handler is a clone out of the table: withdrawing the row while this runs does
            // not cut it short.
            Decision::Allow => match handler {
                Some(handler) => handler.call(cx, params).await,
                None => Err(RpcError::not_found(format!("method `{method}` is not served here"))),
            },
            // Refused on purpose, with the reason in the error — and the capability, when one was
            // missing. Silence here would read as a Harnix bug to whoever hit it.
            Decision::Refuse(reason) => {
                let mut e = RpcError::refused(reason);
                if let Some(cap) = missing {
                    e.data = Some(json!({"capability": cap}));
                }
                Err(e)
            }
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
        let decision = self.table().decide(subject, &*self.grants, &name);
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
    use harnix_contract::s1::{caps, ExtId, PackageCarrier, RowScope, Trust};
    use harnix_kernel::{Access, GrantTable, Handler, KernelPart};
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
        Subject::device("d")
    }

    fn id(s: &str) -> ExtId {
        ExtId::parse(s).unwrap()
    }

    fn router() -> (Router, Arc<Seen>, Arc<GrantTable>) {
        let seen = Arc::new(Seen::default());
        let mut table = RouteTable::new();
        table
            .install(
                &Owner::Kernel(KernelPart::Hub),
                vec![
                    Row::method("harnix.hello", Access::device(), seen.clone()),
                    Row::method("harnix.topics.subscribe", Access::device(), seen.clone()),
                    Row::method("engine.config.set", Access::refused("not for devices"), seen.clone()),
                    Row::method("session.inspect", Access::capability(caps::ENGINE_INSPECT), seen.clone()),
                    Row::method("engine.session.get", Access::device().scoped(RowScope::Session), seen.clone()),
                    Row::method("sessions.list", Access::device().scoped(RowScope::Space), seen.clone()),
                    Row::closed("engine.session.run_turn", Access::hub_owned("the hub's")),
                    Row::topic("harnix.instances", Access::device()),
                    Row::topic("harnix.secret", Access::kernel()),
                ],
            )
            .unwrap();
        table
            .install(&Owner::Kernel(KernelPart::Router), vec![Row::closed(REACHABLE, Access::device())])
            .unwrap();
        table.install(&Owner::extension(id("w")), vec![Row::route("w", access_for(&[Principal::Kernel]))]).unwrap();
        table.install_app("acme.notes", vec![Row::method("app.acme.notes.list", Access::device(), seen.clone())]).unwrap();
        let grants = Arc::new(GrantTable::new());
        grants.set(id("system"), [caps::ENGINE_INSPECT], Trust::Origin);
        grants.set(id("w"), [], Trust::Origin);
        grants.set(
            id("acme.notes"),
            [],
            Trust::consent(["engine.session.get".to_string(), "sessions.list".to_string()], PackageCarrier::Wasm),
        );
        let router = Router::new(table, grants.clone(), Arc::new(Audit::default()));
        (router, seen, grants)
    }

    #[tokio::test]
    async fn a_call_is_decided_once_audited_and_handed_to_its_row() {
        let (router, seen, _) = router();
        let root = CallContext::root(device(), None);
        let call = root.call_id.get();
        let answer = router.call(root, "harnix.hello", json!({"x": 1})).await.unwrap();
        assert_eq!(answer["params"], json!({"x": 1}));
        assert_eq!(answer["subject"], json!({"kind": "device", "id": "d"}));
        let entry = &router.audit().recent(1)[0];
        assert_eq!((entry.method.as_str(), entry.decision, entry.call_id), ("harnix.hello", "allow", Some(call)));

        // Refused with the row's reason, never reaching the handler; an unknown name is not found.
        let e = router.call(CallContext::root(device(), None), "engine.config.set", json!({})).await.unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (codes::REFUSED, "not for devices"));
        let e = router.call(CallContext::root(Subject::Kernel, None), "engine.session.run_turn", json!({})).await.unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
        let e = router.call(CallContext::root(device(), None), "harnix.nothing", json!({})).await.unwrap_err();
        assert_eq!(e.code, codes::METHOD_NOT_FOUND);
        assert_eq!(seen.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_capability_is_what_opens_a_row_and_a_refusal_names_the_one_missing() {
        let (router, _, _) = router();
        let system = CallContext::root(Subject::extension(id("system")), None);
        assert!(router.call(system, "session.inspect", json!({})).await.is_ok());
        let e = router
            .call(CallContext::root(Subject::extension(id("w")), None), "session.inspect", json!({}))
            .await
            .unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
        assert_eq!(e.data, Some(json!({"capability": "engine.inspect"})));
        let e = router.call(CallContext::root(device(), None), "session.inspect", json!({})).await.unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
    }

    #[tokio::test]
    async fn a_package_is_held_to_its_consent_and_its_own_space() {
        let (router, _, _) = router();
        let me = || CallContext::root(Subject::extension(id("acme.notes")), None);
        assert!(router.call(me(), "app.acme.notes.list", json!({})).await.is_ok(), "its own, unlisted");
        let e = router.call(me(), "harnix.hello", json!({})).await.unwrap_err();
        assert_eq!(e.message, OUTSIDE_HUB);
        // A session or a space: only claiming its own.
        assert!(router.call(me(), "engine.session.get", json!({"space": "acme.notes"})).await.is_ok());
        assert!(router.call(me(), "sessions.list", json!({"space": "acme.notes.board"})).await.is_ok());
        for params in [json!({}), json!({"space": "sessions"}), json!({"space": "acme"})] {
            let e = router.call(me(), "engine.session.get", params.clone()).await.unwrap_err();
            assert_eq!((e.code, e.message.as_str()), (codes::REFUSED, harnix_authz::OUTSIDE_OWN_SPACE), "{params}");
        }
        // A native extension is not narrowed.
        assert!(router.call(CallContext::root(Subject::extension(id("w")), None), "engine.session.get", json!({})).await.is_ok());
    }

    #[tokio::test]
    async fn a_page_in_a_frame_has_the_devices_power_and_its_extensions_list_both() {
        let (router, _, grants) = router();
        let page = || {
            let mut cx = CallContext::root(device(), None);
            cx.on_behalf_of = Some(id("acme.notes"));
            cx
        };
        // No framed interface: nothing.
        let e = router.call(page(), "harnix.hello", json!({})).await.unwrap_err();
        assert_eq!(e.message, NO_FRAME);
        grants.set_frame(&id("acme.notes"), Some(["engine.session.get".to_string(), "engine.config.set".to_string()].into()));
        // Its own names; listed ones, narrowed to its space; what it lists the device may not do,
        // it may not either.
        assert!(router.call(page(), "app.acme.notes.list", json!({})).await.is_ok());
        assert!(router.call(page(), "engine.session.get", json!({"space": "acme.notes"})).await.is_ok());
        let e = router.call(page(), "engine.session.get", json!({"space": "sessions"})).await.unwrap_err();
        assert_eq!(e.message, harnix_authz::OUTSIDE_OWN_SPACE);
        let e = router.call(page(), "harnix.hello", json!({})).await.unwrap_err();
        assert_eq!(e.message, OUTSIDE_FRAME);
        let e = router.call(page(), "engine.config.set", json!({})).await.unwrap_err();
        assert_eq!(e.message, "not for devices");
        let entry = &router.audit().recent(1)[0];
        assert_eq!((entry.subject.as_str(), entry.via.as_deref()), ("device:d", Some("ext:acme.notes")));
    }

    #[tokio::test]
    async fn a_call_made_while_handling_another_is_its_child_in_the_audit() {
        let (router, _, _) = router();
        let outer = CallContext::root(device(), None);
        let inner = outer.child(&id("system"), false);
        let answer = router.call(inner, "harnix.hello", json!({})).await.unwrap();
        assert_eq!(answer["parent"], json!(outer.call_id.get()));
        assert_eq!(router.audit().recent(1)[0].parent, Some(outer.call_id.get()));
        assert_eq!(router.audit().recent(1)[0].subject, "ext:system");
    }

    #[tokio::test]
    async fn subscribing_is_decided_by_the_topic_too() {
        let (router, _, _) = router();
        let sub = |topic: &str| json!({"topic": topic});
        assert!(router.call(CallContext::root(device(), None), "harnix.topics.subscribe", sub("harnix.instances")).await.is_ok());
        let e = router.call(CallContext::root(device(), None), "harnix.topics.subscribe", sub("harnix.secret")).await.unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
        let e = router.call(CallContext::root(device(), None), "harnix.topics.subscribe", sub("harnix.none")).await.unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (codes::METHOD_NOT_FOUND, "there is no topic `harnix.none`"));
    }

    /// A page in a frame subscribes as it calls: only to what its interface lists, as
    /// `topic:<name>`, and the device's power besides.
    #[tokio::test]
    async fn a_page_in_a_frame_subscribes_only_to_what_its_interface_lists() {
        let (router, _, grants) = router();
        let page = || {
            let mut cx = CallContext::root(device(), None);
            cx.on_behalf_of = Some(id("acme.notes"));
            cx
        };
        let sub = |topic: &str| json!({"topic": topic, "params": {}});
        let e = router.call(page(), "harnix.topics.subscribe", sub("harnix.instances")).await.unwrap_err();
        assert_eq!(e.message, NO_FRAME);
        // Listing the method is not listing the topic.
        grants.set_frame(&id("acme.notes"), Some(["harnix.topics.subscribe".to_string()].into()));
        let e = router.call(page(), "harnix.topics.subscribe", sub("harnix.instances")).await.unwrap_err();
        assert_eq!(e.message, OUTSIDE_FRAME);
        grants.set_frame(&id("acme.notes"), Some(["topic:harnix.instances".to_string(), "topic:harnix.secret".to_string()].into()));
        assert!(router.call(page(), "harnix.topics.subscribe", sub("harnix.instances")).await.is_ok());
        // Listed, but not the device's to watch: the intersection.
        let e = router.call(page(), "harnix.topics.subscribe", sub("harnix.secret")).await.unwrap_err();
        assert_eq!(e.code, codes::REFUSED);
        let entry = &router.audit().recent(1)[0];
        assert_eq!(entry.via.as_deref(), Some("ext:acme.notes"));
    }

    #[tokio::test]
    async fn reachable_is_answered_here_from_the_rows() {
        let (router, _, _) = router();
        let reach = router.call(CallContext::root(device(), None), REACHABLE, json!({})).await.unwrap();
        assert_eq!(
            reach["methods"],
            json!(["app.acme.notes.list", "harnix.hello", "harnix.reachable", "harnix.topics.subscribe", "engine.session.get", "sessions.list"])
        );
        assert_eq!(reach["topics"], json!(["harnix.instances"]));
    }

    #[test]
    fn a_route_is_decided_and_recorded_with_its_path() {
        let (router, _, _) = router();
        assert_eq!(router.decide_route(&device(), "w", "/x").unwrap_err().code, codes::REFUSED);
        assert!(router.decide_route(&Subject::Kernel, "w", "/x").is_ok());
        assert_eq!(router.decide_route(&device(), "none", "/").unwrap_err().code, codes::METHOD_NOT_FOUND);
        assert_eq!(router.audit().recent(3)[2].method, "route:w/x");
    }
}
