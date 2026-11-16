//! The Agix this build links, as constants: its version and the protocol its daemon speaks.
//!
//! `harnix --version` and `proc.hello` need them before any engine is up, so they are read from
//! the `agix/` submodule's sources here rather than asked of an engine. At run time an engine says
//! it itself, in `daemon.ping` (`EngineControl::identity`); a test holds the two together.

use std::path::Path;

fn main() {
    let agix = Path::new(&std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR")).join("../../agix");
    let manifest = agix.join("Cargo.toml");
    let discovery = agix.join("crates/assembly/src/discovery.rs");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", discovery.display());

    let manifest_text = read(&manifest);
    let version = section(&manifest_text, "[workspace.package]")
        .lines()
        .find_map(|l| value_of(l, "version"))
        .unwrap_or_else(|| panic!("no [workspace.package] version in {}", manifest.display()));
    let discovery_text = read(&discovery);
    let protocol = discovery_text
        .lines()
        .find_map(|l| l.trim().strip_prefix("pub const INSTANCE_PROTOCOL_VERSION: u32 ="))
        .and_then(|rest| rest.trim().trim_end_matches(';').trim().parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no INSTANCE_PROTOCOL_VERSION in {}", discovery.display()));
    println!("cargo:rustc-env=HARNIX_AGIX_VERSION={version}");
    println!("cargo:rustc-env=HARNIX_AGIX_PROTOCOL={protocol}");
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {} (is the agix submodule checked out?): {e}", path.display()))
}

/// The lines of `header`'s table, up to the next one.
fn section<'a>(text: &'a str, header: &str) -> &'a str {
    let Some(start) = text.find(header) else { return "" };
    let body = &text[start + header.len()..];
    let end = body.find("\n[").unwrap_or(body.len());
    &body[..end]
}

/// `key = "value"` on one line.
fn value_of(line: &str, key: &str) -> Option<String> {
    let rest = line.trim().strip_prefix(key)?.trim_start().strip_prefix('=')?;
    Some(rest.trim().trim_matches('"').to_string())
}
