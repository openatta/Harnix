//! `fakehost` — an extension that registers a fake package through the host's registrar
//! (`HostServices`, granted `extensions.manage`), the way the `packages` component does, so the
//! LIVE suites can drive runtime registration without WASM.
//!
//! `fakehost.register {id}` registers a package `<id>` with:
//!
//! | name | who | does |
//! |---|---|---|
//! | `<id>.echo` | device | answers `{params, caller}` and publishes `{params}` on `<id>.ticks` |
//! | `<id>.slow {ms}` | device | answers after `ms` milliseconds: a call in flight across a withdrawal |
//! | `<id>.internal` | kernel | refused to a device |
//! | topic `<id>.ticks` | device, extensions | what `echo` published last |
//! | space `<id>` | — | on `main` |
//!
//! `fakehost.unregister {id}` withdraws it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harnix::prelude::*;
use harnix::{DescriptorBuilder, ExtId};
use harnix_host_api::{ExtensionHandle, HostServices};
use serde_json::{json, Value};

pub struct FakeHost;

#[derive(Default)]
struct Registered(Mutex<BTreeMap<String, ExtensionHandle>>);

fn id_of(p: &Value) -> Result<String, RpcError> {
    p["id"].as_str().map(str::to_string).ok_or_else(|| RpcError::invalid_params("missing id"))
}

/// The fake package's descriptor; `handle` is where its own handle lands once it is registered.
fn descriptor(id: &ExtId, handle: Arc<Mutex<Option<ExtensionHandle>>>) -> harnix::ext::ExtensionDescriptor {
    let manifest = Manifest::new(id.as_str(), "0.0.1");
    let mut d = DescriptorBuilder::new(id.clone(), manifest, harnix::ext::Trust::consent(Vec::<String>::new(), harnix::ext::PackageCarrier::Wasm));
    let own = Access::device().with_extension(id.clone());
    d.method(&format!("{id}.echo")).access(own.clone()).raw(move |call, params| {
        let handle = handle.lock().unwrap().clone();
        async move {
            if let Some(handle) = handle {
                handle.publish("ticks", json!({}), json!({"params": params})).await?;
            }
            Ok(json!({"params": params, "caller": call.subject()}))
        }
    });
    d.method(&format!("{id}.slow")).access(own).raw(|_call, params| async move {
        let ms = params["ms"].as_u64().unwrap_or(500).min(10_000);
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Ok(json!({"slept": ms}))
    });
    d.method(&format!("{id}.internal")).access(Access::kernel()).raw(|_call, _params| async move { Ok(json!({"internal": true})) });
    d.topic(&format!("{id}.ticks"));
    d.space(SpaceDecl::new(id.as_str()).instance(harnix::MAIN_INSTANCE));
    d.build()
}

#[harnix::async_trait]
impl Extension for FakeHost {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new("fakehost", "0.0.1")
            .disclosure("registers a fake package, for the tests")
            .requires([harnix::caps::EXTENSIONS_MANAGE])
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.method("register").access(Access::device()).describe("registers the fake package `id` with the host").raw(
            |call, p| async move {
                let host = call.ext().service::<HostServices>()?;
                let id = ExtId::package(&id_of(&p)?).map_err(|e| RpcError::invalid_params(e.to_string()))?;
                let slot = Arc::new(Mutex::new(None));
                let handle = host.registrar().register(descriptor(&id, slot.clone()), Vec::new()).await?;
                handle.mark(harnix_host_api::Availability::Ready);
                *slot.lock().unwrap() = Some(handle.clone());
                call.state::<Registered>()?.0.lock().unwrap().insert(id.to_string(), handle);
                let summary = host.registrar().registered().into_iter().find(|s| s.id() == id.as_str());
                Ok(json!(summary))
            },
        );
        r.method("unregister").access(Access::device()).describe("withdraws the fake package `id`").raw(|call, p| async move {
            let id = id_of(&p)?;
            let handle = call.state::<Registered>()?.0.lock().unwrap().remove(&id);
            let Some(handle) = handle else {
                return Err(RpcError::invalid_params(format!("no package `{id}` is registered by this extension")));
            };
            Ok(json!(handle.withdraw().await?))
        });
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        cx.provide(Registered::default());
        Ok(())
    }
}
