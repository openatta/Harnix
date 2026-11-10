//! The kernel's half of the route table.
//!
//! Reading this file, the hub's rows and the instance registry's answers "what can a client
//! call" for everything the kernel itself answers. Extensions add their own rows at registration
//! (their methods, topics and routes, and the grants their manifests disclose); the diagnostics
//! page lists the whole table.
//! Everything not named anywhere is refused — the table is an allow-list, so a new method is
//! unreachable until someone writes it down (concept §3.4).

mod hello;
mod ui;

use std::future::Future;
use std::sync::{Arc, OnceLock};

use harnix_contract::RpcError;
use harnix_hub::{Hub, HubRow};
use harnix_instance::{InstanceRegistry, InstanceRow};
use harnix_kernel::{Access, CallContext, Handler, KernelPart, Owner, Rpc};
use harnix_router::{Row, RouteTable};
use serde_json::{json, Value};

use crate::gate::{Behind, Diagnostics};

pub(crate) use hello::Hello;
pub(crate) use ui::{UiModules, UiTopic, UI};

/// `harnix.process`, a method and a topic of the same value.
pub(crate) const PROCESS: &str = "harnix.process";

/// One of the three methods this layer answers.
struct Answer(Arc<OnceLock<Arc<Diagnostics>>>, fn(&Diagnostics, Value) -> Value);

#[async_trait::async_trait]
impl Handler for Answer {
    async fn call(&self, _: CallContext, params: Value) -> Rpc<Value> {
        let diagnostics = self.0.get().ok_or_else(|| {
            RpcError::new(harnix_contract::codes::INTERNAL_ERROR, "the diagnostics are not up yet")
        })?;
        Ok((self.1)(diagnostics, params))
    }
}

/// A row answered by something built after the table is: until it is, it says so.
struct Late<T, F> {
    cell: Arc<OnceLock<Arc<T>>>,
    f: F,
}

#[async_trait::async_trait]
impl<T, F, Fut> Handler for Late<T, F>
where
    T: Send + Sync + 'static,
    F: Fn(Arc<T>) -> Fut + Send + Sync,
    Fut: Future<Output = Rpc<Value>> + Send,
{
    async fn call(&self, _: CallContext, _params: Value) -> Rpc<Value> {
        let answers = self.cell.get().cloned().ok_or_else(|| {
            RpcError::new(harnix_contract::codes::INTERNAL_ERROR, "the kernel is not up yet")
        })?;
        (self.f)(answers).await
    }
}

fn late<T, F, Fut>(cell: &Arc<OnceLock<Arc<T>>>, f: F) -> Arc<dyn Handler>
where
    T: Send + Sync + 'static,
    F: Fn(Arc<T>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Rpc<Value>> + Send + 'static,
{
    Arc::new(Late { cell: cell.clone(), f })
}

/// The kernel's rows: the hub's (its own methods and topics, and the engine's that pass through
/// it), the instance registry's, the ones this layer answers, and `harnix.reachable`, which the
/// router answers itself.
pub fn table(behind: &Behind) -> RouteTable {
    let mut hub_rows: Vec<Row> = Vec::new();
    let mut engine_rows: Vec<Row> = Vec::new();
    for row in Hub::rows(&behind.hub) {
        let (owner, row) = match row {
            HubRow::Method { name, owner, access, handler } => (
                owner,
                match handler {
                    Some(handler) => Row::method(name, access, handler),
                    None => Row::closed(name, access),
                },
            ),
            HubRow::Topic { name, access } => (Owner::Kernel(KernelPart::Hub), Row::topic(name, access)),
        };
        match owner {
            Owner::Engine => engine_rows.push(row),
            _ => hub_rows.push(row),
        }
    }
    let instance_rows: Vec<Row> = InstanceRegistry::rows(&behind.instances)
        .into_iter()
        .map(|row| match row {
            InstanceRow::Method { name, access, handler } => Row::method(name, access, handler),
            InstanceRow::Topic { name, access } => Row::topic(name, access),
        })
        .collect();
    let answer = |f: fn(&Diagnostics, Value) -> Value| -> Arc<dyn Handler> {
        Arc::new(Answer(behind.diagnostics.clone(), f))
    };
    let assembly_rows = vec![
        Row::method("harnix.hello", Access::device(), late(&behind.hello, |h: Arc<Hello>| async move { h.answer().await })),
        Row::method(
            PROCESS,
            Access::device(),
            late(&behind.process, |p: Arc<crate::process::Process>| async move { Ok(p.view()) }),
        ),
        Row::topic(PROCESS, Access::device()),
        // What the page draws for whom, and the modules it loads: a method, and a topic pushed on
        // every change.
        Row::method(UI, Access::device(), late(&behind.hello, |h: Arc<Hello>| async move { Ok(h.ui.view()) })),
        Row::topic(UI, Access::device()),
        Row::method("harnix.contributions", Access::device(), answer(|d, _| d.contributions())),
        // Every extension, and why one does nothing: what a person asks first.
        Row::method("harnix.extensions", Access::device(), answer(|d, _| d.list())),
        Row::method("harnix.diagnostics", Access::device(), answer(|d, _| d.report())),
        // The decisions the router recorded, newest first: what the system space shows as its
        // audit, and what a test reads instead of transcript files.
        Row::method(
            "harnix.audit",
            Access::device(),
            answer(|d, params| {
                let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(200).min(2048) as usize;
                let entries = d.router().map(|r| r.audit().recent(limit)).unwrap_or_default();
                json!({ "entries": entries })
            }),
        ),
    ];
    // Reachable by anyone that got this far: a subject may always ask what it may do.
    let router_rows = vec![Row::closed(harnix_router::REACHABLE, Access::device())];

    let mut table = RouteTable::new();
    for (owner, rows) in [
        (Owner::Kernel(KernelPart::Hub), hub_rows),
        (Owner::Engine, engine_rows),
        (Owner::Kernel(KernelPart::Instances), instance_rows),
        (Owner::Kernel(KernelPart::Assembly), assembly_rows),
        (Owner::Kernel(KernelPart::Router), router_rows),
    ] {
        table
            .install(&owner, rows)
            .unwrap_or_else(|e| panic!("the kernel's own rows collide: {e}"));
    }
    table
}

#[cfg(test)]
mod tests {
    use harnix_authz::Audit;
    use harnix_contract::Subject;
    use harnix_hub::{ENGINE_REFUSALS, HUB_OWNED};
    use harnix_kernel::Dispatch;
    use harnix_router::Router;

    use super::*;

    fn device() -> Subject {
        Subject::Device { id: "test".into() }
    }

    /// The whole point of an allow-list: a name nobody wrote down is refused,
    /// and adding a method is a decision rather than an accident.
    #[test]
    fn an_unlisted_method_is_refused() {
        let table = table(&Behind::default());
        assert!(!table.reachable(&device(), &harnix_kernel::GrantTable::new()).contains(&"session.somethingNew"));
    }

    /// The engine trusts whoever reaches its dispatch. These would hand that
    /// trust to a client (concept §3.4), so they are refused with a reason rather
    /// than left unimplemented.
    #[test]
    fn credential_and_endpoint_methods_are_refused() {
        let table = table(&Behind::default());
        let grants = harnix_kernel::GrantTable::new();
        let names = ENGINE_REFUSALS.iter().map(|(m, _, _)| m).chain(HUB_OWNED.iter().map(|(m, _)| m));
        for method in names {
            assert!(
                !table.reachable(&device(), &grants).contains(method),
                "{method} reachable by a device"
            );
        }
    }

    /// The engine's package lifecycle is not a device's to reach directly,
    /// because reaching it would skip the re-read that keeps what is served in
    /// step with what is installed. `mcp.addServer` configures a
    /// subprocess-spawning tool with no manifest and no disclosure at all.
    #[test]
    fn the_engine_package_lifecycle_and_mcp_are_not_reachable() {
        let table = table(&Behind::default());
        let grants = harnix_kernel::GrantTable::new();
        let reachable = table.reachable(&device(), &grants);
        for method in [
            "plugin.install",
            "plugin.uninstall",
            "plugin.enable",
            "plugin.disable",
            "mcp.addServer",
        ] {
            assert!(!reachable.contains(&method), "{method} reachable by a device");
        }
    }

    /// What the hub owns no extension may list, not even the kernel's own modules: each one is
    /// a way around the hub.
    #[test]
    fn the_hubs_own_methods_cannot_be_listed() {
        let table = table(&Behind::default());
        for (method, _) in HUB_OWNED {
            assert!(table.check_listed(method).is_err(), "{method} could be listed");
        }
    }

    /// Every capability a built-in requires is the kernel's, and opens a row: what its manifest
    /// asks for is something.
    #[test]
    fn every_capability_a_built_in_requires_opens_a_kernel_row() {
        let table = table(&Behind::default());
        let snapshot = harnix_router::Router::new(table, Arc::new(harnix_kernel::GrantTable::new()), Arc::new(Audit::default())).snapshot();
        let required: Vec<&str> = snapshot["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| row["access"]["requires"][0].as_str())
            .collect();
        for cap in harnix_builtin::capabilities().iter().chain(harnix_builtin::apps::REQUIRES) {
            assert!(harnix_contract::s1::caps::kernel(cap).is_some(), "{cap} is not the kernel's");
            if *cap != harnix_contract::s1::caps::INSTANCES_ADMIN {
                assert!(required.contains(&cap.as_str()), "no row requires {cap}");
            }
        }
    }

    /// A row the kernel installed before what answers it was up says so, rather than answering
    /// nothing or panicking.
    #[tokio::test]
    async fn a_kernel_row_before_its_answer_is_up_says_so() {
        let router = Router::new(table(&Behind::default()), Arc::new(harnix_kernel::GrantTable::new()), Arc::new(Audit::default()));
        for method in ["harnix.hello", "harnix.diagnostics"] {
            let e = router.call(CallContext::root(device(), None), method, json!({})).await.unwrap_err();
            assert_eq!(e.code, harnix_contract::codes::INTERNAL_ERROR, "{method}");
        }
    }
}
