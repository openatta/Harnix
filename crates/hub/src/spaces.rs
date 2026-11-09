//! Which space a session belongs to (concept §5.4).
//!
//! A session is bound to one space when it is created and never moves. The binding is Harnix's own
//! record — the engine knows nothing of spaces — kept in `.harnix/spaces.json` beside the instance
//! ledger, with the instance a space's template built for it. A session with no record belongs to
//! the session space: every one created before spaces existed, and every one a caller created
//! without saying.
//!
//! A space needs no declaration here: one with only an interface and an Agent takes the defaults
//! (`system`, the instance's scene). A module declares one to give it a default instance or scene.
//!
//! The same ledger records which sessions Harnix itself started — a module's or the kernel's,
//! rather than a person's at a device — so the system space can mark them.
//!
//! A call that claims a space (`params.space`) is checked against the record: a session of another
//! space is refused with `SPACE_MISMATCH`, naming the space it does belong to. The system space is
//! the one exception — it sees and acts on every space's sessions — and each time it reaches across
//! one, that is audited. This guards against a client's bug sending into the wrong product surface;
//! it is not a security boundary: the claim is the caller's own (concept §6.2).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use harnix_contract::sync::{lock, read, write};
use harnix_contract::{codes, DefaultInstance, RpcError, SpaceDecl, Subject, SESSIONS_SPACE, SYSTEM_SPACE};
use harnix_instance::Owner;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::instances::claimed_space;
use crate::{Hub, SessionKey, SYSTEM};

/// Told about every cross-space act of the system space: who, which method, which session, and
/// the space the session belongs to.
pub type AuditHook = Arc<dyn Fn(&Subject, &str, &str, &str) + Send + Sync>;

#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    /// `instance/session` → space.
    #[serde(default)]
    bindings: BTreeMap<String, String>,
    /// space → the instance built from its template.
    #[serde(default)]
    instances: BTreeMap<String, String>,
    /// `instance/session` of every session a module or the kernel started.
    #[serde(default)]
    harnix: BTreeSet<String>,
}

pub struct Spaces {
    decls: BTreeMap<String, SpaceDecl>,
    /// The ids in `decls` a runtime application added, which only it removes.
    runtime: BTreeSet<String>,
    system_enabled: bool,
    path: PathBuf,
    ledger: Ledger,
}

fn slot(key: &SessionKey) -> String {
    format!("{}/{}", key.0, key.1)
}

impl Spaces {
    pub fn load(path: PathBuf, decls: Vec<SpaceDecl>, system_enabled: bool) -> anyhow::Result<Self> {
        let ledger = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ledger::default(),
            Err(e) => return Err(anyhow::anyhow!("{}: {e}", path.display())),
        };
        let mut all = BTreeMap::new();
        for decl in [SpaceDecl::new(SESSIONS_SPACE)].into_iter().chain(decls)
        {
            all.insert(decl.id.clone(), decl);
        }
        Ok(Self {
            decls: all,
            runtime: BTreeSet::new(),
            system_enabled,
            path,
            ledger,
        })
    }

    /// Add a space declared at runtime. Refused for an id already declared
    /// (assembly's keep precedence), a built-in space's id, an id a call could
    /// not claim, and a default instance built from a template — a runtime
    /// application's space lands on an instance that exists.
    pub fn add_runtime(&mut self, decl: SpaceDecl) -> Result<(), String> {
        let id = decl.id.clone();
        let builtin = [SESSIONS_SPACE, harnix_contract::PROJECTS_SPACE, SYSTEM_SPACE, "flow"];
        if builtin.contains(&id.as_str()) {
            return Err(format!("`{id}` is a built-in space"));
        }
        if self.decls.contains_key(&id) {
            return Err(format!("there is already a space `{id}`"));
        }
        if !self.exists(&id) {
            return Err(format!("`{id}` cannot name a space"));
        }
        if matches!(decl.instance, DefaultInstance::Template(_)) {
            return Err(format!("the space `{id}` names a template; a runtime space lands on an instance that exists"));
        }
        self.runtime.insert(id.clone());
        self.decls.insert(id, decl);
        Ok(())
    }

    /// Remove a space added at runtime; `false` for any other. The sessions
    /// bound to it keep their binding: a space with no declaration takes the
    /// defaults, so they stay where they are and still list under it.
    pub fn remove_runtime(&mut self, id: &str) -> bool {
        if !self.runtime.remove(id) {
            return false;
        }
        self.decls.remove(id);
        true
    }

    fn save(&self) {
        let write = || -> anyhow::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = self.path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(&self.ledger)?)?;
            std::fs::rename(&tmp, &self.path)?;
            Ok(())
        };
        if let Err(e) = write() {
            tracing::warn!(error = %e, "the space ledger was not written");
        }
    }

    pub(crate) fn bound(&self, key: &SessionKey) -> &str {
        self.ledger
            .bindings
            .get(&slot(key))
            .map(String::as_str)
            .unwrap_or(SESSIONS_SPACE)
    }

    fn bind(&mut self, key: &SessionKey, space: &str) {
        if space == SESSIONS_SPACE {
            // The default needs no record, and keeping none keeps the ledger to what differs.
            if self.ledger.bindings.remove(&slot(key)).is_some() {
                self.save();
            }
            return;
        }
        self.ledger.bindings.insert(slot(key), space.to_string());
        self.save();
    }

    fn unbind(&mut self, key: &SessionKey) {
        let bound = self.ledger.bindings.remove(&slot(key)).is_some();
        let started = self.ledger.harnix.remove(&slot(key));
        if bound || started {
            self.save();
        }
    }

    fn mark_harnix(&mut self, key: &SessionKey) {
        if self.ledger.harnix.insert(slot(key)) {
            self.save();
        }
    }

    pub(crate) fn started_by_harnix(&self, key: &SessionKey) -> bool {
        self.ledger.harnix.contains(&slot(key))
    }

    /// Whether a call may claim this space. A space with no backend — only an interface and an
    /// Agent — is declared nowhere here and takes the defaults; the system space may be turned off.
    fn exists(&self, space: &str) -> bool {
        if space == SYSTEM_SPACE {
            return self.system_enabled;
        }
        !space.is_empty()
            && space.len() <= 64
            && space.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    }

    /// The instances a space's sessions can be on: its default, what its template built, and any
    /// instance one of its sessions is bound on.
    fn instances_of(&self, space: &str) -> BTreeSet<String> {
        let mut set = BTreeSet::new();
        if let Some(decl) = self.decls.get(space) {
            if let DefaultInstance::Fixed(id) = &decl.instance {
                set.insert(id.clone());
            }
        }
        if let Some(built) = self.ledger.instances.get(space) {
            set.insert(built.clone());
        }
        for (key, bound) in &self.ledger.bindings {
            if bound == space {
                if let Some((instance, _)) = key.split_once('/') {
                    set.insert(instance.to_string());
                }
            }
        }
        if space == SESSIONS_SPACE {
            set.insert(SYSTEM.to_string());
        }
        set
    }
}

/// Methods about one session that a space claim is checked on.
pub(crate) const CHECKED: &[&str] = &[
    "harnix.attach",
    "harnix.detach",
    "harnix.send",
    "harnix.queue.remove",
    "harnix.sessions.tasks",
    "session.get",
    "session.history",
    "session.entries",
    "harnix.entries.append",
    "session.interrupt",
    "session.close",
    "session.delete",
    "session.fork",
    "session.resume",
    "session.respondToPrompt",
];

impl Hub {
    /// Add a space at runtime (see [`Spaces::add_runtime`]); the
    /// `harnix.spaces` topic is pushed to whoever watches it.
    pub fn add_space(self: &Arc<Self>, decl: SpaceDecl) -> Result<(), String> {
        lock(&self.spaces).add_runtime(decl)?;
        self.refresh(crate::SPACES_TOPIC, json!({}));
        Ok(())
    }

    /// Remove a space added at runtime, and push `harnix.spaces`.
    pub fn remove_space(self: &Arc<Self>, id: &str) -> bool {
        let removed = lock(&self.spaces).remove_runtime(id);
        if removed {
            self.refresh(crate::SPACES_TOPIC, json!({}));
        }
        removed
    }

    pub fn set_audit(&self, hook: AuditHook) {
        *write(&self.audit) = Some(hook);
    }

    /// The space a call claims, which must be one this application has — the system space only
    /// while the profile keeps it (concept §5.5).
    pub(crate) fn claimed(&self, params: &Value) -> Result<String, RpcError> {
        let space = claimed_space(params);
        if !lock(&self.spaces).exists(&space) {
            return Err(RpcError::new(
                codes::SPACE_MISMATCH,
                format!("there is no space `{space}` in this application"),
            ));
        }
        Ok(space)
    }

    /// Check a session call's claimed space against the ledger.
    pub(crate) fn check_space(&self, subject: &Subject, method: &str, params: &Value) -> Result<(), RpcError> {
        let Some(claimed) = params.get("space").and_then(Value::as_str) else {
            return Ok(());
        };
        let spaces = lock(&self.spaces);
        if !spaces.exists(claimed) {
            return Err(RpcError::new(
                codes::SPACE_MISMATCH,
                format!("there is no space `{claimed}` in this application"),
            ));
        }
        let Ok(key) = self.key_of(params) else {
            return Ok(());
        };
        let bound = spaces.bound(&key).to_string();
        drop(spaces);
        if claimed == SYSTEM_SPACE {
            if bound != SYSTEM_SPACE {
                if let Some(hook) = read(&self.audit).as_ref() {
                    hook(subject, method, &format!("{}/{}", key.0, key.1), &bound);
                }
            }
            return Ok(());
        }
        if bound != claimed {
            return Err(RpcError {
                code: codes::SPACE_MISMATCH,
                message: format!(
                    "session `{}` on instance `{}` belongs to the space `{bound}`, not `{claimed}`",
                    key.1, key.0
                ),
                data: Some(json!({"space": bound, "instance": key.0, "session_id": key.1})),
            });
        }
        Ok(())
    }

    /// Where a new session in this space goes, and in what scene: the instance the caller named
    /// (if the space may use it), else the space's default — built from its template the first time.
    pub(crate) async fn place(self: &std::sync::Arc<Self>, params: &mut Value) -> Result<String, RpcError> {
        let space = claimed_space(params);
        let (decl, built) = {
            let spaces = lock(&self.spaces);
            if !spaces.exists(&space) {
                return Err(RpcError::new(codes::SPACE_MISMATCH, format!("there is no space `{space}` in this application")));
            }
            (spaces.decls.get(&space).cloned(), spaces.ledger.instances.get(&space).cloned())
        };
        if let (Some(scene), Some(obj)) = (decl.as_ref().and_then(|d| d.scene.clone()), params.as_object_mut()) {
            obj.entry("scene").or_insert(json!(scene));
        }
        if let Some(named) = params.get("instance").and_then(Value::as_str).map(str::to_string) {
            if let Some(slot) = self.slot(&named) {
                if let Owner::Space(owner) = &slot.owner {
                    if *owner != space && space != SYSTEM_SPACE {
                        return Err(RpcError::new(
                            codes::SPACE_MISMATCH,
                            format!("instance `{named}` belongs to the space `{owner}`, not `{space}`"),
                        ));
                    }
                }
            }
            return Ok(space);
        }
        let instance = match decl.map(|d| d.instance).unwrap_or_default() {
            DefaultInstance::Fixed(id) => id,
            DefaultInstance::Template(template) => match built.filter(|id| self.slot(id).is_some()) {
                Some(id) => id,
                None => {
                    let name = format!("space-{space}");
                    self.build_owned(&template, &name, Owner::Space(space.clone())).await?;
                    lock(&self.spaces).ledger.instances.insert(space.clone(), name.clone());
                    lock(&self.spaces).save();
                    name
                }
            },
        };
        if let Some(obj) = params.as_object_mut() {
            obj.insert("instance".into(), json!(instance));
        }
        Ok(space)
    }

    /// `harnix.sessions.move {instance, session_id, space}` (ipdrun-51): bind an existing session to
    /// another space, transcript and all — an application that reorganized its spaces moves the
    /// sessions it made instead of leaving them behind. The application's, not a device's: the
    /// space is where a product put a session, not where a client would like to see it. The target
    /// must be a space the session's instance may serve (as for a new session there); the system
    /// space is nobody's home.
    pub(crate) async fn move_session(self: &std::sync::Arc<Self>, subject: &Subject, params: Value) -> Result<Value, RpcError> {
        if matches!(subject, Subject::Device { .. }) {
            return Err(RpcError::new(
                codes::REFUSED,
                "which space a session belongs to is the application's to change, not a device's",
            ));
        }
        let key = self.key_of(&params)?;
        let space = params
            .get("space")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| RpcError::invalid_params("missing space"))?
            .to_string();
        {
            let spaces = lock(&self.spaces);
            if space == SYSTEM_SPACE || !spaces.exists(&space) {
                return Err(RpcError::new(codes::SPACE_MISMATCH, format!("there is no space `{space}` to move a session to")));
            }
        }
        if let Some(slot) = self.slot(&key.0) {
            if let Owner::Space(owner) = &slot.owner {
                if *owner != space {
                    return Err(RpcError::new(
                        codes::SPACE_MISMATCH,
                        format!("instance `{}` belongs to the space `{owner}`, not `{space}`", key.0),
                    ));
                }
            }
        }
        // Only a session there is: an unknown one is the engine's not-found, not a record of nothing.
        self.call(&key.0, "session.get", json!({"session_id": key.1})).await?;
        let from = self.space_of(&key);
        if from == space {
            return Ok(json!({"space": space, "from": from, "moved": false}));
        }
        self.bind_space(&key, &space);
        self.refresh("harnix.instances.sessions", json!({"instance": key.0}));
        Ok(json!({"space": space, "from": from, "moved": true}))
    }

    /// What the system space creates is its own.
    pub(crate) fn bind_space(&self, key: &SessionKey, space: &str) {
        lock(&self.spaces).bind(key, space);
    }

    /// Who started a new session: a device is a person; a module or the kernel is Harnix itself.
    pub(crate) fn note_starter(&self, key: &SessionKey, subject: &Subject) {
        if !matches!(subject, Subject::Device { .. }) {
            lock(&self.spaces).mark_harnix(key);
        }
    }

    /// `"harnix"` for a session a module or the kernel started, `"user"` otherwise.
    pub(crate) fn starter_of(&self, key: &SessionKey) -> &'static str {
        if lock(&self.spaces).started_by_harnix(key) {
            "harnix"
        } else {
            "user"
        }
    }

    pub(crate) fn unbind_space(&self, key: &SessionKey) {
        lock(&self.spaces).unbind(key);
    }

    pub(crate) fn space_of(&self, key: &SessionKey) -> String {
        lock(&self.spaces).bound(key).to_string()
    }

    /// `harnix.sessions { space }`: a space's sessions, across the instances it uses; the system
    /// space's is every session of every instance.
    pub(crate) async fn space_sessions(&self, space: &str) -> Result<Value, RpcError> {
        let instances: Vec<String> = if space == SYSTEM_SPACE {
            self.slots().iter().map(|s| s.id.clone()).collect()
        } else {
            lock(&self.spaces).instances_of(space).into_iter().collect()
        };
        let mut rows = Vec::new();
        for instance in instances {
            if self.slot(&instance).is_none() {
                continue;
            }
            let Ok(view) = self.sessions_view(&instance).await else {
                continue;
            };
            for row in view.get("sessions").and_then(Value::as_array).cloned().unwrap_or_default() {
                if space == SYSTEM_SPACE || row.get("space").and_then(Value::as_str) == Some(space) {
                    rows.push(row);
                }
            }
        }
        Ok(json!({"space": space, "sessions": rows}))
    }

    /// `harnix.spaces.list`: every space the backend knows, with where its sessions land.
    pub(crate) fn spaces_view(&self) -> Value {
        let spaces = lock(&self.spaces);
        let mut list: Vec<Value> = spaces
            .decls
            .values()
            .map(|d| {
                json!({
                    "id": d.id,
                    "instance": match &d.instance {
                        DefaultInstance::Fixed(id) => id.clone(),
                        DefaultInstance::Template(t) => format!("template:{t}"),
                    },
                    "built": spaces.ledger.instances.get(&d.id),
                    "scene": d.scene,
                })
            })
            .collect();
        if spaces.system_enabled {
            list.push(json!({"id": SYSTEM_SPACE, "instance": null, "built": null, "scene": null}));
        }
        json!({"spaces": list})
    }

    /// A referenced session's transcript, as a text attachment for a turn in another space
    /// (`@session`, concept §5.4): what crosses is a copy of what was said, not the session.
    pub(crate) async fn reference(&self, reference: &Value) -> Result<Value, RpcError> {
        let key = self.key_of(reference)?;
        let info = self.call(&key.0, "session.get", json!({"session_id": key.1})).await?;
        let page = self
            .call(&key.0, "session.history", json!({"session_id": key.1, "offset": 0, "limit": 200}))
            .await?;
        let mut text = String::new();
        for message in page.get("messages").and_then(Value::as_array).into_iter().flatten() {
            let role = message.get("role").and_then(Value::as_str).unwrap_or("?");
            let body = message_text(message);
            if body.trim().is_empty() {
                continue;
            }
            text.push_str(&format!("[{role}] {body}\n\n"));
        }
        const LIMIT: usize = 40_000;
        if text.len() > LIMIT {
            let cut = text.len() - LIMIT;
            let start = (cut..text.len()).find(|i| text.is_char_boundary(*i)).unwrap_or(text.len());
            text = format!("…\n{}", &text[start..]);
        }
        let name = info.get("name").and_then(Value::as_str).unwrap_or(&key.1);
        Ok(json!({
            "kind": "text",
            "path": format!("session {name} ({})", self.space_of(&key)),
            "content": text,
        }))
    }
}

/// The text of one transcript message, whether its content is a string or blocks.
fn message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spaces(dir: &std::path::Path) -> Spaces {
        Spaces::load(dir.join("spaces.json"), vec![SpaceDecl::new("fleet")], true).unwrap()
    }

    #[test]
    fn a_runtime_space_is_added_and_removed_and_assemblys_stay() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = spaces(dir.path());
        s.add_runtime(SpaceDecl::new("fake").scene("mini")).unwrap();
        assert!(s.decls.contains_key("fake"));
        for taken in ["fleet", "sessions", "projects", "system", "flow", "fake"] {
            assert!(s.add_runtime(SpaceDecl::new(taken)).is_err(), "{taken} was added");
        }
        assert!(s.add_runtime(SpaceDecl::new("bad id")).is_err());
        assert!(s.add_runtime(SpaceDecl::new("templated").instance("template:fleet.side")).is_err());
        // Only what was added at runtime can be removed.
        assert!(!s.remove_runtime("fleet"));
        assert!(!s.remove_runtime("sessions"));
        assert!(s.remove_runtime("fake"));
        assert!(!s.remove_runtime("fake"));
        assert!(!s.decls.contains_key("fake"));
        assert!(s.decls.contains_key("fleet"));
    }
}
