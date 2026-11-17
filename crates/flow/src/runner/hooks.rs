//! Hooks: each event of a run is told to the hooks of the pack that want it. No flow names a hook;
//! one put in the pack applies from the next event on. A hook never blocks a run: deliveries go
//! out in order, one hook at a time, each tried again on failure and then given up.
//!
//! `webhook` (the one kind there is): `POST` to the hook's `action`
//!
//! ```text
//! {hook, delivery, event: {kind, run, flow, at, station?, task?, data?},
//!  run: {id, flow, title, subject, status, phase, usage}}
//! <P>-Event: <kind>   <P>-Delivery: <delivery>   <P>-Timestamp: <ms>   <P>-Signature: sha256=<hex>
//! ```
//!
//! `<P>` is the host's header prefix (`RunnerConfig::hook_headers`; Harnix: `X-Harnix`). The
//! signature (with `secret_env`) is the HMAC-SHA256 of `<timestamp>.<body>` under the variable's
//! value: the other side checks it and refuses an old timestamp. A delivery keeps its id over its
//! tries, so the other side can tell a repeat. `run` is the run as it is when the delivery is
//! made, which may be later than the event. A hook's queue holds at most [`QUEUE_KEPT`] deliveries
//! (beyond them the newest are dropped, and said so); its worker goes away once it has had nothing
//! to do for a while. A URL is never shown with its query or credentials.
//!
//! How a delivery goes out is a [`HookTransport`]: [`Webhooks`] over HTTP (feature `webhooks`), or
//! whatever the host gives.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::lock;
use super::Runner;
use crate::kernel::host::Event;
use crate::kernel::parts::{HookDef, HookKind};

/// How long one try may take.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// The waits before each try again.
const BACKOFF: [Duration; 3] = [Duration::from_secs(1), Duration::from_secs(5), Duration::from_secs(30)];

/// The most deliveries a hook's queue holds.
pub const QUEUE_KEPT: usize = 1000;
/// How long a hook's worker waits for work before it goes away.
const IDLE: Duration = Duration::from_secs(600);

/// How a webhook delivery goes out: one `POST`, the status code of a success, why not otherwise.
/// The error must not carry the URL, which may hold a token.
#[async_trait::async_trait]
pub trait HookTransport: Send + Sync {
    async fn post(&self, url: &str, headers: &[(String, String)], body: Vec<u8>) -> Result<u16, String>;
}

/// Webhooks over HTTP (`reqwest`): the environment's proxy for the world, none for this machine.
#[cfg(feature = "webhooks")]
pub struct Webhooks {
    world: Result<reqwest::Client, String>,
    local: Result<reqwest::Client, String>,
}

#[cfg(feature = "webhooks")]
impl Default for Webhooks {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "webhooks")]
impl Webhooks {
    pub fn new() -> Self {
        Self {
            world: reqwest::Client::builder().timeout(TIMEOUT).build().map_err(|e| e.without_url().to_string()),
            local: reqwest::Client::builder().timeout(TIMEOUT).no_proxy().build().map_err(|e| e.without_url().to_string()),
        }
    }
}

#[cfg(feature = "webhooks")]
#[async_trait::async_trait]
impl HookTransport for Webhooks {
    async fn post(&self, url: &str, headers: &[(String, String)], body: Vec<u8>) -> Result<u16, String> {
        let client = if is_loopback(url) { &self.local } else { &self.world };
        let client = client.as_ref().map_err(|e| format!("no HTTP client: {e}"))?;
        let mut req = client.post(url).header("content-type", "application/json");
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        // The error without the URL: it may carry a token.
        let res = req.body(body).send().await.map_err(|e| e.without_url().to_string())?;
        let code = res.status().as_u16();
        if res.status().is_success() {
            Ok(code)
        } else {
            Err(format!("HTTP {code}"))
        }
    }
}

/// One delivery waiting to go out.
pub(crate) struct Delivery {
    hook: HookDef,
    body: Value,
    kind: String,
    run: String,
}

/// The hooks' side of the runner: each hook's queue, by name (one worker per hook keeps its
/// deliveries in order), each hook's last delivery, and how they go out.
pub(crate) struct Hooks {
    queues: Mutex<HashMap<String, mpsc::Sender<Delivery>>>,
    /// Each hook's last delivery: hook → `{at, event, status, error?}`.
    status: Mutex<BTreeMap<String, Value>>,
    transport: Option<Arc<dyn HookTransport>>,
}

impl Hooks {
    /// With `transport`, or the HTTP one when the feature is on; with neither, hooks are not
    /// delivered (and said so in their status).
    pub(crate) fn new(transport: Option<Arc<dyn HookTransport>>) -> Self {
        #[cfg(feature = "webhooks")]
        let transport = transport.or_else(|| Some(Arc::new(Webhooks::new()) as Arc<dyn HookTransport>));
        Self { queues: Mutex::new(HashMap::new()), status: Mutex::new(BTreeMap::new()), transport }
    }

    fn set_status(&self, hook: &str, v: Value) {
        lock(&self.status).insert(hook.to_string(), v);
    }

    /// The hooks of the pack with each one's last delivery.
    pub(crate) fn listing(&self, hooks: &[HookDef]) -> Vec<Value> {
        let status = lock(&self.status);
        hooks
            .iter()
            .map(|h| {
                let mut v = serde_json::to_value(h).unwrap_or(Value::Null);
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("secret_env");
                    obj.insert("action".into(), json!(redact_url(&h.action)));
                    obj.insert("signed".into(), json!(h.secret_env.is_some()));
                    obj.insert("last".into(), status.get(&h.name).cloned().unwrap_or(Value::Null));
                }
                v
            })
            .collect()
    }
}

/// Tell the hooks that want them about `events` (already committed).
pub(crate) async fn notify(rn: &Arc<Runner>, events: &[Event]) {
    if events.is_empty() || rn.stopping() {
        return;
    }
    let engine = rn.engine();
    if engine.packs().hooks().is_empty() && !rn.trust().any_trusted() {
        return;
    }
    // A run in a project is told to its project's hooks too (the project's packs over the rest).
    let mut runs: HashMap<String, Option<(Value, Vec<HookDef>)>> = HashMap::new();
    for e in events {
        if !runs.contains_key(&e.run) {
            let seen = engine.record(&e.run).await.ok().and_then(|r| {
                let hooks = engine.packs_for(r.project_root.as_deref()).hooks();
                let summary = serde_json::to_value(r.summary()).ok()?;
                Some((summary, hooks))
            });
            runs.insert(e.run.clone(), seen);
        }
        let Some(Some((run, hooks))) = runs.get(&e.run) else {
            continue;
        };
        let flow = run.get("flow").and_then(Value::as_str).unwrap_or_default();
        for h in hooks.iter().filter(|h| h.wants(&e.kind, flow)) {
            let body = body_of(h, e, flow, run);
            let d = Delivery { hook: h.clone(), body, kind: e.kind.clone(), run: e.run.clone() };
            enqueue(rn, d);
        }
    }
}

/// What a hook is sent for an event.
pub(crate) fn body_of(h: &HookDef, e: &Event, flow: &str, run: &Value) -> Value {
    let pick = |k: &str| run.get(k).cloned().unwrap_or(Value::Null);
    json!({
        "hook": h.name,
        "delivery": uuid::Uuid::new_v4().simple().to_string(),
        "event": {
            "kind": e.kind, "run": e.run, "flow": flow, "at": e.at,
            "station": e.station, "task": e.task,
            "data": e.data,
        },
        "run": {
            "id": pick("id"), "flow": pick("flow"), "title": pick("title"), "subject": pick("subject"),
            "status": pick("status"), "phase": pick("phase"), "usage": pick("usage"),
        },
    })
}

fn enqueue(rn: &Arc<Runner>, d: Delivery) {
    let mut queues = lock(&rn.hooks.queues);
    let name = d.hook.name.clone();
    if let Some(tx) = queues.get(&name) {
        match tx.try_send(d) {
            Ok(()) => return,
            Err(mpsc::error::TrySendError::Full(d)) => {
                tracing::warn!(hook = name, event = d.kind, "a hook's queue is full; a delivery is dropped");
                rn.hooks.set_status(
                    &name,
                    json!({"at": now_ms(), "event": d.kind, "run": d.run, "status": "dropped",
                        "error": format!("more than {QUEUE_KEPT} deliveries waiting")}),
                );
                return;
            }
            Err(mpsc::error::TrySendError::Closed(d)) => {
                queues.remove(&name);
                drop(queues);
                return enqueue(rn, d);
            }
        }
    }
    let (tx, rx) = mpsc::channel(QUEUE_KEPT);
    let _ = tx.try_send(d);
    queues.insert(name.clone(), tx);
    let rn2 = rn.clone();
    rn.spawn(async move { work(rn2, name, rx).await });
}

async fn work(rn: Arc<Runner>, name: String, mut rx: mpsc::Receiver<Delivery>) {
    let Some(transport) = rn.hooks.transport.clone() else {
        tracing::warn!(hook = name, "hooks have no transport here");
        while let Some(d) = rx.recv().await {
            rn.hooks.set_status(
                &d.hook.name,
                json!({"at": now_ms(), "event": d.kind, "run": d.run, "status": "failed", "error": "no transport for hooks here"}),
            );
        }
        return;
    };
    loop {
        let next = tokio::select! {
            next = tokio::time::timeout(IDLE, rx.recv()) => next,
            _ = rn.cancelled().cancelled() => return,
        };
        let d = match next {
            Ok(Some(d)) => d,
            Ok(None) => return,
            Err(_) => {
                // Nothing for a while: go, unless something came in meanwhile.
                let mut queues = lock(&rn.hooks.queues);
                rx.close();
                match rx.try_recv() {
                    Ok(d) => {
                        drop(queues);
                        enqueue(&rn, d);
                        while let Ok(d) = rx.try_recv() {
                            enqueue(&rn, d);
                        }
                    }
                    Err(_) => {
                        queues.remove(&name);
                    }
                }
                return;
            }
        };
        if rn.stopping() {
            return;
        }
        let mut tries = 0usize;
        let outcome = loop {
            tries += 1;
            let r = send(transport.as_ref(), &rn.cfg.hook_headers, &d).await;
            match (&r, BACKOFF.get(tries - 1)) {
                (Ok(_), _) | (Err(_), None) => break r,
                (Err(_), Some(wait)) => {
                    tokio::select! {
                        _ = tokio::time::sleep(*wait) => {}
                        _ = rn.cancelled().cancelled() => return,
                    }
                }
            }
        };
        let mut status = json!({
            "at": now_ms(), "event": d.kind, "run": d.run, "tries": tries,
            "delivery": d.body.get("delivery"),
        });
        match outcome {
            Ok(code) => {
                status["status"] = json!("delivered");
                status["code"] = json!(code);
            }
            Err(e) => {
                tracing::warn!(hook = d.hook.name, event = d.kind, error = %e, "hook not delivered");
                status["status"] = json!("failed");
                status["error"] = json!(e);
            }
        }
        rn.hooks.set_status(&d.hook.name, status);
    }
}

/// The headers a delivery carries, under the host's prefix.
pub(crate) fn headers_of(prefix: &str, d: &Delivery, ts: &str, signature: Option<String>) -> Vec<(String, String)> {
    let p = prefix.trim_end_matches('-').to_ascii_lowercase();
    let mut h = vec![
        (format!("{p}-event"), d.kind.clone()),
        (format!("{p}-delivery"), d.body.get("delivery").and_then(Value::as_str).unwrap_or_default().to_string()),
        (format!("{p}-timestamp"), ts.to_string()),
    ];
    if let Some(s) = signature {
        h.push((format!("{p}-signature"), s));
    }
    h
}

/// One try; the status code of a success.
async fn send(transport: &dyn HookTransport, prefix: &str, d: &Delivery) -> Result<u16, String> {
    match d.hook.kind {
        HookKind::Webhook => {
            let body = serde_json::to_vec(&d.body).map_err(|e| e.to_string())?;
            let ts = now_ms().to_string();
            let signature = match &d.hook.secret_env {
                Some(var) => {
                    let secret = std::env::var(var).map_err(|_| format!("{var} is not set"))?;
                    let signed = [ts.as_bytes(), b".", &body].concat();
                    Some(format!("sha256={}", hmac_sha256(secret.as_bytes(), &signed)))
                }
                None => None,
            };
            transport.post(&d.hook.action, &headers_of(prefix, d, &ts, signature), body).await
        }
    }
}

/// Whether a URL is this machine's.
pub(crate) fn is_loopback(url: &str) -> bool {
    let host = url.split_once("://").map_or(url, |(_, r)| r).split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = if host.starts_with('[') {
        host.split(']').next().unwrap_or_default().trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or_default()
    };
    host == "localhost"
        || host.parse::<std::net::IpAddr>().is_ok_and(|ip| match ip {
            std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_unspecified(),
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback() || v6.is_unspecified() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
            }
        })
}

/// A URL as it may be shown: no credentials, no query, no fragment.
pub fn redact_url(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let cut = rest.len() > authority.len() + path.len() || authority.contains('@');
    let shown = if scheme.is_empty() { format!("{host}{path}") } else { format!("{scheme}://{host}{path}") };
    if cut {
        format!("{shown} (…)")
    } else {
        shown
    }
}

/// Hex HMAC-SHA256 (RFC 2104).
pub(crate) fn hmac_sha256(key: &[u8], msg: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    const BLOCK: usize = 64;
    let mut k = if key.len() > BLOCK { Sha256::digest(key).to_vec() } else { key.to_vec() };
    k.resize(BLOCK, 0);
    let pad = |b: u8| k.iter().map(|x| x ^ b).collect::<Vec<u8>>();
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(msg).finalize();
    let outer = Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize();
    outer.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{RunnerConfig, ScriptedHost};
    use crate::{Actor, CreateIn, Engine, MemPacks, MemStore};

    #[test]
    fn this_machine_is_not_reached_through_a_proxy() {
        for url in ["http://127.0.0.1:4099/h", "http://localhost/x", "http://[::1]:80/", "http://u@127.0.0.2/"] {
            assert!(is_loopback(url), "{url}");
        }
        for url in ["https://erp.example.com/h", "http://10.0.0.1/", "http://localhost.example/"] {
            assert!(!is_loopback(url), "{url}");
        }
        assert!(is_loopback("http://[::ffff:127.0.0.1]:8/") && is_loopback("http://0.0.0.0:9/"));
    }

    #[test]
    fn a_url_is_shown_without_what_it_may_carry() {
        assert_eq!(redact_url("https://erp.example.com/h"), "https://erp.example.com/h");
        assert_eq!(redact_url("https://u:pw@erp.example.com/h?token=s#x"), "https://erp.example.com/h (…)");
        assert_eq!(redact_url("http://127.0.0.1:9/a?k=1"), "http://127.0.0.1:9/a (…)");
    }

    #[test]
    fn hmac_matches_the_rfc_4231_vector() {
        // RFC 4231, test case 2.
        assert_eq!(
            hmac_sha256(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    /// One post: the URL, the headers, the body.
    type Post = (String, Vec<(String, String)>, Value);

    /// A transport that remembers what was posted and answers as told.
    struct Posted {
        calls: Mutex<Vec<Post>>,
        answer: Mutex<Result<u16, String>>,
    }

    #[async_trait::async_trait]
    impl HookTransport for Posted {
        async fn post(&self, url: &str, headers: &[(String, String)], body: Vec<u8>) -> Result<u16, String> {
            let body: Value = serde_json::from_slice(&body).unwrap();
            self.calls.lock().unwrap().push((url.to_string(), headers.to_vec(), body));
            self.answer.lock().unwrap().clone()
        }
    }

    /// A run's events reach the hooks that want them through the host's transport, under its
    /// header prefix, signed when the hook has a secret; the status says what became of each.
    #[tokio::test(start_paused = true)]
    async fn events_go_to_the_hooks_through_the_transport_and_the_status_says_so() {
        std::env::set_var("HARNIX_FLOW_HOOK_TEST_SECRET", "s3cret");
        let posted = Arc::new(Posted { calls: Mutex::default(), answer: Mutex::new(Ok(204)) });
        let packs = MemPacks::new()
            .hook("{name: erp, kind: webhook, on: [run.created], action: 'http://127.0.0.1:9/h?token=t', secret_env: HARNIX_FLOW_HOOK_TEST_SECRET}")
            .hook("{name: other, kind: webhook, on: [run.finished], action: 'https://x.example/h'}")
            .flow("{name: f, stations: [{id: a, exit: [{kind: confirm}]}]}");
        let engine = Arc::new(Engine::new(Arc::new(MemStore::new()), Arc::new(packs)));
        let cfg = RunnerConfig { hook_headers: "X-Acme".into(), hook_transport: Some(posted.clone()), ..Default::default() };
        let rn = Runner::for_tests_with(engine.clone(), ScriptedHost::new(), cfg);
        let o = engine.create(CreateIn { flow: "f".into(), subject: "s".into(), ..Default::default() }, Actor::device("d")).await.unwrap();
        notify(&rn, &o.events).await;
        // The worker posts on its own task: let it.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        let calls = posted.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "only the hook that wants run.created: {calls:?}");
        let (url, headers, body) = &calls[0];
        assert_eq!(url, "http://127.0.0.1:9/h?token=t");
        let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["x-acme-event", "x-acme-delivery", "x-acme-timestamp", "x-acme-signature"]);
        assert_eq!(headers[0].1, "run.created");
        assert!(headers[3].1.starts_with("sha256="));
        assert_eq!(body["hook"], "erp");
        assert_eq!(body["event"]["kind"], "run.created");
        assert_eq!(body["run"]["flow"], "f");
        let listing = rn.hook_status();
        let erp = listing.iter().find(|h| h["name"] == "erp").unwrap();
        assert_eq!(erp["action"], "http://127.0.0.1:9/h (…)", "the URL is shown without its query");
        assert_eq!(erp["signed"], true);
        assert!(erp.get("secret_env").is_none());
        assert_eq!(erp["last"]["status"], "delivered");
        assert_eq!(erp["last"]["code"], 204);
        assert_eq!(listing.iter().find(|h| h["name"] == "other").unwrap()["last"], Value::Null);

        // A failure is tried again, then given up, and said so.
        *posted.answer.lock().unwrap() = Err("HTTP 500".into());
        notify(&rn, &o.events).await;
        tokio::time::sleep(Duration::from_secs(60)).await;
        let erp = rn.hook_status().into_iter().find(|h| h["name"] == "erp").unwrap();
        assert_eq!(erp["last"]["status"], "failed");
        assert_eq!(erp["last"]["tries"], 4);
        assert_eq!(posted.calls.lock().unwrap().len(), 5);
    }
}
