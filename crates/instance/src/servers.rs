//! MCP servers this process offers the daemons it spawns (a runtime application's tools).
//!
//! A runtime application runs in one VM, in this process. An instance built in this process reaches
//! its tools directly; a daemon this process spawned is another process, and reaches them over MCP
//! instead — an endpoint per application, served by this process. What goes into the daemon is a
//! set of `mcp_servers` entries in its global tier: written before it is spawned, and again (with
//! a `config.reload`) whenever the set changes — an application enabled, disabled, installed or
//! removed — or this process learns where it listens.
//!
//! This is the spawned carrier's own hook, and so the instance layer's: which files a daemon's
//! data root holds is not the hub's business, and an instance built in this process, or a daemon
//! somebody else runs, has none. [`SharedServers`] is the set, and the daemons configured with it.
//!
//! **Each daemon process presents a credential of its own.** [`ServerTokens`] issues one before the
//! process is spawned, written into each entry as `Authorization: Bearer …`; when the process goes,
//! that credential is revoked, and the file it was written into opens nothing any more. Going is any
//! way out: its instance stopped or restarted (a new link, a new credential), its link down for
//! good, or the process exited and its link about to start it again — the process started then
//! reads a new credential, written over the old one before it is spawned.
//!
//! Written the way inherited providers are (`inherit.rs`): Harnix records which entries it wrote,
//! in `<data root>/.harnix-servers.json`, and only ever adds, changes or removes those. An entry
//! the instance configures itself under the same name is the instance's, and stays.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use harnix_contract::sync::lock;
use serde_json::{json, Map, Value};

use crate::daemon_link::DaemonLink;
use crate::inherit::{read, write_private};
use crate::link::{EngineLink, LinkStatus};

/// The record of what was written, beside the instance's own settings.
pub(crate) const RECORD: &str = ".harnix-servers.json";

/// How long a daemon whose settings changed is waited for before it is asked to reload them.
const READY_WAIT: Duration = Duration::from_secs(60);

/// Who mints the credential a spawned daemon presents to the servers, and forgets it when the
/// daemon goes: whoever serves them.
pub trait ServerTokens: Send + Sync {
    /// A credential for one daemon, admitted from now on.
    fn issue(&self) -> String;
    /// That daemon has gone: its credential admits nothing any more.
    fn revoke(&self, token: &str);
}

/// The servers every daemon this process spawns is configured with, and the daemons configured.
#[derive(Default)]
pub struct SharedServers {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    servers: Map<String, Value>,
    tokens: Option<Arc<dyn ServerTokens>>,
    daemons: Vec<Weak<Configured>>,
}

impl SharedServers {
    /// Who issues each daemon's credential. Set before the first daemon is spawned; one spawned
    /// without it presents none.
    pub fn issue_with(&self, tokens: Arc<dyn ServerTokens>) {
        lock(&self.inner).tokens = Some(tokens);
    }

    /// Set the servers, and bring every daemon configured with them in line: each one whose
    /// settings changed reloads them once it is ready. Answers without waiting for the reloads.
    pub fn set(&self, servers: Map<String, Value>) {
        let changed: Vec<Arc<Configured>> = {
            let mut inner = lock(&self.inner);
            inner.servers = servers;
            inner.daemons.retain(|d| d.strong_count() > 0);
            // Under the lock, so a daemon configured meanwhile is written this set or a later one,
            // never an earlier one after it.
            let live: Vec<Arc<Configured>> = inner.daemons.iter().filter_map(Weak::upgrade).collect();
            live.into_iter()
                .filter(|d| !d.revoked.load(Ordering::Acquire) && d.write(&inner.servers))
                .collect()
        };
        for daemon in changed {
            daemon.reload();
        }
    }

    /// One daemon about to be spawned with its data in `root`: its credential issued, and the
    /// servers written into its settings before it reads them.
    pub(crate) fn configure(self: &Arc<Self>, instance: &str, root: &Path) -> Arc<Configured> {
        let mut inner = lock(&self.inner);
        let configured = Arc::new(Configured {
            instance: instance.to_string(),
            root: root.to_path_buf(),
            token: Mutex::new(inner.tokens.as_ref().map(|t| t.issue())),
            tokens: inner.tokens.clone(),
            shared: Arc::downgrade(self),
            revoked: AtomicBool::new(false),
            link: OnceLock::new(),
        });
        configured.write(&inner.servers);
        inner.daemons.retain(|d| d.strong_count() > 0);
        inner.daemons.push(Arc::downgrade(&configured));
        configured
    }
}

/// One spawned daemon's part of [`SharedServers`]: where its settings are, the credential its
/// process presents, and its link, to reload through. Held by the link; the credential is revoked
/// when the process goes ([`Configured::renew`] when the link starts another), and at the latest
/// when this is dropped.
pub(crate) struct Configured {
    instance: String,
    root: PathBuf,
    /// The credential of the process running now, or about to be spawned.
    token: Mutex<Option<String>>,
    tokens: Option<Arc<dyn ServerTokens>>,
    /// The set, read again when the credential is renewed.
    shared: Weak<SharedServers>,
    revoked: AtomicBool,
    link: OnceLock<Weak<DaemonLink>>,
}

impl std::fmt::Debug for Configured {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Configured").field("instance", &self.instance).finish_non_exhaustive()
    }
}

impl Configured {
    pub(crate) fn attach(&self, link: &Arc<DaemonLink>) {
        let _ = self.link.set(Arc::downgrade(link));
    }

    /// `servers`, each presenting this daemon's credential.
    fn entries(&self, servers: &Map<String, Value>) -> Map<String, Value> {
        let token = lock(&self.token).clone();
        servers
            .iter()
            .map(|(name, config)| {
                let mut config = config.clone();
                if let (Some(token), Some(entry)) = (&token, config.as_object_mut()) {
                    let headers = entry.entry("headers").or_insert_with(|| json!({}));
                    if let Some(headers) = headers.as_object_mut() {
                        headers.insert("Authorization".into(), json!(format!("Bearer {token}")));
                    }
                }
                (name.clone(), config)
            })
            .collect()
    }

    /// Bring the settings in line; `true` when they changed.
    fn write(&self, servers: &Map<String, Value>) -> bool {
        match merge(&self.root, &self.entries(servers)) {
            Ok(changed) => changed,
            Err(e) => {
                tracing::warn!(instance = %self.instance, error = %e, "the shared MCP servers were not written");
                false
            }
        }
    }

    /// A daemon still starting cannot take the call, and reads the file when it is up only if it
    /// was not already past reading it: wait until it is ready, and ask.
    fn reload(self: &Arc<Self>) {
        let Some(link) = self.link.get().and_then(Weak::upgrade) else { return };
        let instance = self.instance.clone();
        tokio::spawn(async move {
            let mut status = link.status();
            let settled = status.wait_for(|s| s.is_ready() || matches!(s, LinkStatus::Down { .. }));
            let ready = matches!(tokio::time::timeout(READY_WAIT, settled).await, Ok(Ok(s)) if s.is_ready());
            if !ready {
                tracing::warn!(%instance, "shared MCP servers written; the instance was not ready to reload them");
                return;
            }
            if let Err(e) = link.call("config.reload", json!({})).await {
                tracing::warn!(%instance, error = %e.message, "shared MCP servers written, not reloaded");
            }
        });
    }

    /// The daemon has gone, and its link with it: its credential admits nothing any more. Once.
    pub(crate) fn revoke(&self) {
        if self.revoked.swap(true, Ordering::AcqRel) {
            return;
        }
        // Under the credential's lock, after the flag: a renewal that got there first is revoked
        // here too, and one that comes after sees the flag.
        if let (Some(tokens), Some(token)) = (&self.tokens, lock(&self.token).take()) {
            tokens.revoke(&token);
        }
    }

    /// The daemon's process exited and its link is about to start another: the one that exited
    /// presented a credential that admits nothing any more, and the one about to start reads a
    /// new one, written into its settings before it is spawned.
    pub(crate) fn renew(&self) {
        let (Some(tokens), Some(shared)) = (&self.tokens, self.shared.upgrade()) else {
            return;
        };
        // In the order `SharedServers::set` takes them, so a change of the set meanwhile is
        // written with one credential or the other, never the old one after this.
        let inner = lock(&shared.inner);
        {
            let mut token = lock(&self.token);
            if self.revoked.load(Ordering::Acquire) {
                return;
            }
            if let Some(old) = token.replace(tokens.issue()) {
                tokens.revoke(&old);
            }
        }
        self.write(&inner.servers);
    }
}

impl Drop for Configured {
    fn drop(&mut self) {
        self.revoke();
    }
}

/// Bring one instance's `mcp_servers` in line with `servers`. `Ok(true)` when it changed.
pub(crate) fn merge(instance_root: &Path, servers: &Map<String, Value>) -> std::io::Result<bool> {
    let settings_path = instance_root.join("settings.json");
    let record_path = instance_root.join(RECORD);
    let mut settings = read(&settings_path)?;
    let record = read(&record_path)?;
    let owned: Vec<String> = record
        .get("mcp_servers")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    if servers.is_empty() && owned.is_empty() {
        return Ok(false);
    }

    let before = settings.clone();
    if !settings.is_object() {
        settings = json!({});
    }
    if !settings.get("mcp_servers").is_some_and(Value::is_object) {
        settings["mcp_servers"] = json!({});
    }
    let entries = settings["mcp_servers"].as_object_mut().expect("an object");
    for name in &owned {
        if !servers.contains_key(name) {
            entries.remove(name);
        }
    }
    let mut now_owned = Vec::new();
    for (name, config) in servers {
        if entries.contains_key(name) && !owned.contains(name) {
            tracing::warn!(server = %name, "the instance configures an MCP server of this name itself; left as it is");
            continue;
        }
        entries.insert(name.clone(), config.clone());
        now_owned.push(name.clone());
    }
    // An empty section says nothing an absent one does not.
    if entries.is_empty() {
        settings.as_object_mut().expect("an object").remove("mcp_servers");
    }

    let changed = settings != before;
    if changed {
        write_private(&settings_path, &settings)?;
    }
    write_private(&record_path, &json!({"mcp_servers": now_owned}))?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(root: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(root.join("settings.json")).unwrap()).unwrap()
    }

    fn servers(names: &[&str]) -> Map<String, Value> {
        names
            .iter()
            .map(|n| (n.to_string(), json!({"type": "remote", "url": format!("x/{n}")})))
            .collect()
    }

    #[test]
    fn written_entries_follow_the_set_and_the_instances_own_stay() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("settings.json"),
            json!({"memory_enabled": true, "mcp_servers": {"mine": {"type": "stdio", "command": "x"}, "taken": {"type": "stdio", "command": "own"}}})
                .to_string(),
        )
        .unwrap();
        assert!(merge(root, &servers(&["app-a", "taken"])).unwrap());
        let s = settings(root);
        assert_eq!(s["mcp_servers"]["app-a"]["url"], json!("x/app-a"));
        assert_eq!(s["mcp_servers"]["taken"]["command"], json!("own"), "the instance's own entry was replaced");
        assert!(s["mcp_servers"].get("mine").is_some());
        assert_eq!(s["memory_enabled"], json!(true));
        assert!(!merge(root, &servers(&["app-a", "taken"])).unwrap(), "nothing changed, and it changed");

        // Withdrawn: gone, and only what was written.
        assert!(merge(root, &servers(&[])).unwrap());
        let s = settings(root);
        assert!(s["mcp_servers"].get("app-a").is_none());
        assert!(s["mcp_servers"].get("mine").is_some() && s["mcp_servers"].get("taken").is_some());
    }

    #[test]
    fn nothing_to_offer_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!merge(dir.path(), &Map::new()).unwrap());
        assert!(!dir.path().join("settings.json").exists());
    }

    /// Tokens issued and revoked, as the serving side would keep them.
    #[derive(Default)]
    struct Kept {
        live: Mutex<Vec<String>>,
        next: std::sync::atomic::AtomicU32,
    }

    impl ServerTokens for Kept {
        fn issue(&self) -> String {
            let token = format!("t{}", self.next.fetch_add(1, Ordering::Relaxed));
            lock(&self.live).push(token.clone());
            token
        }
        fn revoke(&self, token: &str) {
            lock(&self.live).retain(|t| t != token);
        }
    }

    fn bearer(root: &Path, server: &str) -> Value {
        settings(root)["mcp_servers"][server]["headers"]["Authorization"].clone()
    }

    #[test]
    fn each_daemon_presents_its_own_credential_until_it_goes() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        let kept = Arc::new(Kept::default());
        let shared = Arc::new(SharedServers::default());
        shared.issue_with(kept.clone());
        shared.set(servers(&["app-x"]));

        // Written before the daemon reads them, each with a credential of its own.
        let first = shared.configure("a", &a);
        let second = shared.configure("b", &b);
        assert_eq!(bearer(&a, "app-x"), json!("Bearer t0"));
        assert_eq!(bearer(&b, "app-x"), json!("Bearer t1"));
        assert_eq!(settings(&a)["mcp_servers"]["app-x"]["url"], json!("x/app-x"));
        assert_eq!(*lock(&kept.live), ["t0", "t1"]);

        // A change reaches every daemon still there, with its own credential.
        shared.set(servers(&["app-x", "app-y"]));
        assert_eq!(bearer(&a, "app-y"), json!("Bearer t0"));
        assert_eq!(bearer(&b, "app-y"), json!("Bearer t1"));

        // One goes (stopped, down, or its link dropped): its credential admits nothing, its
        // settings are left alone, and the other's are kept in line.
        first.revoke();
        first.revoke();
        assert_eq!(*lock(&kept.live), ["t1"]);
        shared.set(servers(&["app-y"]));
        assert!(settings(&a)["mcp_servers"].get("app-x").is_some(), "a daemon that went was written to");
        assert!(settings(&b)["mcp_servers"].get("app-x").is_none());
        drop(second);
        assert!(lock(&kept.live).is_empty(), "dropped, and its credential still admitted");
    }

    /// A daemon whose process exited and is started again by its link: the process that exited
    /// presented a credential that admits nothing now, and the one started reads a new one. Gone
    /// for good afterwards, the new one is revoked too, and a renewal then issues nothing.
    #[test]
    fn a_daemon_started_again_presents_a_new_credential() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("a");
        let kept = Arc::new(Kept::default());
        let shared = Arc::new(SharedServers::default());
        shared.issue_with(kept.clone());
        shared.set(servers(&["app-x"]));
        let daemon = shared.configure("a", &root);
        assert_eq!(bearer(&root, "app-x"), json!("Bearer t0"));

        daemon.renew();
        assert_eq!(bearer(&root, "app-x"), json!("Bearer t1"), "the process started again reads the old credential");
        assert_eq!(*lock(&kept.live), ["t1"], "the exited process's credential still admits");
        // A change of the set afterwards carries the new one.
        shared.set(servers(&["app-x", "app-y"]));
        assert_eq!(bearer(&root, "app-y"), json!("Bearer t1"));

        daemon.revoke();
        assert!(lock(&kept.live).is_empty());
        daemon.renew();
        assert!(lock(&kept.live).is_empty(), "a daemon gone for good was issued a credential");
        assert_eq!(bearer(&root, "app-x"), json!("Bearer t1"), "a daemon gone for good was written to");
    }

    #[test]
    fn a_fresh_instance_gets_the_section_and_loses_it_again() {
        let dir = tempfile::tempdir().unwrap();
        assert!(merge(dir.path(), &servers(&["app-a"])).unwrap());
        assert!(settings(dir.path())["mcp_servers"].get("app-a").is_some());
        assert!(merge(dir.path(), &Map::new()).unwrap());
        assert!(settings(dir.path()).get("mcp_servers").is_none());
    }
}
