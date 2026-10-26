//! Where instances come from (design §5.2, §5.4).
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
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::contrib::EngineContrib;
use crate::profile::EngineProfile;

/// Who an instance belongs to. Stopping a module stops what it owns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum Owner {
    /// The profile's.
    Kernel,
    Module(String),
    /// Built from the interface. Becomes the space it was built from once
    /// spaces exist (M5).
    Ui,
}

impl From<Owner> for String {
    fn from(owner: Owner) -> String {
        match owner {
            Owner::Kernel => "kernel".into(),
            Owner::Module(id) => format!("module:{id}"),
            Owner::Ui => "ui".into(),
        }
    }
}

impl TryFrom<String> for Owner {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        match s.as_str() {
            "kernel" => Ok(Owner::Kernel),
            "ui" => Ok(Owner::Ui),
            _ => s
                .strip_prefix("module:")
                .map(|id| Owner::Module(id.to_string()))
                .ok_or_else(|| format!("unknown owner `{s}`")),
        }
    }
}

impl std::fmt::Display for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&String::from(self.clone()))
    }
}

/// How a template's instances are carried.
#[derive(Clone)]
pub enum TemplateCarrier {
    /// In this process. Only a module's template can carry native
    /// contributions; a profile has none to give.
    InProcess {
        engine: EngineProfile,
        contrib: EngineContrib,
    },
    /// A daemon this process starts and keeps running: `harnix daemon`
    /// unless `program` says otherwise.
    Spawn {
        engine: EngineProfile,
        program: Option<PathBuf>,
        args: Vec<String>,
    },
    Attach(AttachTo),
}

impl TemplateCarrier {
    pub fn carrier(&self) -> crate::link::Carrier {
        match self {
            TemplateCarrier::InProcess { .. } => crate::link::Carrier::InProcess,
            TemplateCarrier::Spawn { .. } => crate::link::Carrier::Spawn,
            TemplateCarrier::Attach(_) => crate::link::Carrier::Attach,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            TemplateCarrier::InProcess { .. } => "in_process",
            TemplateCarrier::Spawn { .. } => "spawn",
            TemplateCarrier::Attach(_) => "attach",
        }
    }
}

/// A daemon somebody else runs, serving only this Harnix (design §5.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachTo {
    /// Its unix socket.
    #[serde(default)]
    pub socket: Option<PathBuf>,
    /// Or its loopback TCP listener, with the credential holding its token.
    #[serde(default)]
    pub tcp: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Clone)]
pub struct InstanceTemplate {
    pub id: String,
    pub carrier: TemplateCarrier,
    /// Under Harnix's instances directory. `{name}` and every declared
    /// parameter may be used.
    pub data_root: String,
    /// Parameters besides `name` a caller may fill.
    pub params: Vec<String>,
    /// Whether the interface may build from it (D18).
    pub ui: bool,
    pub max: Option<usize>,
}

impl InstanceTemplate {
    pub fn new(id: impl Into<String>, carrier: TemplateCarrier) -> Self {
        Self {
            id: id.into(),
            carrier,
            data_root: "{name}".into(),
            params: Vec::new(),
            ui: false,
            max: None,
        }
    }

    pub fn data_root(mut self, pattern: impl Into<String>) -> Self {
        self.data_root = pattern.into();
        self
    }

    pub fn param(mut self, name: impl Into<String>) -> Self {
        self.params.push(name.into());
        self
    }

    pub fn ui(mut self) -> Self {
        self.ui = true;
        self
    }

    pub fn max(mut self, max: usize) -> Self {
        self.max = Some(max);
        self
    }

    /// Check what a caller filled in and say where the instance's data goes.
    /// Anything the template does not open is refused, not ignored.
    pub fn resolve(
        &self,
        name: &str,
        params: &BTreeMap<String, String>,
        instances_dir: &Path,
    ) -> Result<PathBuf, String> {
        check_value("name", name)?;
        for (key, value) in params {
            if !self.params.contains(key) {
                return Err(format!(
                    "template `{}` does not take the parameter `{key}`",
                    self.id
                ));
            }
            check_value(key, value)?;
        }
        let mut root = self.data_root.replace("{name}", name);
        for key in &self.params {
            let Some(value) = params.get(key) else {
                return Err(format!("template `{}` needs the parameter `{key}`", self.id));
            };
            root = root.replace(&format!("{{{key}}}"), value);
        }
        if root.contains('{') || root.split('/').any(|part| part == ".." || part.is_empty()) {
            return Err(format!("template `{}` resolves to a bad data root `{root}`", self.id));
        }
        Ok(instances_dir.join(root))
    }
}

/// Names and parameter values become directory names and instance ids, so
/// they are held to what is safe as both.
pub fn check_value(what: &str, value: &str) -> Result<(), String> {
    let ok = !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !value.starts_with('.');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "`{what}` must be 1–64 letters, digits, `-`, `_` or `.`, not starting with `.`"
        ))
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

/// `.harnix/instances.json`: Harnix's own record, apart from every engine.
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

    fn template() -> InstanceTemplate {
        InstanceTemplate::new(
            "t",
            TemplateCarrier::Attach(AttachTo {
                socket: Some("/nowhere".into()),
                tcp: None,
                token: None,
            }),
        )
        .data_root("{project}/{name}")
        .param("project")
    }

    #[test]
    fn a_parameter_the_template_does_not_open_is_refused() {
        let params = BTreeMap::from([
            ("project".to_string(), "p".to_string()),
            ("program".to_string(), "/bin/sh".to_string()),
        ]);
        let err = template().resolve("a", &params, Path::new("/x")).unwrap_err();
        assert!(err.contains("program"), "{err}");
    }

    #[test]
    fn a_value_cannot_climb_out_of_the_instances_directory() {
        let params = BTreeMap::from([("project".to_string(), "..".to_string())]);
        assert!(template().resolve("a", &params, Path::new("/x")).is_err());
        assert!(template()
            .resolve("../a", &BTreeMap::new(), Path::new("/x"))
            .is_err());
    }

    #[test]
    fn the_data_root_is_filled_in() {
        let params = BTreeMap::from([("project".to_string(), "p".to_string())]);
        assert_eq!(
            template().resolve("a", &params, Path::new("/x")).unwrap(),
            PathBuf::from("/x/p/a")
        );
    }

    #[test]
    fn owners_read_back() {
        for owner in [Owner::Kernel, Owner::Ui, Owner::Module("m".into())] {
            let s: String = owner.clone().into();
            assert_eq!(Owner::try_from(s).unwrap(), owner);
        }
    }
}
