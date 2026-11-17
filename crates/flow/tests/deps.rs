//! The library is taken in whole by other hosts than Harnix (docs/embed.en.md), so what it links
//! is a promise too: nothing of Harnix but itself and nothing of Agix, anywhere in what it
//! compiles — its own dependencies, theirs, its dev-dependencies — and every version written in
//! its own manifest, none inherited from the workspace it happens to be built in. Read off
//! `cargo metadata` of whatever workspace holds the directory, so the check holds after a copy.

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use serde_json::Value;

const NAME: &str = "harnix-flow";

fn manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

/// Whether `path` (as cargo metadata gives it) is this library's manifest, links resolved.
fn is_this(path: &str) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canonical(Path::new(path)) == canonical(&manifest())
}

/// The platform this is built for, as rustc names it.
fn host() -> String {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let out = std::process::Command::new(rustc).arg("-vV").output().expect("rustc -vV runs");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.lines().find_map(|l| l.strip_prefix("host: ")).expect("rustc names its host").trim().to_string()
}

/// The resolve for this platform: offline, so only what is already downloaded is read. Every
/// other platform's dependencies are the manifest's own ([`its_direct_dependencies_are_from_a_registry`]).
fn metadata() -> Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--offline", "--filter-platform", &host(), "--manifest-path"])
        .arg(manifest())
        .output()
        .expect("cargo metadata runs");
    assert!(out.status.success(), "cargo metadata: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("cargo metadata's JSON")
}

/// Every package the library compiles, itself included: the resolve graph walked from it, over
/// every kind of dependency.
fn closure(meta: &Value) -> Vec<&Value> {
    let packages = meta["packages"].as_array().expect("packages");
    let by_id = |id: &str| packages.iter().find(|p| p["id"] == id).expect("a resolved package is listed");
    let nodes = meta["resolve"]["nodes"].as_array().expect("resolve.nodes");
    let root = packages
        .iter()
        .find(|p| p["name"] == NAME && is_this(p["manifest_path"].as_str().unwrap_or_default()))
        .expect("the library is in its workspace");
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([root["id"].as_str().unwrap().to_string()]);
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let node = nodes.iter().find(|n| n["id"] == id.as_str()).expect("a node per package");
        for dep in node["deps"].as_array().into_iter().flatten() {
            queue.push_back(dep["pkg"].as_str().unwrap().to_string());
        }
    }
    seen.iter().map(|id| by_id(id)).collect()
}

#[test]
fn it_links_nothing_of_harnix_or_agix() {
    let meta = metadata();
    let mut offenders = Vec::new();
    for package in closure(&meta) {
        let name = package["name"].as_str().unwrap_or_default();
        let path = package["manifest_path"].as_str().unwrap_or_default();
        if name == NAME && is_this(path) {
            continue;
        }
        if name == "harnix" || name.starts_with("harnix-") {
            offenders.push(format!("{name}: Harnix's ({path})"));
        } else if name == "agix" || name.starts_with("agix-") {
            offenders.push(format!("{name}: Agix's ({path})"));
        } else if package["source"].is_null() {
            // Not from a registry: a crate of the workspace or a path beside it, which a host
            // copying the directory does not get.
            offenders.push(format!("{name}: a local crate ({path})"));
        }
    }
    assert!(offenders.is_empty(), "the flow library compiles:\n{}", offenders.join("\n"));
}

/// Its own dependencies, for every platform and of every kind: none by path, none of Harnix or
/// Agix.
#[test]
fn its_direct_dependencies_are_from_a_registry() {
    let meta = metadata();
    let packages = meta["packages"].as_array().expect("packages");
    let me = packages
        .iter()
        .find(|p| p["name"] == NAME && is_this(p["manifest_path"].as_str().unwrap_or_default()))
        .expect("the library is in its workspace");
    let mut offenders = Vec::new();
    for dep in me["dependencies"].as_array().expect("dependencies") {
        let name = dep["name"].as_str().unwrap_or_default();
        if !dep["path"].is_null() || name.starts_with("harnix") || name.starts_with("agix") {
            offenders.push(format!("{name} ({})", dep["path"].as_str().unwrap_or("a registry")));
        }
    }
    assert!(offenders.is_empty(), "the flow library depends on:\n{}", offenders.join("\n"));
}

#[test]
fn every_version_is_its_own() {
    let text = std::fs::read_to_string(manifest()).expect("the manifest");
    let inherited: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter(|l| l.replace(' ', "").contains("workspace=true") || l.replace(' ', "").contains(".workspace="))
        .collect();
    assert!(inherited.is_empty(), "inherited from the workspace:\n{}", inherited.join("\n"));
}
