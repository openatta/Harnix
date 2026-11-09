//! Who is allowed to open a channel at all.
//!
//! Two shapes, and which one applies is decided by where the listener is
//! bound rather than by a setting (§6.3):
//!
//! - **Loopback** — the per-process token. Binding to loopback keeps the
//!   network out; the token keeps another page on this machine out, because a
//!   page cannot read a cross-origin response but *can* open a socket to
//!   `127.0.0.1`.
//! - **Reachable** — the token *and* a paired device signing a challenge this
//!   server chose. The token alone cannot carry a reachable listener: it is
//!   one secret shared by every device, so revoking one device would mean
//!   rotating it for all of them, which nobody does.
//!
//! The challenge is single-use and short-lived, so a signature captured off
//! the wire is worth nothing the second time.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use harnix_authz::{devices_challenge as challenge, tokens_equal, Devices};
use tokio::sync::Mutex;

/// How long an unanswered challenge is worth keeping. A round trip, not a
/// session.
const CHALLENGE_TTL: Duration = Duration::from_secs(60);

/// Unanswered challenges held at once. `/challenge` is reachable before
/// admission, so without a ceiling every request to it was a minute of
/// memory, bounded only by how fast someone could ask. A person pairing a
/// device needs one; a thousand outstanding is a client misbehaving, and it
/// is told to come back later rather than allowed to grow the table.
pub(crate) const MAX_OPEN: usize = 1024;

#[derive(Default)]
pub struct Challenges {
    open: Mutex<HashMap<String, (Vec<u8>, SystemTime)>>,
}

impl Challenges {
    /// Mint one. Returned base64, to travel in JSON. `None` when [`MAX_OPEN`]
    /// are already outstanding (or the system has no randomness to give).
    pub async fn issue(&self, now: SystemTime) -> Option<(String, String)> {
        let mut open = self.open.lock().await;
        open.retain(|_, (_, expires)| now <= *expires);
        if open.len() >= MAX_OPEN {
            return None;
        }
        let bytes = challenge::fresh()?.to_vec();
        let id = uuid::Uuid::new_v4().simple().to_string();
        open.insert(id.clone(), (bytes.clone(), now + CHALLENGE_TTL));
        Some((id, base64::engine::general_purpose::STANDARD.encode(&bytes)))
    }

    /// Spend one. `None` means unknown, expired, or already used — one answer
    /// for all three, because telling them apart tells a prober which of its
    /// guesses were closer.
    pub async fn redeem(&self, id: &str, now: SystemTime) -> Option<Vec<u8>> {
        let mut open = self.open.lock().await;
        let (bytes, expires) = open.remove(id)?;
        (now <= expires).then_some(bytes)
    }
}

/// Hand out something to sign.
///
/// The only endpoint reachable before admission, and it gives away nothing: a
/// challenge is random bytes with a minute to live, useless without the
/// private key of a device that was paired.
pub async fn mint(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let Some((id, bytes)) = state.challenges.issue(SystemTime::now()).await else {
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "too many challenges outstanding; try again in a minute",
        )
            .into_response();
    };
    axum::Json(serde_json::json!({"challenge_id": id, "challenge": bytes})).into_response()
}

/// Redeem a pairing code for a device record.
///
/// The one call a reachable listener takes before any device exists, and the
/// only thing that breaks the circle — pairing is a method, methods need
/// admission, and admission needs a paired device. What stands in for
/// admission here is the code itself: short-lived, single-use, burned after a
/// few wrong guesses, and it reached the person through a channel this
/// process does not control (the console it printed on).
///
/// The per-process token is still required, so this is not a door onto the
/// network — it is the second half of one the operator opened deliberately.
pub async fn pair(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let get = |key: &str| {
        body.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    if !tokens_equal(&get("token"), &state.config.token) {
        return (axum::http::StatusCode::UNAUTHORIZED, "bad token").into_response();
    }
    let now = SystemTime::now();
    state.devices.sweep(now);
    let stamp = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into());
    match state
        .devices
        .complete_pairing(&get("code"), &get("public_key"), now, &stamp)
    {
        Ok(device) => axum::Json(serde_json::json!({"device": device})).into_response(),
        Err(e) => (axum::http::StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
    }
}

/// Answer to "may this connection exist".
pub enum Verdict {
    Admit { device: Option<String> },
    Refuse(&'static str),
}

/// The one refusal for a device that did not prove itself: an unknown or
/// revoked device, a challenge that is unknown, expired or spent, and a
/// signature that does not match are the same answer. Three answers told a
/// prober which device ids exist — the same reason pairing has one
/// (`Devices::complete_pairing`) and a launch link has one.
pub(crate) const NOT_ADMITTED: &str = "this device could not be admitted";

/// Check a handshake body against what this listener requires.
pub async fn admit(
    admission: crate::Admission,
    expected_token: &str,
    devices: &Devices,
    challenges: &Challenges,
    body: &serde_json::Value,
) -> Verdict {
    let token = body.get("token").and_then(|v| v.as_str()).unwrap_or("");
    if !tokens_equal(token, expected_token) {
        return Verdict::Refuse("bad token");
    }
    if admission == crate::Admission::Token {
        return Verdict::Admit { device: None };
    }

    let get = |key: &str| body.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let (Some(device_id), Some(challenge_id), Some(signature)) =
        (get("device_id"), get("challenge_id"), get("signature"))
    else {
        return Verdict::Refuse(
            "this listener is reachable from the network, so a paired device must sign a challenge",
        );
    };
    // The challenge is spent whether or not the device is known, so a refusal
    // takes the same steps either way.
    let bytes = challenges.redeem(&challenge_id, SystemTime::now()).await;
    let device = devices.get(&device_id);
    let (Some(bytes), Some(device)) = (bytes, device) else {
        return Verdict::Refuse(NOT_ADMITTED);
    };
    if !challenge::verify(&device.public_key, &bytes, &signature) {
        return Verdict::Refuse(NOT_ADMITTED);
    }
    devices.seen(&device_id, SystemTime::now());
    Verdict::Admit {
        device: Some(device_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::KeyPair as _;

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    #[tokio::test]
    async fn the_open_challenges_are_capped_and_expire() {
        let challenges = Challenges::default();
        for _ in 0..MAX_OPEN {
            assert!(challenges.issue(now()).await.is_some());
        }
        assert!(challenges.issue(now()).await.is_none(), "the table grew past its ceiling");
        // A minute later the old ones are gone and there is room again.
        let later = now() + CHALLENGE_TTL + Duration::from_secs(1);
        assert!(challenges.issue(later).await.is_some());
    }

    #[tokio::test]
    async fn challenges_are_random_and_single_use() {
        let challenges = Challenges::default();
        let (a, bytes_a) = challenges.issue(now()).await.unwrap();
        let (_, bytes_b) = challenges.issue(now()).await.unwrap();
        assert_ne!(bytes_a, bytes_b);
        let decoded = base64::engine::general_purpose::STANDARD.decode(&bytes_a).unwrap();
        assert_eq!(decoded.len(), challenge::LEN);
        assert!(challenges.redeem(&a, now()).await.is_some());
        assert!(challenges.redeem(&a, now()).await.is_none());
    }

    /// Unknown device, spent challenge and wrong signature: one answer, so a
    /// refusal does not say which device ids exist.
    #[tokio::test]
    async fn every_failed_proof_is_one_refusal() {
        let rng = ring::rand::SystemRandom::new();
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
        let engine = base64::engine::general_purpose::STANDARD;
        let public = engine.encode(pair.public_key().as_ref());

        let devices = Devices::default();
        devices.begin_pairing("ABC123", "laptop", SystemTime::now());
        let device = devices.complete_pairing("ABC123", &public, SystemTime::now(), "t0").unwrap();
        let challenges = Challenges::default();

        let attempt = |device_id: String, challenge_id: String, signature: String| {
            serde_json::json!({"token": "t", "device_id": device_id, "challenge_id": challenge_id, "signature": signature})
        };
        let refusal = |verdict: Verdict| match verdict {
            Verdict::Refuse(reason) => reason,
            Verdict::Admit { .. } => panic!("admitted"),
        };
        let sign = |b64: &str| engine.encode(pair.sign(&engine.decode(b64).unwrap()).as_ref());
        let check = |body: serde_json::Value| {
            let (devices, challenges) = (&devices, &challenges);
            async move { admit(crate::Admission::PairedDevice, "t", devices, challenges, &body).await }
        };

        let (id, bytes) = challenges.issue(SystemTime::now()).await.unwrap();
        let unknown = refusal(check(attempt("dev-nobody".into(), id, sign(&bytes))).await);
        let (id, _) = challenges.issue(SystemTime::now()).await.unwrap();
        let wrong = refusal(check(attempt(device.id.clone(), id, engine.encode([0u8; 64]))).await);
        let spent = refusal(check(attempt(device.id.clone(), "no-such".into(), sign(&bytes))).await);
        assert_eq!(unknown, NOT_ADMITTED);
        assert_eq!(wrong, NOT_ADMITTED);
        assert_eq!(spent, NOT_ADMITTED);

        // And the token is still checked first, whole.
        let bad = refusal(check(serde_json::json!({"token": "u"})).await);
        assert_eq!(bad, "bad token");

        // A proof that holds admits, and the device is marked seen.
        let (id, bytes) = challenges.issue(SystemTime::now()).await.unwrap();
        match check(attempt(device.id.clone(), id, sign(&bytes))).await {
            Verdict::Admit { device: Some(id) } => assert_eq!(id, device.id),
            _ => panic!("a valid proof was refused"),
        }
        assert!(devices.get(&device.id).unwrap().last_seen.is_some());
    }
}
