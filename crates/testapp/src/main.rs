//! `harnix-testapp` — the built-ins plus modules the LIVE suites drive.
//!
//! Each module exists to show one thing about the module contract, and some
//! fail on purpose: a failure has to stay with the module that failed.
//!
//! | module | shows |
//! |---|---|
//! | `probe` | typed methods per principal, config and credentials, topics, a route, a migration |
//! | `peer` | one module calling another as `Module(peer)` |
//! | `fails-start` | a failed `start` disables only that module |
//! | `fails-migrate` | a failed `migrate` disables only that module |
//! | `probe` (again) | a second module with a taken id is refused at registration |

use std::sync::atomic::{AtomicU64, Ordering};

use harnix::axum::routing::get;
use harnix::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

fn main() -> anyhow::Result<()> {
    harnix::App::builder()
        .name("harnix-testapp", "0.0.1")
        .with_builtins()
        .module(Probe)
        .module(Peer)
        .module(FailsStart)
        .module(FailsMigrate)
        .module(Duplicate)
        .run()
}

/* ── probe ─────────────────────────────────────────────────────────────── */

struct Probe;

#[derive(Deserialize)]
struct ProbeConfig {
    #[serde(default)]
    greeting: Option<String>,
    #[serde(default)]
    key: Option<CredentialRef>,
}

struct ProbeState {
    greeting: String,
    key: Result<String, String>,
    counter: AtomicU64,
}

#[derive(Deserialize)]
struct EchoParams {
    text: String,
}

#[harnix::async_trait]
impl Module for Probe {
    const API_VERSION: u32 = harnix::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new("probe", "0.0.1").disclosure("a module for the tests")
    }

    fn methods(&self, m: &mut MethodRegistry) {
        m.method("echo", Principal::Device)
            .describe("says back what it was told")
            .schema(json!({"type": "object", "required": ["text"], "properties": {"text": {"type": "string"}}}))
            .handler(|cx: CallContext, p: EchoParams| async move {
                Ok(json!({"text": p.text, "from": cx.subject()}))
            });
        m.method("internal", Principal::Kernel)
            .handler(|_cx: CallContext, _p: Value| async move { Ok(json!({"internal": true})) });
        m.method("for_peer", Principal::Module("peer".into()))
            .handler(|cx: CallContext, _p: Value| async move { Ok(json!({"caller": cx.subject()})) });
        m.method("config", Principal::Device)
            .handler(|cx: CallContext, _p: Value| async move {
                let state = cx.state::<ProbeState>()?;
                Ok(json!({
                    "greeting": state.greeting,
                    "key": state.key.as_ref().map(|k| format!("{} chars", k.len())).map_err(Clone::clone),
                    "data_dir": cx.module().data_dir(),
                }))
            });
        m.method("bump", Principal::Device)
            .handler(|cx: CallContext, p: Value| async move {
                let state = cx.state::<ProbeState>()?;
                let n = state.counter.fetch_add(1, Ordering::SeqCst) + 1;
                let events = cx.module().events();
                events.publish("counter", json!({}), json!({"n": n})).await?;
                if let Some(job) = p.get("job_id") {
                    events
                        .publish("job", json!({"job_id": job}), json!({"job_id": job, "n": n}))
                        .await?;
                }
                Ok(json!({"n": n}))
            });
        m.method("call", Principal::Device)
            .describe("calls another method as this module")
            .handler(|cx: CallContext, p: Value| async move {
                let method = p["method"].as_str().unwrap_or_default().to_string();
                cx.module().call(&method, p["params"].clone()).await
            });
    }

    fn events(&self, e: &mut EventRegistry) {
        e.topic("counter");
        e.topic("job").param("job_id");
        e.topic("secret").only(&[Principal::Kernel]);
    }

    fn routes(&self, r: &mut RouteRegistry) {
        let router = harnix::axum::Router::new()
            .route("/hello", get(|| async { "hello from probe" }));
        r.serve(&[Principal::Device], router);
    }

    async fn migrate(&self, cx: &MigrateContext) -> anyhow::Result<MigrationReport> {
        let marker = cx.data_dir.join("schema");
        let applied = std::fs::read_to_string(&marker).unwrap_or_default();
        let mut done = Vec::new();
        for step in ["001-create", "002-index"] {
            if !applied.lines().any(|l| l == step) {
                done.push(step.to_string());
            }
        }
        let all: Vec<&str> = applied.lines().chain(done.iter().map(String::as_str)).collect();
        std::fs::write(&marker, all.join("\n"))?;
        Ok(MigrationReport::applied(done))
    }

    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        let config: ProbeConfig = cx.config()?;
        let key = match &config.key {
            Some(r) => cx
                .credentials()
                .get(&r.credential)
                .map(|s| s.expose().to_string()),
            None => Err("no key configured".into()),
        };
        cx.provide(ProbeState {
            greeting: config.greeting.unwrap_or_else(|| "hello".into()),
            key,
            counter: AtomicU64::new(0),
        });
        cx.events().publish("counter", json!({}), json!({"n": 0})).await?;
        Ok(())
    }
}

/* ── peer ──────────────────────────────────────────────────────────────── */

struct Peer;

#[harnix::async_trait]
impl Module for Peer {
    const API_VERSION: u32 = harnix::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new("peer", "0.0.1")
    }

    fn methods(&self, m: &mut MethodRegistry) {
        m.method("relay", Principal::Device)
            .handler(|cx: CallContext, p: Value| async move {
                let method = p["method"].as_str().unwrap_or_default().to_string();
                cx.module().call(&method, json!({})).await
            });
    }
}

/* ── failures ──────────────────────────────────────────────────────────── */

struct FailsStart;

#[harnix::async_trait]
impl Module for FailsStart {
    const API_VERSION: u32 = harnix::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new("fails-start", "0.0.1")
    }

    fn methods(&self, m: &mut MethodRegistry) {
        m.method("ping", Principal::Device)
            .handler(|_cx: CallContext, _p: Value| async move { Ok(json!("pong")) });
    }

    fn routes(&self, r: &mut RouteRegistry) {
        r.serve(
            &[Principal::Device],
            harnix::axum::Router::new().route("/hello", get(|| async { "should not be served" })),
        );
    }

    async fn start(&self, _cx: ModuleContext) -> anyhow::Result<()> {
        anyhow::bail!("this module fails to start on purpose")
    }
}

struct FailsMigrate;

#[harnix::async_trait]
impl Module for FailsMigrate {
    const API_VERSION: u32 = harnix::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new("fails-migrate", "0.0.1")
    }

    fn methods(&self, m: &mut MethodRegistry) {
        m.method("ping", Principal::Device)
            .handler(|_cx: CallContext, _p: Value| async move { Ok(json!("pong")) });
    }

    async fn migrate(&self, _cx: &MigrateContext) -> anyhow::Result<MigrationReport> {
        anyhow::bail!("this module's migration fails on purpose")
    }
}

struct Duplicate;

#[harnix::async_trait]
impl Module for Duplicate {
    const API_VERSION: u32 = harnix::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new("probe", "9.9.9")
    }

    fn methods(&self, m: &mut MethodRegistry) {
        m.method("echo", Principal::Device)
            .handler(|_cx: CallContext, _p: Value| async move { Ok(json!("the second probe")) });
    }
}
