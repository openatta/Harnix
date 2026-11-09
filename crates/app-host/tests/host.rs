//! The host against the real showcase (`examples/apps/showcase`), with a fake platform and every
//! call of the port, both ways, carried through JSON: put in place, started, its methods, tools,
//! topics and hook used, stopped and taken back.
#![cfg(feature = "compile")]

mod common;

use common::*;
use harnix_app_port::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn the_showcase_runs_behind_a_port_that_carries_over_a_wire() {
    let rig = Rig::new(|_| {});
    let parts = rig.install(|m| m).await;
    assert_eq!(parts["agent"]["instances"], json!(["system"]));
    assert_eq!(parts["skills"]["system"], json!(["showcase-tour"]));
    // In order: the agent side put in place, switched on with it, then its names, then its tools.
    let said = rig.platform.said.lock().unwrap().clone();
    let at = |what: &str| said.iter().position(|s| s.starts_with(what)).unwrap_or_else(|| panic!("{what}: {said:?}"));
    assert!(at("install_agent example-showcase system") < at("switch_agent example-showcase on"));
    assert!(at("switch_agent") < at("register example.showcase"));
    assert!(at("register") < at("subscribe harnix.sessions.status"), "init ran after it was registered");
    assert!(at("subscribe") < at("expose_tools 7"));

    let registration = rig.platform.registered.lock().unwrap()[0].clone();
    assert!(registration.methods.iter().any(|m| m.name == "state" && m.subjects == ["device", "module"]));
    assert_eq!(registration.topics, ["notes", "heartbeat"]);
    assert_eq!(registration.spaces[0].id, ID);
    let status = rig.status().await;
    assert_eq!((status["state"].clone(), status["world"].clone()), (json!("running"), json!("backend")));
    assert_eq!(status["health"]["subscriptions"], json!(1));
    assert_eq!(status["tools"][0]["name"], json!("app__example_showcase__echo"));

    // Methods: the schema first, then the backend; its topic published in order; who called it.
    assert_eq!(rig.platform.device_call("notes.add", json!({"text": "one"})).await.unwrap(), json!({"count": 1}));
    rig.platform.device_call("notes.add", json!({"text": "two"})).await.unwrap();
    let bad = rig.platform.device_call("notes.add", json!({"text": 3})).await.unwrap_err();
    assert_eq!(bad.code, codes::INVALID_PARAMS);
    let listed = rig.platform.device_call("notes.list", json!({})).await.unwrap();
    assert_eq!(listed["notes"][0]["by"], json!("device:d1"));
    eventually("both notes published", async || rig.platform.published.lock().unwrap().len() >= 2).await;
    let counts: Vec<_> = rig.platform.published.lock().unwrap().iter().map(|(_, v)| v["count"].clone()).collect();
    assert_eq!(counts, [json!(1), json!(2)]);
    // The value its subscription answered with reached `on-topic`; a later one is delivered.
    eventually("the first value", async || {
        rig.platform.device_call("state", json!({})).await.unwrap()["status_seen"] == json!(1)
    })
    .await;
    rig.control
        .deliver(TopicValue {
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
    assert_eq!(rig.platform.said("call "), ["call session.create", "call harnix.send"]);
    assert_eq!(rig.platform.device_call("overreach", json!({})).await.unwrap()["refused"], json!(true));
    assert_eq!(rig.status().await["calls"]["refused"][0]["method"], json!("harnix.apps.uninstall"));

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
    match rig.control.call_tool(tool("echo", json!({"text": "hi"}), "c1")).await.unwrap() {
        ToolReply::Output { structured, is_error, .. } => {
            assert_eq!(structured, Some(json!({"echoed": "hi"})));
            assert!(!is_error);
        }
        other => panic!("{other:?}"),
    }
    rig.control.call_tool(tool("slow", json!({"steps": 3, "ms": 10}), "c2")).await.unwrap();
    assert_eq!(*rig.platform.progress.lock().unwrap(), ["step 1 of 3", "step 2 of 3", "step 3 of 3"]);
    let control = rig.control.clone();
    let slow = tokio::spawn(async move { control.call_tool(tool("slow", json!({"steps": 100, "ms": 200}), "c3")).await });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    rig.control.cancel("c3".into()).await.unwrap();
    assert!(matches!(
        slow.await.unwrap().unwrap(),
        ToolReply::Failed { failure: Failure::Cancelled, .. }
    ));
    match rig.control.call_tool(tool("board", json!({}), "c4")).await.unwrap() {
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
    assert!(matches!(rig.control.hook(hook("app__example_showcase__vetoed")).await.unwrap(), HookReply::Block(_)));
    assert_eq!(rig.control.hook(hook("Bash")).await.unwrap(), HookReply::Proceed);

    // Stopped: its tools withdrawn, then its names, its subscription given back, its agent side off;
    // a method answers that it is not running.
    rig.control.stop(Stop { app: ID.into(), parts: parts.clone() }).await.unwrap();
    assert!(rig.platform.offers.lock().unwrap().last().unwrap().tools.is_empty());
    assert_eq!(rig.platform.said("withdraw"), ["withdraw example.showcase"]);
    assert_eq!(rig.platform.said("unsubscribe"), ["unsubscribe harnix.sessions.status"]);
    assert_eq!(rig.platform.said("switch_agent example-showcase off").len(), 1);
    assert_eq!(rig.platform.said("remove_skills"), ["remove_skills showcase-tour"]);
    let gone = rig.platform.device_call("notes.list", json!({})).await.unwrap_err();
    assert_eq!(gone.data.unwrap()["failure"], json!("stopped"));
    assert_eq!(rig.status().await, json!({"state": "stopped"}));
    // Taken back.
    let taken = rig.control.unplace(Unplace { app: ID.into(), parts }).await.unwrap();
    assert_eq!(taken["agent"]["package"], json!("example-showcase"));
    assert_eq!(rig.platform.said("uninstall_agent"), ["uninstall_agent example-showcase system"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn what_does_not_start_is_undone_and_said() {
    // Unsigned, where unsigned code does not run: refused before the agent side is switched on.
    let rig = Rig::new(|o| o.admission.dev = false);
    let version = rig.unpack(|m| m);
    let start = Start {
        app: ID.into(),
        version,
        app_dir: rig.app_dir(),
        parts: json!({}),
    };
    let err = rig.control.start(start.clone()).await.unwrap_err();
    assert!(err.message.contains("[apps] dev = true"), "{}", err.message);
    assert!(rig.platform.said("register").is_empty());
    assert_eq!(rig.status().await["state"], json!("failed"));
    rig.control.forget(ID.into()).await.unwrap();
    assert_eq!(rig.status().await["state"], json!("stopped"));

    // A tool row for a tool the component does not have: started, found out, and taken back.
    let rig = Rig::new(|_| {});
    let version = rig.unpack(|m| m.replace("tool = \"count\"", "tool = \"counted\""));
    let parts = json!({"agent": {"package": "example-showcase", "version": "0.1.0", "instances": ["system"]}});
    let err = rig
        .control
        .start(Start { app: ID.into(), version, app_dir: rig.app_dir(), parts })
        .await
        .unwrap_err();
    assert!(err.message.contains("does not list as a tool"), "{}", err.message);
    assert_eq!(rig.platform.said("withdraw"), ["withdraw example.showcase"]);
    assert_eq!(rig.platform.said("switch_agent example-showcase off").len(), 1);
    assert!(rig.platform.said("expose_tools").is_empty());

    // An instance that is not there: refused by name before anything reaches the agent engine.
    let rig = Rig::new(|_| {});
    let version = rig.unpack(|m| m.replace("install_into = [\"system\"]", "install_into = [\"nowhere\"]"));
    let err = rig
        .control
        .place(Place { app: ID.into(), app_dir: rig.app_dir(), version })
        .await
        .unwrap_err();
    assert!(err.message.contains("no such instance"), "{}", err.message);
    assert!(rig.platform.said("install_agent").is_empty());
}
