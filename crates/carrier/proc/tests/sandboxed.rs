//! A package on the process carrier, in its sandbox: the host starts the test application (built
//! by a nested cargo, with `proc`) as the package's extension process, inside the operating
//! system's sandbox, and the list says so. What the sandbox lets the process reach is
//! `crates/proc/tests/sandbox.rs`; here, that a process started the way a product starts one is
//! in it, and still comes up and answers.
//!
//! macOS always runs it. Linux runs it when Landlock and Yama are there, and otherwise checks that
//! no process starts and the list says why.

use std::path::{Path, PathBuf};

use harnix::Subject;
use serde_json::{json, Value};

/// `harnix-testapp` built with `proc`, copied out of the target directory so that another build
/// of it there does not change it under the test.
fn testapp(into: &Path) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::process::Command::new(env!("CARGO"))
        .args(["build", "-q", "-p", "harnix-testapp", "--features", "proc", "--message-format=json"])
        .current_dir(&root)
        .output()
        .expect("cargo runs");
    assert!(out.status.success(), "cargo build: {}", String::from_utf8_lossy(&out.stderr));
    let built = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|m| m["target"]["name"] == "harnix-testapp")
        .find_map(|m| m["executable"].as_str().map(PathBuf::from))
        .expect("cargo names the binary it built");
    let program = into.join("harnix-testapp");
    std::fs::copy(&built, &program).unwrap();
    program
}

/// An unsigned package `id`, with a space and an interface, packed.
fn package(dir: &Path, id: &str) -> PathBuf {
    let src = dir.join(format!("src-{id}"));
    std::fs::create_dir_all(src.join("ui")).unwrap();
    let manifest = format!(
        "[app]\nid = \"{id}\"\nversion = \"1.0.0\"\nharnix_api = 2\npublisher = \"acme\"\ntitle = \"Board\"\n\n\
         [capabilities]\ncalls = [\"harnix.session.create\"]\n\n[[space]]\nid = \"{id}\"\n"
    );
    std::fs::write(src.join("harnix-ext.toml"), manifest).unwrap();
    std::fs::write(src.join("ui/page.html"), "<!doctype html>").unwrap();
    let out = dir.join(format!("{id}.hxext"));
    harnix_package::package::zip_dir(&src, &out).unwrap();
    out
}

fn entry<'a>(list: &'a Value, id: &str) -> Option<&'a Value> {
    list["packages"].as_array()?.iter().find(|a| a["id"] == id)
}

/// Whether `pid` runs in a sandbox (macOS's own question of it).
#[cfg(target_os = "macos")]
fn sandboxed(pid: i64) -> bool {
    extern "C" {
        fn sandbox_check(pid: libc::pid_t, operation: *const libc::c_char, kind: libc::c_int, ...) -> libc::c_int;
    }
    // SAFETY: with no operation and no filter, the call only asks whether the process has a sandbox.
    unsafe { sandbox_check(pid as libc::pid_t, std::ptr::null(), 0) == 1 }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_package_process_starts_in_its_sandbox() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    std::env::set_current_dir(&dir).unwrap();
    let program = testapp(&dir);
    let board = package(&dir, "acme.board");

    std::env::set_var("HARNIX_STATE_DIR", dir.join("state"));
    let profile = format!("[ext.packages]\ndev = true\ncarrier = \"process\"\nproc_program = \"{}\"\n", program.display());
    let app = harnix::App::builder().extensions(harnix::first_party::standard_with(harnix::first_party::packages()));
    let kernel = harnix::testing::Kernel::start_with(app, &dir, Some(&profile)).await.expect("the process starts");
    let call = |method: &'static str, params: Value| kernel.call(Subject::device("d"), method, params);

    let sandbox = call("packages.list", json!({})).await.unwrap()["runtime"]["sandbox"].clone();
    if sandbox["mode"] == "unavailable" {
        assert!(!cfg!(target_os = "macos"), "macOS has `sandbox-exec`: {sandbox}");
        eprintln!("not run: {}", sandbox["why"]);
        let e = call("packages.installFile", json!({"path": board})).await.unwrap_err();
        assert!(e.message.contains("cannot be sandboxed"), "{}", e.message);
        kernel.shutdown().await;
        return;
    }
    assert_eq!(sandbox["mode"], "on", "{sandbox}");

    call("packages.installFile", json!({"path": board})).await.unwrap();
    let list = call("packages.list", json!({})).await.unwrap();
    let board = entry(&list, "acme.board").expect("its entry");
    assert_eq!((board["runtime"]["state"].as_str(), board["sandbox"].as_str()), (Some("running"), Some("on")), "{board}");
    let pid = board["runtime"]["process"]["pid"].as_i64().expect("its pid");
    #[cfg(target_os = "macos")]
    assert!(sandboxed(pid), "its process {pid} is not in a sandbox");
    let _ = pid;
    kernel.shutdown().await;
}
