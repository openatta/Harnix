//! A child started with a sandbox: on macOS inside `sandbox-exec`, on Linux under Landlock applied
//! between fork and exec. What a real extension process can reach under it is
//! `crates/proc/tests/sandbox.rs`.

use std::time::Duration;

use harnix_supervise::{ChildSpec, SandboxSpec};

/// What `/bin/sh -c <script>` wrote to `out` in `dir`, run under `spec`.
async fn run(spec: SandboxSpec, script: &str, out: &std::path::Path) -> String {
    let mut child = ChildSpec::new("/bin/sh").args(["-c", script]).sandbox(spec).spawn().expect("it starts sandboxed");
    tokio::time::timeout(Duration::from_secs(10), child.wait()).await.unwrap().unwrap();
    std::fs::read_to_string(out).unwrap_or_default()
}

/// macOS always runs it; Linux when Landlock and Yama are there, and otherwise says it did not.
#[tokio::test]
async fn a_sandboxed_child_reaches_its_own_directory_and_not_the_one_beside_it() {
    if let Err(why) = harnix_supervise::sandbox::available() {
        assert!(!cfg!(target_os = "macos"), "macOS has `sandbox-exec`: {why}");
        eprintln!("not run: {why}");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let (own, beside) = (root.join("own"), root.join("beside"));
    std::fs::create_dir_all(&own).unwrap();
    std::fs::create_dir_all(&beside).unwrap();
    std::fs::write(beside.join("secret"), "s").unwrap();
    let out = own.join("out");
    let script = format!(
        "cat {secret} >/dev/null 2>&1 && echo read >> {out} || echo refused >> {out}; : > {beside}/x 2>/dev/null && echo wrote >> {out} || echo refused >> {out}",
        secret = beside.join("secret").display(),
        beside = beside.display(),
        out = out.display(),
    );
    assert_eq!(run(SandboxSpec::new().read_write(&own), &script, &out).await, "refused\nrefused\n");
    assert!(!beside.join("x").exists());
}

#[tokio::test]
async fn a_relative_sandbox_path_is_refused_before_anything_starts() {
    let e = ChildSpec::new("/bin/sh").args(["-c", "true"]).sandbox(SandboxSpec::new().read_only("pkg")).spawn().err().unwrap();
    assert!(e.to_string().contains("not absolute"), "{e}");
}
