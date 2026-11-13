//! A call's chain, end to end in one process: a backend whose call comes back into itself while it
//! still serves the call that made it is refused at once, by the chain the call carries — however
//! the product routed it, and whether a call of the product's or a timer of its own started it —
//! and a chain that holds it on a call it answered long ago (a turn queued by a method that has
//! returned) is not.
#![cfg(feature = "compile")]

mod common;

use std::time::Duration;

use common::*;
use harnix_contract::sync::lock;
use harnix_wasm_port::*;
use serde_json::{json, Value};

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

/// The time a refusal may take: it is decided before the backend is asked, never by a timeout.
const AT_ONCE: Duration = Duration::from_millis(100);

#[tokio::test(flavor = "multi_thread")]
async fn a_backend_that_calls_back_into_itself_is_refused_at_once() {
    let rig = Rig::new(|_| {});
    rig.install(|m| m).await;
    // `overreach` calls out; the product routes that call into the application's own `state`,
    // from another task, as a link between processes carries it: nothing of the calling task
    // comes along, only the call's metadata.
    rig.platform.reroute.set("state".into()).unwrap();
    let device = CallMeta::new(next_id());
    let answer = rig.platform.device_call_as(device.clone(), "overreach", json!({})).await.unwrap();
    assert_eq!(answer["refused"], json!(true), "{answer}");
    assert!(answer["message"].as_str().unwrap().contains("would re-enter the backend of `example.showcase`"), "{answer}");
    let (refused, took) = lock(&rig.platform.routed_errors)[0].clone();
    assert_eq!(refused.code, codes::EXTENSION_UNAVAILABLE);
    assert_eq!(refused.data.unwrap()["failure"], json!("reentrant"));
    assert!(took < AT_ONCE, "refused after {took:?}");

    // The call the backend made named the device's call as its parent, and held the application
    // on it.
    let made = lock(&rig.platform.metas)[0].clone();
    assert_eq!(made.parent_id(), Some(device.call_id()));
    assert!(made.holds(ID, device.call_id()), "{made:?}");
    // Not recycled: the backend answers as it did.
    assert_eq!(rig.status().await["health"]["recycles"], json!(0));

    // The same chain once that call was answered: nothing waits on it any more.
    let later = CallMeta::new(next_id()).chain(made.links().iter().cloned());
    rig.platform.device_call_as(later, "state", json!({})).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_timer_that_calls_back_into_its_own_backend_is_refused_at_once() {
    let rig = Rig::new(|_| {});
    rig.install(|m| m).await;
    // Routed back from another task: nothing of the timer's task comes along.
    rig.platform.across.store(true, std::sync::atomic::Ordering::Relaxed);
    let armed = rig.platform.device_call("recall", json!({"later": true})).await.unwrap();
    assert_eq!(armed["armed"], json!(true), "{armed}");
    let mut recalled = Value::Null;
    eventually("the timer's call back", async || {
        recalled = rig.platform.device_call("state", json!({})).await.unwrap()["recalled"].clone();
        !recalled.is_null()
    })
    .await;
    assert_eq!(recalled["refused"], json!(true), "{recalled}");
    assert!(recalled["message"].as_str().unwrap().contains("would re-enter the backend of `example.showcase`"), "{recalled}");
    let (refused, took) = lock(&rig.platform.routed_errors)[0].clone();
    assert_eq!(refused.code, codes::EXTENSION_UNAVAILABLE);
    assert_eq!(refused.data.unwrap()["failure"], json!("reentrant"));
    assert!(took < AT_ONCE, "refused after {took:?}");

    // No call of the product's waits on the firing: the call it made names the firing as its
    // parent, and holds the application on it alone.
    let made = lock(&rig.platform.metas)[0].clone();
    let firing = made.parent_id().expect("made within the firing").to_string();
    assert_eq!(made.links().len(), 1, "{made:?}");
    assert!(made.holds(ID, &firing), "{made:?}");
    assert_eq!(rig.status().await["health"]["recycles"], json!(0));

    // The firing done, the same chain waits on nothing.
    let later = CallMeta::new(next_id()).chain(made.links().iter().cloned());
    rig.platform.device_call_as(later, "state", json!({})).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_tool_calls_carries_its_turns_chain_and_is_let_through_when_nothing_waits() {
    let rig = Rig::new(|_| {});
    rig.install(|m| m).await;
    // The tool's turn was started by a call of the application's own that has long returned.
    let turn = CallMeta::new(next_id()).chain([ChainLink::new(ID, "p-answered"), ChainLink::new("acme.other", "9")]);
    let board = ToolCall {
        app: ID.into(),
        tool: "board".into(),
        input: json!({}),
        call_id: "b1".into(),
        project: None,
        progress: false,
    };
    match rig.control.call_tool(turn.clone(), board).await.unwrap() {
        ToolReply::Output { content, .. } => assert!(content.contains("the board has"), "{content}"),
        other => panic!("{other:?}"),
    }
    // Its call into the backend was made for the tool call, carrying the turn's chain — and no
    // link of its own: a tool runs in an instance of its own and holds no backend.
    let made = lock(&rig.platform.metas).last().cloned().unwrap();
    assert_eq!(made.parent_id(), Some(turn.call_id()));
    assert_eq!(made.links(), turn.links());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_past_its_deadline_is_not_put_to_the_backend() {
    let rig = Rig::new(|_| {});
    rig.install(|m| m).await;
    let calls = rig.status().await["calls"]["calls"].clone();
    let late = CallMeta::new(next_id()).deadline_ms(now_ms().saturating_sub(1));
    let e = rig.platform.device_call_as(late, "state", json!({})).await.unwrap_err();
    assert_eq!(e.code, codes::DEADLINE_EXCEEDED);
    assert_eq!(rig.status().await["calls"]["calls"], calls, "the backend was not asked");
    // A deadline still ahead goes to every call made for it.
    let soon = CallMeta::new(next_id()).deadline_ms(now_ms() + 60_000);
    rig.platform.device_call_as(soon.clone(), "session.start", json!({"message": "hi"})).await.unwrap();
    assert!(lock(&rig.platform.metas).iter().all(|m| m.deadline() == soon.deadline()));
}
