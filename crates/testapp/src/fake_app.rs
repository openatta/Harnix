//! `fakehost` — a module that hosts a fake runtime application through the kernel's app host
//! (`ModuleContext::apps`), so the LIVE suites can drive runtime registration without WASM.
//!
//! `app.fakehost.register {id}` registers an application `<id>` with:
//!
//! | name | who | does |
//! |---|---|---|
//! | `app.<id>.echo` | device | answers `{params, caller}` and publishes `{params}` on `app.<id>.ticks` |
//! | `app.<id>.slow {ms}` | device | answers after `ms` milliseconds: a call in flight across a withdrawal |
//! | `app.<id>.internal` | kernel | refused to a device |
//! | topic `app.<id>.ticks` | device, module | what `echo` published last |
//! | space `<id>` | — | on `system` |
//!
//! `app.fakehost.unregister {id}` withdraws it. With `[apps] enabled = false` there is no host and
//! both are refused.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harnix::prelude::*;
use harnix::{AppHandle, AppHost, AppSpec, Subject};
use serde_json::{json, Value};

pub struct FakeHost;

#[derive(Default)]
struct Registered(Mutex<BTreeMap<String, AppHandle>>);

fn host(cx: &CallContext) -> Result<AppHost, RpcError> {
    cx.module()
        .apps()
        .ok_or_else(|| RpcError::refused("this process hosts no runtime applications ([apps] enabled = false)"))
}

fn id_of(p: &Value) -> Result<String, RpcError> {
    p["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| RpcError::invalid_params("missing id"))
}

fn spec(id: &str, handle: Arc<Mutex<Option<AppHandle>>>) -> AppSpec {
    AppSpec::new(id)
        .method("echo", &[Principal::Device], move |caller: Subject, params: Value| {
            let handle = handle.lock().unwrap().clone();
            async move {
                if let Some(handle) = handle {
                    handle.publish("ticks", json!({}), json!({"params": params})).await?;
                }
                Ok(json!({"params": params, "caller": caller}))
            }
        })
        .method("slow", &[Principal::Device], |_caller, params: Value| async move {
            let ms = params["ms"].as_u64().unwrap_or(500).min(10_000);
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Ok(json!({"slept": ms}))
        })
        .method("internal", &[Principal::Kernel], |_caller, _params| async move { Ok(json!({"internal": true})) })
        .topic("ticks", &[])
        .space(SpaceDecl::new(id).instance(harnix::SYSTEM_INSTANCE))
}

#[harnix::async_trait]
impl Module for FakeHost {
    const API_VERSION: u32 = harnix::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new("fakehost", "0.0.1").disclosure("registers a fake runtime application, for the tests")
    }

    fn methods(&self, m: &mut MethodRegistry) {
        m.method("register", Principal::Device)
            .describe("registers the fake application `id` with the app host")
            .handler(|cx: CallContext, p: Value| async move {
                let host = host(&cx)?;
                let id = id_of(&p)?;
                let slot = Arc::new(Mutex::new(None));
                let handle = host.register(spec(&id, slot.clone())).await?;
                *slot.lock().unwrap() = Some(handle.clone());
                cx.state::<Registered>()?.0.lock().unwrap().insert(id.clone(), handle);
                let summary = host.registered().into_iter().find(|s| s.id == id);
                Ok(json!(summary))
            });
        m.method("unregister", Principal::Device)
            .describe("withdraws the fake application `id`")
            .handler(|cx: CallContext, p: Value| async move {
                let host = host(&cx)?;
                let id = id_of(&p)?;
                let withdrawn = host.withdraw(&id).await?;
                cx.state::<Registered>()?.0.lock().unwrap().remove(&id);
                Ok(json!(withdrawn))
            });
    }

    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        cx.provide(Registered::default());
        Ok(())
    }
}
