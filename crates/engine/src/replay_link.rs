//! Recording, and replay: what a session is created and resumed with — the [`ReplayLink`] every
//! link this process builds is wrapped in. Its scene layers' presets ([`Presets`]) are applied on
//! the same way in, first: below the hub, which refuses a caller's say in what they fill.
//!
//! Recording is on by default, and it is this layer that turns it on, on the way into the
//! engine, because every way a runnable session comes into being — `session.create`, and the
//! `session.resume` behind opening a past one — goes through a link the instance layer built.
//!
//! Without a recording there is no answer to "what did the model actually see", and that
//! question is not optional (concept §6.4). No `dir` is named: the engine resolves the root from
//! settings and the reader resolves the same way, so there is one answer rather than two.
//!
//! # Which mode is not a caller's decision
//!
//! A caller cannot ask for `replay`, or for a different root: the key is overwritten, not
//! defaulted. Replay is a **process-level** decision, made by whoever started this one — the
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
//! not: the conversation can be reopened, and what the model saw should still be there.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use harnix_contract::sync::lock;
use harnix_contract::RpcError;
use harnix_engine_api::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_api::ops::EngineOps;
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::daemon_link::DaemonLink;
use crate::gate::{GatedOps, WakeGate};
use crate::presets::Presets;
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

    /// What one instance's link records with. `recordings` is where its engine writes them, when
    /// this process can say: a deleted session's is removed from there.
    pub fn of(self: &Arc<Self>, instance: &str, recordings: Option<PathBuf>) -> Recorder {
        Recorder {
            instance: instance.to_string(),
            recording: self.clone(),
            recordings,
        }
    }
}

/// One instance's side of [`Recording`]: what a link applies to every call it makes.
pub struct Recorder {
    instance: String,
    recording: Arc<Recording>,
    recordings: Option<PathBuf>,
}

impl Recorder {
    /// A call as the engine is to see it: a session's creation or resumption with the recorder
    /// options, anything else as it is. Also the recording a creation names, to remember once it
    /// answers.
    pub fn prepare(&self, method: &str, mut params: Value) -> (Value, Option<String>) {
        match method {
            translate::SESSION_CREATE => {
                let named = named(&params);
                (self.options(params), named)
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
                (self.options(params), None)
            }
            _ => (params, None),
        }
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

    fn options(&self, params: Value) -> Value {
        with_recording(
            params,
            self.recording.replay_from.as_deref(),
            self.recording.strict.load(Ordering::Relaxed),
        )
    }

    /// A deleted session's recording goes with it — unless the call only asked what would go.
    fn deleted(&self, method: &str, params: &Value, answer: &Result<Value, RpcError>) {
        let dry_run = params.get("dry_run").and_then(Value::as_bool) == Some(true);
        if method != translate::SESSION_DELETE || dry_run || answer.is_err() {
            return;
        }
        let (Some(root), Some(session)) = (&self.recordings, params.get("session_id").and_then(Value::as_str)) else {
            return;
        };
        let dir = root.join(session);
        if dir.is_dir() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                tracing::warn!(dir = %dir.display(), error = %e, "recording not removed");
            }
        }
    }
}

/// A link's calls, through its scene layers' presets and its recorder.
struct Recorded {
    rpc: Arc<dyn Rpc>,
    recorder: Recorder,
    presets: Arc<Presets>,
}

#[async_trait::async_trait]
impl Rpc for Recorded {
    async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let deleting = (method == translate::SESSION_DELETE).then(|| params.clone());
        let params = self.presets.prepare(method, params);
        let (params, named) = self.recorder.prepare(method, params);
        let answer = self.rpc.call(method, params).await;
        self.recorder.answered(named, &answer);
        if let Some(params) = deleting {
            self.recorder.deleted(method, &params, &answer);
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
    /// `link`, its sessions created as `presets` say and recorded by `recorder`. A gated daemon's
    /// link keeps its gate: a call through the recorder wakes a daemon let go while idle, as one
    /// straight to the link would.
    pub fn daemon(link: Arc<DaemonLink>, recorder: Recorder, presets: Arc<Presets>) -> Arc<Self> {
        let gate: Option<Arc<dyn WakeGate>> = link.gate();
        let rpc = link.rpc();
        Arc::new(Self { inner: link, ops: GatedOps { agix: AgixOps(Arc::new(Recorded { rpc, recorder, presets })), gate } })
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

/// `params` with the recorder options this process decides: record, or replay from `replay_from`
/// (by the name the call gave, if any).
fn with_recording(mut params: Value, replay_from: Option<&std::path::Path>, strict: bool) -> Value {
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
        None => json!({"mode": "record"}),
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
        let lenient = with_recording(named.clone(), Some(dir), false);
        assert_eq!(
            lenient["options"]["recorder"],
            json!({"mode": "replay", "name": "fixture", "dir": "/rec", "strict": false}),
            "a caller's own mode and strictness are overwritten"
        );
        let strict = with_recording(named.clone(), Some(dir), true);
        assert_eq!(strict["options"]["recorder"]["strict"], true);
        assert_eq!(
            with_recording(named, None, true)["options"]["recorder"],
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
        let (sent, named) = recorder.prepare("session.create", json!({"options": {"recorder": {"name": "fixture"}}}));
        assert_eq!(sent["options"]["recorder"]["mode"], "replay");
        recorder.answered(named, &Ok(json!({"session_id": "s1"})));
        let (resumed, _) = recorder.prepare("session.resume", json!({"session_id": "s1"}));
        assert_eq!(resumed["options"]["recorder"]["name"], "fixture");
        let (other, _) = recording.of("v", None).prepare("session.resume", json!({"session_id": "s1"}));
        assert_eq!(other["options"]["recorder"]["name"], Value::Null, "another instance's s1 is another session");
        let (passed, _) = recorder.prepare("session.get", json!({"session_id": "s1"}));
        assert_eq!(passed, json!({"session_id": "s1"}), "anything else goes as it is");
    }

    /// Outside replay a name is not remembered: it means nothing there.
    #[test]
    fn outside_replay_every_session_records() {
        let recording = Recording::new(None);
        let recorder = recording.of("w", None);
        let (sent, named) = recorder.prepare("session.create", json!({"options": {"recorder": {"name": "x"}}}));
        assert_eq!(sent["options"]["recorder"], json!({"mode": "record"}));
        recorder.answered(named, &Ok(json!({"session_id": "s1"})));
        assert!(lock(&recording.named).is_empty());
    }

    /// A layer's presets go on before the recorder's options, and the recorder still decides its
    /// own: a resumed session keeps both.
    #[tokio::test]
    async fn presets_go_on_before_the_recorder_and_the_recorder_still_decides() {
        struct Echo;
        #[async_trait::async_trait]
        impl Rpc for Echo {
            async fn call(&self, _: &str, params: Value) -> Result<Value, RpcError> {
                Ok(params)
            }
        }
        let layer = harnix_engine_api::SceneLayer::new("probe", harnix_engine_api::SceneBase::builtin("harnix")).auto_name(false);
        let rec = Recorded {
            rpc: Arc::new(Echo),
            recorder: Recording::new(Some(PathBuf::from("/rec"))).of("w", None),
            presets: Presets::new(&[layer], "harnix").unwrap(),
        };
        let sent = rec.call(translate::SESSION_CREATE, json!({"scene": "probe", "options": {"recorder": {"mode": "record", "name": "f"}}})).await.unwrap();
        assert_eq!(sent["scene"], "harnix");
        assert_eq!((&sent["options"]["attended"], &sent["options"]["auto_name"]), (&json!(true), &json!(false)));
        assert_eq!(sent["options"]["recorder"]["mode"], "replay");
        let resumed = rec.call(translate::SESSION_RESUME, json!({"session_id": "f"})).await.unwrap();
        assert_eq!(resumed["options"]["attended"], true);
        assert_eq!(resumed["options"]["recorder"]["mode"], "replay");
    }

    /// Deleting a session removes its recording; asking what a delete would remove, or closing
    /// it, does not.
    #[tokio::test]
    async fn a_deleted_sessions_recording_goes_with_it() {
        struct Answers;
        #[async_trait::async_trait]
        impl Rpc for Answers {
            async fn call(&self, _: &str, _: Value) -> Result<Value, RpcError> {
                Ok(json!({}))
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let recorded = |root: &std::path::Path| Recorded {
            rpc: Arc::new(Answers),
            recorder: Recording::new(None).of("w", Some(root.to_path_buf())),
            presets: Presets::none(),
        };
        let rec = recorded(dir.path());
        std::fs::create_dir_all(dir.path().join("s1")).unwrap();
        rec.call(translate::SESSION_CLOSE, json!({"session_id": "s1"})).await.unwrap();
        rec.call(translate::SESSION_DELETE, json!({"session_id": "s1", "dry_run": true})).await.unwrap();
        assert!(dir.path().join("s1").is_dir());
        rec.call(translate::SESSION_DELETE, json!({"session_id": "s1"})).await.unwrap();
        assert!(!dir.path().join("s1").exists());
    }
}
