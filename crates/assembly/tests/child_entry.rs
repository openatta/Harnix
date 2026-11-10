//! A subcommand this binary is re-executed as (`<product> ext-proc …`) runs its entry before the
//! product does anything: no profile read, no state lock taken, no module assembled.

use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use harnix_assembly::App;
use harnix_module::{Manifest, MethodRegistry, Module};

static ENTERED: Mutex<Vec<Vec<OsString>>> = Mutex::new(Vec::new());
static ASSEMBLED: AtomicBool = AtomicBool::new(false);

fn ext_proc(args: Vec<OsString>) -> anyhow::Result<()> {
    ENTERED.lock().unwrap().push(args);
    Ok(())
}

fn failing(_args: Vec<OsString>) -> anyhow::Result<()> {
    anyhow::bail!("the entry's own error")
}

/// A module that says when it is asked what it is: what assembling the product would do first.
struct Watched;

#[async_trait::async_trait]
impl Module for Watched {
    const API_VERSION: u32 = harnix_module::MODULE_API_VERSION;

    fn manifest(&self) -> Manifest {
        ASSEMBLED.store(true, Ordering::SeqCst);
        Manifest::new("watched", "0.0.1")
    }

    fn methods(&self, _methods: &mut MethodRegistry) {
        ASSEMBLED.store(true, Ordering::SeqCst);
    }
}

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

#[test]
fn ext_proc_runs_its_entry_and_nothing_of_the_product() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    // Where the product would take its lock and read its token: never made.
    std::env::set_var("HARNIX_STATE_DIR", &state);
    // A profile that is not there: a product that read it would fail to start.
    let missing = dir.path().join("missing.toml");
    let missing = missing.to_str().unwrap();
    App::builder()
        .with_builtins()
        .module(Watched)
        .child_entry("ext-proc", ext_proc)
        .args(args(&["product", "ext-proc", "--ext", "a.b", "--wire-fd", "3", "--profile", missing]))
        .run()
        .unwrap();
    assert_eq!(*ENTERED.lock().unwrap(), [args(&["--ext", "a.b", "--wire-fd", "3", "--profile", missing])]);
    assert!(!ASSEMBLED.load(Ordering::SeqCst), "a module was asked what it is");
    assert!(!state.exists(), "the state directory was touched");
}

#[test]
fn the_entrys_error_is_the_processs() {
    let e = App::builder()
        .child_entry("ext-proc", failing)
        .args(args(&["product", "ext-proc"]))
        .run()
        .unwrap_err();
    assert_eq!(e.to_string(), "the entry's own error");
}

#[test]
fn a_build_without_extension_processes_says_so() {
    let e = App::builder().args(args(&["product", "ext-proc", "--ext", "a.b"])).run().unwrap_err();
    assert!(e.to_string().contains("this build has no extension processes"), "{e}");
}

#[test]
#[should_panic(expected = "is a subcommand of the product itself")]
fn an_entry_cannot_take_the_products_own_subcommand() {
    let _ = App::builder().child_entry("daemon", ext_proc);
}
