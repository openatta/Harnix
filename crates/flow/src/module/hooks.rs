//! Hooks (xops-04 §5): each event of a run is told to the hooks of the pack that want it. No flow
//! names a hook; one put in the pack applies from the next event on. A hook never blocks a run:
//! deliveries go out in order, one hook at a time, each tried again on failure and then given up.
//!
//! `webhook` (the one kind there is): `POST` to the hook's `action`
//!
//! ```text
//! {hook, delivery, event: {kind, run, flow, at, station?, task?, data?},
//!  run: {id, flow, title, subject, status, phase, usage}}
//! X-Harnix-Event: <kind>   X-Harnix-Delivery: <delivery>   X-Harnix-Signature: sha256=<hex>
//! ```
//!
//! The signature (with `secret_env`) is the HMAC-SHA256 of the body under the variable's value.
//! A delivery keeps its id over its tries, so the other side can tell a repeat.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

/// One delivery waiting to go out.
pub(crate) struct Delivery {
    hook: HookDef,
    body: Value,
    kind: String,
    run: String,
}

/// Each hook's queue, by name: one worker per hook keeps its deliveries in order.
#[derive(Default)]
pub(crate) struct Queues(Mutex<HashMap<String, mpsc::UnboundedSender<Delivery>>>);

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
    let mut queues = sh.hook_queues.0.lock().unwrap_or_else(|e| e.into_inner());
    let name = d.hook.name.clone();
    if let Some(tx) = queues.get(&name) {
        match tx.send(d) {
            Ok(()) => return,
            Err(mpsc::error::SendError(d)) => {
                queues.remove(&name);
                drop(queues);
                return enqueue(sh, d);
            }
        }
    }
    let (tx, rx) = mpsc::unbounded_channel();
    let _ = tx.send(d);
    queues.insert(name, tx);
    let sh = sh.clone();
    tokio::spawn(async move { work(sh, rx).await });
}

async fn work(sh: Arc<Shared>, mut rx: mpsc::UnboundedReceiver<Delivery>) {
    let client = match reqwest::Client::builder().timeout(TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "flow: hooks have no HTTP client");
            return;
        }
    };
    while let Some(d) = rx.recv().await {
        if sh.stopping() {
            return;
        }
        let mut tries = 0usize;
        let outcome = loop {
            tries += 1;
            let r = send(&client, &d).await;
            match (&r, BACKOFF.get(tries - 1)) {
                (Ok(_), _) | (Err(_), None) => break r,
                (Err(_), Some(wait)) => {
                    if sh.stopping() {
                        return;
                    }
                    tokio::time::sleep(*wait).await;
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
                tracing::warn!(hook = d.hook.name, event = d.kind, error = e, "flow: hook not delivered");
                status["status"] = json!("failed");
                status["error"] = json!(e);
            }
        }
        sh.hook_status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(d.hook.name.clone(), status);
    }
}

/// One try; the status code of a success.
async fn send(client: &reqwest::Client, d: &Delivery) -> Result<u16, String> {
    match d.hook.kind {
        HookKind::Webhook => {
            let body = serde_json::to_vec(&d.body).map_err(|e| e.to_string())?;
            let mut req = client
                .post(&d.hook.action)
                .header("content-type", "application/json")
                .header("x-harnix-event", &d.kind)
                .header(
                    "x-harnix-delivery",
                    d.body.get("delivery").and_then(Value::as_str).unwrap_or_default(),
                );
            if let Some(var) = &d.hook.secret_env {
                let secret = std::env::var(var).map_err(|_| format!("{var} is not set"))?;
                req = req.header("x-harnix-signature", format!("sha256={}", hmac_sha256(secret.as_bytes(), &body)));
            }
            let res = req.body(body).send().await.map_err(|e| e.to_string())?;
            let code = res.status().as_u16();
            if res.status().is_success() {
                Ok(code)
            } else {
                Err(format!("HTTP {code}"))
            }
        }
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
    let status = sh.hook_status.lock().unwrap_or_else(|e| e.into_inner());
    hooks
        .iter()
        .map(|h| {
            let mut v = serde_json::to_value(h).unwrap_or(Value::Null);
            if let Some(obj) = v.as_object_mut() {
                obj.remove("secret_env");
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
    fn hmac_matches_the_rfc_4231_vector() {
        // RFC 4231, test case 2.
        assert_eq!(
            hmac_sha256(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }
}
