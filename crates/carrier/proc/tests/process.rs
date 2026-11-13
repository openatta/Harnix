//! Packages on the process carrier, on a real process (`harnix::testing::Kernel`): each package's
//! extension process is the test application (built by a nested cargo, with `proc`) re-executed
//! as `ext-proc`, started by the host and kept by the carrier.
//!
//! What is exercised is the product's side: the registration it makes before the process starts,
//! what it checks of the process — its build, what it registers — and how it keeps it: up again
//! after a crash, set aside after three, refused and ended when it is not what was consented to.
//! The packages have no component (nothing to compile); one's process runs another manifest than
//! the one installed, through a `proc_program` that hands it another package directory.
//!
//! One test, one process: the state directory is the process's environment. Its processes start
//! unsandboxed (`profile`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use harnix::{codes, Subject};
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

/// The manifest of an unsigned package `id` with a space, and `more` after it.
fn manifest(id: &str, more: &str) -> String {
    format!(
        "[app]\nid = \"{id}\"\nversion = \"1.0.0\"\nharnix_api = 2\npublisher = \"acme\"\ntitle = \"Board\"\n\n\
         [capabilities]\ncalls = [\"harnix.session.create\"]\n\n[[space]]\nid = \"{id}\"\n{more}"
    )
}

/// The package `id` as a directory, and packed.
fn package(dir: &Path, id: &str) -> (PathBuf, PathBuf) {
    let src = dir.join(format!("src-{id}"));
    std::fs::create_dir_all(src.join("ui")).unwrap();
    std::fs::write(src.join("harnix-ext.toml"), manifest(id, "")).unwrap();
    std::fs::write(src.join("ui/page.html"), "<!doctype html>").unwrap();
    let out = dir.join(format!("{id}.hxext"));
    harnix_package::package::zip_dir(&src, &out).unwrap();
    (src, out)
}

/// A program that runs `real`, but hands the process of `id` the package at `other` instead of
/// its own: a process that runs what was not installed.
fn swapping(dir: &Path, real: &Path, id: &str, other: &Path) -> PathBuf {
    let script = dir.join("swapping.sh");
    // `ext-proc --ext <id> --pkg <dir> --run <dir> --wire-fd 3`, as the host starts it.
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\npkg=\"$5\"\nif [ \"$3\" = \"{id}\" ]; then pkg=\"{}\"; fi\n\
             exec \"{}\" \"$1\" \"$2\" \"$3\" \"$4\" \"$pkg\" \"$6\" \"$7\" \"$8\" \"$9\"\n",
            other.display(),
            real.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn entry<'a>(list: &'a Value, id: &str) -> Option<&'a Value> {
    list["packages"].as_array()?.iter().find(|a| a["id"] == id)
}

fn alive(pid: i64) -> bool {
    // SAFETY: signal 0 only asks whether the pid exists.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

struct Product {
    kernel: harnix::testing::Kernel,
}

impl Product {
    async fn call(&self, method: &str, params: Value) -> Result<Value, harnix::RpcError> {
        self.kernel.call(Subject::device("d"), method, params).await
    }

    async fn runtime(&self, id: &str) -> Value {
        let list = self.call("packages.list", json!({})).await.unwrap();
        entry(&list, id).map(|e| e["runtime"].clone()).unwrap_or(Value::Null)
    }

    /// Its runtime, once `f` says it is what was waited for.
    async fn until(&self, id: &str, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let runtime = self.runtime(id).await;
            if f(&runtime) {
                return runtime;
            }
            assert!(Instant::now() < deadline, "never: {what}; last {runtime}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

async fn start(dir: &Path, app: harnix::AppBuilder, profile: &str) -> Product {
    std::env::set_var("HARNIX_STATE_DIR", dir.join("state"));
    let kernel = harnix::testing::Kernel::start_with(app, dir, Some(profile)).await.expect("the process starts");
    Product { kernel }
}

/// Unsandboxed: the swapping script starts a program, and hands a process a package, that no
/// sandbox would let it reach. A process in its sandbox is `tests/sandboxed.rs`.
fn profile(program: &Path) -> String {
    format!("[ext.packages]\ndev = true\ncarrier = \"process\"\nproc_program = \"{}\"\nsandbox = \"off\"\n", program.display())
}

#[tokio::test(flavor = "multi_thread")]
async fn packages_run_kept_and_checked_in_processes_of_their_own() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    // What an engine writes beside where it runs goes in the scratch directory, not the crate's.
    std::env::set_current_dir(&dir).unwrap();
    let real = testapp(&dir);
    let (_, board) = package(&dir, "acme.board");
    let (tampered_src, tampered) = package(&dir, "acme.tampered");
    // What the process of `acme.tampered` runs: one more space than was installed.
    let other = dir.join("other-acme.tampered");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("harnix-ext.toml"), manifest("acme.tampered", "\n[[space]]\nid = \"acme.tampered.more\"\n")).unwrap();
    std::fs::copy(tampered_src.join("ui/page.html"), other.join("page.html")).unwrap();
    let program = swapping(&dir, &real, "acme.tampered", &other);

    let app = harnix::App::builder().extensions(harnix::first_party::standard_with(harnix::first_party::packages()));
    let product = start(&dir, app, &profile(&program)).await;

    // Installed, registered by the product, and up in a process of its own.
    let installed = product.call("packages.installFile", json!({"path": board})).await.unwrap();
    assert_eq!(installed["trust"], "unsigned", "{installed}");
    let runtime = product.runtime("acme.board").await;
    assert_eq!((runtime["state"].as_str(), runtime["carrier"].as_str()), (Some("running"), Some("process")), "{runtime}");
    let pid = runtime["process"]["pid"].as_i64().expect("its pid");
    assert!(alive(pid));
    let extensions = product.call("harnix.extensions", json!({})).await.unwrap();
    let ext = extensions["extensions"].as_array().unwrap().iter().find(|e| e["id"] == "acme.board").expect("an extension");
    assert_eq!((ext["trust"].as_str(), ext["carrier"].as_str()), (Some("consent"), Some("process")), "{ext}");
    // Its data and its own directories are under `run/`; its record and its files are not its.
    let run = dir.join("state/ext/acme.board/run");
    assert!(run.join("home").is_dir() && run.join("tmp").is_dir(), "{}", run.display());
    let record: Value = serde_json::from_slice(&std::fs::read(dir.join("state/ext/acme.board/install.json")).unwrap()).unwrap();
    assert_eq!(record["digest"].as_str().map(str::len), Some(64), "{record}");

    // Killed: up again, on a new process, its rows never gone.
    // SAFETY: a signal to a process this test's product started.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    let again = product
        .until("acme.board", "up again on another pid", |r| r["state"] == "running" && r["process"]["pid"].as_i64().is_some_and(|p| p != pid))
        .await;
    assert_eq!(again["process"]["starts"], 2, "{again}");
    let list = product.call("packages.list", json!({})).await.unwrap();
    assert_eq!(entry(&list, "acme.board").unwrap()["registered"], true);

    // Three crashes in a row, each soon after it started: set aside, its rows kept and saying so.
    let second_pid = again["process"]["pid"].as_i64().unwrap();
    // SAFETY: as above.
    unsafe { libc::kill(second_pid as i32, libc::SIGKILL) };
    let third = product
        .until("acme.board", "up a third time", |r| r["state"] == "running" && r["process"]["pid"].as_i64().is_some_and(|p| p != second_pid))
        .await;
    let last = third["process"]["pid"].as_i64().unwrap();
    // SAFETY: as above.
    unsafe { libc::kill(last as i32, libc::SIGKILL) };
    let faulted = product.until("acme.board", "set aside", |r| r["state"] == "faulted").await;
    assert!(faulted["reason"].as_str().is_some_and(|r| r.contains("3 times") || r.contains("given up")), "{faulted}");
    let list = product.call("packages.list", json!({})).await.unwrap();
    assert_eq!(entry(&list, "acme.board").unwrap()["registered"], true, "its rows stay");
    // Switched off and on: it starts over.
    product.call("packages.disable", json!({"id": "acme.board"})).await.unwrap();
    assert_eq!(product.runtime("acme.board").await["state"], "stopped");
    product.call("packages.enable", json!({"id": "acme.board"})).await.unwrap();
    assert_eq!(product.runtime("acme.board").await["state"], "running");

    // A process that registers what was not installed — another manifest, one more space — is
    // refused and ended, and the install is undone.
    let e = product.call("packages.installFile", json!({"path": tampered})).await.unwrap_err();
    assert_eq!(e.code, codes::REFUSED, "{e:?}");
    assert!(e.message.contains("digest") || e.message.contains("acme.tampered.more"), "{}", e.message);
    assert!(!dir.join("state/ext/acme.tampered").exists());
    let list = product.call("packages.list", json!({})).await.unwrap();
    assert!(entry(&list, "acme.tampered").is_none(), "{list}");
    let swapped = std::process::Command::new("/usr/bin/pgrep").args(["-f", "ext acme.tampered"]).output().unwrap();
    assert!(String::from_utf8_lossy(&swapped.stdout).trim().is_empty(), "its process is still there");

    let running = product.runtime("acme.board").await["process"]["pid"].as_i64().unwrap();
    product.call("packages.uninstall", json!({"id": "acme.board"})).await.unwrap();
    assert!(!alive(running), "uninstalled, and its process still there");
    product.kernel.shutdown().await;

    // A product of another build: the process's hello names one Harnix, the product another, and
    // the process is refused before it is asked anything.
    let other_build = harnix_wasm_port::socket::Hello { version: "0.0.0-another".into(), ..harnix_proc::this_build() };
    let packages = harnix::first_party::Packages::new().carrier(harnix_carrier_proc::carrier(other_build));
    let app = harnix::App::builder().extensions(harnix::first_party::standard_with(packages));
    let second = dir.join("second");
    std::fs::create_dir_all(&second).unwrap();
    let product = start(&second, app, &profile(&real)).await;
    let e = product.call("packages.installFile", json!({"path": board})).await.unwrap_err();
    assert!(e.message.contains("another build") && e.message.contains("0.0.0-another"), "{}", e.message);
    assert!(!second.join("state/ext/acme.board").exists());
    product.kernel.shutdown().await;
}
