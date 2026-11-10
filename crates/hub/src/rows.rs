//! The hub's rows: each method it answers, with how far it is open and what answers it.
//!
//! The router decides a call by its row and hands it to that row's handler; nothing here looks at
//! a method's name to find out what to do. The engine's methods that pass through the hub are
//! rows too, in [`crate::engine_rows`].

use std::future::Future;
use std::sync::{Arc, OnceLock};

use harnix_contract::s1::{caps, RowScope};
use harnix_contract::{codes, RpcError};
use harnix_kernel::{Access, CallContext, Handler, KernelPart, Owner, Rpc};
use serde_json::Value;

use crate::{spaces, Hub};

/// Where the hub's rows find the hub. The rows are installed before the hub is built — with
/// every other name, so that a module cannot take one — and the hub is put here once it is.
pub type HubCell = Arc<OnceLock<Arc<Hub>>>;

/// One row the hub owns, or answers for the engine.
pub enum HubRow {
    Method {
        name: &'static str,
        owner: Owner,
        access: Access,
        /// `None` for what nobody may call: the hub's own, refused to everyone.
        handler: Option<Arc<dyn Handler>>,
    },
    Topic { name: &'static str, access: Access },
}

/// What answers one of the hub's rows: a function of the hub, the call and its params, with the
/// space a call claims checked first where the method acts on one session.
struct Answer<F> {
    hub: HubCell,
    /// The method, for the space check's audit.
    name: &'static str,
    checked: bool,
    f: F,
}

/// What answers `name`: `f`, given the hub, the call and its params.
pub(crate) fn answer<F, Fut>(hub: &HubCell, name: &'static str, f: F) -> Arc<dyn Handler>
where
    F: Fn(Arc<Hub>, CallContext, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Rpc<Value>> + Send + 'static,
{
    Arc::new(Answer {
        hub: hub.clone(),
        name,
        checked: spaces::CHECKED.contains(&name),
        f,
    })
}

#[async_trait::async_trait]
impl<F, Fut> Handler for Answer<F>
where
    F: Fn(Arc<Hub>, CallContext, Value) -> Fut + Send + Sync,
    Fut: Future<Output = Rpc<Value>> + Send,
{
    async fn call(&self, cx: CallContext, params: Value) -> Rpc<Value> {
        let hub = self
            .hub
            .get()
            .cloned()
            .ok_or_else(|| RpcError::new(codes::INTERNAL_ERROR, "the hub is not up yet"))?;
        if self.checked {
            hub.check_space(&cx.subject, self.name, &params)?;
        }
        (self.f)(hub, cx, params).await
    }
}

/// One of the hub's own methods, open to devices: about one session when a claimed space is
/// checked on it ([`spaces::CHECKED`]), else about none.
fn own<F, Fut>(hub: &HubCell, name: &'static str, f: F) -> HubRow
where
    F: Fn(Arc<Hub>, CallContext, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Rpc<Value>> + Send + 'static,
{
    let scope = if spaces::CHECKED.contains(&name) { RowScope::Session } else { RowScope::Open };
    HubRow::Method {
        name,
        owner: Owner::Kernel(KernelPart::Hub),
        access: Access::device().scoped(scope),
        handler: Some(answer(hub, name, f)),
    }
}

/// [`own`], about one space.
fn own_space<F, Fut>(hub: &HubCell, name: &'static str, f: F) -> HubRow
where
    F: Fn(Arc<Hub>, CallContext, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Rpc<Value>> + Send + 'static,
{
    HubRow::Method {
        name,
        owner: Owner::Kernel(KernelPart::Hub),
        access: Access::device().scoped(RowScope::Space),
        handler: Some(answer(hub, name, f)),
    }
}

/// The hub's own topics: the instance tree as whole values (concept §3.2). Each with the
/// parameters it is subscribed by.
pub const HUB_TOPICS: &[(&str, &[&str])] = &[
    ("harnix.instances", &[]),
    ("harnix.instances.sessions", &["instance"]),
    ("harnix.sessions.tasks", &["instance", "session_id"]),
    ("harnix.sessions.status", &[]),
    ("harnix.process", &[]),
];

impl Hub {
    /// Every row the hub answers: its own methods and topics, and the engine's that pass through
    /// it. `hub` holds the hub once it is built; until then each answers that it is not up.
    pub fn rows(hub: &HubCell) -> Vec<HubRow> {
        let mut rows = vec![
            own(hub, "harnix.hello", |h, _, _| async move { h.hello().await }),
            own_space(hub, "harnix.sessions", |h, _, p| async move {
                if p.get("space").is_some() {
                    return h.space_sessions(&h.claimed(&p)?).await;
                }
                let instance = h.instance_of(&p)?;
                h.sessions_view(&instance).await
            }),
            own(hub, "harnix.attach", |h, cx, p| async move { h.attach(&cx, p).await }),
            own(hub, "harnix.detach", |h, cx, p| async move { h.detach(&cx, p).await }),
            own(hub, "harnix.send", |h, cx, p| async move { h.send(&cx, p).await }),
            own(hub, "harnix.queue.remove", |h, _, p| async move { h.queue_remove(p).await }),
            own(hub, "harnix.queue.edit", |h, _, p| async move { h.queue_edit(p).await }),
            own(hub, "harnix.subscribe", |h, cx, p| async move {
                if p.get("topic").and_then(Value::as_str) == Some("harnix.process") {
                    h.tick_process();
                }
                h.subscribe(&cx, p).await
            }),
            own(hub, "harnix.unsubscribe", |h, cx, p| async move { h.unsubscribe(&cx, p).await }),
            own(hub, "harnix.instances.list", |h, _, _| async move { Ok(h.instances_view()) }),
            own(hub, "harnix.instances.templates", |h, cx, p| async move {
                Ok(h.templates_view(&cx.subject, &p).await)
            }),
            own(hub, "harnix.instances.sessions", |h, _, p| async move {
                let instance = h.instance_of(&p)?;
                h.sessions_view(&instance).await
            }),
            own(hub, "harnix.instances.create", |h, cx, p| async move {
                h.create_instance(&cx.subject, p, false).await
            }),
            own(hub, "harnix.instances.ensure", |h, cx, p| async move {
                h.create_instance(&cx.subject, p, true).await
            }),
            own(hub, "harnix.instances.stop", |h, cx, p| async move { h.stop_instance(&cx.subject, p).await }),
            own(hub, "harnix.instances.restart", |h, cx, p| async move {
                h.restart_instance(&cx.subject, p).await
            }),
            own(hub, "harnix.instances.wait", |h, _, p| async move { h.wait_instance(p).await }),
            own(hub, "harnix.sessions.tasks", |h, _, p| async move {
                let key = h.key_of(&p)?;
                h.tasks_view(&key).await
            }),
            own(hub, "harnix.entries.append", |h, cx, p| async move { h.append_entry(&cx.subject, p).await }),
            own(hub, "harnix.sessions.move", |h, cx, p| async move { h.move_session(&cx.subject, p).await }),
            own(hub, "harnix.prompts.answerer", |h, cx, p| async move { h.answerer(&cx.subject, &p) }),
            own(hub, "harnix.spaces.list", |h, _, _| async move { Ok(h.spaces_view()) }),
            own(hub, "harnix.process", |h, _, _| async move { Ok(h.process()) }),
            // Intercepted engine methods, the hub's semantics: still the device's to ask for.
            own(hub, "session.interrupt", |h, _, p| async move { h.interrupt(p).await }),
            own(hub, "session.close", |h, _, p| async move { h.close_or_delete("session.close", p).await }),
            own(hub, "session.delete", |h, _, p| async move { h.close_or_delete("session.delete", p).await }),
            // After a provider changes, the instances whose template inherits the system
            // instance's get it too (ipdrun-11): whoever may write the engine's providers. Listed
            // with the engine's methods, as the method table has always shown it.
            HubRow::Method {
                name: "harnix.instances.inheritProviders",
                owner: Owner::Engine,
                access: Access::capability(caps::ENGINE_CONFIG_WRITE),
                handler: Some(answer(hub, "harnix.instances.inheritProviders", |h, _, _| async move {
                    h.inherit_providers().await
                })),
            },
        ];
        rows.extend(HUB_TOPICS.iter().map(|(name, _)| HubRow::Topic { name, access: Access::device() }));
        rows.extend(crate::engine_rows::rows(hub));
        rows
    }
}
