//! Idle hibernation, against the showcase made on-demand (its `watch_sessions` off, so `init`
//! subscribes to nothing), idling 300 ms here (`HostOptions::idle`) — through the same JSON
//! loopback as every other host test.
#![cfg(feature = "compile")]

mod common;

use std::time::Duration;

use common::*;
use harnix_app_port::*;
use serde_json::{json, Value};

const IDLE: Duration = Duration::from_millis(300);

fn on_demand(watch: bool) -> impl Fn(String) -> String {
    move |m: String| {
        let m = if watch { m } else { m.replace("watch_sessions = true", "watch_sessions = false") };
        format!("{m}\n[backend]\nresident = \"on-demand\"\n")
    }
}

async fn idle_rig(edit: impl Fn(String) -> String) -> (Rig, Value) {
    let rig = Rig::new(|o| o.idle = Some(IDLE));
    let parts = rig.install(edit).await;
    (rig, parts)
}

async fn state(rig: &Rig) -> Value {
    rig.status().await["state"].clone()
}

async fn hibernated(rig: &Rig) {
    eventually("hibernating", async || state(rig).await == json!("hibernating")).await;
}

fn tool(name: &str, input: Value, call_id: &str) -> ToolCall {
    ToolCall {
        app: ID.into(),
        tool: name.into(),
        input,
        call_id: call_id.into(),
        project: None,
        progress: false,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_it_hibernates_and_a_call_starts_it_with_its_store_and_without_its_memory() {
    let (rig, _) = idle_rig(on_demand(false)).await;
    let status = rig.status().await;
    assert_eq!((status["resident"].clone(), status["idle_ms"].clone()), (json!("on-demand"), json!(300)));
    rig.platform.device_call("notes.add", json!({"text": "kept"})).await.unwrap();
    assert_eq!(rig.platform.device_call("counter.bump", json!({})).await.unwrap()["counter"], json!(1));
    let before = rig.platform.device_call("state", json!({})).await.unwrap();

    hibernated(&rig).await;
    let health = rig.status().await["health"].clone();
    assert_eq!((health["hibernating"].clone(), health["recycles"].clone(), health["faults"].clone()), (json!(true), json!(0), json!(0)));
    // A tool runs in an instance of its own, and wakes nothing.
    assert!(matches!(
        rig.control.call_tool(tool("echo", json!({"text": "hi"}), "t1")).await.unwrap(),
        ToolReply::Output { .. }
    ));
    assert_eq!(state(&rig).await, json!("hibernating"));

    let after = rig.platform.device_call("state", json!({})).await.unwrap();
    assert!(after["started_ms"].as_u64() >= before["started_ms"].as_u64());
    assert_eq!((after["notes"].clone(), after["counter"].clone()), (json!(1), json!(0)));
    let status = rig.status().await;
    assert_eq!(status["state"], json!("running"));
    assert_eq!((status["health"]["wakes"].clone(), status["health"]["recycles"].clone()), (json!(1), json!(0)));
    // A tool that asks the backend wakes it like any call.
    hibernated(&rig).await;
    match rig.control.call_tool(tool("board", json!({}), "t2")).await.unwrap() {
        ToolReply::Output { content, .. } => assert!(content.contains("1 notes") || content.contains("the board"), "{content}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(rig.status().await["health"]["wakes"], json!(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_at_once_on_a_hibernating_backend_start_one_instance() {
    let (rig, _) = idle_rig(on_demand(false)).await;
    hibernated(&rig).await;
    let calls: Vec<_> = (0..4)
        .map(|_| {
            let platform = rig.platform.clone();
            tokio::spawn(async move { platform.device_call("counter.bump", json!({})).await.unwrap()["counter"].clone() })
        })
        .collect();
    let mut counts = Vec::new();
    for call in calls {
        counts.push(call.await.unwrap().as_u64().unwrap());
    }
    counts.sort();
    assert_eq!(counts, [1, 2, 3, 4], "one instance counted them all");
    assert_eq!(rig.status().await["health"]["wakes"], json!(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subscription_a_timer_or_a_call_in_flight_holds_it_up() {
    // Subscribed in `init`.
    let (rig, _) = idle_rig(on_demand(true)).await;
    tokio::time::sleep(IDLE * 4).await;
    assert_eq!(state(&rig).await, json!("running"));

    // A timer firing four times, 200 ms apart: up while armed, hibernating once it is done.
    let (rig, _) = idle_rig(on_demand(false)).await;
    rig.platform.device_call("timer.start", json!({"ms": 200, "times": 4})).await.unwrap();
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_millis(600) {
        assert_eq!(state(&rig).await, json!("running"), "hibernated with a timer armed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    hibernated(&rig).await;
    assert_eq!(rig.status().await["health"]["wakes"], json!(0));

    // A tool call running longer than the idle: up until it ends.
    let (rig, _) = idle_rig(on_demand(false)).await;
    let control = rig.control.clone();
    let slow = tokio::spawn(async move { control.call_tool(tool("slow", json!({"steps": 8, "ms": 100}), "s")).await });
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(state(&rig).await, json!("running"), "hibernated under a tool call");
    slow.await.unwrap().unwrap();
    hibernated(&rig).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn resident_always_it_never_hibernates() {
    let rig = Rig::new(|o| o.idle = Some(IDLE));
    rig.install(|m| m.replace("watch_sessions = true", "watch_sessions = false")).await;
    tokio::time::sleep(IDLE * 4).await;
    let status = rig.status().await;
    assert_eq!((status["state"].clone(), status["resident"].clone(), status["idle_ms"].clone()), (json!("running"), json!("always"), Value::Null));
}

#[tokio::test(flavor = "multi_thread")]
async fn stopped_while_hibernating_it_is_stopped_and_starts_again_from_its_record() {
    let (rig, parts) = idle_rig(on_demand(false)).await;
    rig.platform.device_call("notes.add", json!({"text": "kept"})).await.unwrap();
    hibernated(&rig).await;
    rig.control.stop(Stop { app: ID.into(), parts: parts.clone() }).await.unwrap();
    assert_eq!(rig.status().await, json!({"state": "stopped"}));
    rig.control
        .start(Start { app: ID.into(), version: "0.1.0".into(), app_dir: rig.app_dir(), parts })
        .await
        .unwrap();
    assert_eq!(state(&rig).await, json!("running"));
    assert_eq!(rig.platform.device_call("notes.list", json!({})).await.unwrap()["notes"][0]["text"], json!("kept"));
}
