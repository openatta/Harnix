//! An instance's part of the profile: `[engine]`, `[[instances]]`, `[[instance_templates]]`.
//!
//! The assembly reads the whole profile and hands each section to its owner; these three are this
//! crate's, each read strictly — a key nobody reads is an error naming it, not a line quietly
//! ignored.

use std::path::PathBuf;

pub use harnix_engine_api::template::EngineProfile;
use serde::{Deserialize, Serialize};

/// The sections this crate owns, by their names in a profile.
pub const SECTIONS: &[&str] = &["engine", "instances", "instance_templates"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InstanceSections {
    /// `[engine]`: the `main` instance's engine.
    pub engine: EngineProfile,
    /// `[[instances]]`: instances besides `main`, built at every start.
    pub instances: Vec<InstanceDecl>,
    /// `[[instance_templates]]`: what extensions and the interface may build instances from.
    /// Spawn or attach only — a profile has no native contributions to give an in-process one.
    pub instance_templates: Vec<TemplateDecl>,
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
    /// Its instances see `main`'s providers under their own (ipdrun-11).
    #[serde(default)]
    pub inherit_providers: bool,
    /// Stopping one of its instances ends what its sessions left running (ipdrun-07).
    #[serde(default)]
    pub reap_on_stop: bool,
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
        CarrierKind::InProcess => TemplateCarrier::InProcess { engine: engine.clone() },
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
        carrier_of(
            self.carrier,
            &self.engine,
            &self.program,
            &self.args,
            &self.attach,
        )
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
        let carrier = carrier_of(
            self.carrier,
            &self.engine,
            &self.program,
            &self.args,
            &self.attach,
        )
        .map_err(|e| anyhow::anyhow!("[[instance_templates]] `{}`: {e}", self.id))?;
        let mut template = crate::templates::InstanceTemplate::new(self.id.clone(), carrier).data_root(self.data_root.clone());
        for param in &self.params {
            template = template.param(param.clone());
        }
        template.ui = self.ui;
        template.max = self.max;
        template.inherit_providers = self.inherit_providers;
        template.reap_on_stop = self.reap_on_stop;
        Ok(template)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> anyhow::Result<InstanceSections> {
        Ok(toml::from_str(text)?)
    }

    /// A misspelled key is an error, not a silently ignored line.
    #[test]
    fn an_unknown_key_is_refused() {
        let err = read("[engine]\nscenee = \"chat\"\n").unwrap_err();
        assert!(err.to_string().contains("scenee"), "{err}");
    }

    #[test]
    fn instances_and_templates_read() {
        let sections = read(
            r#"
            [[instances]]
            id = "side"
            carrier = "in_process"
            [instances.engine]
            scene = "harnix"

            [[instance_templates]]
            id = "worker"
            carrier = "spawn"
            ui = true
            params = ["project"]
            data_root = "workers/{project}/{name}"
            "#,
        )
        .unwrap();
        assert_eq!(sections.instances[0].engine.scene, "harnix");
        let template = sections.instance_templates[0].template().unwrap();
        assert!(template.ui);
        assert_eq!(template.carrier.kind(), "spawn");
    }

    #[test]
    fn a_profile_template_cannot_be_in_process() {
        let sections = read("[[instance_templates]]\nid = \"t\"\ncarrier = \"in_process\"\n").unwrap();
        assert!(sections.instance_templates[0].template().is_err());
    }
}
