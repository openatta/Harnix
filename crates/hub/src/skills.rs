//! Which packages' skills a session lists, and keeping the sessions in memory to date.
//!
//! A package's skills are a directory of its own (`<state>/ext/<id>/skills`); the engine reads it
//! as a host skill directory the session lists (`options.skill_dirs`). The hub says which
//! directories a session lists from the space it is in: a package that declared spaces is seen in
//! those only, one that declared none in every space, and a `cross_space` space sees all. A package
//! switched on or off, installed, updated or removed changes who sees what; the sessions in
//! memory are told, and a session that was not in memory is told when it next is subscribed to.

use std::path::PathBuf;
use std::sync::Arc;

use harnix_contract::sync::{lock, read, write};
use harnix_contract::{RpcError, SkillSource};
use harnix_engine_link::ops::{EngineOps, SessionSetup};
use harnix_engine_link::SessionInfo;
use serde_json::Value;

use crate::{Hub, SessionKey};

/// The most directories an engine lists for one session.
const MAX_DIRS: usize = 64;

impl Hub {
    /// `by`'s skills directories, in place of what it published before. The sessions in memory
    /// are brought up to date when that changed anything.
    pub fn set_skill_sources(self: &Arc<Self>, by: &str, sources: Vec<SkillSource>) {
        {
            let mut all = write(&self.skills);
            if all.get(by).map(|s| s.as_slice()).unwrap_or_default() == sources.as_slice() {
                return;
            }
            match sources.is_empty() {
                true => all.remove(by),
                false => all.insert(by.to_string(), sources),
            };
        }
        let hub = self.clone();
        tokio::spawn(async move { hub.reconcile_live().await });
    }

    /// The directories a session of `space` on `instance` lists: nothing for an engine that is
    /// not on this machine (an attached daemon may be anywhere), an extension's own engine only its own package's,
    /// a `cross_space` space all of them. Otherwise the packages that declared `space` come first, then
    /// those that declared none: of two skills of one name the first listed is the one seen.
    pub(crate) fn skill_dirs_for(&self, instance: &str, space: &str) -> Vec<PathBuf> {
        let own = self.slot(instance).and_then(|slot| slot.extension).map(|id| id.as_str().to_string());
        let attached = self.linked(instance).is_some_and(|l| !l.link.on_this_machine());
        let crosses = lock(&self.spaces).crosses(space);
        let all: Vec<SkillSource> = read(&self.skills).values().flatten().cloned().collect();
        pick(&all, own.as_deref(), attached, crosses, space)
    }

    /// Bring one session's listed directories to what its space says, when they differ. A
    /// sub-agent follows its parent and is left alone.
    pub(crate) async fn reconcile_skill_dirs(&self, key: &SessionKey, info: &SessionInfo) {
        if info.value().get("session_kind").and_then(Value::as_str).is_some_and(|k| k != "primary") {
            return;
        }
        let want = self.skill_dirs_for(&key.0, &self.space_of(key));
        let have: Vec<PathBuf> = info.value()["skill_dirs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d["path"].as_str().map(PathBuf::from))
            .collect();
        if want == have {
            return;
        }
        let Ok(linked) = self.engine(&key.0) else { return };
        if let Err(e) = self.list_skill_dirs(linked.ops(), &key.1, &want).await {
            tracing::warn!(instance = %key.0, session = %key.1, error = %e.message, "the session's skill directories were not updated");
        }
    }

    /// Every session in memory that the hub watches, up to date.
    async fn reconcile_live(self: &Arc<Self>) {
        let keys: Vec<SessionKey> = {
            let guard = self.inner.lock().await;
            guard.sessions.iter().filter(|(_, s)| s.subscribed).map(|(k, _)| k.clone()).collect()
        };
        for key in keys {
            let Ok(linked) = self.engine(&key.0) else { continue };
            // A session that is not in memory has nothing to change now: it is brought up to date
            // when it is subscribed to.
            if let Ok(info) = linked.ops().get(&key.1).await {
                if info.loaded() {
                    self.reconcile_skill_dirs(&key, &info).await;
                }
            }
        }
    }

    /// `dirs` as the session's listed directories. The engine refusing some (a package's directory
    /// inside a project, say) does not stop the session having the rest: they are left out, once.
    pub(crate) async fn list_skill_dirs(&self, ops: &dyn EngineOps, session: &str, dirs: &[PathBuf]) -> Result<(), RpcError> {
        match ops.set_skill_dirs(session, dirs).await {
            Err(e) => match refused_dirs(&e, dirs.len()) {
                Some(bad) => {
                    tracing::warn!(session, refused = bad.len(), "the engine refuses some skill directories; listing the rest");
                    let rest: Vec<PathBuf> = dirs.iter().enumerate().filter(|(i, _)| !bad.contains(i)).map(|(_, d)| d.clone()).collect();
                    ops.set_skill_dirs(session, &rest).await.map(|_| ())
                }
                None => Err(e),
            },
            Ok(_) => Ok(()),
        }
    }

    /// A new session of the engine, set up with `setup`; the skill directories the engine refuses
    /// are left out.
    pub(crate) async fn create_listing(
        &self,
        ops: &dyn EngineOps,
        params: Value,
        setup: SessionSetup,
    ) -> Result<harnix_engine_link::SessionCreated, RpcError> {
        let dirs = setup.skill_dirs.clone();
        match ops.create(params.clone(), setup.clone()).await {
            Err(e) => match refused_dirs(&e, dirs.len()) {
                Some(bad) => {
                    tracing::warn!(refused = bad.len(), "the engine refuses some skill directories; creating the session with the rest");
                    let rest: Vec<PathBuf> = dirs.iter().enumerate().filter(|(i, _)| !bad.contains(i)).map(|(_, d)| d.clone()).collect();
                    ops.create(params, setup.skill_dirs(rest)).await
                }
                None => Err(e),
            },
            ok => ok,
        }
    }
}

/// The directories `sources` give a session, by the rules in the module's comment.
fn pick(all: &[SkillSource], own: Option<&str>, attached: bool, crosses: bool, space: &str) -> Vec<PathBuf> {
    if attached {
        return Vec::new();
    }
    let mut chosen: Vec<&SkillSource> = match own {
        Some(ext) => all.iter().filter(|s| s.ext == ext).collect(),
        None if crosses => all.iter().collect(),
        None => {
            let named = all.iter().filter(|s| !space.is_empty() && s.spaces.iter().any(|x| x == space));
            let shared = all.iter().filter(|s| s.spaces.is_empty());
            named.chain(shared).collect()
        }
    };
    chosen.sort_by(|a, b| a.ext.cmp(&b.ext));
    // The group that declared the space stays before the shared one: a stable sort by group.
    if own.is_none() && !crosses {
        chosen.sort_by_key(|s| s.spaces.is_empty());
    }
    let mut dirs: Vec<PathBuf> = chosen.into_iter().map(|s| s.dir.clone()).collect();
    dirs.dedup();
    if dirs.len() > MAX_DIRS {
        tracing::warn!(listed = dirs.len(), "more skill directories than a session lists: the rest are left out");
        dirs.truncate(MAX_DIRS);
    }
    dirs
}

/// The indexes of the directories an engine's refusal names, when it names nothing else.
fn refused_dirs(e: &RpcError, listed: usize) -> Option<Vec<usize>> {
    if e.code != harnix_contract::codes::INVALID_PARAMS {
        return None;
    }
    let items = e.data.as_ref()?.get("invalid_options")?.as_array()?;
    let mut bad = Vec::new();
    for item in items {
        let field = item.get("field")?.as_str()?;
        let rest = field.strip_prefix("skill_dirs")?;
        match rest.strip_prefix('[') {
            Some(index) => bad.push(index.split(']').next()?.parse::<usize>().ok()?),
            // The list as a whole (too many, or no skills module): none can be kept.
            None => bad.extend(0..listed),
        }
    }
    (!bad.is_empty()).then_some(bad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source(ext: &str, spaces: &[&str]) -> SkillSource {
        SkillSource::new(ext, format!("/state/ext/{ext}/skills"), spaces.iter().map(|s| s.to_string()).collect())
    }

    fn names(dirs: Vec<PathBuf>) -> Vec<String> {
        dirs.iter().map(|d| d.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn a_space_sees_its_own_packages_first_then_the_shared_ones() {
        let all = [source("zed.shared", &[]), source("acme.notes", &["acme.notes"]), source("acme.docs", &[]), source("other.app", &["other.app"])];
        assert_eq!(names(pick(&all, None, false, false, "acme.notes")), ["acme.notes", "acme.docs", "zed.shared"]);
        // No space: only the shared ones.
        assert_eq!(names(pick(&all, None, false, false, "")), ["acme.docs", "zed.shared"]);
    }

    #[test]
    fn the_system_space_sees_every_package() {
        let all = [source("b", &["x"]), source("a", &[]), source("c", &["y"])];
        assert_eq!(names(pick(&all, None, false, true, "system")), ["a", "b", "c"]);
    }

    #[test]
    fn an_extensions_own_engine_sees_only_its_own_package() {
        let all = [source("a", &[]), source("b", &["b"])];
        assert_eq!(names(pick(&all, Some("b"), false, false, "b")), ["b"]);
        assert!(pick(&all, Some("nobody"), false, false, "b").is_empty());
    }

    #[test]
    fn an_attached_engine_lists_none() {
        let all = [source("a", &[])];
        assert!(pick(&all, None, true, true, "system").is_empty());
    }

    #[test]
    fn no_more_than_the_engine_lists() {
        let all: Vec<SkillSource> = (0..70).map(|i| source(&format!("p{i:02}"), &[])).collect();
        assert_eq!(pick(&all, None, false, false, "s").len(), MAX_DIRS);
    }

    #[test]
    fn the_engines_refusal_names_the_directories_to_leave_out() {
        let refusal = |fields: &[&str]| {
            let mut e = RpcError::invalid_params("bad options");
            e.data = Some(json!({"invalid_options": fields.iter().map(|f| json!({"field": f})).collect::<Vec<_>>()}));
            e
        };
        assert_eq!(refused_dirs(&refusal(&["skill_dirs[1].path", "skill_dirs[3].path"]), 5), Some(vec![1, 3]));
        assert_eq!(refused_dirs(&refusal(&["skill_dirs"]), 2), Some(vec![0, 1]));
        // Something else is wrong too: not ours to repair.
        assert_eq!(refused_dirs(&refusal(&["skill_dirs[0].path", "attended"]), 2), None);
        assert_eq!(refused_dirs(&RpcError::refused("no"), 2), None);
    }
}
