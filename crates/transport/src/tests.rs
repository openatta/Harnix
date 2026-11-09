//! Both topologies, served on a real loopback listener against a hub that is
//! only a client registry: what revoking a device, an event stream dropping,
//! and shutting down look like from the client's side of the wire.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use futures::{SinkExt as _, StreamExt as _};
use harnix_contract::sync::lock;
use harnix_contract::{Frame, FrameSink, Gate, RpcError, Subject};
use harnix_hub::ClientId;
use ring::signature::KeyPair as _;
use serde_json::{json, Value};

use crate::session::ClientRegistry;
use crate::{Admission, BrowserAdmission, Channels, Config, Extensions, Face, LaunchLinks};

/// The part of a hub a transport talks to, and nothing behind it.
#[derive(Default)]
pub(crate) struct FakeHub {
    next: AtomicU64,
    sinks: Mutex<HashMap<ClientId, Arc<dyn FrameSink>>>,
    removed: Mutex<Vec<ClientId>>,
}

impl FakeHub {
    pub fn removed(&self) -> Vec<ClientId> {
        lock(&self.removed).clone()
    }

    /// A host event to every client, as the hub's broadcast does; how many took it.
    pub async fn broadcast(&self, payload: &str) -> usize {
        let sinks: Vec<Arc<dyn FrameSink>> = lock(&self.sinks).values().cloned().collect();
        let mut took = 0;
        for sink in sinks {
            if sink.wants(harnix_contract::Channel::HostEvents) && sink.deliver(Frame::host(payload.into())).await {
                took += 1;
            }
        }
        took
    }
}

#[async_trait::async_trait]
impl ClientRegistry for FakeHub {
    async fn add_client(&self, sink: Arc<dyn FrameSink>) -> ClientId {
        let id = ClientId(self.next.fetch_add(1, Ordering::Relaxed) + 1);
        lock(&self.sinks).insert(id, sink);
        id
    }

    fn describe_client(&self, _id: ClientId, _description: Value) {}

    async fn remove_client(&self, id: ClientId) {
        lock(&self.sinks).remove(&id);
        lock(&self.removed).push(id);
    }
}

/// Answers every call with who made it.
struct Echo;

#[async_trait::async_trait]
impl Gate for Echo {
    async fn call(&self, subject: &Subject, method: &str, _params: Value) -> Result<Value, RpcError> {
        if method == "test.panic" {
            panic!("a handler that panics");
        }
        if method == "test.busy" {
            let mut e = RpcError::new(harnix_contract::codes::BUSY, "a turn is running");
            e.data = Some(json!({"turn": "t1"}));
            return Err(e);
        }
        Ok(json!({"subject": subject, "method": method}))
    }
}

struct NoBulk;

#[async_trait::async_trait]
impl crate::BulkStore for NoBulk {
    async fn claim(&self, _token: &str) -> Option<(std::path::PathBuf, usize)> {
        None
    }
}

const TOKEN: &str = "the-token";

/// Straight to the listener: a proxy from the environment would answer for it.
fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

struct Served {
    base: String,
    ws: String,
    hub: Arc<FakeHub>,
    devices: Arc<harnix_authz::Devices>,
    channels: Channels,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    server: tokio::task::JoinHandle<anyhow::Result<()>>,
}

/// Both topologies on a loopback port, with `admission` as a reachable node would have it.
async fn serve(admission: Admission) -> Served {
    let hub = Arc::new(FakeHub::default());
    let devices = Arc::new(harnix_authz::Devices::default());
    let (router, _, channels) = crate::assemble(
        Arc::new(Echo),
        hub.clone(),
        Arc::new(NoBulk),
        devices.clone(),
        Config {
            topologies: vec![harnix_contract::Topology::SingleDuplex, harnix_contract::Topology::SplitStreams],
            token: TOKEN.into(),
            face: Face::Headless,
            tls: None,
            admission,
            max_upload_bytes: 1024,
            browser: BrowserAdmission::PageToken,
            links: Arc::new(LaunchLinks::default()),
            extra_origins: Vec::new(),
        },
        Extensions::default(),
    );
    let (bound_tx, bound_rx) = tokio::sync::oneshot::channel::<Vec<SocketAddr>>();
    let (stop, stopping) = tokio::sync::oneshot::channel::<()>();
    let addrs = [SocketAddr::from(([127, 0, 0, 1], 0))];
    let server = tokio::spawn(async move {
        crate::serve_with(
            &addrs,
            router,
            None,
            move |bound| {
                let _ = bound_tx.send(bound);
            },
            async move {
                let _ = stopping.await;
            },
        )
        .await
    });
    let addr = bound_rx.await.expect("bound")[0];
    Served {
        base: format!("http://{addr}"),
        ws: format!("ws://{addr}/ws?token={TOKEN}"),
        hub,
        devices,
        channels,
        stop: Some(stop),
        server,
    }
}

/// A device paired with a real key, and a way to sign what the server asks.
struct Device {
    id: String,
    pair: ring::signature::Ed25519KeyPair,
}

fn pair(devices: &harnix_authz::Devices, code: &str) -> Device {
    let rng = ring::rand::SystemRandom::new();
    let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
    let public = base64::engine::general_purpose::STANDARD.encode(pair.public_key().as_ref());
    let now = std::time::SystemTime::now();
    devices.begin_pairing(code, "test", now);
    let device = devices.complete_pairing(code, &public, now, "0").unwrap();
    Device { id: device.id, pair }
}

/// What a handshake carries to prove the device: a fresh challenge, signed.
async fn proof(http: &reqwest::Client, base: &str, device: &Device) -> Value {
    let challenge: Value = http.get(format!("{base}/challenge")).send().await.unwrap().json().await.unwrap();
    let engine = base64::engine::general_purpose::STANDARD;
    let bytes = engine.decode(challenge["challenge"].as_str().unwrap()).unwrap();
    json!({
        "device_id": device.id,
        "challenge_id": challenge["challenge_id"],
        "signature": engine.encode(device.pair.sign(&bytes).as_ref()),
    })
}

fn hello(topology: &str, extra: Value) -> Value {
    let mut body = json!({
        "token": TOKEN,
        "protocol_version": harnix_contract::PROTOCOL_VERSION,
        "contrib_api_version": harnix_contract::CONTRIB_API_VERSION,
        "topology": topology,
    });
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        body.extend(extra.clone());
    }
    body
}

async fn split_handshake(http: &reqwest::Client, base: &str, extra: Value) -> String {
    let answer: Value = http
        .post(format!("{base}/handshake"))
        .json(&hello("split_streams", extra))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    answer["credential"].as_str().expect("a credential").to_string()
}

async fn rpc(http: &reqwest::Client, base: &str, credential: &str) -> (u16, Value) {
    let response = http
        .post(format!("{base}/rpc?token={credential}"))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "harnix.ping", "params": {}}))
        .send()
        .await
        .unwrap();
    (response.status().as_u16(), response.json().await.unwrap_or(Value::Null))
}

/// The next `data:` line of an event stream; `None` when the stream ended.
async fn next_event(stream: &mut (impl futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Unpin), buffer: &mut String) -> Option<String> {
    loop {
        if let Some(cut) = buffer.find("\n\n") {
            let chunk: String = buffer.drain(..cut + 2).collect();
            if let Some(data) = chunk.lines().find_map(|l| l.strip_prefix("data:")) {
                return Some(data.trim().to_string());
            }
            continue;
        }
        let bytes = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .expect("the stream said nothing in 5s")?
            .ok()?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));
    }
}

/// An event stream that dropped is opened again, and events reach it — the
/// browser's own reconnect used to get 409 for good.
#[tokio::test]
async fn a_dropped_event_stream_reopens_and_still_receives() {
    let served = serve(Admission::Token).await;
    let http = client();
    let credential = split_handshake(&http, &served.base, json!({})).await;
    let url = format!("{}/events/host?token={credential}", served.base);

    let first = http.get(&url).send().await.unwrap();
    assert_eq!(first.status(), 200);
    drop(first);
    // Let the server see the reader go.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let again = http.get(&url).send().await.unwrap();
    assert_eq!(again.status(), 200, "the stream could not be opened again");
    let mut stream = again.bytes_stream();
    assert_eq!(served.hub.broadcast(r#"{"after":"reconnect"}"#).await, 1);
    let mut buffer = String::new();
    assert_eq!(next_event(&mut stream, &mut buffer).await.as_deref(), Some(r#"{"after":"reconnect"}"#));
}

/// Revoking a device ends all of its channels at once: its split credential
/// is refused, its event stream ends, its WebSocket is closed with the reason
/// — and another device's channels go on.
#[tokio::test]
async fn revoking_a_device_ends_all_of_its_channels() {
    let served = serve(Admission::PairedDevice).await;
    let http = client();
    let phone = pair(&served.devices, "PHONE42");
    let laptop = pair(&served.devices, "LAPTOP42");

    let credential = split_handshake(&http, &served.base, proof(&http, &served.base, &phone).await).await;
    assert_eq!(rpc(&http, &served.base, &credential).await.0, 200);
    let mut events = http
        .get(format!("{}/events/session?token={credential}", served.base))
        .send()
        .await
        .unwrap()
        .bytes_stream();

    let (mut socket, _) = tokio_tungstenite::connect_async(&served.ws).await.unwrap();
    let handshake = json!({"jsonrpc": "2.0", "id": 1, "method": "harnix.handshake",
        "params": hello("single_duplex", proof(&http, &served.base, &phone).await)});
    socket.send(tokio_tungstenite::tungstenite::Message::text(handshake.to_string())).await.unwrap();
    let accepted = socket.next().await.unwrap().unwrap().into_text().unwrap();
    assert!(accepted.contains("\"credential\""), "{accepted}");

    let other = split_handshake(&http, &served.base, proof(&http, &served.base, &laptop).await).await;

    assert!(served.devices.revoke(&phone.id));
    assert_eq!(served.channels.revoke_device(&phone.id), 2);

    // The credential, at once.
    let (status, body) = rpc(&http, &served.base, &credential).await;
    assert_eq!(status, 401, "{body}");
    assert_eq!(body["error"]["code"], harnix_contract::codes::HANDSHAKE_REFUSED);
    // The event stream ends.
    let mut buffer = String::new();
    assert_eq!(next_event(&mut events, &mut buffer).await, None);
    // The socket is closed, saying why.
    let closed = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .expect("the socket stayed open");
    match closed {
        Some(Ok(tokio_tungstenite::tungstenite::Message::Close(Some(frame)))) => {
            assert_eq!(u16::from(frame.code), 1008);
            assert!(frame.reason.contains("revoked"), "{}", frame.reason);
        }
        other => panic!("not a close frame: {other:?}"),
    }
    // And a revoked device cannot come back.
    let refused = http
        .post(format!("{}/handshake", served.base))
        .json(&hello("split_streams", proof(&http, &served.base, &phone).await))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 401);

    // Another device is untouched.
    assert_eq!(rpc(&http, &served.base, &other).await.0, 200);
    // The hub forgot both of the revoked one's clients.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(served.hub.removed().len(), 2);
}

/// A credential that was never issued is a 401 with the JSON-RPC reason.
#[tokio::test]
async fn an_unknown_credential_is_unauthorized() {
    let served = serve(Admission::Token).await;
    let http = client();
    let (status, body) = rpc(&http, &served.base, "no-such-credential").await;
    assert_eq!(status, 401);
    assert_eq!(body["error"]["code"], harnix_contract::codes::HANDSHAKE_REFUSED);
    let stream = http.get(format!("{}/events/host?token=nope", served.base)).send().await.unwrap();
    assert_eq!(stream.status(), 401);
}

/// Shutting down: the listener stops accepting, open channels are told and
/// end, and the server returns — an open event stream does not hold it up.
#[tokio::test]
async fn shutting_down_stops_the_listener_and_ends_the_channels() {
    let mut served = serve(Admission::Token).await;
    let http = client();
    let credential = split_handshake(&http, &served.base, json!({})).await;
    let mut events = http
        .get(format!("{}/events/host?token={credential}", served.base))
        .send()
        .await
        .unwrap()
        .bytes_stream();
    let (mut socket, _) = tokio_tungstenite::connect_async(&served.ws).await.unwrap();
    let handshake = json!({"jsonrpc": "2.0", "id": 1, "method": "harnix.handshake", "params": hello("single_duplex", json!({}))});
    socket.send(tokio_tungstenite::tungstenite::Message::text(handshake.to_string())).await.unwrap();
    socket.next().await.unwrap().unwrap();

    let _ = served.stop.take().unwrap().send(());
    served.channels.end_all().await;
    let finished = tokio::time::timeout(std::time::Duration::from_secs(5), &mut served.server)
        .await
        .expect("the server did not finish");
    finished.unwrap().expect("a requested stop is not an error");

    let mut buffer = String::new();
    assert_eq!(next_event(&mut events, &mut buffer).await, None);
    match socket.next().await {
        Some(Ok(tokio_tungstenite::tungstenite::Message::Close(Some(frame)))) => assert_eq!(u16::from(frame.code), 1001),
        other => panic!("not a close frame: {other:?}"),
    }
    assert!(http.get(format!("{}/challenge", served.base)).send().await.is_err(), "still accepting");
}

/// A call that panics behind the gate is answered, on both topologies — not a
/// promise that never settles.
#[tokio::test]
async fn a_call_that_panics_is_answered_with_an_error() {
    let served = serve(Admission::Token).await;
    let http = client();
    let credential = split_handshake(&http, &served.base, json!({})).await;
    let response = http
        .post(format!("{}/rpc?token={credential}", served.base))
        .json(&json!({"jsonrpc": "2.0", "id": 7, "method": "test.panic", "params": {}}))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], harnix_contract::codes::INTERNAL_ERROR, "{body}");

    let (mut socket, _) = tokio_tungstenite::connect_async(&served.ws).await.unwrap();
    let handshake = json!({"jsonrpc": "2.0", "id": 1, "method": "harnix.handshake", "params": hello("single_duplex", json!({}))});
    socket.send(tokio_tungstenite::tungstenite::Message::text(handshake.to_string())).await.unwrap();
    socket.next().await.unwrap().unwrap();
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "test.panic", "params": {}});
    socket.send(tokio_tungstenite::tungstenite::Message::text(call.to_string())).await.unwrap();
    let reply = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .expect("no answer")
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["id"], 2);
    assert_eq!(reply["error"]["code"], harnix_contract::codes::INTERNAL_ERROR, "{reply}");
}

/// A frame refused before the gate, and an error from behind it, read the same on both
/// topologies: the same code, message and id, and `data` whole.
#[tokio::test]
async fn both_topologies_refuse_and_answer_alike() {
    let served = serve(Admission::Token).await;
    let http = client();
    let credential = split_handshake(&http, &served.base, json!({})).await;
    let split = |frame: Value| {
        let http = http.clone();
        let url = format!("{}/rpc?token={credential}", served.base);
        async move { http.post(url).json(&frame).send().await.unwrap().json::<Value>().await.unwrap() }
    };

    let (mut socket, _) = tokio_tungstenite::connect_async(&served.ws).await.unwrap();
    let handshake = json!({"jsonrpc": "2.0", "id": 1, "method": "harnix.handshake", "params": hello("single_duplex", json!({}))});
    socket.send(tokio_tungstenite::tungstenite::Message::text(handshake.to_string())).await.unwrap();
    socket.next().await.unwrap().unwrap();
    let mut duplex = async |frame: Value| -> Value {
        socket.send(tokio_tungstenite::tungstenite::Message::text(frame.to_string())).await.unwrap();
        let reply = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
            .await
            .expect("no answer")
            .unwrap()
            .unwrap();
        serde_json::from_str(&reply.into_text().unwrap()).unwrap()
    };

    let no_method = json!({"jsonrpc": "2.0", "id": 5, "params": {}});
    let (a, b) = (split(no_method.clone()).await, duplex(no_method).await);
    assert_eq!(a, b);
    assert_eq!(a["id"], 5);
    assert_eq!(a["error"]["code"], harnix_contract::codes::INVALID_REQUEST);

    let busy = json!({"jsonrpc": "2.0", "id": 6, "method": "test.busy", "params": {}});
    let (a, b) = (split(busy.clone()).await, duplex(busy).await);
    assert_eq!(a, b);
    assert_eq!(a["error"]["data"]["turn"], "t1", "{a}");

    // A handshake for the other topology is refused on each, saying where it is established.
    let wrong = http
        .post(format!("{}/handshake", served.base))
        .json(&hello("single_duplex", json!({})))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(wrong["error"]["code"], harnix_contract::codes::HANDSHAKE_REFUSED, "{wrong}");
    assert!(wrong["error"]["message"].as_str().unwrap().contains("own connection"), "{wrong}");
    let (mut other, _) = tokio_tungstenite::connect_async(&served.ws).await.unwrap();
    let handshake = json!({"jsonrpc": "2.0", "id": 9, "method": "harnix.handshake", "params": hello("split_streams", json!({}))});
    other.send(tokio_tungstenite::tungstenite::Message::text(handshake.to_string())).await.unwrap();
    let refused: Value = serde_json::from_str(&other.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
    assert_eq!(refused["id"], 9);
    assert_eq!(refused["error"]["code"], harnix_contract::codes::HANDSHAKE_REFUSED, "{refused}");
    assert!(refused["error"]["message"].as_str().unwrap().contains("POST /handshake"), "{refused}");
}
