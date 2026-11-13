//! The host against the real showcase (`examples/apps/showcase`), with a fake platform and every
//! call of the port, both ways, carried through JSON: put in place, started, its methods, tools,
//! topics and hook used, stopped and taken back.
#![cfg(feature = "compile")]

mod common;

use common::*;
use harnix_wasm_port::*;
use harnix_contract::sync::lock;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn the_showcase_runs_behind_a_port_that_carries_over_a_wire() {
    let rig = Rig::new(|_| {});
    let parts = rig.install(|m| m).await;
    assert_eq!(parts["agent"]["instances"], json!(["main"]));
    assert_eq!(parts["skills"]["main"], json!(["showcase-tour"]));
    // In order: the agent side put in place, switched on with it, then its names, then its tools.
    let said = lock(&rig.platform.said).clone();
    let at = |what: &str| said.iter().position(|s| s.starts_with(what)).unwrap_or_else(|| panic!("{what}: {said:?}"));
    assert!(at("install_agent example-showcase main") < at("switch_agent example-showcase on"));
    assert!(at("switch_agent") < at("register example.showcase"));
    assert!(at("register") < at("subscribe harnix.sessions.status"), "init ran after it was registered");
    assert!(at("subscribe") < at("expose_tools 8"));

    let registration = lock(&rig.platform.registered)[0].clone();
    assert_eq!(registration.digest.len(), 64, "the manifest's sha256");
    assert_eq!(registration.tools.len(), 8, "its declared tools");
    assert!(registration.methods.iter().any(|m| m.name == "state" && m.subjects == ["device", "module"]));
    assert_eq!(registration.topics, ["notes", "heartbeat", "fetched"]);
    assert_eq!(registration.spaces[0].id, ID);
    let status = rig.status().await;
    assert_eq!((status["state"].clone(), status["world"].clone()), (json!("running"), json!("backend")));
    assert_eq!(status["health"]["subscriptions"], json!(1));
    assert_eq!(status["tools"][0]["name"], json!("ext__example_showcase__echo"));

    // Methods: the schema first, then the backend; its topic published in order; who called it.
    assert_eq!(rig.platform.device_call("notes.add", json!({"text": "one"})).await.unwrap(), json!({"count": 1}));
    rig.platform.device_call("notes.add", json!({"text": "two"})).await.unwrap();
    let bad = rig.platform.device_call("notes.add", json!({"text": 3})).await.unwrap_err();
    assert_eq!(bad.code, codes::INVALID_PARAMS);
    let listed = rig.platform.device_call("notes.list", json!({})).await.unwrap();
    assert_eq!(listed["notes"][0]["by"], json!("device:d1"));
    eventually("both notes published", async || lock(&rig.platform.published).len() >= 2).await;
    let counts: Vec<_> = lock(&rig.platform.published).iter().map(|(_, v)| v["count"].clone()).collect();
    assert_eq!(counts, [json!(1), json!(2)]);
    // The value its subscription answered with reached `on-topic`; a later one is delivered.
    eventually("the first value", async || {
        rig.platform.device_call("state", json!({})).await.unwrap()["status_seen"] == json!(1)
    })
    .await;
    rig.control
        .deliver(CallMeta::default(), TopicValue {
            app: ID.into(),
            topic: "harnix.sessions.status".into(),
            value: json!({"sessions": [{}, {}, {}]}),
        })
        .await
        .unwrap();
    eventually("the delivered value", async || {
        rig.platform.device_call("state", json!({})).await.unwrap()["running_sessions"] == json!(3)
    })
    .await;
    // As itself, as far as its hub: one through, one refused and kept.
    rig.platform.device_call("session.start", json!({"message": "hi"})).await.unwrap();
    assert_eq!(rig.platform.said("call "), ["call harnix.session.create", "call harnix.session.send"]);
    assert_eq!(rig.platform.device_call("overreach", json!({})).await.unwrap()["refused"], json!(true));
    assert_eq!(rig.status().await["calls"]["refused"][0]["method"], json!("packages.uninstall"));

    // Tools: an instance each; progress in order, all of it before the answer; a cancellation;
    // and one that asks the backend, as the application, through the platform and back.
    let tool = |name: &str, input: serde_json::Value, call_id: &str| ToolCall {
        app: ID.into(),
        tool: name.into(),
        input,
        call_id: call_id.into(),
        project: None,
        progress: true,
    };
    match rig.control.call_tool(CallMeta::default(), tool("echo", json!({"text": "hi"}), "c1")).await.unwrap() {
        ToolReply::Output { structured, is_error, .. } => {
            assert_eq!(structured, Some(json!({"echoed": "hi"})));
            assert!(!is_error);
        }
        other => panic!("{other:?}"),
    }
    rig.control.call_tool(CallMeta::default(), tool("slow", json!({"steps": 3, "ms": 10}), "c2")).await.unwrap();
    assert_eq!(*lock(&rig.platform.progress), ["step 1 of 3", "step 2 of 3", "step 3 of 3"]);
    let control = rig.control.clone();
    let slow = tokio::spawn(async move { control.call_tool(CallMeta::default(), tool("slow", json!({"steps": 100, "ms": 200}), "c3")).await });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    rig.control.cancel(CallMeta::default(), "c3".into()).await.unwrap();
    assert!(matches!(
        slow.await.unwrap().unwrap(),
        ToolReply::Failed { failure: Failure::Cancelled, .. }
    ));
    match rig.control.call_tool(CallMeta::default(), tool("board", json!({}), "c4")).await.unwrap() {
        ToolReply::Output { content, .. } => assert!(content.contains("the board has 2 notes"), "{content}"),
        other => panic!("{other:?}"),
    }

    // Its hook refuses the tool its configuration names, and lets the rest through.
    let hook = |tool: &str| HookCall {
        app: ID.into(),
        event: "tool.before".into(),
        payload: json!({"tool": tool, "input": {}, "session_id": "s1", "depth": 0}),
        call_id: format!("h-{tool}"),
    };
    assert!(matches!(rig.control.hook(CallMeta::default(), hook("ext__example_showcase__vetoed")).await.unwrap(), HookReply::Block(_)));
    // Another's tool is not its to see (`[events] tools = "own"`): no instance is even started.
    let calls = rig.status().await["health"]["calls"].clone();
    assert_eq!(rig.control.hook(CallMeta::default(), hook("Bash")).await.unwrap(), HookReply::Proceed);
    assert_eq!(rig.status().await["health"]["calls"], calls, "the hook was not run for `Bash`");

    // Stopped: its tools withdrawn, then its names, its subscription given back, its agent side off;
    // a method answers that it is not running.
    rig.control.stop(CallMeta::default(), Stop { app: ID.into(), parts: parts.clone() }).await.unwrap();
    assert!(lock(&rig.platform.offers).last().unwrap().tools.is_empty());
    assert_eq!(rig.platform.said("withdraw"), ["withdraw example.showcase"]);
    assert_eq!(rig.platform.said("unsubscribe"), ["unsubscribe harnix.sessions.status"]);
    assert_eq!(rig.platform.said("switch_agent example-showcase off").len(), 1);
    assert_eq!(rig.platform.said("remove_skills"), ["remove_skills showcase-tour"]);
    let gone = rig.platform.device_call("notes.list", json!({})).await.unwrap_err();
    assert_eq!(gone.code, codes::EXTENSION_UNAVAILABLE);
    assert_eq!(gone.data.unwrap()["failure"], json!("stopped"));
    assert_eq!(rig.status().await, json!({"state": "stopped"}));
    // Taken back.
    let taken = rig.control.unplace(CallMeta::default(), Unplace { app: ID.into(), parts }).await.unwrap();
    assert_eq!(taken["agent"]["package"], json!("example-showcase"));
    assert_eq!(rig.platform.said("uninstall_agent"), ["uninstall_agent example-showcase main"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn what_does_not_start_is_undone_and_said() {
    // A component that is not the one the product verified: refused before anything is
    // registered. So is a signed package the product gave no sha256 for.
    let rig = Rig::new(|_| {});
    let version = rig.unpack(|m| m);
    let mut start = rig.start_of(&version, json!({}));
    start.cwasm_sha256 = Some("00".repeat(32));
    let err = rig.control.start(CallMeta::default(), start.clone()).await.unwrap_err();
    assert!(err.message.contains("not the one that was verified"), "{}", err.message);
    assert!(rig.platform.said("register").is_empty());
    assert_eq!(rig.status().await["state"], json!("failed"));
    start.cwasm_sha256 = None;
    let err = rig.control.start(CallMeta::default(), start.clone()).await.unwrap_err();
    assert!(err.message.contains("given no sha256"), "{}", err.message);
    rig.control.forget(CallMeta::default(), ID.into()).await.unwrap();
    assert_eq!(rig.status().await["state"], json!("stopped"));
    // Unsigned, it is the product's word that it may run: it starts.
    start.trust = "unsigned".into();
    rig.control.start(CallMeta::default(), start).await.unwrap();
    assert_eq!(rig.status().await["state"], json!("running"));

    // A tool row for a tool the manifest does not declare: started, found out, and taken back.
    let rig = Rig::new(|_| {});
    let version = rig.unpack(|m| m.replace("tool = \"count\"", "tool = \"counted\""));
    let parts = json!({"agent": {"package": "example-showcase", "version": "0.1.0", "instances": ["main"]}});
    let err = rig
        .control
        .start(CallMeta::default(), rig.start_of(&version, parts))
        .await
        .unwrap_err();
    assert!(err.message.contains("does not declare as a `[[tool]]`"), "{}", err.message);
    assert_eq!(rig.platform.said("withdraw"), ["withdraw example.showcase"]);
    assert_eq!(rig.platform.said("switch_agent example-showcase off").len(), 1);
    assert!(rig.platform.said("expose_tools").is_empty());

    // A manifest that declares a tool the component does not list, or describes one differently:
    // the manifest is what was consented to, so the component is the one refused.
    for edit in [
        |m: String| m.replace("name = \"count\"", "name = \"counter\""),
        |m: String| m.replace("Say `text` back, exactly", "Say something else back"),
    ] {
        let rig = Rig::new(|_| {});
        let version = rig.unpack(edit);
        let parts = json!({"agent": {"package": "example-showcase", "version": "0.1.0", "instances": ["main"]}});
        let err = rig
            .control
            .start(CallMeta::default(), rig.start_of(&version, parts))
            .await
            .unwrap_err();
        assert!(err.message.contains("does not provide the tools its manifest declares"), "{}", err.message);
        assert!(rig.platform.said("expose_tools").is_empty());
    }

    // An instance that is not there: refused by name before anything reaches the agent engine.
    let rig = Rig::new(|_| {});
    let version = rig.unpack(|m| m.replace("install_into = [\"main\"]", "install_into = [\"nowhere\"]"));
    let err = rig
        .control
        .place(CallMeta::default(), Place { app: ID.into(), app_dir: rig.app_dir(), version, instances: None })
        .await
        .unwrap_err();
    assert!(err.message.contains("no such instance"), "{}", err.message);
    assert!(rig.platform.said("install_agent").is_empty());
}

/// The showcase with its hook told to refuse `Bash`, and `[events]` as `events` says.
fn refusing_bash(events: &'static str) -> impl Fn(String) -> String {
    move |m: String| m.replace("[events]\ntools = \"own\"", &format!("[events]\ntools = \"{events}\""))
}

fn blocked_bash() -> Rig {
    Rig::configured(json!({"blocked_tool": "Bash"}))
}

/// Another application's tool, by the name a session knows it by.
const OTHERS_TOOL: &str = "ext__example_other__echo";

fn blocked_others_tool() -> Rig {
    Rig::configured(json!({"blocked_tool": OTHERS_TOOL}))
}

async fn hook_for(rig: &Rig, tool: &str) -> HookReply {
    rig.control
        .hook(CallMeta::default(), HookCall {
            app: ID.into(),
            event: "tool.before".into(),
            payload: json!({"tool": tool, "input": {"command": "rm -rf /"}, "session_id": "s1", "depth": 0}),
            call_id: format!("h-{tool}"),
        })
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn its_hook_sees_every_tool_call_only_when_its_manifest_asks() {
    let own = blocked_bash();
    own.install(refusing_bash("own")).await;
    assert_eq!(hook_for(&own, "Bash").await, HookReply::Proceed, "not its to see, so not its to refuse");

    let all = blocked_bash();
    all.install(refusing_bash("all")).await;
    assert!(matches!(hook_for(&all, "Bash").await, HookReply::Block(_)));
    assert_eq!(hook_for(&all, "Read").await, HookReply::Proceed);

    // Another application's tool is not its own either: the same as a built-in one.
    let own = blocked_others_tool();
    own.install(refusing_bash("own")).await;
    let calls = own.status().await["health"]["calls"].clone();
    assert_eq!(hook_for(&own, OTHERS_TOOL).await, HookReply::Proceed, "another application's tool was its to refuse");
    assert_eq!(own.status().await["health"]["calls"], calls, "the hook ran for another application's tool");
    let all = blocked_others_tool();
    all.install(refusing_bash("all")).await;
    assert!(matches!(hook_for(&all, OTHERS_TOOL).await, HookReply::Block(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn its_secret_is_the_profiles_never_the_processs() {
    // This process has one by that name; the application is not given it.
    std::env::set_var("SHOWCASE_TOKEN", "the-process-own");
    let without = Rig::new(|_| {});
    without.install(|m| m).await;
    let state = without.platform.device_call("state", json!({})).await.unwrap();
    assert_eq!((state["secret_set"].clone(), state["secret_length"].clone()), (json!(false), json!(0)));

    let with = Rig::new(|_| {});
    *lock(&with.secrets) = [("SHOWCASE_TOKEN".to_string(), "from-the-profile".to_string())].into();
    with.install(|m| m).await;
    let state = with.platform.device_call("state", json!({})).await.unwrap();
    assert_eq!(state["secret_length"], json!("from-the-profile".len()));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_generation_moves_with_what_its_status_says() {
    let rig = Rig::new(|_| {});
    let before = rig.host.generation();
    let parts = rig.install(|m| m).await;
    let started = rig.host.generation();
    assert!(started > before, "a start moves it");
    // Each of these changes what its status answers, and moves it.
    rig.platform.device_call("notes.add", json!({"text": "one"})).await.unwrap();
    let called = rig.host.generation();
    assert!(called > started, "a method call");
    assert!(rig.status().await["calls"]["calls"].as_u64().unwrap() >= 1);
    let tool = ToolCall {
        app: ID.into(),
        tool: "echo".into(),
        input: json!({"text": "hi"}),
        call_id: "g1".into(),
        project: None,
        progress: false,
    };
    rig.control.call_tool(CallMeta::default(), tool).await.unwrap();
    let tooled = rig.host.generation();
    assert!(tooled > called, "a tool call");
    rig.control.stop(CallMeta::default(), Stop { app: ID.into(), parts }).await.unwrap();
    assert!(rig.host.generation() > tooled, "a stop");
}
