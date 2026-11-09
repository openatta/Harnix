//! A resident backend let go while idle (`AppVm::hibernate`) and started again by the next call
//! into it: what holds it up, what comes back and what does not, and that two calls at once start
//! one instance.
#![cfg(feature = "compile")]

mod common;

use common::*;
use harnix_wasm::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

async fn ok(vm: &AppVm, method: &str, params: Value) -> Value {
    vm.handle(method, &params, &json!({"subject": "kernel"}))
        .await
        .unwrap()
        .unwrap()
}

fn caps() -> Capabilities {
    Capabilities {
        storage_bytes: 1000,
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hibernating_backend_starts_again_with_its_store_and_without_its_memory() {
    let rig = rig("backend-probe", caps(), limits(5_000)).await;
    assert_eq!(ok(&rig.vm, "count", json!({})).await, json!(1));
    ok(&rig.vm, "kv.put", json!({"key": "k", "size": "5"})).await;
    assert!(rig.vm.hibernate().await);
    let health = rig.vm.health();
    assert!(health.hibernating && rig.vm.hibernating());
    assert_eq!((health.state, health.recycles, health.faults), (VmState::Running, 0, 0));
    assert!(!rig.vm.hibernate().await, "already");

    // Its tools still run, each in an instance of its own, and wake nothing.
    rig.vm
        .call_tool("peek", &json!({}), "c", None, None, &CancellationToken::new())
        .await
        .unwrap();
    assert!(rig.vm.hibernating());

    // A call starts it again: its memory from nothing, its store as it was.
    assert_eq!(ok(&rig.vm, "count", json!({})).await, json!(1));
    assert_eq!(ok(&rig.vm, "kv.get", json!({"key": "k"})).await, json!(5));
    let health = rig.vm.health();
    assert!(!health.hibernating);
    assert_eq!((health.wakes, health.recycles), (1, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subscription_or_a_timer_holds_the_backend_up() {
    let subscribed = rig("backend-probe", caps(), limits(5_000)).await;
    ok(&subscribed.vm, "subscribe", json!({"topic": "harnix.sessions.status"})).await;
    assert!(!subscribed.vm.hibernate().await, "subscribed");
    let timed = rig("backend-probe", caps(), limits(5_000)).await;
    ok(&timed.vm, "timer", json!({"id": "t", "ms": "60000"})).await;
    assert!(!timed.vm.hibernate().await, "a timer armed");
    assert_eq!(timed.vm.health().timers, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_at_once_on_a_hibernating_backend_start_one_instance() {
    let rig = rig("backend-probe", caps(), limits(5_000)).await;
    assert!(rig.vm.hibernate().await);
    let calls: Vec<_> = (0..4)
        .map(|_| {
            let vm = rig.vm.clone();
            tokio::spawn(async move { ok(&vm, "count", json!({})).await })
        })
        .collect();
    let mut counts = Vec::new();
    for call in calls {
        counts.push(call.await.unwrap());
    }
    counts.sort_by_key(|v| v.as_u64());
    assert_eq!(counts, [json!(1), json!(2), json!(3), json!(4)], "one instance counted them all");
    assert_eq!(rig.vm.health().wakes, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn stopped_while_hibernating_it_stays_stopped() {
    let rig = rig("backend-probe", caps(), limits(5_000)).await;
    assert!(rig.vm.hibernate().await);
    rig.vm.stop().await;
    assert!(matches!(
        rig.vm.handle("count", &json!({}), &json!({})).await,
        Err(CallFailure::Unavailable(_))
    ));
    assert_eq!(rig.vm.health().wakes, 0);
}
