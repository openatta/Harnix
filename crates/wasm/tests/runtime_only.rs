//! The product's build loads what the store's compiler produced.
//!
//! The product links no compiler and the store's build tool does, and an
//! artifact is accepted only by an engine with the same fingerprint and the
//! same configuration record. Nothing short of the real pair shows that the
//! two builds agree, so this runs in a build *without* `compile`, has
//! `harnix-wasm-compile` (built with it) compile a fixture, and loads the
//! result.
#![cfg(not(feature = "compile"))]

mod common;

use common::*;
use harnix_wasm::*;
use serde_json::json;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn an_artifact_from_the_store_compiler_runs_in_a_build_without_one() {
    let wasm = fixture_wasm("tools-probe");
    let dir = tempfile::tempdir().unwrap();
    let (input, output) = (
        dir.path().join("probe.wasm"),
        dir.path().join("probe.cwasm"),
    );
    std::fs::write(&input, wasm).unwrap();

    let run = std::process::Command::new(env!("CARGO"))
        .args([
            "run",
            "-q",
            "-p",
            "harnix-wasm",
            "--features",
            "compile",
            "--bin",
            "harnix-wasm-compile",
            "--",
        ])
        .arg(&input)
        .arg(&output)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo should be runnable");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let compiled_for = String::from_utf8(run.stdout).unwrap().trim().to_string();

    let engine = WasmEngine::new().unwrap();
    assert_eq!(
        compiled_for,
        engine.fingerprint(),
        "the compiler and the product disagree on the fingerprint"
    );
    let artifact = std::fs::read(&output).unwrap();
    // SAFETY: produced a moment ago by this crate's own build tool.
    let component = unsafe { engine.load(&artifact, &compiled_for) }.unwrap();

    let services = Services {
        host: Arc::new(MockHost::default()),
        kv: Arc::new(MemoryKv::new()),
        http: Arc::new(NoNetwork),
    };
    let vm = AppVm::start(
        &engine,
        &component,
        spec(Capabilities::default(), limits(5_000), dir.path()),
        services,
    )
    .await
    .unwrap();
    let out = vm
        .call_tool(
            "echo",
            &json!({"text": "loaded"}),
            "c",
            None,
            None,
            &Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(out.content, "loaded");
}
