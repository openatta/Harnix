//! A child started with a sandbox: on macOS inside `sandbox-exec`, on Linux handed its spec to
//! apply itself. What a real extension process can reach under it is `crates/proc/tests/sandbox.rs`.

use std::time::Duration;

use harnix_supervise::{ChildSpec, SandboxSpec};

/// What `/bin/sh -c <script>` wrote to `out` in `dir`, run under `spec`.
async fn run(spec: SandboxSpec, script: &str, out: &std::path::Path) -> String {
    let mut child = ChildSpec::new("/bin/sh").args(["-c", script]).sandbox(spec).spawn().expect("it starts sandboxed");
    tokio::time::timeout(Duration::from_secs(10), child.wait()).await.unwrap().unwrap();
    std::fs::read_to_string(out).unwrap_or_default()
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_sandboxed_child_reaches_its_own_directory_and_not_the_one_beside_it() {
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

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_child_on_linux_is_handed_its_spec_to_apply() {
    if let Err(why) = harnix_supervise::sandbox::available() {
        eprintln!("not run: {why}");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let script = format!("printf '%s' \"${}\" > {}", harnix_supervise::sandbox::ENV, out.display());
    let spec = SandboxSpec::new().read_write(tmp.path());
    let said = run(spec.clone(), &script, &out).await;
    assert_eq!(said, spec.to_env(&std::fs::canonicalize("/bin/sh").unwrap()).unwrap());
}

#[tokio::test]
async fn a_relative_sandbox_path_is_refused_before_anything_starts() {
    let e = ChildSpec::new("/bin/sh").args(["-c", "true"]).sandbox(SandboxSpec::new().read_only("pkg")).spawn().err().unwrap();
    assert!(e.to_string().contains("not absolute"), "{e}");
}
