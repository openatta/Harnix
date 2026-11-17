//! The profile: what this process is, said once, declaratively.
//!
//! The assembly reads the whole file and hands each section to its owner: `[engine]`,
//! `[[instances]]` and `[[instance_templates]]` to the instance layer; `[transport]`, `[brand]`,
//! `[shell]`, `[ui]` and `[extensions]` are read here; `[ext.<id>]` is the extension
//! `<id>`'s, read as its own configuration type. A section nobody owns, and a key its owner does
//! not read, stop the start, naming it: a profile that quietly does not mean what it says is the
//! failure this file exists to avoid. Nothing can rewrite it while the process runs.

use std::collections::BTreeMap;
use std::path::PathBuf;

use harnix_contract::Topology;
use harnix_instance::profile::{InstanceSections, SECTIONS as INSTANCE_SECTIONS};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Who reads a top-level section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    Instances,
    Assembly,
    /// `[ext.<id>]`: each extension its own.
    Extensions,
}

/// The sections the assembly reads itself, on its own and the transport's behalf.
const ASSEMBLY_SECTIONS: &[&str] = &["transport", "brand", "shell", "ui", "extensions"];

/// The extensions' sections, `[ext.<id>]`.
const EXT: &str = "ext";

fn owner(section: &str) -> Option<Owner> {
    if INSTANCE_SECTIONS.contains(&section) {
        Some(Owner::Instances)
    } else if ASSEMBLY_SECTIONS.contains(&section) {
        Some(Owner::Assembly)
    } else if section == EXT {
        Some(Owner::Extensions)
    } else {
        None
    }
}

/// The whole profile, each section read by its owner.
#[derive(Debug, Clone, Default)]
pub(crate) struct Profile {
    /// `[engine]`, `[[instances]]`, `[[instance_templates]]`.
    pub instances: InstanceSections,
    pub transport: TransportProfile,
    pub brand: BrandProfile,
    pub shell: ShellProfile,
    pub ui: UiProfile,
    pub extensions: ExtensionsProfile,
    /// `[ext.<id>]`, as written: each handed to the extension `<id>`, which reads it as its own
    /// configuration. One no extension here owns stops the start when the extensions are known.
    pub ext: BTreeMap<String, Value>,
}

impl Profile {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let table: toml::Table = toml::from_str(text)?;
        let mut instances = toml::Table::new();
        let mut profile = Profile::default();
        for (section, value) in table {
            match owner(&section) {
                Some(Owner::Instances) => {
                    instances.insert(section, value);
                }
                Some(Owner::Assembly) => match section.as_str() {
                    "transport" => profile.transport = read(&section, value)?,
                    "brand" => profile.brand = read(&section, value)?,
                    "shell" => profile.shell = read(&section, value)?,
                    "ui" => profile.ui = read(&section, value)?,
                    _ => profile.extensions = read(&section, value)?,
                },
                Some(Owner::Extensions) => {
                    let toml::Value::Table(sections) = value else {
                        anyhow::bail!("[{EXT}] is a table of `[{EXT}.<id>]` sections, one per extension");
                    };
                    for (id, value) in sections {
                        let value = serde_json::to_value(value)
                            .map_err(|e| anyhow::anyhow!("[{EXT}.{id}]: {e}"))?;
                        profile.ext.insert(id, value);
                    }
                }
                None => anyhow::bail!(
                    "the profile's `{section}` is not a section any part of this process reads (the sections are {})",
                    INSTANCE_SECTIONS.iter().chain(ASSEMBLY_SECTIONS).chain([&EXT]).copied().collect::<Vec<_>>().join(", ")
                ),
            }
        }
        profile.instances = toml::Value::Table(instances).try_into()?;
        profile.check()?;
        Ok(profile)
    }

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("profile {}: {e}", path.display()))?;
        Self::parse(&text)
    }

    fn check(&self) -> anyhow::Result<()> {
        if let Some(id) = &self.brand.product_id {
            let mut chars = id.chars();
            let valid = chars.next().is_some_and(|c| c.is_ascii_lowercase())
                && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !valid {
                anyhow::bail!("[brand] product_id `{id}`: lowercase letters, digits and `-`, starting with a letter");
            }
        }
        Ok(())
    }
}

/// One section, read strictly.
fn read<T: DeserializeOwned>(section: &str, value: toml::Value) -> anyhow::Result<T> {
    value.try_into().map_err(|e: toml::de::Error| anyhow::anyhow!("[{section}]: {}", e.message()))
}

/// `[extensions]`: which of the extensions linked in this process are off.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ExtensionsProfile {
    /// Linked, and switched off: registered, so nobody else takes its names, but nothing of it
    /// is installed, migrated or started.
    pub off: Vec<String>,
    /// The space a call that claims none is checked as, in place of the one an extension
    /// declared the default: one an enabled extension declares.
    pub default_space: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TransportProfile {
    /// Topologies this deployment serves. A client picks one at handshake;
    /// anything else is refused with a reason rather than downgraded.
    pub topologies: Vec<Topology>,
    /// The address to bind: an IP literal (`127.0.0.1`, `::1`, `0.0.0.0`), not a host name. A
    /// string because the profile is a file; it is parsed as the process starts, and one that is
    /// not an address stops the start, naming it.
    pub host: String,
    pub port: u16,
    /// Keep the token out of the page: a browser is admitted by a one-time
    /// launch link exchanged for a session cookie (`--launch-link`).
    pub launch_link: bool,
    /// Origins a browser may connect from besides loopback, exactly as a
    /// browser serializes them (`https://192.168.1.20:4317`).
    pub extra_origins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ShellProfile {
    /// Whether the composer may change a session's model:
    /// `harnix.session.setModel`, scoped to the space the caller claims (ipdrun-12). Off unless
    /// written, so a product that left model choice to the settings page keeps it there.
    pub model_switch: bool,
    /// Whether the shell draws its "New session" button and takes ⌘K to start one (ipdrun-46).
    /// On unless written: `false` hides both in every space; a space's own `newSession` still
    /// applies on top. Only the interface; the hub still takes `harnix.session.create`.
    pub new_session: bool,
    /// What an open right column does when the window is too narrow for three (≤ 960px, ipdrun-50):
    /// `over` (the default, as before) maximizes it over the conversation; `beside` keeps the three
    /// columns side by side — the sidebar a rail, the right column narrower — down to 720px.
    pub narrow_details: NarrowDetails,
}

/// `[shell] narrow_details` (ipdrun-50): the wire's enum, read from the profile as it is told at hello.
pub(crate) use harnix_contract::wire::NarrowDetails;

impl Default for ShellProfile {
    fn default() -> Self {
        Self {
            model_switch: false,
            new_session: true,
            narrow_details: NarrowDetails::Over,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct BrandProfile {
    /// The product's name: the page title, the left column's top.
    pub name: Option<String>,
    /// An SVG file, relative to the profile, shown beside the name.
    pub icon: Option<PathBuf>,
    /// Names the product's own directory, `~/.<product_id>`: lowercase letters, digits and `-`,
    /// starting with a letter. `harnix` when absent.
    pub product_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct UiProfile {
    /// A built interface on disk, served in place of the one compiled in. `None` serves the
    /// embedded one; no static face at all is `--headless`, for a pure RPC node (concept
    /// §3.3.4, §5.1).
    pub dir: Option<PathBuf>,
    /// Whether packages' interfaces reach the page — a trusted one's modules, every other one's
    /// frames. Off, none does.
    pub runtime_modules: bool,
}

impl Default for UiProfile {
    fn default() -> Self {
        Self {
            dir: None,
            runtime_modules: true,
        }
    }
}

impl Default for TransportProfile {
    fn default() -> Self {
        Self {
            // One bidirectional connection, which is the edge default: out
            // there a connection costs more than middleware does (concept §7.2).
            // A deployment behind a proxy adds `split_streams` and gets the
            // proxy's middleware back.
            topologies: vec![Topology::SingleDuplex],
            host: "127.0.0.1".into(),
            port: 4080,
            launch_link: false,
            extra_origins: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A misspelled key is an error, not a silently ignored line, whoever owns the section.
    #[test]
    fn an_unknown_key_is_refused_naming_it() {
        let err = Profile::parse("[engine]\nscenee = \"chat\"\n").unwrap_err();
        assert!(err.to_string().contains("scenee"), "{err}");
        let err = Profile::parse("[shell]\nspacez = []\n").unwrap_err();
        assert!(err.to_string().contains("spacez"), "{err}");
    }

    /// A section no part of the process reads stops the start, named; no name is special.
    #[test]
    fn a_section_nobody_owns_is_refused_naming_it() {
        let err = Profile::parse("[nobody]\nx = 1\n").unwrap_err();
        assert!(err.to_string().contains("`nobody`"), "{err}");
    }

    /// Each extension's section is kept as written, for the extension to read as its own: whether
    /// one is owned is known once the extensions are (`host::lifecycle`).
    #[test]
    fn an_extensions_section_is_its_own() {
        let profile = Profile::parse("[ext.probe]\ngreeting = \"hi\"\n[ext.nobody]\nx = 1\n").unwrap();
        assert_eq!(profile.ext["probe"]["greeting"], "hi");
        assert!(profile.ext.contains_key("nobody"));
    }

    #[test]
    fn the_shell_draws_and_switches_as_written() {
        let profile = Profile::parse("").unwrap();
        assert!(profile.ui.runtime_modules);
        assert!(!profile.shell.model_switch);
        assert!(profile.shell.new_session);
        let profile = Profile::parse("[shell]\nmodel_switch = true\n").unwrap();
        assert!(profile.shell.model_switch);
        // Which spaces there are is which extensions are: `[extensions] off`, not the shell's.
        assert!(Profile::parse("[shell]\nspaces = [\"sessions\"]\n").is_err());
    }

    #[test]
    fn the_right_column_goes_over_when_narrow_unless_told() {
        let shell = |text: &str| Profile::parse(text).unwrap().shell;
        assert_eq!(shell("").narrow_details, NarrowDetails::Over);
        assert_eq!(shell("[shell]\nnarrow_details = \"beside\"\n").narrow_details, NarrowDetails::Beside);
        assert!(Profile::parse("[shell]\nnarrow_details = \"under\"\n").is_err());
    }

    #[test]
    fn a_product_id_names_a_directory() {
        let profile = Profile::parse("[brand]\nname = \"Helm\"\nproduct_id = \"helm\"\n").unwrap();
        assert_eq!(profile.brand.product_id.as_deref(), Some("helm"));
        for bad in ["Helm", "../x", "1helm", ""] {
            assert!(Profile::parse(&format!("[brand]\nproduct_id = \"{bad}\"\n")).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_instance_sections_go_to_the_instance_layer() {
        let profile = Profile::parse("[engine]\nscene = \"code\"\n[[instances]]\nid = \"side\"\ncarrier = \"in_process\"\n").unwrap();
        assert_eq!(profile.instances.engine.scene, "code");
        assert_eq!(profile.instances.instances[0].id, "side");
    }

    #[test]
    fn the_edge_default_is_one_connection() {
        assert_eq!(Profile::default().transport.topologies, vec![Topology::SingleDuplex]);
    }

    #[test]
    fn extensions_can_be_switched_off() {
        let profile = Profile::parse("[extensions]\noff = [\"flow\"]\ndefault_space = \"home\"\n").unwrap();
        assert_eq!(profile.extensions.off, ["flow"]);
        assert_eq!(profile.extensions.default_space.as_deref(), Some("home"));
        assert!(Profile::parse("[extensions]\nof = []\n").is_err());
    }
}
