//! A runtime application as it is distributed and trusted: the layer every kind of host shares.
//!
//! An application is a package (`.hxapp`, [`package`]): a manifest ([`manifest`]: its
//! declarations, its interface's [`surface`]s, its [`inline`] points), an optional WebAssembly
//! component, an optional agent package (`agent/`) and an optional interface (`ui/`). Where one
//! comes from and how far it is trusted: [`signature`] (who signed it, over which bytes),
//! [`store`] (the catalog, and downloading from it), [`acquire`] (verifying a package or an
//! unpacked one against the keys a process trusts, disclosing it, fetching it), [`disclosure`]
//! (what a person is shown and consents to), [`publish`] (the publisher's and the store's tools,
//! and the `harnix-app` binary under the `compile` feature).
//!
//! Nothing here runs anything. The host that runs an application (`harnix-app-host`) and the
//! product that installs it (`harnix-apps`, for an application embedded in the product's process)
//! both read packages with this, so a package means the same to either.

pub mod acquire;
pub mod disclosure;
pub mod inline;
pub mod manifest;
pub mod package;
pub mod publish;
pub mod signature;
pub mod store;
pub mod surface;

/// Whether `id` can be an application's id: the install records' rule too, which is why it is the
/// contract's.
pub use harnix_contract::apps::valid_id;

/// Every application tool's name in a session matches this: what each scene of an instance that
/// offers them has to admit besides its own tool list.
pub const TOOL_PATTERN: &str = "app__*";

/// The name a session sees one of an application's tools by: `app__<id>__<tool>`, with the id's
/// dots as `_` (a model's tool names are letters, digits, `_` and `-`; an id has no `_`, so this is
/// one-to-one).
pub fn tool_name(app: &str, tool: &str) -> String {
    format!("app__{}__{tool}", app.replace('.', "_"))
}

/// The name a spawned daemon knows an application's MCP server by (its tools are
/// `mcp__<server>__<tool>` there).
pub fn server_name(app: &str) -> String {
    format!("app-{}", app.replace('.', "_"))
}
