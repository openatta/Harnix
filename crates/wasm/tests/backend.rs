//! The resident backend against a real component (`fixtures/backend-probe`):
//! methods and the host services they reach, the state that lives in the
//! instance, and what happens to that instance when a call times out or traps.
#![cfg(feature = "compile")]

mod common;

use common::*;
use harnix_wasm::*;
use serde_json::{json, Value};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

async fn call(
    vm: &AppVm,
    method: &str,
    params: Value,
) -> Result<Result<Value, AppError>, CallFailure> {
    vm.handle(
        method,
        &params,
        &json!({"subject": {"kind": "device", "id": "d1"}}),
    )
    .await
}

async fn ok(vm: &AppVm, method: &str, params: Value) -> Value {
    call(vm, method, params).await.unwrap().unwrap()
}

fn backend_caps() -> Capabilities {
    Capabilities {
        storage_bytes: 1000,
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_method_reaches_the_backend_with_its_params_caller_and_configuration() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    assert_eq!(rig.vm.world(), World::Backend);
    assert_eq!(
        ok(&rig.vm, "echo", json!({"a": [1, 2]})).await,
        json!({"a": [1, 2]})
    );
    assert_eq!(
        ok(&rig.vm, "caller", json!({})).await,
        json!({"subject": {"kind": "device", "id": "d1"}})
    );
    assert_eq!(
        ok(&rig.vm, "config", json!({})).await,
        json!({"greeting": "hi"})
    );

    // The application's own error, passed through rather than judged.
    let err = call(&rig.vm, "fail", json!({})).await.unwrap().unwrap_err();
    assert_eq!((err.code, err.message.as_str()), (42, "failing as asked"));
    assert_eq!(err.data, Some(json!({"why": "test"})));
    let err = call(&rig.vm, "not-json", json!({}))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(err.code, -32603);
    assert_eq!(rig.vm.health().faults, 0, "an answer is not a fault");
}

#[tokio::test(flavor = "multi_thread")]
async fn state_lives_in_the_resident_instance_and_tools_cannot_see_it() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    for n in 1..=3 {
        assert_eq!(ok(&rig.vm, "count", json!({})).await, json!(n));
    }
    // The backend world's tools run in instances of their own.
    let tools = rig.vm.list_tools(&CancellationToken::new()).await.unwrap();
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["peek", "explode"]
    );
    let out = rig
        .vm
        .call_tool(
            "peek",
            &json!({}),
            "c",
            None,
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(out.content, "0");
}

#[tokio::test(flavor = "multi_thread")]
async fn host_calls_and_publishes_go_to_the_host_as_the_application() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    let answer = ok(
        &rig.vm,
        "call",
        json!({"target": "harnix.sessions", "x": 1}),
    )
    .await;
    assert_eq!(answer["answered"], "harnix.sessions");
    assert_eq!(
        rig.host.calls.lock().unwrap().as_slice(),
        [(
            "acme.probe".to_string(),
            "harnix.sessions".to_string(),
            json!({"target": "harnix.sessions", "x": 1})
        )]
    );
    let refused = call(&rig.vm, "call", json!({"target": "test.refuse"}))
        .await
        .unwrap()
        .unwrap_err();
    assert!(refused.message.contains("may not call"), "{refused:?}");

    assert_eq!(
        ok(&rig.vm, "publish", json!({"topic": "notes", "n": 1})).await,
        json!(true)
    );
    assert_eq!(
        rig.host.published_on("notes"),
        [json!({"topic": "notes", "n": 1})]
    );
    let refused = call(&rig.vm, "publish", json!({"topic": "undeclared"}))
        .await
        .unwrap()
        .unwrap_err();
    assert!(refused.message.contains("not a topic"), "{refused:?}");
}

/// A backend waiting on `host.call` still holds its instance; a call routed
/// back into it must be refused, not wait for itself forever.
#[tokio::test(flavor = "multi_thread")]
async fn a_call_routed_back_into_the_same_backend_is_refused_rather_than_deadlocking() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    rig.host.route_to.set(rig.vm.clone()).ok().unwrap();
    let answer = tokio::time::timeout(
        Duration::from_secs(3),
        call(&rig.vm, "call", json!({"target": "app.acme.probe.count"})),
    )
    .await
    .expect("deadlocked")
    .unwrap()
    .unwrap_err();
    assert!(answer.message.contains("re-enter"), "{answer:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_is_shared_and_bounded_by_the_quota() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    assert_eq!(
        ok(&rig.vm, "kv.put", json!({"key": "a", "size": "600"})).await,
        json!(true)
    );
    assert_eq!(ok(&rig.vm, "kv.get", json!({"key": "a"})).await, json!(600));
    let refused = call(&rig.vm, "kv.put", json!({"key": "b", "size": "600"}))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.code, 2);
    assert!(refused.message.contains("quota"), "{refused:?}");
    assert_eq!(
        ok(&rig.vm, "kv.get", json!({"key": "b"})).await,
        Value::Null
    );
    // Rewriting the same key smaller fits.
    assert_eq!(
        ok(&rig.vm, "kv.put", json!({"key": "a", "size": "10"})).await,
        json!(true)
    );
    assert_eq!(rig.kv.used_bytes(), 11);
    let health = rig.vm.health();
    assert_eq!(
        (health.storage_bytes, health.storage_limit_bytes),
        (11, 1000)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_with_no_quota_declared_takes_nothing() {
    let rig = rig("backend-probe", Capabilities::default(), limits(5_000)).await;
    let refused = call(&rig.vm, "kv.put", json!({"key": "a", "size": "1"}))
        .await
        .unwrap()
        .unwrap_err();
    assert!(refused.message.contains("quota"), "{refused:?}");
}

/// A call stopped halfway leaves the instance in a state nobody can vouch
/// for, so it is replaced: the next call works, on a fresh instance.
#[tokio::test(flavor = "multi_thread")]
async fn a_method_that_outlives_its_timeout_is_stopped_and_the_backend_restarted() {
    let rig = rig("backend-probe", backend_caps(), limits(300)).await;
    assert_eq!(ok(&rig.vm, "count", json!({})).await, json!(1));

    assert_eq!(
        call(&rig.vm, "spin", json!({})).await.unwrap_err(),
        CallFailure::TimedOut
    );

    assert_eq!(
        ok(&rig.vm, "count", json!({})).await,
        json!(1),
        "a fresh instance"
    );
    assert_eq!(
        ok(&rig.vm, "config", json!({})).await,
        json!({"greeting": "hi"}),
        "init ran again"
    );
    let health = rig.vm.health();
    assert_eq!(health.state, VmState::Running);
    assert_eq!(
        (
            health.timeouts,
            health.recycles,
            health.rebuilds,
            health.faults
        ),
        (1, 1, 0, 0)
    );
    assert!(health.last_error.unwrap().contains("restarted"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_backend_that_traps_is_rebuilt_once_and_disabled_the_second_time() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    assert_eq!(ok(&rig.vm, "count", json!({})).await, json!(1));

    let err = call(&rig.vm, "trap", json!({})).await.unwrap_err();
    assert!(
        matches!(&err, CallFailure::Faulted(m) if m.contains("unreachable") || m.contains("trap")),
        "{err:?}"
    );
    assert_eq!(
        ok(&rig.vm, "count", json!({})).await,
        json!(1),
        "rebuilt, so its state is gone"
    );
    assert_eq!(rig.vm.health().rebuilds, 1);

    call(&rig.vm, "trap", json!({})).await.unwrap_err();
    let health = rig.vm.health();
    assert!(
        matches!(&health.state, VmState::Disabled { reason } if reason.contains("crashed again")),
        "{health:?}"
    );
    assert_eq!(health.memory_bytes, 0, "the instance was dropped");
    let err = call(&rig.vm, "count", json!({})).await.unwrap_err();
    assert!(
        matches!(&err, CallFailure::Unavailable(m) if m.contains("disabled")),
        "{err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_made_at_once_queue_for_the_one_instance() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    let calls: Vec<_> = (0..20)
        .map(|_| {
            let vm = rig.vm.clone();
            tokio::spawn(async move { ok(&vm, "count", json!({})).await })
        })
        .collect();
    let mut seen = Vec::new();
    for call in calls {
        seen.push(call.await.unwrap().as_u64().unwrap());
    }
    seen.sort();
    assert_eq!(seen, (1..=20).collect::<Vec<u64>>(), "each call saw the one before it");
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_while_a_call_holds_the_backend_starts_nothing_new() {
    let rig = rig("backend-probe", backend_caps(), limits(1_000)).await;
    let vm = rig.vm.clone();
    let spinning = tokio::spawn(async move { call(&vm, "spin", json!({})).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let vm = rig.vm.clone();
    let queued = tokio::spawn(async move { call(&vm, "count", json!({})).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    rig.vm.stop().await;
    assert_eq!(spinning.await.unwrap().unwrap_err(), CallFailure::TimedOut);
    let waiting = queued.await.unwrap().unwrap_err();
    assert!(
        matches!(&waiting, CallFailure::Unavailable(m) if m.contains("stopped")),
        "{waiting:?}"
    );
    let health = rig.vm.health();
    assert_eq!(health.state, VmState::Stopped, "not reported broken for stopping");
    assert_eq!(health.recycles, 0, "no fresh instance for a VM that is going");
    assert_eq!(health.memory_bytes, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_after_a_healthy_period_is_rebuilt_like_a_first_one() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    rig.vm.set_healthy_period(Duration::from_millis(300));
    call(&rig.vm, "trap", json!({})).await.unwrap_err();
    tokio::time::sleep(Duration::from_millis(400)).await;
    call(&rig.vm, "trap", json!({})).await.unwrap_err();
    let health = rig.vm.health();
    assert_eq!(health.state, VmState::Running, "{health:?}");
    assert_eq!(health.rebuilds, 2);
    assert_eq!(ok(&rig.vm, "count", json!({})).await, json!(1));
    // Within the period, the second crash is the last.
    call(&rig.vm, "trap", json!({})).await.unwrap_err();
    assert!(matches!(rig.vm.health().state, VmState::Disabled { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn files_under_data_count_toward_the_storage_quota() {
    let caps = Capabilities {
        fs: vec![FsGrant {
            path: "${data}".into(),
            writable: true,
        }],
        ..backend_caps()
    };
    let rig = rig("backend-probe", caps, limits(5_000)).await;
    assert_eq!(
        ok(&rig.vm, "file.write", json!({"name": "f", "size": "700"})).await,
        json!(true)
    );
    let health = rig.vm.health();
    assert_eq!((health.storage_bytes, health.storage_files_bytes), (700, 700));
    // The files have taken 700 of the 1000: a value that would fit an empty store does not.
    let refused = call(&rig.vm, "kv.put", json!({"key": "a", "size": "400"}))
        .await
        .unwrap()
        .unwrap_err();
    assert!(refused.message.contains("files under /data"), "{refused:?}");
    assert_eq!(
        ok(&rig.vm, "kv.put", json!({"key": "a", "size": "200"})).await,
        json!(true)
    );
    assert_eq!(rig.vm.health().storage_bytes, 901, "the key counts too");

    // Over the quota, the resident instance itself is replaced: the one that takes over has /data
    // read-only, so its next write is refused — no trap or timeout needed to get there.
    assert_eq!(ok(&rig.vm, "count", json!({})).await, json!(1));
    ok(&rig.vm, "file.write", json!({"name": "g", "size": "200"})).await;
    let health = rig.vm.health();
    assert!(health.storage_bytes >= 1000);
    assert_eq!((health.storage_recycles, health.recycles, health.rebuilds), (1, 0, 0));
    assert_eq!(ok(&rig.vm, "count", json!({})).await, json!(1), "a fresh instance");
    let refused = call(&rig.vm, "file.write", json!({"name": "h", "size": "1"}))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.code, 5, "{refused:?}");
    assert!(!rig.dir.path().join("data/h").exists());
    // Deleting is how it makes room, and it still may; back under, it may write again.
    assert_eq!(
        ok(&rig.vm, "file.delete", json!({"name": "g"})).await,
        json!(true)
    );
    assert_eq!(rig.vm.health().storage_recycles, 2);
    assert_eq!(
        ok(&rig.vm, "file.write", json!({"name": "h", "size": "1"})).await,
        json!(true)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tool_that_keeps_faulting_is_set_aside_and_the_backend_goes_on() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    assert_eq!(ok(&rig.vm, "count", json!({})).await, json!(1));
    for _ in 0..FAULT_LIMIT {
        rig.vm
            .call_tool("explode", &json!({}), "c", None, None, &CancellationToken::new())
            .await
            .unwrap_err();
    }
    let err = rig
        .vm
        .call_tool("explode", &json!({}), "c", None, None, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(&err, CallFailure::Unavailable(m) if m.contains("set aside")), "{err:?}");
    // The backend kept its instance and its memory; the other tool runs.
    assert_eq!(ok(&rig.vm, "count", json!({})).await, json!(2));
    rig.vm
        .call_tool("peek", &json!({}), "c", None, None, &CancellationToken::new())
        .await
        .unwrap();
    let health = rig.vm.health();
    assert_eq!(health.state, VmState::Running);
    assert_eq!((health.rebuilds, health.recycles), (0, 0));
    assert_eq!(health.tools_aside.keys().collect::<Vec<_>>(), ["explode"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_that_cannot_get_the_backend_in_time_is_answered_busy() {
    let rig = rig("backend-probe", backend_caps(), limits(300)).await;
    let spin = |vm: AppVm| tokio::spawn(async move { call(&vm, "spin", json!({})).await });
    let first = spin(rig.vm.clone());
    tokio::time::sleep(Duration::from_millis(20)).await;
    let second = spin(rig.vm.clone());
    tokio::time::sleep(Duration::from_millis(20)).await;
    // Behind two calls that each hold the backend for its whole timeout: not served in time.
    let err = call(&rig.vm, "count", json!({})).await.unwrap_err();
    assert!(matches!(&err, CallFailure::Unavailable(m) if m.starts_with("busy")), "{err:?}");
    assert_eq!(first.await.unwrap().unwrap_err(), CallFailure::TimedOut);
    let _ = second.await.unwrap();
    assert!(rig.vm.health().queue_timeouts >= 1);
    assert_eq!(rig.vm.health().state, VmState::Running);
    ok(&rig.vm, "count", json!({})).await;
}

/// A store that cannot be read, as a disk that went away would be.
struct BrokenKv;

impl KvStore for BrokenKv {
    fn get(&self, _key: &str) -> Result<Option<Vec<u8>>, KvError> {
        Err(KvError::Io("/somewhere/secret/kv/k: input/output error".into()))
    }
    fn put(&self, _key: &str, _value: Option<&[u8]>, _quota: u64) -> Result<(), KvError> {
        Err(KvError::Io("/somewhere/secret/kv/k: input/output error".into()))
    }
    fn used_bytes(&self) -> u64 {
        0
    }
    fn clear(&self) -> Result<(), KvError> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn storage_that_cannot_be_read_is_an_error_not_an_absent_key() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    assert_eq!(ok(&rig.vm, "kv.get", json!({"key": "k"})).await, json!(null));
    assert_eq!(ok(&rig.vm, "kv.delete", json!({"key": "k"})).await, json!(true), "deleting nothing is fine");

    let dir = tempfile::tempdir().unwrap();
    let services = Services {
        host: std::sync::Arc::new(MockHost::default()),
        kv: std::sync::Arc::new(BrokenKv),
        http: std::sync::Arc::new(NoNetwork),
    };
    let vm = AppVm::start(
        engine(),
        &component("backend-probe"),
        spec(backend_caps(), limits(5_000), dir.path()),
        services,
    )
    .await
    .unwrap();
    for method in ["kv.get", "kv.delete"] {
        let err = call(&vm, method, json!({"key": "k"})).await.unwrap().unwrap_err();
        assert!(err.message.contains("could not be reached"), "{method}: {err:?}");
        assert!(!err.message.contains("/somewhere"), "the host's path is not the component's: {err:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_timer_fires_once_into_the_backend() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    assert_eq!(
        ok(&rig.vm, "timer", json!({"id": "t1", "ms": "50"})).await,
        json!(true)
    );
    // Re-arming replaces: one firing, not two.
    ok(&rig.vm, "timer", json!({"id": "t1", "ms": "80"})).await;
    assert_eq!(rig.vm.health().timers, 1);
    eventually(Duration::from_secs(3), "the timer", || {
        !rig.host.published_on("timer").is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(rig.host.published_on("timer"), [json!({"id": "t1"})]);
    assert_eq!(rig.vm.health().timers, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn timers_are_bounded_in_number() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    for i in 0..MAX_TIMERS {
        ok(
            &rig.vm,
            "timer",
            json!({"id": format!("t{i}"), "ms": "60000"}),
        )
        .await;
    }
    let refused = call(&rig.vm, "timer", json!({"id": "one-more", "ms": "1"}))
        .await
        .unwrap()
        .unwrap_err();
    assert!(refused.message.contains("timers armed"), "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subscription_delivers_topic_values_in_order() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    assert_eq!(
        ok(
            &rig.vm,
            "subscribe",
            json!({"topic": "harnix.sessions.status"})
        )
        .await,
        json!(true)
    );
    assert_eq!(
        rig.host.subscribed.lock().unwrap().as_slice(),
        ["harnix.sessions.status"]
    );
    let refused = call(&rig.vm, "subscribe", json!({"topic": "forbidden.topic"}))
        .await
        .unwrap()
        .unwrap_err();
    assert!(refused.message.contains("hub"), "{refused:?}");

    for n in 0..5 {
        rig.vm
            .deliver_topic("harnix.sessions.status", &json!({"n": n}));
    }
    eventually(Duration::from_secs(3), "five deliveries", || {
        rig.host.published_on("seen").len() == 5
    })
    .await;
    let seen: Vec<Value> = rig
        .host
        .published_on("seen")
        .iter()
        .map(|v| v["value"]["n"].clone())
        .collect();
    assert_eq!(seen, [0, 1, 2, 3, 4]);
}

/// Subscriptions and timers belong to the instance that made them. When it
/// is replaced they are undone — and `init` makes its own again.
#[tokio::test(flavor = "multi_thread")]
async fn a_replaced_instance_takes_its_subscriptions_and_timers_with_it() {
    let rig = rig_with("backend-probe", |dir| {
        let mut s = spec(backend_caps(), limits(300), dir);
        s.config = json!({"init_topic": "from.init"});
        s
    })
    .await;
    ok(&rig.vm, "subscribe", json!({"topic": "from.call"})).await;
    ok(&rig.vm, "timer", json!({"id": "late", "ms": "600"})).await;
    assert_eq!(rig.vm.health().subscriptions, 2);

    assert_eq!(
        call(&rig.vm, "spin", json!({})).await.unwrap_err(),
        CallFailure::TimedOut
    );

    let mut gone = rig.host.unsubscribed.lock().unwrap().clone();
    gone.sort();
    assert_eq!(gone, ["from.call", "from.init"]);
    assert_eq!(
        rig.host.subscribed.lock().unwrap().as_slice(),
        ["from.init", "from.call", "from.init"],
        "init subscribed again on the fresh instance"
    );
    let health = rig.vm.health();
    assert_eq!((health.subscriptions, health.timers), (1, 0));
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        rig.host.published_on("timer").is_empty(),
        "the old instance's timer was cancelled"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_calls_stop_and_releases_everything() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    ok(&rig.vm, "subscribe", json!({"topic": "a.topic"})).await;
    ok(&rig.vm, "timer", json!({"id": "t", "ms": "60000"})).await;

    rig.vm.stop().await;

    assert_eq!(rig.host.published_on("stopped"), [json!({})]);
    assert_eq!(
        rig.host.unsubscribed.lock().unwrap().as_slice(),
        ["a.topic"]
    );
    let health = rig.vm.health();
    assert_eq!(health.state, VmState::Stopped);
    assert_eq!(
        (health.subscriptions, health.timers, health.memory_bytes),
        (0, 0, 0)
    );
    let err = call(&rig.vm, "count", json!({})).await.unwrap_err();
    assert!(
        matches!(&err, CallFailure::Unavailable(m) if m.contains("stopped")),
        "{err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_backend_whose_init_fails_does_not_start() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = spec(backend_caps(), limits(5_000), dir.path());
    s.config = json!({"mode": "fail-init"});
    let services = Services {
        host: std::sync::Arc::new(MockHost::default()),
        kv: std::sync::Arc::new(MemoryKv::new()),
        http: std::sync::Arc::new(NoNetwork),
    };
    let err = AppVm::start(engine(), &component("backend-probe"), s, services)
        .await
        .err()
        .expect("init refused")
        .to_string();
    assert!(
        err.contains("init") && err.contains("asks init to fail"),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_health_snapshot_serializes_for_the_system_space() {
    let rig = rig("backend-probe", backend_caps(), limits(5_000)).await;
    ok(&rig.vm, "count", json!({})).await;
    let v = serde_json::to_value(rig.vm.health()).unwrap();
    assert_eq!(v["app"], "acme.probe");
    assert_eq!(v["world"], "backend");
    assert_eq!(v["state"], "running");
    assert_eq!(v["calls"], 1);
    assert!(
        v["memory_bytes"].as_u64().unwrap() > 0,
        "the resident instance holds memory"
    );
    assert_eq!(v["memory_limit_bytes"], 32 << 20);
}
