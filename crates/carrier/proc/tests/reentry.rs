//! A backend on the process carrier that calls back into itself from a timer — no call of the
//! product's waiting on it — is refused in its process at once, through the real product: the
//! product starts the chain with the process, held on the firing the process says it answers, and
//! the process knows that firing as one its backend is still answering.
//!
//! The showcase (`examples/packages/showcase`), packed by the author's tool and installed unsigned
//! (`dev`), runs in the test application built with `proc` and re-executed as `ext-proc`, as in
//! `tests/process.rs`, unsandboxed: what the sandbox does is `tests/sandboxed.rs`. One test, one
//! process: the state directory is the process's environment.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use harnix::Subject;
use serde_json::{json, Value};

const SHOWCASE: &str = "example.showcase";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Run `cargo` with `args` in `dir`, or fail saying what did not build.
fn cargo(dir: &Path, args: &[&str], env: &[(&str, &Path)]) -> String {
    let mut command = std::process::Command::new(env!("CARGO"));
    command.args(args).current_dir(dir);
    for (k, v) in env {
        command.env(k, v);
    }
    let out = command.output().expect("cargo runs");
    assert!(out.status.success(), "cargo {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// `harnix-testapp` built with `proc`, copied out of the target directory so that another build
/// of it there does not change it under the test.
fn testapp(into: &Path) -> PathBuf {
    let out = cargo(&root(), &["build", "-q", "-p", "harnix-testapp", "--features", "proc", "--message-format=json"], &[]);
    let built = out
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|m| m["target"]["name"] == "harnix-testapp")
        .find_map(|m| m["executable"].as_str().map(PathBuf::from))
        .expect("cargo names the binary it built");
    let program = into.join("harnix-testapp");
    std::fs::copy(&built, &program).unwrap();
    program
}

/// The showcase, its component compiled and packed by the author's tool, unsigned.
fn showcase(into: &Path) -> PathBuf {
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("showcase");
    cargo(&root().join("examples/packages/showcase/component"), &["build", "--release", "--target", "wasm32-wasip2"], &[(
        "CARGO_TARGET_DIR",
        &target,
    )]);
    let wasm = target.join("wasm32-wasip2/release/showcase_tools.wasm");
    let out = into.join("showcase.hxext");
    let (wasm, out_s) = (wasm.to_str().unwrap(), out.to_str().unwrap());
    cargo(
        &root(),
        &[
            "run", "-q", "-p", "harnix-package", "--features", "compile", "--bin", "harnix-pack", "--", "pack",
            "examples/packages/showcase", "--wasm", wasm, "-o", out_s,
        ],
        &[],
    );
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn a_timer_of_a_process_that_calls_back_into_its_own_backend_is_refused_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    // What an engine writes beside where it runs goes in the scratch directory, not the crate's.
    std::env::set_current_dir(&dir).unwrap();
    let program = testapp(&dir);
    let package = showcase(&dir);

    std::env::set_var("HARNIX_STATE_DIR", dir.join("state"));
    let profile = format!("[ext.packages]\ndev = true\ncarrier = \"process\"\nproc_program = \"{}\"\nsandbox = \"off\"\n", program.display());
    let app = harnix::App::builder().extensions(harnix::first_party::standard_with(harnix::first_party::packages()));
    let kernel = harnix::testing::Kernel::start_with(app, &dir, Some(&profile)).await.expect("the process starts");
    let call = async |method: &str, params: Value| kernel.call(Subject::device("d"), method, params).await;

    call("packages.installFile", json!({"path": package})).await.unwrap();
    let state = format!("{SHOWCASE}.state");
    let armed = call(&format!("{SHOWCASE}.recall"), json!({"later": true})).await.unwrap();
    assert_eq!(armed["armed"], json!(true), "{armed}");
    let deadline = Instant::now() + Duration::from_secs(30);
    let recalled = loop {
        let recalled = call(&state, json!({})).await.unwrap()["recalled"].clone();
        if !recalled.is_null() {
            break recalled;
        }
        assert!(Instant::now() < deadline, "the timer never called back");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    // Refused in the process, as re-entering its backend, well before any timeout would have.
    assert_eq!(recalled["refused"], json!(true), "{recalled}");
    assert!(recalled["message"].as_str().unwrap().contains("re-enter"), "{recalled}");
    assert!(recalled["took_ms"].as_u64().unwrap() < 100, "{recalled}");
    // Its backend answers as it did: not recycled.
    let list = call("packages.list", json!({})).await.unwrap();
    let runtime = list["packages"].as_array().unwrap().iter().find(|p| p["id"] == SHOWCASE).unwrap()["runtime"].clone();
    assert_eq!(runtime["carrier"], json!("process"), "{runtime}");
    assert_eq!(runtime["health"]["recycles"], json!(0), "{runtime}");

    call("packages.uninstall", json!({"id": SHOWCASE})).await.unwrap();
    kernel.shutdown().await;
}
