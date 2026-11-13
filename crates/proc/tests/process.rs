//! `harnix-testapp ext-proc`, the real process (built by a nested cargo, as `tests/sandbox.rs`
//! does): what it does with the descriptors it is handed, before anything else.

use std::io::{BufRead, BufReader};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use harnix_supervise::ChildSpec;
use serde_json::{json, Value};

const ID: &str = "example.showcase";

/// `harnix-testapp` built with `probe` (which has `proc`), copied out of the target directory so
/// that another build of it there does not change it under the test.
fn program(into: &Path) -> PathBuf {
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

/// What it exits with, started with `args` and the descriptors `inherit` (at 3, 4); what it says
/// goes to this test's stderr.
async fn started(program: &Path, args: &[&str], inherit: Vec<OwnedFd>) -> Option<i32> {
    let mut spec = ChildSpec::new(program).args(["ext-proc", "--ext", ID]).args(args.iter().copied());
    for (n, fd) in inherit.into_iter().enumerate() {
        spec = spec.inherit_fd(3 + n as i32, fd);
    }
    let mut child = spec.spawn().expect("it starts");
    let status = tokio::time::timeout(Duration::from_secs(30), child.wait()).await.expect("it ends").unwrap();
    status.code()
}

#[tokio::test(flavor = "multi_thread")]
async fn no_socket_at_the_wires_descriptor_ends_it_with_78() {
    let tmp = tempfile::tempdir().unwrap();
    let program = program(tmp.path());

    // Nothing at 3.
    let code = started(&program, &[], vec![]).await;
    assert_eq!(code, Some(78));

    // A file at 3.
    let file = std::fs::File::open(tmp.path().join("harnix-testapp")).unwrap();
    let code = started(&program, &[], vec![OwnedFd::from(file)]).await;
    assert_eq!(code, Some(78));

    // A socket at 3 and a file at 4, which it was told is its engine's.
    let (_ours, theirs) = UnixStream::pair().unwrap();
    let file = std::fs::File::open(tmp.path().join("harnix-testapp")).unwrap();
    let code = started(&program, &["--engine-fd", "4"], vec![OwnedFd::from(theirs), OwnedFd::from(file)]).await;
    assert_eq!(code, Some(78));
}

#[tokio::test(flavor = "multi_thread")]
async fn what_it_starts_is_not_handed_the_wire_or_the_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let program = program(tmp.path());
    let run = tmp.path().join("run");
    std::fs::create_dir_all(&run).unwrap();
    let (ours, theirs) = UnixStream::pair().unwrap();
    let (_engine, engine) = UnixStream::pair().unwrap();
    let mut child = ChildSpec::new(&program)
        .args(["ext-proc", "--ext", ID, "--probe", &json!({}).to_string(), "--engine-fd", "4"])
        .args(["--run".to_string(), run.display().to_string()])
        .inherit_fd(3, OwnedFd::from(theirs))
        .inherit_fd(4, OwnedFd::from(engine))
        .spawn()
        .expect("it starts");
    let answer = tokio::task::spawn_blocking(move || {
        ours.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        let mut line = String::new();
        BufReader::new(ours).read_line(&mut line).map(|_| line)
    })
    .await
    .unwrap()
    .expect("the probe answers on its wire");
    let _ = tokio::time::timeout(Duration::from_secs(30), child.wait()).await;
    let answer: Value = serde_json::from_str(&answer).unwrap_or_else(|e| panic!("{e}: {answer:?}"));
    let checks = answer["checks"].as_array().expect("checks");
    for what in ["the wire", "the engine's stream"] {
        let check = checks
            .iter()
            .find(|c| c["check"] == json!(format!("a child is not handed {what}")))
            .unwrap_or_else(|| panic!("no check for {what} among {checks:?}"));
        assert_eq!(check["ok"], json!(true), "{check}");
    }
}
