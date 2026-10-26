//! What this process is, said once, declaratively.
//!
//! A profile names the scenes, the providers, the interface bundle and the
//! transport topology. Assembly does what it says and nothing more, and
//! nothing can rewrite it while the process runs — whatever could rewrite
//! this would be deciding what this process is (§3.1, §10).
//!
//! Extensions are not named here. Scripts are configured in Agix's own
//! settings tiers and plugins are installed through Agix's own
//! installer; a second place to declare them would be a second truth.
//!
//! Changing shape is a configuration change, not a fork. That is the first of
//! the three levels of customization (§8) and the one that costs no code.

use std::collections::BTreeMap;
use std::path::PathBuf;

use harnix_contract::Topology;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub engine: EngineProfile,
    pub transport: TransportProfile,
    pub ui: UiProfile,
    /// `[[instances]]`: instances besides `system`, built at every start.
    pub instances: Vec<InstanceDecl>,
    /// `[[instance_templates]]`: what modules and the interface may build
    /// instances from. Spawn or attach only — a profile has no native
    /// contributions to give an in-process one.
    pub instance_templates: Vec<TemplateDecl>,
    /// `[system_space]`: the debugging face, which a product can turn off.
    pub system_space: SystemSpaceProfile,
    /// `[modules.<id>]`: each compile-time module's own section, handed to
    /// it as written. Secrets are references (`{ credential = "…" }`),
    /// never values (design §7.4).
    pub modules: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EngineProfile {
    /// Default scene for `session.create`, and the scope its state lives under.
    pub scene: String,
    /// Scenes activated on top of the default.
    pub scenes: Vec<String>,
    pub model: String,
    pub max_tokens: u32,
    pub session_cap: usize,
    pub session_idle_timeout_secs: u64,
    pub permission_prompt_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TransportProfile {
    /// Topologies this deployment serves. A client picks one at handshake;
    /// anything else is refused with a reason rather than downgraded.
    pub topologies: Vec<Topology>,
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
pub struct SystemSpaceProfile {
    /// Off, no client can claim the system space, and none can act across spaces.
    pub enabled: bool,
}

impl Default for SystemSpaceProfile {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiProfile {
    /// Where the built interface is. `None` serves no static face at all —
    /// the `--headless` shape, for a pure RPC node (§3.3.4, §5.1).
    pub dir: Option<PathBuf>,
}

/// How a profile's instance or template is carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CarrierKind {
    InProcess,
    Spawn,
    Attach,
}

/// One `[[instances]]` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceDecl {
    pub id: String,
    pub carrier: CarrierKind,
    /// `[instances.engine]`: what an in-process or spawned engine runs.
    #[serde(default)]
    pub engine: EngineProfile,
    /// Under Harnix's instances directory; the id when absent.
    #[serde(default)]
    pub data_root: Option<String>,
    /// Spawn: the program, when not this binary's `daemon` subcommand.
    #[serde(default)]
    pub program: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Attach: where the daemon is.
    #[serde(default)]
    pub attach: Option<crate::templates::AttachTo>,
}

/// One `[[instance_templates]]` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateDecl {
    pub id: String,
    pub carrier: CarrierKind,
    #[serde(default)]
    pub engine: EngineProfile,
    #[serde(default = "default_data_root")]
    pub data_root: String,
    #[serde(default)]
    pub params: Vec<String>,
    #[serde(default)]
    pub ui: bool,
    #[serde(default)]
    pub max: Option<usize>,
    #[serde(default)]
    pub program: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub attach: Option<crate::templates::AttachTo>,
}

fn default_data_root() -> String {
    "{name}".into()
}

fn carrier_of(
    kind: CarrierKind,
    engine: &EngineProfile,
    program: &Option<PathBuf>,
    args: &[String],
    attach: &Option<crate::templates::AttachTo>,
) -> anyhow::Result<crate::templates::TemplateCarrier> {
    use crate::templates::TemplateCarrier;
    Ok(match kind {
        CarrierKind::InProcess => TemplateCarrier::InProcess {
            engine: engine.clone(),
            contrib: crate::EngineContrib::default(),
        },
        CarrierKind::Spawn => TemplateCarrier::Spawn {
            engine: engine.clone(),
            program: program.clone(),
            args: args.to_vec(),
        },
        CarrierKind::Attach => TemplateCarrier::Attach(
            attach
                .clone()
                .ok_or_else(|| anyhow::anyhow!("an attached instance needs `[…attach]`"))?,
        ),
    })
}

impl InstanceDecl {
    pub fn carrier(&self) -> anyhow::Result<crate::templates::TemplateCarrier> {
        carrier_of(self.carrier, &self.engine, &self.program, &self.args, &self.attach)
            .map_err(|e| anyhow::anyhow!("[[instances]] `{}`: {e}", self.id))
    }
}

impl TemplateDecl {
    pub fn template(&self) -> anyhow::Result<crate::templates::InstanceTemplate> {
        if self.carrier == CarrierKind::InProcess {
            anyhow::bail!(
                "[[instance_templates]] `{}`: a profile template is spawn or attach; \
                 an in-process template comes from a module, which has native contributions to give it",
                self.id
            );
        }
        let carrier = carrier_of(self.carrier, &self.engine, &self.program, &self.args, &self.attach)
            .map_err(|e| anyhow::anyhow!("[[instance_templates]] `{}`: {e}", self.id))?;
        Ok(crate::templates::InstanceTemplate {
            id: self.id.clone(),
            carrier,
            data_root: self.data_root.clone(),
            params: self.params.clone(),
            ui: self.ui,
            max: self.max,
        })
    }
}

impl Default for EngineProfile {
    fn default() -> Self {
        Self {
            scene: "coding".into(),
            scenes: Vec::new(),
            model: "claude-sonnet-4-6".into(),
            max_tokens: 2000,
            session_cap: 32,
            session_idle_timeout_secs: 3600,
            permission_prompt_timeout_secs: 300,
        }
    }
}

impl Default for TransportProfile {
    fn default() -> Self {
        Self {
            // One bidirectional connection, which is the edge default: out
            // there a connection costs more than middleware does (§7.2).
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

impl Profile {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(text)?)
    }

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("profile {}: {e}", path.display()))?;
        Self::parse(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A misspelled key is an error, not a silently ignored line. A profile
    /// that quietly does not mean what it says is the failure mode this whole
    /// file exists to avoid.
    #[test]
    fn an_unknown_key_is_refused() {
        let err = Profile::parse("[engine]\nscenee = \"chat\"\n").unwrap_err();
        assert!(err.to_string().contains("scenee"), "{err}");
    }

    #[test]
    fn instances_and_templates_read() {
        let profile = Profile::parse(
            r#"
            [[instances]]
            id = "side"
            carrier = "in_process"
            [instances.engine]
            scene = "chat"

            [[instance_templates]]
            id = "worker"
            carrier = "spawn"
            ui = true
            params = ["project"]
            data_root = "workers/{project}/{name}"
            "#,
        )
        .unwrap();
        assert_eq!(profile.instances[0].engine.scene, "chat");
        let template = profile.instance_templates[0].template().unwrap();
        assert!(template.ui);
        assert_eq!(template.carrier.kind(), "spawn");
    }

    #[test]
    fn a_profile_template_cannot_be_in_process() {
        let profile = Profile::parse(
            "[[instance_templates]]\nid = \"t\"\ncarrier = \"in_process\"\n",
        )
        .unwrap();
        assert!(profile.instance_templates[0].template().is_err());
    }

    #[test]
    fn the_edge_default_is_one_connection() {
        assert_eq!(
            Profile::default().transport.topologies,
            vec![Topology::SingleDuplex]
        );
    }
}
