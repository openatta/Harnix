//! An extension package as it is distributed and trusted: the layer every kind of host shares.
//!
//! A package is a package (`.hxext`, [`package`]): a manifest ([`manifest`]: its
//! declarations, its interface's [`surface`]s, its [`inline`] points), an optional WebAssembly
//! component, an optional agent package (`agent/`) and an optional interface (`ui/`). Where one
//! comes from and how far it is trusted: [`signature`] (who signed it, over which bytes),
//! [`store`] (the catalog, and downloading from it), [`acquire`] (verifying a package or an
//! unpacked one against the keys a process trusts, disclosing it, fetching it), [`disclosure`]
//! (what a person is shown and consents to), [`publish`] (the publisher's and the store's tools,
//! and the `harnix-pack` binary under the `compile` feature), and [`egress`] (where its HTTP may
//! go, decided on the product's side).
//!
//! Nothing here runs anything. The host that runs a package (`harnix-wasm-host`) and the product
//! that installs it (`harnix-ext-packages`, and the carrier it runs on) all read packages with
//! this, so a package means the same to each.

pub mod acquire;
pub mod disclosure;
pub mod egress;
pub mod inline;
pub mod manifest;
pub mod package;
pub mod publish;
pub mod signature;
pub mod store;
pub mod surface;

/// Whether `id` can be a package's id: the install records' rule too, which is why it is the
/// contract's.
pub use harnix_contract::ext_id::valid_id;

/// The most tools one process's sessions are offered from packages.
pub const MAX_TOOLS: usize = 64;

/// The name a session sees one of a package's tools by: `ext__<id>__<tool>`, the id's `.` and `-`
/// and the tool's `-` as `_`: the engine's tool names are letters, digits and `_`, starting with a
/// letter, at most 64. Not one-to-one, so a package whose names would meet another's is refused at
/// install ([`tool_clashes`]).
pub fn tool_name(app: &str, tool: &str) -> String {
    format!("ext__{}__{}", app.replace(['.', '-'], "_"), tool.replace('-', "_"))
}

/// Whether `name` is one the engine takes for a tool: `^[A-Za-z][A-Za-z0-9_]{0,63}$`.
pub fn engine_tool_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The names `manifest`'s tools would take that one of `installed`'s already has, as
/// `(name, the id of the package that has it)`. `installed` may include `manifest`'s own package
/// (an update replaces itself): it is skipped.
pub fn tool_clashes<'a>(
    manifest: &manifest::Manifest,
    installed: impl IntoIterator<Item = &'a manifest::Manifest>,
) -> Vec<(String, String)> {
    let ours: std::collections::BTreeSet<String> =
        manifest.tools.iter().map(|t| tool_name(&manifest.app.id, &t.name)).collect();
    let mut out = Vec::new();
    for other in installed.into_iter().filter(|m| m.app.id != manifest.app.id) {
        for tool in &other.tools {
            let name = tool_name(&other.app.id, &tool.name);
            if ours.contains(&name) {
                out.push((name, other.app.id.clone()));
            }
        }
    }
    out
}

/// What refuses `manifest` beside `installed`: a tool named as another package's, or more than
/// [`MAX_TOOLS`] tools in all (a disabled package counts: it may be enabled any time).
pub fn check_tools_beside<'a>(
    manifest: &manifest::Manifest,
    installed: impl IntoIterator<Item = &'a manifest::Manifest> + Clone,
) -> Result<(), String> {
    if let Some((name, other)) = tool_clashes(manifest, installed.clone()).into_iter().next() {
        return Err(format!(
            "`{name}` would be the name of a tool of both `{}` and `{other}`: the package cannot be installed beside it",
            manifest.app.id
        ));
    }
    let n = tool_count(manifest, installed);
    if n > MAX_TOOLS {
        return Err(format!("a process offers at most {MAX_TOOLS} package tools in its sessions; this would make {n}"));
    }
    Ok(())
}

/// How many tools a process's sessions are offered with `manifest` added to `installed` (its own
/// package, if listed there, counted once, as `manifest` has it).
pub fn tool_count<'a>(
    manifest: &manifest::Manifest,
    installed: impl IntoIterator<Item = &'a manifest::Manifest>,
) -> usize {
    manifest.tools.len()
        + installed.into_iter().filter(|m| m.app.id != manifest.app.id).map(|m| m.tools.len()).sum::<usize>()
}

/// The name a spawned daemon knows a package's MCP server by (its tools are
/// `mcp__<server>__<tool>` there).
pub fn server_name(app: &str) -> String {
    format!("ext-{}", app.replace('.', "_"))
}

/// The longest note a package's hook may add after one of its tools' calls, in characters.
pub const MAX_NOTE: usize = 2000;

/// What a note of the package `app` tells the model about a call of `tool`, as a reminder before
/// its next step: nothing for a tool not its own, and at most [`MAX_NOTE`] characters, under its
/// name. A downloaded package does not get to write after every tool the model calls. Every
/// carrier's hooks read it so (`harnix-carrier-shared`, and an extension process's own engine in
/// `harnix-proc`).
pub fn note_for(app: &str, tool: &str, note: &str) -> Option<String> {
    if !tool.starts_with(&tool_name(app, "")) {
        return None;
    }
    let cut = note.chars().count() > MAX_NOTE;
    let mut note: String = note.chars().take(MAX_NOTE).collect();
    if cut {
        note.push('…');
    }
    Some(format!("[{app}] {note}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_goes_on_its_own_tools_calls_only_and_only_so_long() {
        assert_eq!(note_for("acme.notes", "ext__acme_notes__search", "seen").as_deref(), Some("[acme.notes] seen"));
        for tool in ["Bash", "ext__acme_other__search", "ext__acme_notesx__search"] {
            assert_eq!(note_for("acme.notes", tool, "seen"), None, "{tool}");
        }
        let long = "x".repeat(MAX_NOTE + 10);
        let note = note_for("acme.notes", "ext__acme_notes__search", &long).unwrap();
        assert_eq!(note.chars().filter(|c| *c == 'x').count(), MAX_NOTE);
        assert!(note.ends_with('…'));
    }
}
