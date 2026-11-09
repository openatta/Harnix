//! Hooks (xops-04 §5): each event of a run is told to the hooks of the pack that want it. No flow
//! names a hook; one put in the pack applies from the next event on. A hook never blocks a run:
//! deliveries go out in order, one hook at a time, each tried again on failure and then given up.
//!
//! `webhook` (the one kind there is): `POST` to the hook's `action`
//!
//! ```text
//! {hook, delivery, event: {kind, run, flow, at, station?, task?, data?},
//!  run: {id, flow, title, subject, status, phase, usage}}
//! X-Harnix-Event: <kind>   X-Harnix-Delivery: <delivery>   X-Harnix-Timestamp: <ms>
//! X-Harnix-Signature: sha256=<hex>
//! ```
//!
//! The signature (with `secret_env`) is the HMAC-SHA256 of `<timestamp>.<body>` under the
//! variable's value: the other side checks it and refuses an old timestamp. A delivery keeps its
//! id over its tries, so the other side can tell a repeat. `run` is the run as it is when the
//! delivery is made, which may be later than the event. A hook's queue holds at most
//! [`QUEUE_KEPT`] deliveries (beyond them the newest are dropped, and said so); its worker goes
//! away once it has had nothing to do for a while. A URL is never shown with its query or
//! credentials.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harnix_contract::sync::lock;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::shared::Shared;
use crate::kernel::host::Event;
use crate::kernel::parts::{HookDef, HookKind};

/// How long one try may take.
const TIMEOUT: Duration = Duration::from_secs(10);
/// The waits before each try again.
const BACKOFF: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(5),
    Duration::from_secs(30),
];

/// The most deliveries a hook's queue holds.
pub const QUEUE_KEPT: usize = 1000;
/// How long a hook's worker waits for work before it goes away.
const IDLE: Duration = Duration::from_secs(600);

/// One delivery waiting to go out.
pub(crate) struct Delivery {
    hook: HookDef,
    body: Value,
    kind: String,
    run: String,
}

/// Each hook's queue, by name: one worker per hook keeps its deliveries in order.
#[derive(Default)]
pub(crate) struct Queues(Mutex<HashMap<String, mpsc::Sender<Delivery>>>);

/// Tell the hooks that want them about `events` (already committed).
pub(crate) async fn notify(sh: &Arc<Shared>, events: &[Event]) {
    if events.is_empty() || sh.stopping() {
        return;
    }
    let Ok(engine) = sh.engine() else { return };
    let hooks = engine.packs().hooks();
    if hooks.is_empty() {
        return;
    }
    let mut runs: HashMap<String, Option<Value>> = HashMap::new();
    for e in events {
        if !runs.contains_key(&e.run) {
            let summary = engine
                .record(&e.run)
                .await
                .ok()
                .map(|r| r.summary())
                .and_then(|s| serde_json::to_value(s).ok());
            runs.insert(e.run.clone(), summary);
        }
        let Some(Some(run)) = runs.get(&e.run) else {
            continue;
        };
        let flow = run.get("flow").and_then(Value::as_str).unwrap_or_default();
        for h in hooks.iter().filter(|h| h.wants(&e.kind, flow)) {
            let body = body_of(h, e, flow, run);
            let d = Delivery {
                hook: h.clone(),
                body,
                kind: e.kind.clone(),
                run: e.run.clone(),
            };
            enqueue(sh, d);
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

fn enqueue(sh: &Arc<Shared>, d: Delivery) {
    let mut queues = lock(&sh.hook_queues.0);
    let name = d.hook.name.clone();
    if let Some(tx) = queues.get(&name) {
        match tx.try_send(d) {
            Ok(()) => return,
            Err(mpsc::error::TrySendError::Full(d)) => {
                tracing::warn!(hook = name, event = d.kind, "flow: a hook's queue is full; a delivery is dropped");
                set_status(sh, &name, json!({"at": now_ms(), "event": d.kind, "run": d.run, "status": "dropped",
                    "error": format!("more than {QUEUE_KEPT} deliveries waiting")}));
                return;
            }
            Err(mpsc::error::TrySendError::Closed(d)) => {
                queues.remove(&name);
                drop(queues);
                return enqueue(sh, d);
            }
        }
    }
    let (tx, rx) = mpsc::channel(QUEUE_KEPT);
    let _ = tx.try_send(d);
    queues.insert(name.clone(), tx);
    let sh2 = sh.clone();
    sh.spawn(async move { work(sh2, name, rx).await });
}

fn set_status(sh: &Shared, hook: &str, v: Value) {
    lock(&sh.hook_status).insert(hook.to_string(), v);
}

async fn work(sh: Arc<Shared>, name: String, mut rx: mpsc::Receiver<Delivery>) {
    // The environment's proxy for the world, none for this machine.
    let clients = (
        reqwest::Client::builder().timeout(TIMEOUT).build(),
        reqwest::Client::builder().timeout(TIMEOUT).no_proxy().build(),
    );
    let (Ok(world), Ok(local)) = clients else {
        tracing::warn!("flow: hooks have no HTTP client");
        return;
    };
    loop {
        let next = tokio::select! {
            next = tokio::time::timeout(IDLE, rx.recv()) => next,
            _ = sh.cancel.cancelled() => return,
        };
        let d = match next {
            Ok(Some(d)) => d,
            Ok(None) => return,
            Err(_) => {
                // Nothing for a while: go, unless something came in meanwhile.
                let mut queues = lock(&sh.hook_queues.0);
                rx.close();
                match rx.try_recv() {
                    Ok(d) => {
                        drop(queues);
                        enqueue(&sh, d);
                        while let Ok(d) = rx.try_recv() {
                            enqueue(&sh, d);
                        }
                    }
                    Err(_) => {
                        queues.remove(&name);
                    }
                }
                return;
            }
        };
        if sh.stopping() {
            return;
        }
        let mut tries = 0usize;
        let outcome = loop {
            tries += 1;
            let client = if is_loopback(&d.hook.action) { &local } else { &world };
            let r = send(client, &d).await;
            match (&r, BACKOFF.get(tries - 1)) {
                (Ok(_), _) | (Err(_), None) => break r,
                (Err(_), Some(wait)) => {
                    tokio::select! {
                        _ = tokio::time::sleep(*wait) => {}
                        _ = sh.cancel.cancelled() => return,
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
                tracing::warn!(hook = d.hook.name, event = d.kind, error = %e, "flow: hook not delivered");
                status["status"] = json!("failed");
                status["error"] = json!(e);
            }
        }
        set_status(&sh, &d.hook.name, status);
    }
}

/// One try; the status code of a success.
async fn send(client: &reqwest::Client, d: &Delivery) -> Result<u16, String> {
    match d.hook.kind {
        HookKind::Webhook => {
            let body = serde_json::to_vec(&d.body).map_err(|e| e.to_string())?;
            let ts = now_ms().to_string();
            let mut req = client
                .post(&d.hook.action)
                .header("content-type", "application/json")
                .header("x-harnix-event", &d.kind)
                .header(
                    "x-harnix-delivery",
                    d.body.get("delivery").and_then(Value::as_str).unwrap_or_default(),
                )
                .header("x-harnix-timestamp", &ts);
            if let Some(var) = &d.hook.secret_env {
                let secret = std::env::var(var).map_err(|_| format!("{var} is not set"))?;
                let signed = [ts.as_bytes(), b".", &body].concat();
                req = req.header("x-harnix-signature", format!("sha256={}", hmac_sha256(secret.as_bytes(), &signed)));
            }
            // The error without the URL: it may carry a token.
            let res = req
                .body(body)
                .send()
                .await
                .map_err(|e| e.without_url().to_string())?;
            let code = res.status().as_u16();
            if res.status().is_success() {
                Ok(code)
            } else {
                Err(format!("HTTP {code}"))
            }
        }
    }
}

/// Whether a URL is this machine's.
fn is_loopback(url: &str) -> bool {
    let host = url
        .split_once("://")
        .map_or(url, |(_, r)| r)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
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
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
            }
        })
}

/// A URL as it may be shown: no credentials, no query, no fragment.
pub(crate) fn redact_url(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let cut = rest.len() > authority.len() + path.len() || authority.contains('@');
    let shown = if scheme.is_empty() {
        format!("{host}{path}")
    } else {
        format!("{scheme}://{host}{path}")
    };
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
    let mut k = if key.len() > BLOCK {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    k.resize(BLOCK, 0);
    let pad = |b: u8| k.iter().map(|x| x ^ b).collect::<Vec<u8>>();
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(msg).finalize();
    let outer = Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize();
    outer.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// The hooks of the pack with each one's last delivery, for `app.flow.hooks`.
pub(crate) fn listing(sh: &Shared, hooks: &[HookDef]) -> Vec<Value> {
    let status = lock(&sh.hook_status);
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
