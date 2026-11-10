//! Instance templates as an extension declares them: what the instances built from one run on,
//! where their data goes, what a caller may fill in, and what the extension contributes to each
//! engine. The host keeps the templates and builds from them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::contribution::EngineContribution;

/// An engine's settings: the profile's `[engine]`, and what a template's instances run with.
#[non_exhaustive]
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

impl Default for EngineProfile {
    fn default() -> Self {
        Self {
            scene: "harnix".into(),
            scenes: Vec::new(),
            model: "claude-sonnet-4-6".into(),
            max_tokens: 2000,
            session_cap: 32,
            session_idle_timeout_secs: 3600,
            permission_prompt_timeout_secs: 300,
        }
    }
}

impl EngineProfile {
    /// The defaults: the `harnix` scene and nothing beside it.
    pub fn new() -> Self {
        Self::default()
    }

    /// The scene a session that names none is created in.
    pub fn scene(mut self, scene: &str) -> Self {
        self.scene = scene.to_string();
        self
    }

    /// Scenes served beside it.
    pub fn scenes<S: Into<String>>(mut self, scenes: impl IntoIterator<Item = S>) -> Self {
        self.scenes = scenes.into_iter().map(Into::into).collect();
        self
    }

    pub fn model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }
}

/// How a template's instances are carried.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum TemplateCarrier {
    /// In this process. Only a module's template can carry native contributions
    /// ([`InstanceTemplate::contribute`]); a profile has none to give.
    InProcess { engine: EngineProfile },
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
    pub fn kind(&self) -> &'static str {
        match self {
            TemplateCarrier::InProcess { .. } => "in_process",
            TemplateCarrier::Spawn { .. } => "spawn",
            TemplateCarrier::Attach(_) => "attach",
        }
    }
}

/// A daemon somebody else runs, serving only this Harnix (concept §3.1.1).
#[non_exhaustive]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

impl AttachTo {
    /// Over its unix socket.
    pub fn socket(path: impl Into<PathBuf>) -> Self {
        Self { socket: Some(path.into()), ..Self::default() }
    }

    /// Over its loopback TCP listener; `token` names the credential holding its token.
    pub fn tcp(addr: &str, token: &str) -> Self {
        Self { tcp: Some(addr.to_string()), token: Some(token.to_string()), ..Self::default() }
    }
}

/// A template an extension declares: what its instances run on, where their data goes, and what a
/// caller may fill in. Built with [`InstanceTemplate::in_process`], [`InstanceTemplate::spawn`]
/// or [`InstanceTemplate::attach`]; the fields are read by the host.
#[non_exhaustive]
#[derive(Clone)]
pub struct InstanceTemplate {
    pub id: String,
    pub carrier: TemplateCarrier,
    /// Under Harnix's instances directory. `{name}` and every declared
    /// parameter may be used.
    pub data_root: String,
    /// Parameters besides `name` a caller may fill.
    pub params: Vec<String>,
    /// Whether the interface may build from it: a device fills only `params`, so nothing that
    /// should stay the template's own is ever a device's to choose.
    pub ui: bool,
    pub max: Option<usize>,
    /// Whether its instances see the providers configured for `system` — the settings page's —
    /// under their own (ipdrun-11).
    pub inherit_providers: bool,
    /// Stopping or restarting one of its instances ends every process its sessions left running
    /// (ipdrun-07). See [`InstanceTemplate::reap_on_stop`].
    pub reap_on_stop: bool,
    /// Native contributions built again for each instance, whichever carrier runs it (ipdrun-10).
    /// See [`InstanceTemplate::contribute`].
    pub contribute: Option<Contribute>,
}

/// What [`InstanceTemplate::contribute`] keeps: called once per engine it builds, in whichever
/// process that engine runs.
pub type Contribute = Arc<dyn Fn(&mut EngineContribution) + Send + Sync>;

impl InstanceTemplate {
    /// Its instances are engines in this process.
    pub fn in_process(id: &str, engine: EngineProfile) -> Self {
        Self::new(id, TemplateCarrier::InProcess { engine })
    }

    /// Its instances are daemons this process starts: this binary's `daemon` subcommand.
    pub fn spawn(id: &str, engine: EngineProfile) -> Self {
        Self::new(id, TemplateCarrier::Spawn { engine, program: None, args: Vec::new() })
    }

    /// Its instances are a daemon somebody else runs.
    pub fn attach(id: &str, to: AttachTo) -> Self {
        Self::new(id, TemplateCarrier::Attach(to))
    }

    /// A template carried as `carrier` says: what a profile's `[[instance_templates]]` is.
    pub fn new(id: impl Into<String>, carrier: TemplateCarrier) -> Self {
        Self {
            id: id.into(),
            carrier,
            data_root: "{name}".into(),
            params: Vec::new(),
            ui: false,
            max: None,
            inherit_providers: false,
            reap_on_stop: false,
            contribute: None,
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

    /// This template's instances see the providers configured for `system` (the settings page's),
    /// under their own: Harnix merges them into each instance's global settings tier when it is
    /// built and every time the settings page changes a provider, then has it reload. A provider
    /// the instance configures itself under the same id wins.
    pub fn inherit_providers(mut self) -> Self {
        self.inherit_providers = true;
        self
    }

    /// Stopping or restarting one of this template's instances ends every process its sessions
    /// left running, including what a command deliberately left in the background.
    ///
    /// For a spawned daemon: it leads a session of its own, and once it is gone — `stop`, killed,
    /// crashed, about to be started again — whatever is still in that session gets TERM, a grace,
    /// then KILL. The daemon also has the engine's own reaping on
    /// (`sandbox.reap_process_groups`), so on an ordinary exit it has ended its sessions'
    /// processes itself; this is for the daemon that did not get the chance. Being in its own
    /// session, it no longer gets the terminal's Ctrl-C; its exit follows stdin closing, as it
    /// always did. For an in-process engine, only the engine's own reaping applies: it ends a
    /// session's process groups when the session ends. A template that does not ask leaves
    /// background processes alone, as before.
    pub fn reap_on_stop(mut self) -> Self {
        self.reap_on_stop = true;
        self
    }

    /// Native contributions for this template's instances, whichever carrier runs them: scenes,
    /// session customizers, a credential source. In process, they are added after the carrier's
    /// own `contrib`. For a spawned daemon of this binary, the daemon builds them again in its own
    /// process: it is started with `--template <id>`, calls every module's `templates()` (and
    /// nothing else of the module — no `start`, no methods) to find this one, and calls `f` there.
    /// They share no memory with the module; what they need of its state comes through Harnix's
    /// methods or files. A template whose `program` is somebody else's gets no `--template`.
    pub fn contribute(mut self, f: impl Fn(&mut EngineContribution) + Send + Sync + 'static) -> Self {
        self.contribute = Some(Arc::new(f));
        self
    }

    /// What [`InstanceTemplate::contribute`] contributes, built once more.
    pub fn contribution(&self) -> Option<EngineContribution> {
        self.contribute.as_ref().map(|f| {
            let mut contrib = EngineContribution::new();
            f(&mut contrib);
            contrib
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> InstanceTemplate {
        InstanceTemplate::attach("t", AttachTo::socket("/nowhere"))
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
}
