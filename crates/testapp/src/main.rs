//! `harnix-testapp` — Harnix's own extensions plus extensions the LIVE suites drive.
//!
//! Each extension exists to show one thing about the extension interface, and some fail on
//! purpose: a failure has to stay with the extension that failed.
//!
//! | extension | shows |
//! |---|---|
//! | `probe` | typed methods per caller, config and credentials, topics, a route, a migration |
//! | `peer` | one extension calling another as itself |
//! | `fails-start` | a failed `start` disables only that extension |
//! | `fails-migrate` | a failed `migrate` disables only that extension |
//! | `probe` (again) | a second extension with a taken id is refused at registration |
//! | `fleet` | its own in-process instance and a space on it, headless sessions, a tap left unread |
//! | `lifecycle` | what `stop` does on the way out: a mark, a slow finish, one that never returns |
//! | `flow` | the flow extension (xops-01) with two demonstration flows: see `flow_demo.rs` |
//! | `fakehost` | a fake package registered and withdrawn through the host's registrar: see `fake_app.rs` |
//!
//! Built with `wasm`, it runs packages too, and trusts the mock store the LIVE suites make
//! (`tests/live/fixtures/store`) in place of the store's own key — which nothing but a test
//! application can do; and with `HARNIX_TESTAPP_APP_IDLE_MS` set, it lets an on-demand package's
//! backend go after that much idle rather than its `idle_secs`.
//!
//! Built with `proc`, it is also an extension process: `harnix-testapp ext-proc …` runs one
//! extension package behind the extension wire, as a product in mode 3 starts it.
//!
//! `probe` also refuses, before it reaches the model, any message with `blocked-word` in it (a
//! prompt gate on every session of `main`), and `fleet` has two templates with the same native
//! contributions — a scene with a tool of its own, a marked system prompt, the same refusal — one
//! spawned and one in process.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use harnix::axum::routing::get;
use harnix::engine::{
    EngineContribution, Permission, PromptDecision, PromptFacts, PromptGate, PromptText, SceneBase, SceneLayer, Tool, ToolCx,
    ToolError, ToolOutput, ToolSpec,
};
use harnix::prelude::*;
use harnix::{ExtId, Tap};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

mod fake_app;
mod flow_demo;

/// The key the LIVE suites' mock store countersigns with (its secret is beside it).
#[cfg(any(feature = "wasm", feature = "proc"))]
const MOCK_STORE_KEY: &str = include_str!("../../../tests/live/fixtures/store/mock-store.pub");

fn main() -> anyhow::Result<()> {
    let builder = harnix::App::builder()
        .name("harnix-testapp", "0.0.1")
        .extensions(harnix::first_party::standard())
        .extension(Probe)
        .ungranted()
        .extension(Peer)
        .ungranted()
        .extension(FailsStart)
        .ungranted()
        .extension(FailsMigrate)
        .ungranted()
        .extension(Duplicate)
        .ungranted()
        .extension(Fleet)
        .grant([harnix::caps::ENGINE_INSPECT, harnix::caps::ENGINE_CONFIG_READ])
        .extension(Lifecycle::default())
        .ungranted()
        .extension(flow_demo::module())
        .grant([harnix::caps::ENGINE_CONTROL, harnix::caps::ENGINE_INSPECT])
        // The host's registrar, the way the `packages` component holds it.
        .extension(fake_app::FakeHost)
        .grant([harnix::caps::EXTENSIONS_MANAGE]);
    #[cfg(any(feature = "wasm", feature = "proc"))]
    let builder = {
        // The mock store's key, instead of the one built in: only a test application says this.
        let packages = harnix::first_party::packages().store_keys([MOCK_STORE_KEY.trim()]);
        // An on-demand package's backend idles this long here, so a suite sees it let go
        // (`tests/live/api/packages-idle.spec.ts`); a manifest's `idle_secs` is never under a
        // minute.
        let packages = match std::env::var("HARNIX_TESTAPP_APP_IDLE_MS").ok().and_then(|ms| ms.parse().ok()) {
            Some(ms) => packages.idle(Duration::from_millis(ms)),
            None => packages,
        };
        builder.extension(packages).grant(harnix::first_party::PACKAGES_GRANTS)
    };
    builder.run()
}

/// Open to whoever the row names: a person, and every extension.
fn device() -> Access {
    Access::device().with_extensions()
}

/* ── probe ─────────────────────────────────────────────────────────────── */

struct Probe;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
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
impl Extension for Probe {
    type Config = ProbeConfig;

    fn manifest(&self) -> Manifest {
        Manifest::new("probe", "0.0.1").disclosure("an extension for the tests")
    }

    fn register(&self, r: &mut Registrar<'_, ProbeConfig>) {
        // Every session of `main` refuses a message with `blocked-word` in it (ipdrun-13).
        r.engine().prompt_gate(Arc::new(Blocklist));
        r.method("echo")
            .access(device())
            .describe("says back what it was told")
            .schema(json!({"type": "object", "required": ["text"], "properties": {"text": {"type": "string"}}}))
            .handler(|call: Call, p: EchoParams| async move { Ok(json!({"text": p.text, "from": call.subject()})) });
        r.method("sleep")
            .access(device())
            .describe("answers after `ms` milliseconds: a slow call, for the in-flight cap")
            .raw(|_call, p| async move {
                let ms = p["ms"].as_u64().unwrap_or(100).min(10_000);
                tokio::time::sleep(Duration::from_millis(ms)).await;
                Ok(json!({"slept": ms}))
            });
        r.method("internal").raw(|_call, _p| async move { Ok(json!({"internal": true})) });
        r.method("for_peer")
            .access(Access::only(ExtId::native("peer").expect("an id")))
            .raw(|call, _p| async move { Ok(json!({"caller": call.subject()})) });
        r.method("config").access(device()).raw(|call, _p| async move {
            let state = call.state::<ProbeState>()?;
            Ok(json!({
                "greeting": state.greeting,
                "key": state.key.as_ref().map(|k| format!("{} chars", k.len())).map_err(Clone::clone),
                "data_dir": call.ext().data_dir(),
            }))
        });
        r.method("bump").access(device()).raw(|call, p| async move {
            let state = call.state::<ProbeState>()?;
            let n = state.counter.fetch_add(1, Ordering::SeqCst) + 1;
            let topics = call.ext().topics();
            topics.publish("counter", json!({}), json!({"n": n})).await?;
            if let Some(job) = p.get("job_id") {
                topics.publish("job", json!({"job_id": job}), json!({"job_id": job, "n": n})).await?;
            }
            Ok(json!({"n": n}))
        });
        r.method("note")
            .access(device())
            .describe("writes an entry into a session's log, and lists this extension's")
            .raw(|call, p| async move {
                let session = SessionRef::new(
                    p["instance"].as_str().unwrap_or(harnix::MAIN_INSTANCE),
                    p["session_id"].as_str().unwrap_or_default(),
                );
                let sessions = call.ext().sessions();
                let written = sessions
                    .append_entry(&session, Entry::new(p["event"].as_str().unwrap_or("note"), p["payload"].clone()))
                    .await?;
                Ok(json!({"written": written, "mine": sessions.entries(&session).await?}))
            });
        r.method("call").access(device()).describe("calls another method as this extension").raw(|call, p| async move {
            let method = p["method"].as_str().unwrap_or_default().to_string();
            call.call(&method, p["params"].clone()).await
        });
        r.topic("counter");
        r.topic("job").param("job_id");
        r.topic("secret").access(Access::kernel());
        let router = harnix::axum::Router::new().route("/hello", get(|| async { "hello from probe" }));
        r.route("", device(), harnix::ext::http::axum::router(router));
    }

    async fn migrate(&self, cx: &MigrateContext) -> anyhow::Result<MigrationReport> {
        let marker = cx.data_dir().join("schema");
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

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        let config: ProbeConfig = cx.config()?;
        let key = match &config.key {
            Some(r) => cx.credentials().get(&r.credential).map(|s| s.expose().to_string()),
            None => Err("no key configured".into()),
        };
        cx.provide(ProbeState {
            greeting: config.greeting.unwrap_or_else(|| "hello".into()),
            key,
            counter: AtomicU64::new(0),
        });
        cx.topics().publish("counter", json!({}), json!({"n": 0})).await?;
        Ok(())
    }
}

/* ── peer ──────────────────────────────────────────────────────────────── */

struct Peer;

#[harnix::async_trait]
impl Extension for Peer {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new("peer", "0.0.1")
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.method("relay").access(device()).raw(|call, p| async move {
            let method = p["method"].as_str().unwrap_or_default().to_string();
            call.call(&method, json!({})).await
        });
    }
}

/* ── failures ──────────────────────────────────────────────────────────── */

struct FailsStart;

#[harnix::async_trait]
impl Extension for FailsStart {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new("fails-start", "0.0.1")
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.method("ping").access(device()).raw(|_call, _p| async move { Ok(json!("pong")) });
        let router = harnix::axum::Router::new().route("/hello", get(|| async { "should not be served" }));
        r.route("", device(), harnix::ext::http::axum::router(router));
    }

    async fn start(&self, _cx: ExtensionContext) -> anyhow::Result<()> {
        anyhow::bail!("this extension fails to start on purpose")
    }
}

struct FailsMigrate;

#[harnix::async_trait]
impl Extension for FailsMigrate {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new("fails-migrate", "0.0.1")
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.method("ping").access(device()).raw(|_call, _p| async move { Ok(json!("pong")) });
    }

    async fn migrate(&self, _cx: &MigrateContext) -> anyhow::Result<MigrationReport> {
        anyhow::bail!("this extension's migration fails on purpose")
    }
}

struct Duplicate;

#[harnix::async_trait]
impl Extension for Duplicate {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new("probe", "9.9.9")
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.method("echo").access(device()).raw(|_call, _p| async move { Ok(json!("the second probe")) });
    }
}

/* ── fleet ─────────────────────────────────────────────────────────────── */

/// Instances and sessions from the server side (concept §3.2, §3.1.1).
///
/// At start it makes sure its own in-process instance exists — a different scene from `main`,
/// which is enough to tell the two apart — and opens again every headless session it started in
/// an earlier run.
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

/// A session spec from what a test asked for.
fn spec(instance: Option<&str>, scene: Option<&str>, project_root: Option<&str>, recording: Option<&str>) -> SessionSpec {
    let mut spec = SessionSpec::new();
    if let Some(instance) = instance {
        spec = spec.instance(instance);
    }
    if let Some(scene) = scene {
        spec = spec.scene(scene);
    }
    if let Some(root) = project_root {
        spec = spec.project_root(root);
    }
    if let Some(recording) = recording {
        spec = spec.recording(recording);
    }
    spec
}

impl Fleet {
    fn ledger(cx: &ExtensionContext) -> std::path::PathBuf {
        cx.data_dir().join("sessions.json")
    }

    async fn save(cx: &ExtensionContext, sessions: &[SessionRef]) -> Result<(), RpcError> {
        let rows: Vec<Value> = sessions.iter().map(|s| json!({"instance": s.instance, "session_id": s.session_id})).collect();
        std::fs::write(Self::ledger(cx), serde_json::to_vec(&rows).unwrap_or_default())
            .map_err(|e| RpcError::new(harnix::codes::INTERNAL_ERROR, e.to_string()))
    }
}

#[harnix::async_trait]
impl Extension for Fleet {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new("fleet", "0.0.1")
            .disclosure("builds an instance of its own and runs headless sessions")
            .requires([harnix::caps::ENGINE_INSPECT, harnix::caps::ENGINE_CONFIG_READ])
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        r.template(InstanceTemplate::in_process("side", EngineProfile::new().scene("harnix")));
        // Its instances see the settings page's providers (ipdrun-11).
        r.template(InstanceTemplate::in_process("heir", EngineProfile::new().scene("harnix")).inherit_providers());
        // The same native contributions, carried two ways (ipdrun-10).
        r.template(InstanceTemplate::spawn("native-spawn", native_engine()).contribute(native));
        r.template(InstanceTemplate::in_process("native-local", native_engine()).contribute(native));
        // A space whose sessions land on this extension's own instance.
        r.space(SpaceDecl::new("fleet").instance(FLEET_INSTANCE).scene("harnix"));

        r.method("headless")
            .access(device())
            .describe("starts `count` sessions with a model each, and sends each the message if there is one")
            .handler(|call: Call, p: HeadlessParams| async move {
                let cx = call.ext().clone();
                let state = call.state::<FleetState>()?;
                let mut started = Vec::new();
                for i in 0..p.count {
                    let spec = spec(p.instance.as_deref(), Some("harnix"), p.project_root.as_deref(), p.recording.as_deref())
                        .model(&format!("headless-model-{i}"));
                    let session = cx.sessions().create(spec).await?;
                    let sent = match &p.message {
                        Some(message) => cx.sessions().send(&session, message, OnBusy::Queue).await?,
                        None => Value::Null,
                    };
                    started.push(json!({"instance": session.instance, "session_id": session.session_id, "sent": sent}));
                    state.sessions.lock().await.push(session);
                }
                Fleet::save(&cx, &state.sessions.lock().await).await?;
                Ok(json!({"started": started}))
            });
        r.method("ensure_many")
            .access(device())
            .describe("`count` ensures of one name at once, from one extension: what each answered")
            .raw(|call, p| async move {
                let cx = call.ext().clone();
                let template = p["template"].as_str().unwrap_or("fleet.side").to_string();
                let name = p["name"].as_str().unwrap_or_default().to_string();
                let count = p["count"].as_u64().unwrap_or(8) as usize;
                let instances = cx.instances();
                let all = futures::future::join_all((0..count).map(|_| instances.ensure(&template, &name, BTreeMap::new()))).await;
                let rows: Vec<Value> = all
                    .into_iter()
                    .map(|r| match r {
                        Ok(view) => json!({"id": view["id"]}),
                        Err(e) => json!({"error": e}),
                    })
                    .collect();
                Ok(json!({"answers": rows}))
            });
        r.method("settle_behind")
            .access(device())
            .describe("one turn watched by two taps, one too small and unread until the turn settled: how each says it ended")
            .raw(|call, p| async move {
                let sessions = call.ext().sessions();
                let session = sessions
                    .create(spec(None, Some("harnix"), p["project_root"].as_str(), p["recording"].as_str()))
                    .await?;
                let mut small = sessions.tap_with_capacity(&session, 1).await?;
                let mut wide = sessions.tap(&session).await?;
                sessions.send(&session, p["message"].as_str().unwrap_or("hello"), OnBusy::Queue).await?;
                let late = |what: &str| RpcError::new(harnix::codes::INTERNAL_ERROR, format!("{what} never settled"));
                let wide = tokio::time::timeout(Duration::from_secs(60), wide.next_settled()).await.map_err(|_| late("the wide tap"))?;
                let narrow =
                    tokio::time::timeout(Duration::from_secs(5), small.next_settled()).await.map_err(|_| late("the small tap"))?;
                let lagged = matches!(
                    tokio::time::timeout(Duration::from_millis(500), small.next()).await,
                    Ok(Some(TapEvent::Lagged { .. }))
                );
                Ok(json!({"wide": wide, "narrow": narrow, "lagged": lagged}))
            });
        r.method("providers").access(device()).describe("the providers an instance's engine has, as it says").raw(|call, p| async move {
            call.call("engine.config.getProvider", json!({"instance": p["instance"]})).await
        });
        r.method("ready_session")
            .access(device())
            .describe("ensure_ready an instance, then at once create a session on it")
            .raw(|call, p| async move {
                let cx = call.ext().clone();
                let template = p["template"].as_str().unwrap_or_default();
                let name = p["name"].as_str().unwrap_or_default();
                let view = cx.instances().ensure_ready(template, name, BTreeMap::new(), Duration::from_secs(60)).await?;
                let session = cx.sessions().create(spec(Some(name), None, None, None)).await?;
                let again = cx.instances().wait_ready(name, Duration::from_secs(5)).await?;
                Ok(json!({"status": view["status"], "session_id": session.session_id, "waited": again["status"]}))
            });
        r.method("native_session")
            .access(device())
            .describe("ensure_ready one of this extension's instances, then create a session on it in `scene`")
            .raw(|call, p| async move {
                let cx = call.ext().clone();
                let template = p["template"].as_str().unwrap_or_default();
                let name = p["name"].as_str().unwrap_or_default();
                cx.instances().ensure_ready(template, name, BTreeMap::new(), Duration::from_secs(60)).await?;
                let session = cx
                    .sessions()
                    .create(spec(Some(name), p["scene"].as_str(), p["project_root"].as_str(), p["recording"].as_str()))
                    .await?;
                Ok(json!({"instance": session.instance, "session_id": session.session_id}))
            });
        r.method("sessions").access(device()).describe("the headless sessions this extension started, in any run").raw(
            |call, _p| async move {
                let state = call.state::<FleetState>()?;
                let rows: Vec<Value> = state
                    .sessions
                    .lock()
                    .await
                    .iter()
                    .map(|s| json!({"instance": s.instance, "session_id": s.session_id}))
                    .collect();
                Ok(json!({"sessions": rows}))
            },
        );
        r.method("model").access(device()).describe("which model a session runs, as the engine says").raw(|call, p| async move {
            let context = call
                .call("engine.session.inspectContext", json!({"instance": p["instance"], "session_id": p["session_id"]}))
                .await?;
            Ok(json!({"model": context.get("model")}))
        });
        r.method("context").access(device()).describe("what a session's next model call would carry, as the engine says").raw(
            |call, p| async move {
                call.call("engine.session.inspectContext", json!({"instance": p["instance"], "session_id": p["session_id"]})).await
            },
        );
        r.method("tap").access(device()).describe("opens a tap on a session and does not read it").handler(
            |call: Call, p: TapParams| async move {
                let state = call.state::<FleetState>()?;
                let session = SessionRef::new(p.instance, p.session_id);
                let tap = call.ext().sessions().tap_with_capacity(&session, p.capacity).await?;
                *state.tap.lock().await = Some(tap);
                Ok(json!({"open": true}))
            },
        );
        r.method("drain").access(device()).describe("reads the tap until it has been quiet for a moment").handler(
            |call: Call, p: DrainParams| async move {
                let state = call.state::<FleetState>()?;
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
                        Ok(Some(_)) | Ok(None) | Err(_) => break,
                    }
                }
                Ok(json!({"frames": frames, "lagged": lagged, "last_seq": tap.last_seq(), "history_total": history_total}))
            },
        );
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        cx.instances()
            .ensure("fleet.side", FLEET_INSTANCE, BTreeMap::new())
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.message))?;
        let mut sessions = Vec::new();
        if let Ok(text) = std::fs::read_to_string(Fleet::ledger(&cx)) {
            let rows: Vec<Value> = serde_json::from_str(&text).unwrap_or_default();
            for row in rows {
                let (Some(instance), Some(session_id)) = (row["instance"].as_str(), row["session_id"].as_str()) else {
                    continue;
                };
                match cx.sessions().open(instance, session_id).await {
                    Ok(session) => sessions.push(session),
                    Err(e) => tracing::warn!(session = session_id, error = %e.message, "a headless session did not reopen"),
                }
            }
        }
        cx.provide(FleetState { sessions: Mutex::new(sessions), tap: Mutex::new(None) });
        Ok(())
    }
}

/* ── lifecycle ─────────────────────────────────────────────────────────── */

/// What happens at the end: `stop` leaves a mark in the extension's data directory, and can be
/// told to take its time or never return.
#[derive(Default)]
struct Lifecycle {
    cx: std::sync::OnceLock<ExtensionContext>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct LifecycleConfig {
    /// `stop` writes this many files, spread over half a second, before it returns.
    slow_stop_files: usize,
    /// `stop` never returns.
    hang_on_stop: bool,
}

#[harnix::async_trait]
impl Extension for Lifecycle {
    type Config = LifecycleConfig;

    fn manifest(&self) -> Manifest {
        Manifest::new("lifecycle", "0.0.1")
    }

    fn register(&self, _r: &mut Registrar<'_, LifecycleConfig>) {}

    /// Also where the process listens: not known yet during `start`, and written down once it is.
    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        let dir = cx.data_dir().to_path_buf();
        std::fs::write(dir.join("listen-at-start.json"), json!({"listen": cx.listen().map(|l| l.url().to_string())}).to_string())?;
        let watcher = cx.clone();
        tokio::spawn(async move {
            let listen = watcher.listening().await;
            let addrs: Vec<String> = listen.addrs().iter().map(ToString::to_string).collect();
            let _ = std::fs::write(dir.join("listen.json"), json!({"url": listen.url(), "addrs": addrs}).to_string());
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
impl PromptGate for Blocklist {
    async fn inspect(&self, message: &PromptFacts) -> PromptDecision {
        if message.content().contains(BLOCKED_WORD) {
            PromptDecision::Block(format!("probe refuses messages with `{BLOCKED_WORD}` in them"))
        } else {
            PromptDecision::Proceed
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
fn native(e: &mut EngineContribution) {
    e.scene(
        SceneLayer::new(NATIVE_SCENE, SceneBase::builtin("harnix"))
            .name("probe native")
            .description("the harnix scene with a tool of its own")
            .tool(Arc::new(FixedTool::new()))
            .prompt(PromptText::plain(NATIVE_MARKER))
            // Naming the session is a model call no recording has.
            .auto_name(false),
    );
    e.prompt_gate(Arc::new(Blocklist));
}

/// The engine of `fleet`'s native templates: the engine's own scenes; [`NATIVE_SCENE`] is a layer
/// over one, which the hub sets a session up from.
fn native_engine() -> EngineProfile {
    EngineProfile::new()
}

/// Answers [`NATIVE_ANSWER`], whatever it is asked.
struct FixedTool(ToolSpec);

impl FixedTool {
    fn new() -> Self {
        Self(ToolSpec::new(NATIVE_TOOL, "Answers a fixed string.", json!({"type": "object", "properties": {}})).read_only())
    }
}

#[harnix::async_trait]
impl Tool for FixedTool {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }
    fn permission(&self, _: &Value) -> Permission {
        Permission::Allow
    }
    async fn call(&self, _cx: ToolCx, _input: Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(NATIVE_ANSWER))
    }
}
