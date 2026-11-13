//! What the route table's rows are answered by, where the answer is the assembly's: the ones this
//! layer answers itself, and the diagnostics page.
//!
//! An extension's method cannot shadow a hub method: both are rows in one table, and the table
//! refuses a name already taken.

use std::sync::{Arc, OnceLock, RwLock, Weak};

use harnix_contract::sync::read;
use harnix_hub::HubCell;
use harnix_router::Router;
use serde_json::{json, Value};

use crate::host::ExtensionStatus;

/// What the kernel's rows are answered by, filled in as each is built: the rows go in the table
/// at registration, with every other name, before the hub or the diagnostics exist.
#[derive(Clone, Default)]
pub(crate) struct Behind {
    pub hub: HubCell,
    pub instances: harnix_instance::RegistryCell,
    pub hello: Arc<OnceLock<Arc<crate::kernel_rows::Hello>>>,
    pub process: Arc<OnceLock<Arc<crate::process::Process>>>,
    pub uploads: Arc<OnceLock<Arc<crate::host::seat::HostUploads>>>,
    pub diagnostics: Arc<OnceLock<Arc<Diagnostics>>>,
}

/// What this process is made of and what did not take: `harnix.diagnostics`, `harnix.extensions`.
pub(crate) struct Diagnostics {
    pub versions: Value,
    pub extensions: RwLock<Vec<ExtensionStatus>>,
    /// The extensions registered while the process runs.
    pub dynamic: OnceLock<Arc<crate::package_table::PackageTable>>,
    /// The router whose table `table` lists; set once it exists, and weak because the router
    /// holds the rows that hold this.
    pub router: OnceLock<Weak<Router>>,
}

impl Diagnostics {
    pub(crate) fn router(&self) -> Option<Arc<Router>> {
        self.router.get().and_then(Weak::upgrade)
    }

    pub(crate) fn report(&self) -> Value {
        let extensions = read(&self.extensions).clone();
        json!({
            "versions": self.versions,
            "modules": extensions,
            "table": self.table(),
            // Over the single-duplex in-flight cap a request is answered `BUSY`; a notification has
            // nothing to answer and is dropped, counted here (ipdrun-01).
            "transport": {"dropped_notifications": harnix_transport::dropped_notifications()},
        })
    }

    /// Every row of the route table, in the row shape the method-table snapshot pins
    /// (`crates/harnix/tests/method-table.snap.json`): empty before the router is set.
    fn table(&self) -> Value {
        match self.router() {
            Some(router) => router.snapshot(),
            None => json!({"rows": []}),
        }
    }

    /// Every extension, linked or registered while the process runs: its trust and carrier, its
    /// version, whether it is enabled and why not, what it was granted, how many rows it has.
    pub(crate) fn list(&self) -> Value {
        let mut out: Vec<Value> = read(&self.extensions).iter().map(ExtensionStatus::summary).collect();
        if let Some(packages) = self.dynamic.get() {
            out.extend(packages.registered().into_iter().map(|a| {
                json!({
                    "id": a.id,
                    "version": null,
                    "trust": "consent",
                    "carrier": harnix_host_api::carrier_name(a.carrier),
                    "enabled": true,
                    "granted": [],
                    "rows": a.methods.len() + a.topics.len(),
                    "by": a.by,
                })
            }));
        }
        json!({ "extensions": out })
    }

    /// Each extension as `harnix.ui` tells the page of it: its id, trust, carrier, version, whether
    /// it is enabled, and what it declared of its interface. One registered while the process runs
    /// is enabled for as long as it is registered.
    pub(crate) fn ui_entries(&self) -> Vec<Value> {
        let mut out: Vec<Value> = read(&self.extensions)
            .iter()
            .map(|e| {
                json!({
                    "id": e.id,
                    "trust": e.trust,
                    "carrier": e.carrier,
                    "version": e.version,
                    "enabled": e.enabled,
                    "ui": e.ui,
                })
            })
            .collect();
        if let Some(packages) = self.dynamic.get() {
            out.extend(packages.registered().into_iter().map(|a| {
                let carrier = harnix_host_api::carrier_name(a.carrier);
                json!({ "id": a.id, "trust": "consent", "carrier": carrier, "version": null, "enabled": true, "ui": null })
            }));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use harnix_authz::Audit;
    use harnix_contract::s1::{PackageCarrier, Trust};
    use harnix_contract::Subject;
    use harnix_kernel::{Access, CallContext, Dispatch, Handler, Rpc};
    use harnix_router::Row;

    use super::*;

    fn snapshot_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../harnix/tests/reachable.snap.json")
    }

    /// What a package's subject reaches, on the kernel's table: its own names, what its `calls`
    /// lists and the table lets through, and nothing else. The answer is `harnix.reachable` asked
    /// as the package. The device's and a native extension's reach are read from a running
    /// process by `tests/live/api/method-table.spec.ts`, into the same file.
    #[tokio::test]
    async fn a_packages_reach_is_the_snapshots() {
        let grants = Arc::new(harnix_kernel::GrantTable::new());
        let router = Router::new(crate::kernel_rows::table(&Behind::default()), grants.clone(), Arc::new(Audit::default()));
        let id = "acme.snapshot";
        let hub: Vec<String> = ["harnix.sessions.list", "harnix.session.send", "harnix.session.interrupt", "topic:harnix.sessions.status", "engine.plugin.install"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let ext = harnix_contract::ExtId::package(id).unwrap();
        grants.set(ext.clone(), [], Trust::consent(hub, PackageCarrier::Wasm));
        struct Nothing;
        #[async_trait::async_trait]
        impl Handler for Nothing {
            async fn call(&self, _: CallContext, _: Value) -> Rpc<Value> {
                Ok(Value::Null)
            }
        }
        let rows = vec![
            Row::method(format!("{id}.echo"), Access::kernel().with_device().with_extension(ext.clone()), Arc::new(Nothing)),
            Row::method(format!("{id}.internal"), Access::only(ext.clone()), Arc::new(Nothing)),
            Row::topic(&format!("{id}.ticks"), Access::only(ext.clone())),
        ];
        router.install_package(&Subject::Kernel, id, rows).unwrap();

        let me = Subject::extension(harnix_contract::ExtId::package(id).unwrap());
        let reach = router.call(CallContext::root(me, None), harnix_router::REACHABLE, json!({})).await.unwrap();
        let mut methods: Vec<String> = serde_json::from_value(reach["methods"].clone()).unwrap();
        let mut topics: Vec<String> = serde_json::from_value(reach["topics"].clone()).unwrap();
        methods.sort();
        topics.sort();
        let actual = json!({"methods": methods, "topics": topics});

        let path = snapshot_path();
        let mut kept: serde_json::Map<String, Value> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if std::env::var("HARNIX_UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
            kept.insert("plugin_app".into(), actual);
            std::fs::write(&path, serde_json::to_string_pretty(&Value::Object(kept)).unwrap() + "\n").unwrap();
            return;
        }
        assert_eq!(
            Some(&actual),
            kept.get("plugin_app"),
            "{} differs; if the change is intended: HARNIX_UPDATE_SNAPSHOTS=1, and say why in the commit",
            path.display()
        );
    }
}
