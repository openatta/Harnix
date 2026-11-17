//! What a session sees beyond its conversation: the skills it may use and the hooks its turns run,
//! each with where it comes from — what a session's context tab shows.
//!
//! The engine's link reads them as the engine says them (`EngineOps::session_skills`,
//! `EngineOps::settings_hooks`); the hub adds which package a skill directory is, and the hooks
//! this process registered on its connection. A hook is never shown by what it runs: a command,
//! a URL, a header or a prompt can carry a credential, so a row says only its event, kind and
//! matcher.

use std::path::Path;

use harnix_contract::sync::read;
use harnix_contract::wire::{self, HookRow, HookSource, SessionSkills, SkillTier};
use harnix_contract::{RpcError, SkillSource};
use harnix_engine_link::HostHook;
use serde_json::Value;

use crate::Hub;

/// What a hook this process registered is, as a row says it.
const HOST_KIND: &str = "host";

impl Hub {
    /// `harnix.session.skills {instance?, session_id}`.
    pub(crate) async fn session_skills(&self, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let linked = self.engine(&key.0)?;
        let mut out = linked.ops().session_skills(&key.1).await?;
        let packages: Vec<SkillSource> = read(&self.skills).values().flatten().cloned().collect();
        with_packages(&mut out, &packages);
        out.instance = key.0;
        out.session_id = key.1;
        Ok(wire::params(&out))
    }

    /// `harnix.session.hooks {instance?, session_id}`.
    pub(crate) async fn session_hooks(&self, params: Value) -> Result<Value, RpcError> {
        let key = self.key_of(&params)?;
        let linked = self.engine(&key.0)?;
        let mut out = linked.ops().settings_hooks(&key.1).await?;
        out.hooks.extend(host_rows(&linked.link.host_hooks()));
        out.instance = key.0;
        out.session_id = key.1;
        Ok(wire::params(&out))
    }
}

/// Each listed directory, and each skill of the space tier, with the package whose directory it is.
fn with_packages(out: &mut SessionSkills, packages: &[SkillSource]) {
    let package_of = |path: &str| packages.iter().find(|s| s.dir == Path::new(path)).map(|s| s.ext.clone());
    for dir in &mut out.dirs {
        dir.package = package_of(&dir.path);
    }
    for skill in out.skills.iter_mut().filter(|s| s.tier == SkillTier::Space) {
        skill.package = skill.dir.as_deref().and_then(package_of);
    }
}

/// The hooks this process registered, as rows: in force, with the providers each asks.
fn host_rows(host: &[HostHook]) -> Vec<HookRow> {
    host.iter()
        .map(|hook| {
            let matcher = (!hook.tools.is_empty()).then(|| hook.tools.join(", "));
            let mut row = HookRow::new(hook.event.as_str(), HookSource::Host, HOST_KIND, matcher.as_deref(), Some(hook.on_error.clone()));
            row.providers = hook.providers.clone();
            row
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use harnix_contract::wire::{SkillDirRow, SkillRow};

    use super::*;

    fn skill(name: &str, tier: SkillTier, dir: Option<&str>) -> SkillRow {
        SkillRow { name: name.into(), description: String::new(), tier, dir: dir.map(str::to_string), package: None, plugin: None, withheld: false }
    }

    #[test]
    fn a_directory_and_its_skills_say_whose_package_they_are() {
        let mut out = SessionSkills {
            instance: String::new(),
            session_id: String::new(),
            listed: true,
            trust: None,
            skills: vec![
                skill("tour", SkillTier::Space, Some("/state/ext/acme.notes/skills")),
                skill("deploy", SkillTier::Project, Some("/state/ext/acme.notes/skills")),
                skill("lost", SkillTier::Space, Some("/elsewhere")),
            ],
            dirs: vec![SkillDirRow { path: "/state/ext/acme.notes/skills".into(), package: None, state: None, skills: vec!["tour".into()], shadowed: Vec::new() }],
        };
        with_packages(&mut out, &[SkillSource::new("acme.notes", "/state/ext/acme.notes/skills", Vec::new())]);
        assert_eq!(out.dirs[0].package.as_deref(), Some("acme.notes"));
        let packages: Vec<Option<&str>> = out.skills.iter().map(|s| s.package.as_deref()).collect();
        assert_eq!(packages, [Some("acme.notes"), None, None], "only the space tier is a package's");
    }

    #[test]
    fn the_hosts_hooks_are_rows_in_force_with_their_providers() {
        let host = [
            HostHook::new("PreToolUse", vec!["*".into()], "allow").providers(vec!["harnix-packages".into()]),
            HostHook::new("UserPromptSubmit", Vec::new(), "block"),
        ];
        let rows = host_rows(&host);
        assert_eq!((rows[0].kind.as_str(), rows[0].label.as_str(), rows[0].providers.clone()), (HOST_KIND, "host · *", vec!["harnix-packages".to_string()]));
        assert_eq!((rows[1].event.as_str(), rows[1].matcher.clone(), rows[1].on_error.as_deref()), ("UserPromptSubmit", None, Some("block")));
        assert!(rows.iter().all(|r| r.in_force && !r.withheld && r.source == HookSource::Host));
    }
}
