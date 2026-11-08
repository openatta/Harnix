//! Loading: an artifact is accepted only by an engine it was compiled for.
#![cfg(feature = "compile")]

mod common;

use common::*;
use harnix_wasm::*;

#[test]
fn an_artifact_loads_on_its_own_engine_and_reports_its_world() {
    assert_eq!(component("tools-probe").world(), World::Tools);
    assert_eq!(component("backend-probe").world(), World::Backend);
    // And on another engine of the same build: the fingerprint is the
    // build's, not the engine instance's.
    let other = WasmEngine::new().unwrap();
    assert_eq!(other.fingerprint(), engine().fingerprint());
    let artifact = fixture_artifact("tools-probe");
    // SAFETY: compiled by this process.
    unsafe { other.load(&artifact, other.fingerprint()) }.unwrap();
}

#[test]
fn an_artifact_labelled_for_another_engine_is_refused() {
    let artifact = fixture_artifact("tools-probe");
    let label = "wasmtime-46.0.0+aarch64-apple-darwin+0123456789ab";
    // SAFETY: refused before deserialization.
    let err = unsafe { engine().load(&artifact, label) }
        .err()
        .unwrap()
        .to_string();
    assert!(
        err.contains(label) && err.contains(engine().fingerprint()),
        "{err}"
    );
}

/// The label is a claim; wasmtime checks the artifact's own record of the
/// configuration it was compiled under, so a mislabelled one fails too.
#[test]
fn an_artifact_compiled_under_another_configuration_is_refused_whatever_its_label() {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    // Without epoch interruption: code that could never be stopped.
    config.epoch_interruption(false);
    let foreign = wasmtime::Engine::new(&config).unwrap();
    let artifact = wasmtime::component::Component::new(&foreign, fixture_wasm("tools-probe"))
        .unwrap()
        .serialize()
        .unwrap();
    // SAFETY: compiled by this process; the point is that it is refused.
    let err = unsafe { engine().load(&artifact, engine().fingerprint()) }
        .err()
        .unwrap();
    assert!(format!("{err:#}").contains("epoch"), "{err:#}");
}

#[test]
fn a_component_that_was_not_precompiled_is_refused() {
    let wasm = fixture_wasm("tools-probe");
    // SAFETY: refused before deserialization.
    let err = unsafe { engine().load(&wasm, engine().fingerprint()) }
        .err()
        .unwrap();
    assert!(
        err.to_string().contains("not a precompiled component"),
        "{err}"
    );
}

#[test]
fn compiling_refuses_what_is_not_a_harnix_app_component() {
    assert!(
        engine().compile(b"\0asm\x01\0\0\0").is_err(),
        "a core module is not a component"
    );
    assert!(engine().compile(b"not wasm at all").is_err());
}
