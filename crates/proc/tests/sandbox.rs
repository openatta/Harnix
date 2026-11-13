//! The sandbox, tried from inside: `harnix-testapp ext-proc --probe` (built with `probe` by a
//! nested cargo, as `crates/testapp/tests/ext_proc.rs` packs its component), started by
//! `harnix-supervise` with a real `SandboxSpec` over a layout like a state directory's, says what
//! it and the shells it starts could reach.
//!
//! macOS always runs it (`sandbox-exec` ships with the system). Linux runs it when Landlock and
//! Yama are there, and otherwise says it did not run, and why.

use std::io::{BufRead, BufReader};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use harnix_supervise::{ChildSpec, SandboxSpec};
use serde_json::{json, Value};

const ID: &str = "example.showcase";

/// `harnix-testapp` built with `probe`, copied out of the target directory so that another build
/// of it there does not change it under the test.
fn probe_program(into: &Path) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::process::Command::new(env!("CARGO"))
        .args(["build", "-q", "-p", "harnix-testapp", "--features", "probe", "--message-format=json"])
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

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Where things are, as a product lays them out.
struct Layout {
    state: PathBuf,
    engine: PathBuf,
}

impl Layout {
    fn new(root: &Path) -> Self {
        let (state, engine) = (root.join("state"), root.join("engine"));
        write(&state.join("token"), "device-token");
        write(&state.join("devices.json"), r#"{"devices": []}"#);
        // A daemon the product spawned: its data root holds the MCP server's bearer.
        write(&state.join("instances/flow.remote/settings.json"), r#"{"mcp": {"Authorization": "Bearer t"}}"#);
        write(&engine.join("settings.json"), r#"{"providers": {"anthropic": {"key": "k"}}}"#);
        write(&engine.join("sessions/s1.jsonl"), "{}\n");
        // This extension: its consent record, its package, its run directory.
        let ext = state.join("ext").join(ID);
        write(&ext.join("install.json"), r#"{"calls": []}"#);
        write(&ext.join("0.1.0/harnix-ext.toml"), "[app]\n");
        write(&ext.join("0.1.0/ui/index.html"), "<p>");
        for dir in ["data", "kv", "engine", "work", "home", "tmp"] {
            std::fs::create_dir_all(ext.join("run").join(dir)).unwrap();
        }
        // Another extension.
        let other = state.join("ext/example.other");
        write(&other.join("install.json"), r#"{"calls": []}"#);
        write(&other.join("run/data/notes.txt"), "theirs");
        write(&other.join("run/engine/settings.json"), r#"{"providers": {}}"#);
        Self { state, engine }
    }

    fn ext(&self) -> PathBuf {
        self.state.join("ext").join(ID)
    }
    fn package(&self) -> PathBuf {
        self.ext().join("0.1.0")
    }
    fn run(&self) -> PathBuf {
        self.ext().join("run")
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_extension_process_reaches_its_own_paths_and_nothing_else_of_the_state() {
    if let Err(why) = harnix_supervise::sandbox::available() {
        if cfg!(target_os = "macos") {
            panic!("macOS has `sandbox-exec`: {why}");
        }
        eprintln!("not run: {why}");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    // The sandbox compares real paths (`/var/folders/…` is `/private/var/folders/…`).
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let program = probe_program(&root);
    let layout = Layout::new(&root);

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || while listener.accept().is_ok() {});

    let other = layout.state.join("ext/example.other");
    let targets = json!({
        "unreadable": {
            "token": layout.state.join("token"),
            "devices.json": layout.state.join("devices.json"),
            "another extension's data": other.join("run/data"),
            "another extension's engine settings": other.join("run/engine/settings.json"),
            "another extension's install.json": other.join("install.json"),
            "the main engine root": layout.engine,
            "the daemon's MCP token": layout.state.join("instances/flow.remote/settings.json"),
            "its install.json": layout.ext().join("install.json"),
        },
        "unwritable": {
            "its package": layout.package(),
            "its install.json": layout.ext().join("install.json"),
        },
        "tcp": addr.to_string(),
    });

    let (ours, theirs) = UnixStream::pair().unwrap();
    let spec = SandboxSpec::new().read_write(layout.run()).read_only(layout.package());
    let mut child = ChildSpec::new(&program)
        .args(["ext-proc", "--ext", ID, "--wire-fd", "3", "--probe", &targets.to_string()])
        .args(["--pkg".to_string(), layout.package().display().to_string()])
        .args(["--run".to_string(), layout.run().display().to_string()])
        .inherit_fd(3, OwnedFd::from(theirs))
        .own_session()
        .sandbox(spec)
        .spawn()
        .expect("the probe starts in its sandbox");

    let answer = tokio::task::spawn_blocking(move || {
        ours.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        let mut line = String::new();
        BufReader::new(ours).read_line(&mut line).map(|_| line)
    })
    .await
    .unwrap()
    .expect("the probe answers on its wire");
    let status = tokio::time::timeout(Duration::from_secs(30), child.wait()).await.unwrap().unwrap();
    assert!(status.success(), "the probe exited {status}; it answered {answer:?}");

    let answer: Value = serde_json::from_str(&answer).unwrap_or_else(|e| panic!("{e}: {answer:?}"));
    let checks = answer["checks"].as_array().expect("checks");
    let failed: Vec<&Value> = checks.iter().filter(|c| c["ok"] != json!(true)).collect();
    assert!(failed.is_empty(), "failed: {}", serde_json::to_string_pretty(&failed).unwrap());

    let names: Vec<&str> = checks.iter().filter_map(|c| c["check"].as_str()).collect();
    let mut expected: Vec<String> = Vec::new();
    for what in targets["unreadable"].as_object().unwrap().keys() {
        expected.push(format!("cannot read {what}"));
        expected.push(format!("a child cannot read {what}"));
    }
    for what in targets["unwritable"].as_object().unwrap().keys() {
        expected.push(format!("cannot write {what}"));
        expected.push(format!("a child cannot write {what}"));
    }
    expected.extend(["can read its package", "can read and write run", "a child can write run", "its VM engine builds"].map(String::from));
    expected.extend(["data", "engine", "home", "kv", "tmp", "work"].map(|d| format!("can read and write run/{d}")));
    expected.push(format!("can reach {addr}"));
    for check in &expected {
        assert!(names.contains(&check.as_str()), "no `{check}` among {names:?}");
    }
    eprintln!("{}", serde_json::to_string_pretty(checks).unwrap());
}
