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
//! | `fleet` | its own in-process instance and a space on it, headless sessions, a tap left unread |
//! | `lifecycle` | what `stop` does on the way out: a mark, a slow finish, one that never returns |
//! | `flow` | the flow module (xops-01) with two demonstration flows: see `flow_demo.rs` |
//! | `fakehost` | a fake runtime application registered and withdrawn through the app host: see `fake_app.rs` |
//!
//! Built with `apps`, it hosts runtime applications too, and trusts the mock store the LIVE suites
//! make (`tests/live/fixtures/store`) in place of the store's own key — which nothing but a test
//! application can do; and with `HARNIX_TESTAPP_APP_IDLE_MS` set, it hibernates an on-demand
//! application's backend after that much idle rather than its `idle_secs`.
//!
//! `probe` also refuses, before it reaches the model, any message with `blocked-word` in it (a prompt
//! middleware on every session of `system`), and `fleet` has two templates with the same native
//! contributions — a scene with a tool of its own, a marked system prompt, the same refusal — one
//! spawned and one in process.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use harnix::axum::routing::get;
use harnix::prelude::*;
use harnix::{PromptCall, PromptDecision, PromptMiddleware, Tap};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

mod fake_app;
mod flow_demo;

/// The key the LIVE suites' mock store countersigns with (its secret is beside it).
const MOCK_STORE_KEY: &str = include_str!("../../../tests/live/fixtures/store/mock-store.pub");

fn main() -> anyhow::Result<()> {
    let builder = harnix::App::builder()
        .name("harnix-testapp", "0.0.1")
        .with_builtins()
        .module(Probe)
        .module(Peer)
        .module(FailsStart)
        .module(FailsMigrate)
        .module(Duplicate)
        .module(Fleet)
        .module(Lifecycle::default())
        .module(flow_demo::module())
        .module(fake_app::FakeHost)
        // The kernel hands its application host to the `apps` built-in alone; the fake host is
        // let in by name, the way only a test application would.
        .app_host("fakehost")
        // The mock store's key, instead of the one built in: only a test application says this.
        .store_keys([MOCK_STORE_KEY.trim()]);
    // An on-demand application's backend idles this long here, so a suite sees it hibernate
    // (`tests/live/api/apps-idle.spec.ts`); a manifest's `idle_secs` is never under a minute.
    let builder = match std::env::var("HARNIX_TESTAPP_APP_IDLE_MS").ok().and_then(|ms| ms.parse().ok()) {
        Some(ms) => builder.app_idle(Duration::from_millis(ms)),
        None => builder,
    };
    builder.run()
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

    /// Every session of `system` refuses a message with `blocked-word` in it (ipdrun-13).
    fn engine(&self, e: &mut EngineContrib) {
        e.customize_session(|b, _cx| b.prompt_middleware(Arc::new(Blocklist)));
    }

    fn methods(&self, m: &mut MethodRegistry) {
        m.method("echo", Principal::Device)
            .describe("says back what it was told")
            .schema(json!({"type": "object", "required": ["text"], "properties": {"text": {"type": "string"}}}))
            .handler(|cx: CallContext, p: EchoParams| async move {
                Ok(json!({"text": p.text, "from": cx.subject()}))
            });
        m.method("sleep", Principal::Device)
            .describe("answers after `ms` milliseconds: a slow call, for the in-flight cap")
            .handler(|_cx: CallContext, p: Value| async move {
                let ms = p["ms"].as_u64().unwrap_or(100).min(10_000);
                tokio::time::sleep(Duration::from_millis(ms)).await;
                Ok(json!({"slept": ms}))
            });
        m.method("internal", Principal::Kernel)
            .handler(|_cx: CallContext, _p: Value| async move { Ok(json!({"internal": true})) });
        m.method("has_app_host", Principal::Device)
            .describe("whether the kernel handed this module the runtime-application host (it must not)")
            .handler(|cx: CallContext, _p: Value| async move { Ok(json!({"has": cx.module().apps().is_some()})) });
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
        m.method("note", Principal::Device)
            .describe("writes an entry into a session's log, and lists this module's")
            .handler(|cx: CallContext, p: Value| async move {
                let session = SessionRef {
                    instance: p["instance"].as_str().unwrap_or(harnix::SYSTEM_INSTANCE).to_string(),
                    session_id: p["session_id"].as_str().unwrap_or_default().to_string(),
                };
                let sessions = cx.module().sessions();
                let written = sessions
                    .append_entry(&session, Entry::new(p["event"].as_str().unwrap_or("note"), p["payload"].clone()))
                    .await?;
                Ok(json!({"written": written, "mine": sessions.entries(&session).await?}))
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

/* ── fleet ─────────────────────────────────────────────────────────────── */

/// Instances and sessions from the server side (design §5.2, §6.1).
///
/// At start it makes sure its own in-process instance exists — a different
/// scene from `system`, which is enough to tell the two apart — and opens
/// again every headless session it started in an earlier run.
struct Fleet;

/// The instance `fleet` builds for itself at start.
const FLEET_INSTANCE: &str = "fleet-side";

struct FleetState {
    sessions: Mutex<Vec<SessionRef>>,
    tap: Mutex<Option<Tap>>,
}

#[derive(Deserialize)]
struct HeadlessParams {
    count: usize,
    #[serde(default)]
    instance: Option<String>,
    project_root: Option<String>,
    recording: Option<String>,
    /// Sent to each at once; none leaves them idle.
    message: Option<String>,
}

#[derive(Deserialize)]
struct TapParams {
    instance: String,
    session_id: String,
    #[serde(default = "default_capacity")]
    capacity: usize,
}

fn default_capacity() -> usize {
    harnix::TAP_CAPACITY
}

#[derive(Deserialize)]
struct DrainParams {
    #[serde(default)]
    catch_up: bool,
}

impl Fleet {
    fn ledger(cx: &ModuleContext) -> std::path::PathBuf {
        cx.data_dir().join("sessions.json")
    }

    async fn save(cx: &ModuleContext, sessions: &[SessionRef]) -> Result<(), RpcError> {
        let rows: Vec<Value> = sessions
            .iter()
            .map(|s| json!({"instance": s.instance, "session_id": s.session_id}))
            .collect();
        std::fs::write(Self::ledger(cx), serde_json::to_vec(&rows).unwrap_or_default())
            .map_err(|e| RpcError::new(harnix::codes::INTERNAL_ERROR, e.to_string()))
    }
}

#[harnix::async_trait]
impl Module for Fleet {
    const API_VERSION: u32 = harnix::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new("fleet", "0.0.1")
            .disclosure("builds an instance of its own and runs headless sessions")
            .grant("session.inspectContext")
            .grant("config.getProvider")
    }

    fn templates(&self, t: &mut InstanceTemplates) {
        t.add(InstanceTemplate::new(
            "side",
            TemplateCarrier::InProcess {
                engine: EngineProfile {
                    scene: "harnix".into(),
                    ..EngineProfile::default()
                },
                contrib: EngineContrib::new(),
            },
        ));
        // Its instances see the settings page's providers (ipdrun-11).
        t.add(
            InstanceTemplate::new(
                "heir",
                TemplateCarrier::InProcess {
                    engine: EngineProfile {
                        scene: "harnix".into(),
                        ..EngineProfile::default()
                    },
                    contrib: EngineContrib::new(),
                },
            )
            .inherit_providers(),
        );
        // The same native contributions, carried two ways (ipdrun-10).
        t.add(
            InstanceTemplate::new(
                "native-spawn",
                TemplateCarrier::Spawn {
                    engine: native_engine(),
                    program: None,
                    args: Vec::new(),
                },
            )
            .native(native),
        );
        t.add(
            InstanceTemplate::new(
                "native-local",
                TemplateCarrier::InProcess {
                    engine: native_engine(),
                    contrib: EngineContrib::new(),
                },
            )
            .native(native),
        );
    }

    /// A space whose sessions land on this module's own instance.
    fn spaces(&self, s: &mut SpaceRegistry) {
        s.add(SpaceDecl::new("fleet").instance(FLEET_INSTANCE).scene("harnix"));
    }

    fn methods(&self, m: &mut MethodRegistry) {
        m.method("headless", Principal::Device)
            .describe("starts `count` sessions with a model each, and sends each the message if there is one")
            .handler(|cx: CallContext, p: HeadlessParams| async move {
                let module = cx.module().clone();
                let state = cx.state::<FleetState>()?;
                let mut started = Vec::new();
                for i in 0..p.count {
                    let session = module
                        .sessions()
                        .create(SessionSpec {
                            instance: p.instance.clone(),
                            scene: Some("harnix".into()),
                            project_root: p.project_root.clone().map(Into::into),
                            model: Some(format!("headless-model-{i}")),
                            recording: p.recording.clone(),
                            ..SessionSpec::default()
                        })
                        .await?;
                    let sent = match &p.message {
                        Some(message) => module.sessions().send(&session, message, OnBusy::Queue).await?,
                        None => Value::Null,
                    };
                    started.push(json!({
                        "instance": session.instance,
                        "session_id": session.session_id,
                        "sent": sent,
                    }));
                    state.sessions.lock().await.push(session);
                }
                Fleet::save(&module, &state.sessions.lock().await).await?;
                Ok(json!({"started": started}))
            });
        m.method("ensure_many", Principal::Device)
            .describe("`count` ensures of one name at once, from one module: what each answered")
            .handler(|cx: CallContext, p: Value| async move {
                let module = cx.module().clone();
                let template = p["template"].as_str().unwrap_or("fleet.side").to_string();
                let name = p["name"].as_str().unwrap_or_default().to_string();
                let count = p["count"].as_u64().unwrap_or(8) as usize;
                let instances = module.instances();
                let all = futures::future::join_all(
                    (0..count).map(|_| instances.ensure(&template, &name, BTreeMap::new())),
                )
                .await;
                let rows: Vec<Value> = all
                    .into_iter()
                    .map(|r| match r {
                        Ok(view) => json!({"id": view["id"]}),
                        Err(e) => json!({"error": e}),
                    })
                    .collect();
                Ok(json!({"answers": rows}))
            });
        m.method("settle_behind", Principal::Device)
            .describe("one turn watched by two taps, one too small and unread until the turn settled: how each says it ended")
            .handler(|cx: CallContext, p: Value| async move {
                let module = cx.module().clone();
                let sessions = module.sessions();
                let session = sessions
                    .create(SessionSpec {
                        scene: Some("harnix".into()),
                        project_root: p["project_root"].as_str().map(Into::into),
                        recording: p["recording"].as_str().map(str::to_string),
                        ..SessionSpec::default()
                    })
                    .await?;
                let mut small = sessions.tap_with_capacity(&session, 1).await?;
                let mut wide = sessions.tap(&session).await?;
                sessions
                    .send(&session, p["message"].as_str().unwrap_or("hello"), OnBusy::Queue)
                    .await?;
                let late = |what: &str| RpcError::new(harnix::codes::INTERNAL_ERROR, format!("{what} never settled"));
                let wide = tokio::time::timeout(Duration::from_secs(60), wide.next_settled())
                    .await
                    .map_err(|_| late("the wide tap"))?;
                let narrow = tokio::time::timeout(Duration::from_secs(5), small.next_settled())
                    .await
                    .map_err(|_| late("the small tap"))?;
                let lagged = matches!(
                    tokio::time::timeout(Duration::from_millis(500), small.next()).await,
                    Ok(Some(TapEvent::Lagged { .. }))
                );
                Ok(json!({"wide": wide, "narrow": narrow, "lagged": lagged}))
            });
        m.method("providers", Principal::Device)
            .describe("the providers an instance's engine has, as it says")
            .handler(|cx: CallContext, p: Value| async move {
                cx.module()
                    .call("config.getProvider", json!({"instance": p["instance"]}))
                    .await
            });
        m.method("ready_session", Principal::Device)
            .describe("ensure_ready an instance, then at once create a session on it")
            .handler(|cx: CallContext, p: Value| async move {
                let module = cx.module().clone();
                let template = p["template"].as_str().unwrap_or_default();
                let name = p["name"].as_str().unwrap_or_default();
                let view = module
                    .instances()
                    .ensure_ready(template, name, BTreeMap::new(), Duration::from_secs(60))
                    .await?;
                let session = module
                    .sessions()
                    .create(SessionSpec {
                        instance: Some(name.to_string()),
                        ..SessionSpec::default()
                    })
                    .await?;
                let again = module.instances().wait_ready(name, Duration::from_secs(5)).await?;
                Ok(json!({
                    "status": view["status"],
                    "session_id": session.session_id,
                    "waited": again["status"],
                }))
            });
        m.method("native_session", Principal::Device)
            .describe("ensure_ready one of this module's instances, then create a session on it in `scene`")
            .handler(|cx: CallContext, p: Value| async move {
                let module = cx.module().clone();
                let template = p["template"].as_str().unwrap_or_default();
                let name = p["name"].as_str().unwrap_or_default();
                module
                    .instances()
                    .ensure_ready(template, name, BTreeMap::new(), Duration::from_secs(60))
                    .await?;
                let session = module
                    .sessions()
                    .create(SessionSpec {
                        instance: Some(name.to_string()),
                        scene: p["scene"].as_str().map(str::to_string),
                        project_root: p["project_root"].as_str().map(Into::into),
                        recording: p["recording"].as_str().map(str::to_string),
                        ..SessionSpec::default()
                    })
                    .await?;
                Ok(json!({"instance": session.instance, "session_id": session.session_id}))
            });
        m.method("sessions", Principal::Device)
            .describe("the headless sessions this module started, in any run")
            .handler(|cx: CallContext, _p: Value| async move {
                let state = cx.state::<FleetState>()?;
                let rows: Vec<Value> = state
                    .sessions
                    .lock()
                    .await
                    .iter()
                    .map(|s| json!({"instance": s.instance, "session_id": s.session_id}))
                    .collect();
                Ok(json!({"sessions": rows}))
            });
        m.method("model", Principal::Device)
            .describe("which model a session runs, as the engine says")
            .handler(|cx: CallContext, p: Value| async move {
                let context = cx
                    .module()
                    .call(
                        "session.inspectContext",
                        json!({"instance": p["instance"], "session_id": p["session_id"]}),
                    )
                    .await?;
                Ok(json!({"model": context.get("model")}))
            });
        m.method("context", Principal::Device)
            .describe("what a session's next model call would carry, as the engine says")
            .handler(|cx: CallContext, p: Value| async move {
                cx.module()
                    .call(
                        "session.inspectContext",
                        json!({"instance": p["instance"], "session_id": p["session_id"]}),
                    )
                    .await
            });
        m.method("tap", Principal::Device)
            .describe("opens a tap on a session and does not read it")
            .handler(|cx: CallContext, p: TapParams| async move {
                let state = cx.state::<FleetState>()?;
                let session = SessionRef {
                    instance: p.instance,
                    session_id: p.session_id,
                };
                let tap = cx
                    .module()
                    .sessions()
                    .tap_with_capacity(&session, p.capacity)
                    .await?;
                *state.tap.lock().await = Some(tap);
                Ok(json!({"open": true}))
            });
        m.method("drain", Principal::Device)
            .describe("reads the tap until it has been quiet for a moment")
            .handler(|cx: CallContext, p: DrainParams| async move {
                let state = cx.state::<FleetState>()?;
                let mut guard = state.tap.lock().await;
                let Some(tap) = guard.as_mut() else {
                    return Err(RpcError::invalid_params("no tap is open"));
                };
                let mut frames = Vec::new();
                let mut lagged = Vec::new();
                let mut history_total = None;
                loop {
                    let next = tokio::time::timeout(Duration::from_millis(500), tap.next()).await;
                    match next {
                        Ok(Some(TapEvent::Frame(frame))) => frames.push(json!({
                            "method": frame.get("method"),
                            "seq": frame.get("params").and_then(|p| p.get("seq")),
                        })),
                        Ok(Some(TapEvent::Lagged { last_seq })) => {
                            lagged.push(last_seq);
                            if p.catch_up {
                                let attached = tap.catch_up().await?;
                                history_total = attached.get("history_total").cloned();
                            }
                        }
                        Ok(None) | Err(_) => break,
                    }
                }
                Ok(json!({
                    "frames": frames,
                    "lagged": lagged,
                    "last_seq": tap.last_seq(),
                    "history_total": history_total,
                }))
            });
    }

    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        cx.instances()
            .ensure("fleet.side", FLEET_INSTANCE, BTreeMap::new())
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.message))?;
        let mut sessions = Vec::new();
        if let Ok(text) = std::fs::read_to_string(Fleet::ledger(&cx)) {
            let rows: Vec<Value> = serde_json::from_str(&text).unwrap_or_default();
            for row in rows {
                let (Some(instance), Some(session_id)) =
                    (row["instance"].as_str(), row["session_id"].as_str())
                else {
                    continue;
                };
                match cx.sessions().open(instance, session_id).await {
                    Ok(session) => sessions.push(session),
                    Err(e) => tracing_warn(&format!("{session_id}: {}", e.message)),
                }
            }
        }
        cx.provide(FleetState {
            sessions: Mutex::new(sessions),
            tap: Mutex::new(None),
        });
        Ok(())
    }
}

fn tracing_warn(message: &str) {
    eprintln!("fleet: a headless session did not reopen: {message}");
}

/* ── lifecycle ─────────────────────────────────────────────────────────── */

/// What happens at the end: `stop` leaves a mark in the module's data directory, and can be told to
/// take its time or never return.
#[derive(Default)]
struct Lifecycle {
    cx: std::sync::OnceLock<ModuleContext>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LifecycleConfig {
    /// `stop` writes this many files, spread over half a second, before it returns.
    slow_stop_files: usize,
    /// `stop` never returns.
    hang_on_stop: bool,
}

#[harnix::async_trait]
impl Module for Lifecycle {
    const API_VERSION: u32 = harnix::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        Manifest::new("lifecycle", "0.0.1")
    }

    /// Also where the process listens: not known yet during `start`, and written down once it is.
    async fn start(&self, cx: ModuleContext) -> anyhow::Result<()> {
        let dir = cx.data_dir().to_path_buf();
        std::fs::write(
            dir.join("listen-at-start.json"),
            json!({"listen": cx.listen().map(|l| l.url)}).to_string(),
        )?;
        let watcher = cx.clone();
        tokio::spawn(async move {
            let listen = watcher.listening().await;
            let addrs: Vec<String> = listen.addrs.iter().map(ToString::to_string).collect();
            let _ = std::fs::write(
                dir.join("listen.json"),
                json!({"url": listen.url, "addrs": addrs}).to_string(),
            );
        });
        let _ = self.cx.set(cx);
        Ok(())
    }

    async fn stop(&self) {
        let Some(cx) = self.cx.get() else { return };
        let config: LifecycleConfig = cx.config().unwrap_or_default();
        let dir = cx.data_dir().to_path_buf();
        let _ = std::fs::write(dir.join("stopping"), "");
        if config.slow_stop_files > 0 {
            let slow = dir.join("slow");
            let _ = std::fs::create_dir_all(&slow);
            let pause = Duration::from_millis(500 / config.slow_stop_files as u64);
            for i in 0..config.slow_stop_files {
                let _ = std::fs::write(slow.join(format!("{i}")), "x");
                tokio::time::sleep(pause).await;
            }
        }
        if config.hang_on_stop {
            std::future::pending::<()>().await;
        }
        let _ = std::fs::write(dir.join("stopped"), "");
    }
}

/* ── native contributions ──────────────────────────────────────────────── */

/// What a message must not contain, for `Blocklist`.
const BLOCKED_WORD: &str = "blocked-word";

/// Refuses a message with [`BLOCKED_WORD`] in it, opening a turn or steered into one.
struct Blocklist;

#[harnix::async_trait]
impl PromptMiddleware for Blocklist {
    async fn inspect(&self, call: &PromptCall<'_>) -> PromptDecision {
        if call.content.contains(BLOCKED_WORD) {
            PromptDecision::Block {
                message: Some(format!("probe refuses messages with `{BLOCKED_WORD}` in them")),
            }
        } else {
            PromptDecision::Continue
        }
    }
}

/// The scene `native` contributes, and what its tool answers.
const NATIVE_SCENE: &str = "probe-native";
const NATIVE_TOOL: &str = "ProbeFixed";
const NATIVE_ANSWER: &str = "probe-native-answer-7f3a";
/// What `native` puts at the end of every system prompt.
const NATIVE_MARKER: &str = "probe-native-marker-51c2";

/// `fleet`'s templates' native contributions (ipdrun-10): built in this process for the in-process
/// one, and again in the daemon's for the spawned one.
fn native(e: &mut EngineContrib) {
    e.scene(Arc::new(NativeScene(harnix::scenes::harnix::HarnixScene)));
    e.customize_session(|b, _cx| {
        let blocks = harnix::agent::interface::prompt_registry::InMemoryPromptRegistry::new();
        let _ = harnix::agent::interface::prompt_registry::PromptRegistry::register_block(
            blocks.as_ref(),
            harnix::agent::interface::prompt_registry::RegisteredBlock::system("probe-native", 100_000, NATIVE_MARKER),
        );
        b.prompt_registry(blocks).prompt_middleware(Arc::new(Blocklist))
    });
}

/// The engine of `fleet`'s native templates: their scene is active beside the default one.
fn native_engine() -> EngineProfile {
    EngineProfile {
        scenes: vec![NATIVE_SCENE.into()],
        ..EngineProfile::default()
    }
}

/// The harnix scene under another id, with one tool of its own.
struct NativeScene(harnix::scenes::harnix::HarnixScene);

impl harnix::agent::interface::scene::Scene for NativeScene {
    fn id(&self) -> &str {
        NATIVE_SCENE
    }
    fn name(&self) -> &str {
        "probe native"
    }
    fn description(&self) -> &str {
        "the harnix scene with a tool of its own"
    }
    fn build_system_prompt(
        &self,
        ctx: &harnix::agent::interface::scene::ScenePromptContext,
    ) -> Vec<harnix::agent::interface::prompt::PromptBlock> {
        self.0.build_system_prompt(ctx)
    }
    fn tools(&self) -> Vec<String> {
        let mut tools = self.0.tools();
        tools.push(NATIVE_TOOL.into());
        tools
    }
    fn extra_tools(&self) -> Vec<Arc<dyn harnix::agent::tool::Tool>> {
        vec![Arc::new(FixedTool)]
    }
    fn token_budget(&self) -> harnix::agent::interface::scene::TokenBudget {
        self.0.token_budget()
    }
}

/// Answers [`NATIVE_ANSWER`], whatever it is asked.
struct FixedTool;

#[harnix::async_trait]
impl harnix::agent::tool::Tool for FixedTool {
    fn name(&self) -> &str {
        NATIVE_TOOL
    }
    fn description(&self) -> &str {
        "Answers a fixed string."
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    fn is_read_only(&self, _: &Value) -> bool {
        true
    }
    async fn call(
        &self,
        _input: Value,
        _cx: harnix::agent::tool::ToolContext,
        _progress: harnix::agent::tool::ProgressSender,
    ) -> Result<harnix::agent::tool::ToolResult, harnix::agent::error::ToolError> {
        Ok(harnix::agent::tool::ToolResult::text(NATIVE_ANSWER))
    }
}
