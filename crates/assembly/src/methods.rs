//! The kernel's half of the route table.
//!
//! Reading this file and the hub's rows answers "what can a client call" for everything the
//! kernel itself answers. Modules add their own rows at registration (their methods, topics and
//! routes, and the grants their manifests disclose); the diagnostics page lists the whole table.
//! Everything not named anywhere is refused — the table is an allow-list, so a new method is
//! unreachable until someone writes it down (concept §3.4).

use std::sync::{Arc, OnceLock};

use harnix_contract::RpcError;
use harnix_hub::{Hub, HubRow};
use harnix_kernel::{Access, CallContext, Handler, KernelPart, Owner, Rpc};
use harnix_router::{Row, RouteTable};
use serde_json::{json, Value};

use crate::gate::{Behind, Diagnostics};

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

/// The kernel's rows: the hub's (its own methods and topics, and the engine's that pass through
/// it), the three this layer answers, and `harnix.reachable`, which the router answers itself.
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
    let answer = |f: fn(&Diagnostics, Value) -> Value| -> Arc<dyn Handler> {
        Arc::new(Answer(behind.diagnostics.clone(), f))
    };
    let assembly_rows = vec![
        Row::method("harnix.contributions", Access::device(), answer(|d, _| d.contributions())),
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
        assert!(!table.reachable(&device()).contains(&"session.somethingNew"));
    }

    /// The engine trusts whoever reaches its dispatch. These would hand that
    /// trust to a client (concept §3.4), so they are refused with a reason rather
    /// than left unimplemented.
    #[test]
    fn credential_and_endpoint_methods_are_refused() {
        let table = table(&Behind::default());
        for (method, _) in ENGINE_REFUSALS.iter().chain(HUB_OWNED) {
            assert!(
                !table.reachable(&device()).contains(method),
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
        let reachable = table.reachable(&device());
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

    /// What the hub owns is never granted, not even to the kernel's own
    /// modules: each one is a way around the hub.
    #[test]
    fn the_hubs_own_methods_cannot_be_granted() {
        let mut table = table(&Behind::default());
        for (method, _) in HUB_OWNED {
            assert!(table.grant(method, "anything").is_err(), "{method} was grantable");
        }
    }

    /// Every engine method a built-in is granted is a row of the engine's, so the grants its
    /// manifest discloses take.
    #[test]
    fn every_grant_a_built_in_asks_for_is_an_engine_row() {
        let table = table(&Behind::default());
        for method in harnix_builtin::engine_methods().into_iter().chain(harnix_builtin::apps::GRANTS.iter().copied()) {
            assert!(table.check_grant(method, "a-module").is_ok(), "{method} cannot be granted");
        }
    }

    /// A row the kernel installed before what answers it was up says so, rather than answering
    /// nothing or panicking.
    #[tokio::test]
    async fn a_kernel_row_before_its_answer_is_up_says_so() {
        let router = Router::new(table(&Behind::default()), Arc::new(Audit::default()));
        for method in ["harnix.hello", "harnix.diagnostics"] {
            let e = router.call(CallContext::root(device(), None), method, json!({})).await.unwrap_err();
            assert_eq!(e.code, harnix_contract::codes::INTERNAL_ERROR, "{method}");
        }
    }
}
