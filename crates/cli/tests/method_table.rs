//! The route table the Harnix app serves, pinned: every method, topic and route, who owns it and
//! who may reach it. `tests/live/api/method-table.spec.ts` reads the same file from
//! a running process.
//!
//! A change here is a change of what a client may call: `HARNIX_UPDATE_SNAPSHOTS=1` writes the
//! new table, and the commit says why.

use std::path::{Path, PathBuf};

use serde_json::Value;

fn snapshot_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../harnix/tests/method-table.snap.json")
}

/// What this binary assembles: the built-ins and the flow module, granted what `main` grants it.
fn the_harnix_app() -> harnix::AppBuilder {
    harnix::App::builder()
        .with_builtins()
        .extensions(harnix::first_party::standard_with_flow(harnix_ext_flow::Flow::builder().build()))
}

#[test]
fn the_harnix_apps_route_table_is_the_snapshot() {
    let actual = harnix::testing::describe(the_harnix_app()).expect("the Harnix app registers");
    let path = snapshot_path();
    if std::env::var("HARNIX_UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
        std::fs::write(&path, serde_json::to_string_pretty(&actual).unwrap() + "\n").unwrap();
        return;
    }
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(
        actual == kept,
        "{} differs; if the change is intended: HARNIX_UPDATE_SNAPSHOTS=1, and say why in the commit",
        path.display()
    );
}
