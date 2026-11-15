//! Recording, and replay: what a session is created and resumed with — the [`ReplayLink`] every
//! link this process builds is wrapped in.
//!
//! Recording is on by default, and it is this layer that turns it on, on the way into the
//! engine, because every way a runnable session comes into being — `session.create`, and the
//! `session.resume` behind opening a past one — goes through a link the instance layer built.
//!
//! Without a recording there is no answer to "what did the model actually see", and that
//! question is not optional (concept §6.4). The root is this layer's: given as `recorder.dir`, so
//! it is known where to remove a recording from. It is the `recorder.root` the engine's effective
//! settings name, asked of it over the link when a session is created or resumed (and again after
//! its settings were reloaded or the link reconnected), or the instance's `recordings` directory
//! when it names none.
//!
//! # Which mode is not a caller's decision
//!
//! A caller cannot ask for `replay`, or for a different root (`recorder.dir`): the key is
//! overwritten, not defaulted. Replay is a **process-level** decision, made by whoever started this one — the
//! same kind of decision as which engine root to serve, and for the same reason.
//!
//! When the operator has made it, and only then, a caller may name *which* recording to play.
//! That is not a loosening: outside replay the name means nothing, and inside it, naming the
//! fixture is the entire point.
//!
//! # Deleting a session deletes its recording
//!
//! The recorder keeps no retention policy and says so — a recording is a self-contained directory
//! for whoever owns the session to remove, and deleting the session is that moment. Closing is
//! not: the conversation can be reopened, and what the model saw should still be there. What went
//! with the session — its sidechains and forks, which the engine deleted too — loses its recording
//! with it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use harnix_contract::sync::lock;
use harnix_contract::RpcError;
use harnix_engine_api::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_api::ops::EngineOps;
use harnix_engine_api::SceneLayers;
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::daemon_link::DaemonLink;
use crate::gate::{GatedOps, WakeGate};
use crate::translate::{self, AgixOps, Rpc};

/// The process's recording: record, or play back from a directory. One for every instance.
#[derive(Default)]
pub struct Recording {
    /// Where recordings are played back from, when this process was started in replay. `None`
    /// is the ordinary case: record, never play.
    replay_from: Option<PathBuf>,
    /// Replay fails a call whose request differs from the recorded one (ipdrun-45).
    strict: AtomicBool,
    /// In replay, which recording each session was created to play, by `(instance, session)`.
    /// The engine keeps none across a restart of its own (Agix: `recorder` is not persisted), so
    /// a session resumed in a daemon that came back would otherwise call the model for real.
    named: Mutex<HashMap<(String, String), String>>,
}

impl Recording {
    pub fn new(replay_from: Option<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            replay_from,
            ..Self::default()
        })
    }

    /// In replay, fail a call whose request differs from the recorded one rather than report it
    /// (ipdrun-45). Off by default; meaningless outside replay. Set once, at startup, by whoever
    /// started the process in replay — the same operator's decision as replay itself.
    pub fn set_strict(&self, strict: bool) {
        self.strict.store(strict, Ordering::Relaxed);
    }

    /// Where recordings are played back from, when this process was started in replay: an engine
    /// in another process reads them there.
    pub fn replay_from(&self) -> Option<&std::path::Path> {
        self.replay_from.as_deref()
    }

    /// What one instance's link records with. `root` is where its sessions' recordings go when its
    /// engine's settings name no `recorder.root`: given to the engine as `recorder.dir`, and where
    /// a deleted session's are removed from. `None` for a daemon that is somebody else's, which
    /// keeps them where it likes and has none removed.
    pub fn of(self: &Arc<Self>, instance: &str, root: Option<PathBuf>) -> Recorder {
        Recorder {
            instance: instance.to_string(),
            recording: self.clone(),
            fallback: root.map(|r| std::path::absolute(&r).unwrap_or(r)),
            configured: Arc::default(),
        }
    }
}

/// The `recorder.root` an engine's settings name: asked of it once, kept until [`RootCache::forget`].
#[derive(Default)]
pub(crate) struct RootCache(Mutex<Option<Option<PathBuf>>>);

impl RootCache {
    pub(crate) fn forget(&self) {
        *lock(&self.0) = None;
    }

    fn get(&self) -> Option<Option<PathBuf>> {
        lock(&self.0).clone()
    }

    fn set(&self, root: Option<PathBuf>) {
        *lock(&self.0) = Some(root);
    }
}

/// One instance's side of [`Recording`]: what a link applies to every call it makes.
pub struct Recorder {
    instance: String,
    recording: Arc<Recording>,
    /// Where recordings go when the engine's settings name no root.
    fallback: Option<PathBuf>,
    /// What the engine's settings name; the link's, once the recorder is put on one.
    configured: Arc<RootCache>,
}

impl Recorder {
    /// A call as the engine is to see it: a session's creation or resumption with the recorder
    /// options, anything else as it is. Also the recording a creation names, to remember once it
    /// answers.
    pub fn prepare(&self, method: &str, params: Value, root: Option<&Path>) -> (Value, Option<String>) {
        let mut params = params;
        match method {
            translate::SESSION_CREATE => {
                let named = named(&params);
                (self.options(params, root), named)
            }
            translate::SESSION_RESUME => {
                let session = params.get("session_id").and_then(Value::as_str).map(str::to_string);
                let remembered = session.and_then(|s| lock(&self.recording.named).get(&(self.instance.clone(), s)).cloned());
                if let (Some(name), Some(obj)) = (remembered, params.as_object_mut()) {
                    let options = obj.entry("options").or_insert_with(|| json!({}));
                    if let Some(options) = options.as_object_mut() {
                        options.insert("recorder".into(), json!({"name": name}));
                    }
                }
                // A resumed session records too, or its next turn would be the one turn with no
                // envelope. Nothing is written until it makes a call, so reopening a
                // conversation just to read it leaves the previous run's recording intact.
                (self.options(params, root), None)
            }
            _ => (params, None),
        }
    }

    /// Where the engine is to record now: what its settings name, else this instance's own
    /// directory; `None` when the daemon is somebody else's.
    async fn root(&self, rpc: &dyn Rpc) -> Option<PathBuf> {
        let fallback = self.fallback.as_ref()?;
        if self.recording.replay_from.is_some() {
            return Some(fallback.clone());
        }
        let configured = match self.configured.get() {
            Some(known) => known,
            None => match rpc.call(translate::CONFIG_GET, json!({"tier": "effective"})).await {
                Ok(answer) => {
                    let root = answer
                        .pointer("/settings/recorder/root")
                        .and_then(Value::as_str)
                        .map(PathBuf::from)
                        .filter(|p| p.is_absolute());
                    self.configured.set(root.clone());
                    root
                }
                // Not asked again until it answers: the default stands meanwhile.
                Err(e) => {
                    tracing::warn!(instance = %self.instance, error = %e.message, "the recorder root was not read from the engine's settings");
                    None
                }
            },
        };
        Some(configured.unwrap_or_else(|| fallback.clone()))
    }

    /// Every directory a session's recording may be in: where it records now, and where it would
    /// have without a root in the settings — it was created before the setting changed, perhaps.
    fn roots(&self, now: Option<PathBuf>) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = now.into_iter().collect();
        if let Some(fallback) = &self.fallback {
            if !roots.contains(fallback) {
                roots.push(fallback.clone());
            }
        }
        roots
    }

    /// The engine answered a call [`Recorder::prepare`] named a recording for: in replay, the
    /// new session plays that one, now and when it is resumed.
    pub fn answered(&self, named: Option<String>, answer: &Result<Value, RpcError>) {
        let (Some(name), Some(_), Ok(created)) = (named, &self.recording.replay_from, answer) else {
            return;
        };
        if let Some(sid) = created.get("session_id").and_then(Value::as_str) {
            lock(&self.recording.named).insert((self.instance.clone(), sid.to_string()), name);
        }
    }

    fn options(&self, params: Value, root: Option<&Path>) -> Value {
        with_recording(params, self.recording.replay_from.as_deref(), self.recording.strict.load(Ordering::Relaxed), root)
    }

    /// A deleted session's recording goes with it, and so do those of what went with it — its
    /// sidechains and forks, which the engine deleted too. A fork records under its own id; a
    /// sidechain has none of its own, so there is nothing to remove for it. Nothing is removed for
    /// a call that only asked what would go, or failed.
    fn deleted(&self, method: &str, params: &Value, answer: &Result<Value, RpcError>, roots: &[PathBuf]) {
        let dry_run = params.get("dry_run").and_then(Value::as_bool) == Some(true);
        let Ok(deleted) = answer else { return };
        if method != translate::SESSION_DELETE || dry_run || roots.is_empty() {
            return;
        }
        let sidechains = deleted.get("sidechain_ids").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
        let ids = params.get("session_id").and_then(Value::as_str).into_iter().chain(sidechains);
        for id in ids.filter(|id| one_segment(id)) {
            for root in roots {
                let dir = root.join(id);
                if dir.is_dir() {
                    if let Err(e) = std::fs::remove_dir_all(&dir) {
                        tracing::warn!(dir = %dir.display(), error = %e, "recording not removed");
                    }
                }
            }
        }
    }
}

/// Whether `id` is one path segment: joined to a root, it cannot leave it.
fn one_segment(id: &str) -> bool {
    !id.is_empty() && id != "." && id != ".." && !id.contains(['/', '\\', '\0'])
}

/// A link's calls, through its recorder.
struct Recorded {
    rpc: Arc<dyn Rpc>,
    recorder: Recorder,
}

#[async_trait::async_trait]
impl Rpc for Recorded {
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let deleting = (method == translate::SESSION_DELETE).then(|| params.clone());
        let root = match method {
            translate::SESSION_CREATE | translate::SESSION_RESUME | translate::SESSION_DELETE => self.recorder.root(&*self.rpc).await,
            _ => None,
        };
        let (params, named) = self.recorder.prepare(method, params, root.as_deref());
        let answer = self.rpc.call(method, params).await;
        self.recorder.answered(named, &answer);
        if let Some(params) = deleting {
            self.recorder.deleted(method, &params, &answer, &self.recorder.roots(root));
        }
        answer
    }
}

/// A link that records — or, in replay, plays back — every session it creates and resumes: a
/// [`DaemonLink`], with its calls through a [`Recorder`].
pub struct ReplayLink {
    inner: Arc<DaemonLink>,
    ops: GatedOps,
}

impl ReplayLink {
    /// `link`, its sessions recorded by `recorder`. A gated daemon's link keeps its gate: a call
    /// through the recorder wakes a daemon let go while idle, as one straight to the link would.
    pub fn daemon(link: Arc<DaemonLink>, mut recorder: Recorder) -> Arc<Self> {
        // What the link forgets when the engine's settings may have changed is what this reads.
        recorder.configured = link.recorder_root();
        let gate: Option<Arc<dyn WakeGate>> = link.gate();
        let rpc = link.rpc();
        Arc::new(Self { inner: link, ops: GatedOps { agix: AgixOps(Arc::new(Recorded { rpc, recorder })), gate } })
    }
}

#[async_trait::async_trait]
impl EngineControl for ReplayLink {
    fn status(&self) -> watch::Receiver<LinkStatus> {
        self.inner.status()
    }

    fn identity(&self) -> EngineIdentity {
        self.inner.identity()
    }

    fn carrier(&self) -> InstanceCarrier {
        self.inner.carrier()
    }

    fn scene_layers(&self) -> SceneLayers {
        self.inner.scene_layers()
    }

    async fn reload_settings(&self) -> Result<(), RpcError> {
        self.inner.reload_settings().await
    }

    async fn shutdown(&self) {
        self.inner.shutdown().await
    }
}

#[async_trait::async_trait]
impl EngineLink for ReplayLink {
    fn ops(&self) -> &dyn EngineOps {
        &self.ops
    }

    /// The link's own stream: wrapping it is not taking it.
    fn take_events(&self) -> Option<EventReceiver> {
        self.inner.events()
    }

    async fn mark(&self, id: u64) {
        self.inner.mark(id).await
    }
}

/// The recording a call names, if any.
fn named(params: &Value) -> Option<String> {
    params
        .get("options")
        .and_then(|o| o.get("recorder"))
        .and_then(|r| r.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// `params` with the recorder options this process decides: record into `root` (the engine's own
/// when `None`), or replay from `replay_from` (by the name the call gave, if any).
fn with_recording(mut params: Value, replay_from: Option<&Path>, strict: bool, root: Option<&Path>) -> Value {
    let recorder = match replay_from {
        Some(dir) => json!({
            "mode": "replay",
            "name": named(&params),
            "dir": dir.display().to_string(),
            // Divergences are reported, not fatal, unless the operator
            // asked for strict replay (`--replay-strict`, ipdrun-45).
            //
            // Strict replay compares the live request against the recorded
            // one. The prompt carries a date and absolute paths: the engine
            // now dates a replaying session by its recording (Agix ipdrun-06),
            // but paths are the application's to keep stable — fixed roots, or
            // workspace-relative tool paths. Until an application has done
            // that, strictness would only be a test that fails on another
            // machine, so it stays the operator's choice.
            "strict": strict,
        }),
        // Whatever else the caller put in `recorder` is gone with it: a name means nothing
        // outside replay, and the root is not the caller's.
        None => match root {
            Some(root) => json!({"mode": "record", "dir": root.display().to_string()}),
            None => json!({"mode": "record"}),
        },
    };
    let options = params
        .as_object_mut()
        .map(|obj| obj.entry("options").or_insert_with(|| json!({})));
    if let Some(Value::Object(options)) = options {
        options.insert("recorder".into(), recorder);
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_is_lenient_unless_the_operator_asked_for_strict() {
        let dir = std::path::Path::new("/rec");
        let named = json!({"options": {"recorder": {"name": "fixture", "mode": "record", "strict": true}}});
        let lenient = with_recording(named.clone(), Some(dir), false, None);
        assert_eq!(
            lenient["options"]["recorder"],
            json!({"mode": "replay", "name": "fixture", "dir": "/rec", "strict": false}),
            "a caller's own mode and strictness are overwritten"
        );
        let strict = with_recording(named.clone(), Some(dir), true, Some(Path::new("/r")));
        assert_eq!(strict["options"]["recorder"]["strict"], true);
        assert_eq!(strict["options"]["recorder"]["dir"], "/rec", "in replay `dir` is where the recordings are played from");
        assert_eq!(
            with_recording(named, None, true, None)["options"]["recorder"],
            json!({"mode": "record"}),
            "outside replay strictness means nothing"
        );
    }

    /// In replay, a session resumed after its engine came back plays the recording it was
    /// created to play.
    #[test]
    fn a_resumed_session_replays_what_it_was_created_with() {
        let recording = Recording::new(Some(PathBuf::from("/rec")));
        let recorder = recording.of("w", None);
        let (sent, named) = recorder.prepare("session.create", json!({"options": {"recorder": {"name": "fixture"}}}), None);
        assert_eq!(sent["options"]["recorder"]["mode"], "replay");
        recorder.answered(named, &Ok(json!({"session_id": "s1"})));
        let (resumed, _) = recorder.prepare("session.resume", json!({"session_id": "s1"}), None);
        assert_eq!(resumed["options"]["recorder"]["name"], "fixture");
        let (other, _) = recording.of("v", None).prepare("session.resume", json!({"session_id": "s1"}), None);
        assert_eq!(other["options"]["recorder"]["name"], Value::Null, "another instance's s1 is another session");
        let (passed, _) = recorder.prepare("session.get", json!({"session_id": "s1"}), None);
        assert_eq!(passed, json!({"session_id": "s1"}), "anything else goes as it is");
    }

    /// Outside replay a name is not remembered: it means nothing there. The root is this layer's,
    /// on creation and on resumption, and a caller's own is not honoured.
    #[test]
    fn outside_replay_every_session_records_in_the_root_this_layer_names() {
        let recording = Recording::new(None);
        let recorder = recording.of("w", Some(PathBuf::from("/r")));
        let root = Some(Path::new("/r"));
        let (sent, named) = recorder.prepare("session.create", json!({"options": {"recorder": {"name": "x", "dir": "/elsewhere"}}}), root);
        assert_eq!(sent["options"]["recorder"], json!({"mode": "record", "dir": "/r"}));
        recorder.answered(named, &Ok(json!({"session_id": "s1"})));
        assert!(lock(&recording.named).is_empty());
        let (resumed, _) = recorder.prepare("session.resume", json!({"session_id": "s1"}), root);
        assert_eq!(resumed["options"]["recorder"], json!({"mode": "record", "dir": "/r"}));
        let (none, _) = recorder.prepare("session.create", json!({}), None);
        assert_eq!(none["options"]["recorder"], json!({"mode": "record"}), "no root: the engine's own");
    }

    /// An engine answering by method, noting what it was asked.
    struct Engine {
        settings: Value,
        asked: Mutex<Vec<String>>,
        deleted: Value,
    }

    #[async_trait::async_trait]
    impl Rpc for Engine {
        async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
            lock(&self.asked).push(method.to_string());
            match method {
                translate::CONFIG_GET => Ok(json!({"settings": self.settings})),
                translate::SESSION_DELETE => Ok(self.deleted.clone()),
                _ => Ok(params),
            }
        }
    }

    fn engine(settings: Value) -> Arc<Engine> {
        Arc::new(Engine { settings, asked: Mutex::new(Vec::new()), deleted: json!({}) })
    }

    /// The settings' `recorder.root` is where sessions record, asked once and kept until
    /// forgotten; with none, the instance's own directory.
    #[tokio::test]
    async fn the_root_is_what_the_engines_settings_name_asked_once() {
        let configured = engine(json!({"recorder": {"mode": "record", "root": "/configured"}}));
        let rec = Recorded { rpc: configured.clone(), recorder: Recording::new(None).of("w", Some(PathBuf::from("/own"))) };
        let created = rec.call(translate::SESSION_CREATE, json!({})).await.unwrap();
        assert_eq!(created["options"]["recorder"]["dir"], "/configured");
        let resumed = rec.call(translate::SESSION_RESUME, json!({"session_id": "s1"})).await.unwrap();
        assert_eq!(resumed["options"]["recorder"]["dir"], "/configured");
        assert_eq!(lock(&configured.asked).iter().filter(|m| *m == translate::CONFIG_GET).count(), 1);
        rec.recorder.configured.forget();
        rec.call(translate::SESSION_CREATE, json!({})).await.unwrap();
        assert_eq!(lock(&configured.asked).iter().filter(|m| *m == translate::CONFIG_GET).count(), 2, "asked again once forgotten");

        let plain = Recorded { rpc: engine(json!({})), recorder: Recording::new(None).of("w", Some(PathBuf::from("/own"))) };
        let created = plain.call(translate::SESSION_CREATE, json!({})).await.unwrap();
        assert_eq!(created["options"]["recorder"]["dir"], "/own");

        let relative = Recorded { rpc: engine(json!({"recorder": {"root": "rel"}})), recorder: Recording::new(None).of("w", Some(PathBuf::from("/own"))) };
        let created = relative.call(translate::SESSION_CREATE, json!({})).await.unwrap();
        assert_eq!(created["options"]["recorder"]["dir"], "/own", "a root that is not absolute is not one");

        let theirs = engine(json!({"recorder": {"root": "/configured"}}));
        let somebody_elses = Recorded { rpc: theirs.clone(), recorder: Recording::new(None).of("w", None) };
        let created = somebody_elses.call(translate::SESSION_CREATE, json!({})).await.unwrap();
        assert_eq!(created["options"]["recorder"], json!({"mode": "record"}));
        assert!(lock(&theirs.asked).iter().all(|m| m != translate::CONFIG_GET), "a daemon that is not ours is not asked");
    }

    /// A resumed session's options are merged with the recorder's, not replaced: whatever else
    /// they say goes in with it.
    #[tokio::test]
    async fn the_recorder_merges_into_a_resumed_sessions_options() {
        let recording = Recording::new(Some(PathBuf::from("/rec")));
        let rec = Recorded { rpc: engine(json!({})), recorder: recording.of("w", None) };
        let created = rec.call(translate::SESSION_CREATE, json!({"options": {"attended": true, "recorder": {"name": "f"}}})).await.unwrap();
        rec.recorder.answered(Some("f".into()), &Ok(json!({"session_id": "s1"})));
        assert_eq!((&created["options"]["attended"], &created["options"]["recorder"]["mode"]), (&json!(true), &json!("replay")));
        let resumed = rec.call(translate::SESSION_RESUME, json!({"session_id": "s1", "options": {"attended": true}})).await.unwrap();
        assert_eq!(resumed["options"]["attended"], true);
        assert_eq!((&resumed["options"]["recorder"]["mode"], &resumed["options"]["recorder"]["name"]), (&json!("replay"), &json!("f")));
    }

    /// Deleting a session removes its recording, and those of its sidechains and forks that have
    /// one; asking what a delete would remove, or closing it, does not; an id that is not one
    /// path segment is not followed out of the root.
    #[tokio::test]
    async fn a_deleted_sessions_recording_goes_with_it_and_with_what_went_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("rec");
        let outside = dir.path().join("x");
        std::fs::create_dir_all(&outside).unwrap();
        let recorded = |root: Option<&Path>| Recorded {
            rpc: Arc::new(Engine { settings: json!({}), asked: Mutex::new(Vec::new()), deleted: json!({"sidechain_ids": ["f1", "../x"]}) }),
            recorder: Recording::new(None).of("w", root.map(Path::to_path_buf)),
        };
        let make = || {
            for id in ["s1", "f1", "s2"] {
                std::fs::create_dir_all(root.join(id)).unwrap();
            }
        };
        make();
        let rec = recorded(Some(&root));
        rec.call(translate::SESSION_CLOSE, json!({"session_id": "s1"})).await.unwrap();
        rec.call(translate::SESSION_DELETE, json!({"session_id": "s1", "dry_run": true})).await.unwrap();
        assert!(root.join("s1").is_dir() && root.join("f1").is_dir());
        rec.call(translate::SESSION_DELETE, json!({"session_id": "s1"})).await.unwrap();
        assert!(!root.join("s1").exists() && !root.join("f1").exists(), "the session's and its fork's");
        assert!(root.join("s2").is_dir() && outside.is_dir(), "nothing else, and nothing outside the root");

        make();
        recorded(None).call(translate::SESSION_DELETE, json!({"session_id": "s1"})).await.unwrap();
        assert!(root.join("s1").is_dir(), "a daemon that is somebody else's keeps its recordings");
    }
}
