//! Where instances come from (concept §3.1.1).
//!
//! Two sources and no third: instances the profile declares, and instances
//! built from a template. A template is part of assembly — registered at
//! start, never added or rewritten while the process runs. Whoever builds
//! from one fills in only the parameters it opens; the binary, its
//! arguments, its environment and its model endpoint are the template's, so
//! an interface that may build an instance still cannot load a subprocess or
//! point a model somewhere (concept §3.4).
//!
//! The ledger records what was built from templates, so a restart builds the
//! same instances again.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};


/// Who an instance belongs to. Stopping a module stops what it owns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum Owner {
    /// The profile's.
    Kernel,
    Module(String),
    /// Built from the interface, by the space that was in front; or a space's
    /// default instance built from its template.
    Space(String),
}

impl From<Owner> for String {
    fn from(owner: Owner) -> String {
        match owner {
            Owner::Kernel => "kernel".into(),
            Owner::Module(id) => format!("module:{id}"),
            Owner::Space(id) => format!("space:{id}"),
        }
    }
}

impl TryFrom<String> for Owner {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        match s.as_str() {
            "kernel" => Ok(Owner::Kernel),
            _ => {
                if let Some(id) = s.strip_prefix("module:") {
                    Ok(Owner::Module(id.to_string()))
                } else if let Some(id) = s.strip_prefix("space:") {
                    Ok(Owner::Space(id.to_string()))
                } else {
                    Err(format!("unknown owner `{s}`"))
                }
            }
        }
    }
}

impl std::fmt::Display for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&String::from(self.clone()))
    }
}

pub use harnix_engine_api::template::{check_value, AttachTo, Contribute, InstanceTemplate, TemplateCarrier};

/// How the instances of a template with `carrier` are reached: what a down link says it was.
pub fn carrier_of(carrier: &TemplateCarrier) -> harnix_engine_api::InstanceCarrier {
    match carrier {
        TemplateCarrier::Spawn { .. } => harnix_engine_api::InstanceCarrier::Spawn,
        TemplateCarrier::Attach(_) => harnix_engine_api::InstanceCarrier::Attach,
        _ => harnix_engine_api::InstanceCarrier::InProcess,
    }
}

/// What a module declares in `templates()`. Ids get the module's prefix.
#[derive(Default)]
pub struct InstanceTemplates {
    pub(crate) list: Vec<InstanceTemplate>,
}

impl InstanceTemplates {
    pub fn add(&mut self, template: InstanceTemplate) -> &mut Self {
        self.list.push(template);
        self
    }

    pub fn into_list(self) -> Vec<InstanceTemplate> {
        self.list
    }
}

/// One instance built from a template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: String,
    pub template: String,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    pub owner: Owner,
    pub created_ms: u64,
}

/// `kernel/instances.json` in the state directory: Harnix's own record, apart from every engine.
pub struct Ledger {
    path: PathBuf,
    entries: Vec<LedgerEntry>,
}

impl Ledger {
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let entries = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(anyhow::anyhow!("{}: {e}", path.display())),
        };
        Ok(Self { path, entries })
    }

    pub fn entries(&self) -> &[LedgerEntry] {
        &self.entries
    }

    pub fn insert(&mut self, entry: LedgerEntry) -> anyhow::Result<()> {
        self.entries.retain(|e| e.id != entry.id);
        self.entries.push(entry);
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> anyhow::Result<bool> {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        if self.entries.len() == before {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    fn save(&self) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.entries)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners_read_back() {
        for owner in [Owner::Kernel, Owner::Space("s".into()), Owner::Module("m".into())] {
            let s: String = owner.clone().into();
            assert_eq!(Owner::try_from(s).unwrap(), owner);
        }
    }
}
