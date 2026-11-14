//! A package on a real process (`harnix::testing::Kernel`), carried in this process: installed
//! from a file, registered as an extension by the component's registrar, switched off and on,
//! taken out — and one whose consent lists a first-party power, or whose agent side names an
//! instance there is none of, refused with nothing left behind.
//!
//! The packages here have no component (nothing to compile): what is exercised is the
//! component's records and methods and the host path a package takes, not its VM.

use std::path::{Path, PathBuf};

use harnix::{caps, codes, Subject};
use serde_json::{json, Value};

/// An unsigned package `id`, its consent listing `calls`, with a space and an interface file, and
/// `more` at the end of its manifest.
fn package(dir: &Path, id: &str, calls: &[&str], more: &str) -> PathBuf {
    let src = dir.join(format!("src-{id}"));
    std::fs::create_dir_all(src.join("ui")).unwrap();
    let calls = calls.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>().join(", ");
    std::fs::write(
        src.join("harnix-ext.toml"),
        format!(
            "[app]\nid = \"{id}\"\nversion = \"1.0.0\"\nharnix_api = 2\npublisher = \"acme\"\ntitle = \"Board\"\n\n\
             [capabilities]\ncalls = [{calls}]\n\n[[space]]\nid = \"{id}\"\n{more}"
        ),
    )
    .unwrap();
    std::fs::write(src.join("ui/page.html"), "<!doctype html>").unwrap();
    let out = dir.join(format!("{id}.hxext"));
    harnix_package::package::zip_dir(&src, &out).unwrap();
    out
}

fn listed<'a>(list: &'a Value, id: &str) -> Option<&'a Value> {
    list["packages"].as_array()?.iter().find(|a| a["id"] == id)
}

#[tokio::test]
async fn a_package_is_installed_registered_switched_and_taken_out() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    std::env::set_var("HARNIX_STATE_DIR", dir.path().join("state"));
    let app = harnix::App::builder()
        .extensions(harnix::first_party::standard_with(harnix::first_party::packages()));
    let kernel = harnix::testing::Kernel::start_with(app, dir.path(), Some("[ext.packages]\ndev = true\n"))
        .await
        .expect("the process starts");
    let status = kernel.extensions();
    let packages = status.iter().find(|s| s.id == "packages").expect("linked");
    assert!(packages.enabled, "{:?}", packages.reason);
    assert!(packages.granted.contains(&caps::EXTENSIONS_MANAGE.to_string()));
    let device = Subject::device("d");
    let call = |method: &'static str, params: Value| {
        let kernel = &kernel;
        let device = device.clone();
        async move { kernel.call(device, method, params).await }
    };

    // Installed from a file, unsigned (the profile's `dev`), and running: an extension like any.
    let board = package(dir.path(), "acme.board", &["harnix.session.create"], "");
    let installed = call("packages.installFile", json!({"path": board})).await.unwrap();
    assert_eq!((installed["id"].as_str(), installed["trust"].as_str()), (Some("acme.board"), Some("unsigned")));
    let list = call("packages.list", json!({})).await.unwrap();
    let entry = listed(&list, "acme.board").expect("listed");
    assert_eq!((entry["enabled"].as_bool(), entry["registered"].as_bool()), (Some(true), Some(true)), "{entry}");
    assert_eq!(list["runtime"]["carrier"], "wasm");
    let extensions = call("harnix.extensions", json!({})).await.unwrap();
    let ext = extensions["extensions"].as_array().unwrap().iter().find(|e| e["id"] == "acme.board").expect("an extension");
    assert_eq!((ext["trust"].as_str(), ext["carrier"].as_str(), ext["by"].as_str()), (Some("consent"), Some("wasm"), Some("packages")));
    let record = dir.path().join("state/ext/acme.board/install.json");
    assert!(record.exists() && dir.path().join("state/ext/acme.board/1.0.0/harnix-ext.toml").exists());

    // Off: withdrawn; on again: registered again. Twice is refused.
    call("packages.disable", json!({"id": "acme.board"})).await.unwrap();
    let list = call("packages.list", json!({})).await.unwrap();
    assert_eq!(listed(&list, "acme.board").unwrap()["registered"], false);
    call("packages.enable", json!({"id": "acme.board"})).await.unwrap();
    let list = call("packages.list", json!({})).await.unwrap();
    assert_eq!(listed(&list, "acme.board").unwrap()["registered"], true);
    let e = call("packages.installFile", json!({"path": board})).await.unwrap_err();
    assert_eq!(e.code, codes::REFUSED);

    // A consent that lists the system space's power: refused at registration, and nothing of it
    // stays — no record, no directory, no extension.
    let borrower = package(dir.path(), "acme.borrow", &["system.compact"], "");
    let e = call("packages.installFile", json!({"path": borrower})).await.unwrap_err();
    assert_eq!(e.code, codes::REFUSED);
    assert!(e.message.contains("system.compact"), "{}", e.message);
    assert!(!dir.path().join("state/ext/acme.borrow").exists());
    assert!(listed(&call("packages.list", json!({})).await.unwrap(), "acme.borrow").is_none());

    // Its agent side named for an instance there is none of: refused on install, on either
    // carrier, before anything of it is unpacked.
    let nowhere = package(dir.path(), "acme.nowhere", &[], "\n[agent]\ninstall_into = [\"nowhere\"]\n");
    let e = call("packages.installFile", json!({"path": nowhere})).await.unwrap_err();
    assert_eq!(e.code, codes::REFUSED);
    assert!(e.message.contains("`nowhere`, and there is no such instance"), "{}", e.message);
    assert!(!dir.path().join("state/ext/acme.nowhere").exists());

    // Taken out: its directory, its record, its registration.
    let gone = call("packages.uninstall", json!({"id": "acme.board"})).await.unwrap();
    assert_eq!(gone["uninstalled"], true);
    assert!(!dir.path().join("state/ext/acme.board").exists());
    let list = call("packages.list", json!({})).await.unwrap();
    assert!(listed(&list, "acme.board").is_none());
    assert!(list["registered"].as_array().unwrap().is_empty(), "{list}");
    kernel.shutdown().await;
}
