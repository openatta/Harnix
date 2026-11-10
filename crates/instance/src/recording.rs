//! Recording, and replay: what a session is created and resumed with.
//!
//! Recording is on by default, and it is this layer that turns it on, on the way into the
//! engine, because every way a runnable session comes into being — `session.create`, and the
//! `session.resume` behind opening a past one — goes through a link this layer built.
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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use harnix_contract::sync::lock;
use harnix_contract::RpcError;
use serde_json::{json, Value};

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

    /// What one instance's link records with.
    pub fn of(self: &Arc<Self>, instance: &str) -> Recorder {
        Recorder {
            instance: instance.to_string(),
            recording: self.clone(),
        }
    }
}

/// One instance's side of [`Recording`]: what a link applies to every call it makes.
pub struct Recorder {
    instance: String,
    recording: Arc<Recording>,
}

impl Recorder {
    /// A call as the engine is to see it: a session's creation or resumption with the recorder
    /// options, anything else as it is. Also the recording a creation names, to remember once it
    /// answers.
    pub fn prepare(&self, method: &str, mut params: Value) -> (Value, Option<String>) {
        match method {
            "session.create" => {
                let named = named(&params);
                (self.options(params), named)
            }
            "session.resume" => {
                let session = params.get("session_id").and_then(Value::as_str).map(str::to_string);
                let remembered = session.and_then(|s| lock(&self.recording.named).get(&(self.instance.clone(), s)).cloned());
                if let (Some(name), Some(obj)) = (remembered, params.as_object_mut()) {
                    obj.insert("options".into(), json!({"recorder": {"name": name}}));
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
}

/// A link's call, through its recorder when it has one.
pub(crate) async fn recorded<F, Fut>(
    recorder: Option<&Recorder>,
    method: &str,
    params: Value,
    call: F,
) -> Result<Value, RpcError>
where
    F: FnOnce(Value) -> Fut,
    Fut: std::future::Future<Output = Result<Value, RpcError>>,
{
    let Some(recorder) = recorder else {
        return call(params).await;
    };
    let (params, named) = recorder.prepare(method, params);
    let answer = call(params).await;
    recorder.answered(named, &answer);
    answer
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
        let recorder = recording.of("w");
        let (sent, named) = recorder.prepare("session.create", json!({"options": {"recorder": {"name": "fixture"}}}));
        assert_eq!(sent["options"]["recorder"]["mode"], "replay");
        recorder.answered(named, &Ok(json!({"session_id": "s1"})));
        let (resumed, _) = recorder.prepare("session.resume", json!({"session_id": "s1"}));
        assert_eq!(resumed["options"]["recorder"]["name"], "fixture");
        let (other, _) = recording.of("v").prepare("session.resume", json!({"session_id": "s1"}));
        assert_eq!(other["options"]["recorder"]["name"], Value::Null, "another instance's s1 is another session");
        let (passed, _) = recorder.prepare("session.get", json!({"session_id": "s1"}));
        assert_eq!(passed, json!({"session_id": "s1"}), "anything else goes as it is");
    }

    /// Outside replay a name is not remembered: it means nothing there.
    #[test]
    fn outside_replay_every_session_records() {
        let recording = Recording::new(None);
        let recorder = recording.of("w");
        let (sent, named) = recorder.prepare("session.create", json!({"options": {"recorder": {"name": "x"}}}));
        assert_eq!(sent["options"]["recorder"], json!({"mode": "record"}));
        recorder.answered(named, &Ok(json!({"session_id": "s1"})));
        assert!(lock(&recording.named).is_empty());
    }
}
