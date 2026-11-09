//! Sessions and instances, from the server side (design §6.1, §5.2).
//!
//! A module drives sessions the way a client does — through the hub, with
//! the same queueing, interruption, settlement and broadcast — because these
//! are calls through the admission point and nothing else. A headless
//! session is an ordinary session: it shows in its instance's session list,
//! and anyone allowed can attach and watch it.
//!
//! A [`Tap`] is a server-side watcher with a client watcher's semantics: a
//! bounded channel, frames in `seq` order, and when it falls behind it is
//! not waited for — it reads [`TapEvent::Lagged`] and catches up from the
//! hub's buffer, the way a refreshing tab does. A tap nobody reads never
//! holds a session up.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use harnix_contract::sync::lock;
use harnix_contract::{Channel, Frame, FrameSink, RpcError};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::context::ModuleContext;

/// A session's full name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionRef {
    pub instance: String,
    pub session_id: String,
}

/// What a new session is. Everything but the instance is optional and
/// falls back to the instance's defaults.
#[derive(Debug, Clone, Default)]
pub struct SessionSpec {
    /// The space the session belongs to, for good: the session space when absent. Its default
    /// instance and scene apply to whatever this leaves out.
    pub space: Option<String>,
    /// The space's default instance when absent.
    pub instance: Option<String>,
    pub scene: Option<String>,
    pub project_root: Option<std::path::PathBuf>,
    /// A provider id from the instance's settings, and a model it offers.
    pub provider: Option<String>,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub permission_rules: Option<Value>,
    /// A JSON Schema the session's structured output must satisfy (N-107).
    pub output_schema: Option<Value>,
    /// Which recording to play, when the process runs in replay. Ignored
    /// otherwise.
    pub recording: Option<String>,
}

impl SessionSpec {
    fn params(&self) -> Value {
        let mut options = serde_json::Map::new();
        let mut put = |k: &str, v: Option<Value>| {
            if let Some(v) = v {
                options.insert(k.into(), v);
            }
        };
        put("provider", self.provider.clone().map(Value::from));
        put("model", self.model.clone().map(Value::from));
        put("permission_mode", self.permission_mode.clone().map(Value::from));
        put("permission_rules", self.permission_rules.clone());
        put("output_schema", self.output_schema.clone());
        put("recorder", self.recording.clone().map(|name| json!({"name": name})));
        let mut params = json!({"options": options});
        if let Some(space) = &self.space {
            params["space"] = json!(space);
        }
        if let Some(instance) = &self.instance {
            params["instance"] = json!(instance);
        }
        if let Some(scene) = &self.scene {
            params["scene"] = json!(scene);
        }
        if let Some(root) = &self.project_root {
            params["project_root"] = json!(root.display().to_string());
        }
        params
    }
}

/// An entry for [`Sessions::append_entry`]: what happened (`event`, the name the interface's
/// `flow.block` claims it by) and everything needed to draw it.
#[derive(Debug, Clone)]
pub struct Entry {
    pub event: String,
    pub payload: Value,
}

impl Entry {
    pub fn new(event: impl Into<String>, payload: Value) -> Self {
        Self {
            event: event.into(),
            payload,
        }
    }
}

/// What a send does when the session is already running a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnBusy {
    /// Run after it.
    #[default]
    Queue,
    Reject,
    /// Stop it and anything queued, then run this.
    Interrupt,
    /// Give it to the running turn at its next step boundary.
    Steer,
}

impl OnBusy {
    fn as_str(self) -> &'static str {
        match self {
            OnBusy::Queue => "queue",
            OnBusy::Reject => "reject",
            OnBusy::Interrupt => "interrupt",
            OnBusy::Steer => "steer",
        }
    }
}

/// Something a message carries for the turn to read (ipdrun-44), in the shape the engine takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// A file the engine reads when the turn starts: an image by its extension, anything else as
    /// text. The path is the engine's — on its machine, absolute or relative to the session's
    /// project.
    File { path: String },
    /// Text already read; `path` labels it for the model and may be a made-up name.
    Text { path: String, content: String },
    /// An image already decoded: base64, no data-URI prefix.
    Image { media_type: String, data: String },
}

impl Attachment {
    pub fn file(path: impl Into<String>) -> Self {
        Attachment::File { path: path.into() }
    }

    pub fn text(path: impl Into<String>, content: impl Into<String>) -> Self {
        Attachment::Text {
            path: path.into(),
            content: content.into(),
        }
    }

    pub fn image(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Attachment::Image {
            media_type: media_type.into(),
            data: data.into(),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Attachment::File { path } => json!({"kind": "file", "path": path}),
            Attachment::Text { path, content } => {
                json!({"kind": "text", "path": path, "content": content})
            }
            Attachment::Image { media_type, data } => {
                json!({"kind": "image", "media_type": media_type, "data": data})
            }
        }
    }
}

/// How [`Sessions::send_with`] sends: what [`Sessions::send`] takes, and what a message from the
/// interface can carry besides its text.
#[derive(Debug, Clone, Default)]
pub struct SendOptions {
    pub on_busy: OnBusy,
    pub attachments: Vec<Attachment>,
    /// Other sessions whose conversation the turn gets as a text attachment — what `@session`
    /// does in the composer.
    pub references: Vec<SessionRef>,
}

impl SendOptions {
    fn params(&self, session: &SessionRef, message: &str) -> Value {
        let mut params = json!({
            "instance": session.instance,
            "session_id": session.session_id,
            "message": message,
            "on_busy": self.on_busy.as_str(),
        });
        if !self.attachments.is_empty() {
            params["attachments"] = self.attachments.iter().map(Attachment::to_value).collect();
        }
        if !self.references.is_empty() {
            params["references"] = self.references.iter().map(key).collect();
        }
        params
    }
}

pub struct Sessions<'a> {
    pub(crate) cx: &'a ModuleContext,
}

impl Sessions<'_> {
    pub async fn create(&self, spec: SessionSpec) -> Result<SessionRef, RpcError> {
        let created = self.cx.call("session.create", spec.params()).await?;
        Ok(SessionRef {
            instance: created
                .get("instance")
                .and_then(Value::as_str)
                .unwrap_or(harnix_contract::SYSTEM_INSTANCE)
                .to_string(),
            session_id: created
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }

    /// A session that already exists — in memory or only on disk.
    pub async fn open(&self, instance: &str, session_id: &str) -> Result<SessionRef, RpcError> {
        self.cx
            .call("session.get", json!({"instance": instance, "session_id": session_id}))
            .await?;
        Ok(SessionRef {
            instance: instance.to_string(),
            session_id: session_id.to_string(),
        })
    }

    /// `{turn_id}` when a turn started, `{queued, item}` when it waits.
    pub async fn send(&self, session: &SessionRef, message: &str, on_busy: OnBusy) -> Result<Value, RpcError> {
        self.cx
            .call(
                "harnix.send",
                json!({
                    "instance": session.instance,
                    "session_id": session.session_id,
                    "message": message,
                    "on_busy": on_busy.as_str(),
                }),
            )
            .await
    }

    /// [`Sessions::send`] with attachments and session references (ipdrun-44). A message with
    /// attachments is never steered into a running turn: with [`OnBusy::Steer`] it waits for the
    /// next one, as from the interface.
    pub async fn send_with(
        &self,
        session: &SessionRef,
        message: &str,
        options: SendOptions,
    ) -> Result<Value, RpcError> {
        self.cx
            .call("harnix.send", options.params(session, message))
            .await
    }

    pub async fn interrupt(&self, session: &SessionRef) -> Result<Value, RpcError> {
        self.cx
            .call("session.interrupt", key(session))
            .await
    }

    /// Bind the session to another space, transcript and all (`harnix.sessions.move`, ipdrun-51):
    /// for an application that moved a kind of session from one space to another. Already there ⇒
    /// nothing changes. `{space, from, moved}`.
    pub async fn move_to(&self, session: &SessionRef, space: &str) -> Result<Value, RpcError> {
        let mut params = key(session);
        params["space"] = json!(space);
        self.cx.call("harnix.sessions.move", params).await
    }

    pub async fn close(&self, session: &SessionRef) -> Result<Value, RpcError> {
        self.cx.call("session.close", key(session)).await
    }

    /// Write an entry into the session's log: a card, a decision, a job's result — for people, never
    /// for the model (design §7.6). It is filed under this module's namespace (`app.<id>`), reaches
    /// whoever is watching the session at once, and stays through a refresh, a replay and a fork.
    /// The interface draws it with a `flow.block` contribution that claims `{ns, event}`.
    pub async fn append_entry(&self, session: &SessionRef, entry: Entry) -> Result<Value, RpcError> {
        let mut params = key(session);
        params["event"] = json!(entry.event);
        params["payload"] = entry.payload;
        self.cx.call("harnix.entries.append", params).await
    }

    /// This module's entries in the session, oldest first, each with `at`: how many messages come
    /// before it.
    pub async fn entries(&self, session: &SessionRef) -> Result<Vec<Value>, RpcError> {
        let mut params = key(session);
        params["namespaces"] = json!([format!("app.{}", self.cx.id())]);
        let listed = self.cx.call("session.entries", params).await?;
        Ok(listed
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Watch a session from the server side.
    pub async fn tap(&self, session: &SessionRef) -> Result<Tap, RpcError> {
        self.tap_with_capacity(session, TAP_CAPACITY).await
    }

    /// A tap that counts as behind after `capacity` unread frames.
    pub async fn tap_with_capacity(&self, session: &SessionRef, capacity: usize) -> Result<Tap, RpcError> {
        self.cx
            .kernel()
            .tap(&self.cx.subject(), session, capacity.max(1))
            .await
    }
}

fn key(session: &SessionRef) -> Value {
    json!({"instance": session.instance, "session_id": session.session_id})
}

pub struct Instances<'a> {
    pub(crate) cx: &'a ModuleContext,
}

impl Instances<'_> {
    /// Build an instance from a template. A module builds from its own
    /// templates (named `<module>.<template>`) and the profile's.
    pub async fn create(
        &self,
        template: &str,
        name: &str,
        params: BTreeMap<String, String>,
    ) -> Result<Value, RpcError> {
        self.cx
            .call(
                "harnix.instances.create",
                json!({"template": template, "name": name, "params": params}),
            )
            .await
    }

    /// The instance by this name, built from this template if it is not
    /// there yet. What a module calls at start.
    pub async fn ensure(
        &self,
        template: &str,
        name: &str,
        params: BTreeMap<String, String>,
    ) -> Result<Value, RpcError> {
        self.cx
            .call(
                "harnix.instances.ensure",
                json!({"template": template, "name": name, "params": params}),
            )
            .await
    }

    /// [`Instances::ensure`], answering once the instance is ready: a spawned daemon is
    /// `starting` for a moment after it is built, and a session call in that moment is refused.
    /// An instance that goes down is an `INSTANCE_UNAVAILABLE` error at once, with why; so is
    /// running out of `timeout`.
    pub async fn ensure_ready(
        &self,
        template: &str,
        name: &str,
        params: BTreeMap<String, String>,
        timeout: std::time::Duration,
    ) -> Result<Value, RpcError> {
        self.cx
            .call(
                "harnix.instances.ensure",
                json!({
                    "template": template, "name": name, "params": params,
                    "wait": "ready", "timeout_ms": timeout.as_millis() as u64,
                }),
            )
            .await
    }

    /// Wait until an instance is ready — one being restarted, or reconnecting — and answer with
    /// it. The same errors as [`Instances::ensure_ready`].
    pub async fn wait_ready(&self, instance: &str, timeout: std::time::Duration) -> Result<Value, RpcError> {
        self.cx
            .call(
                "harnix.instances.wait",
                json!({"instance": instance, "timeout_ms": timeout.as_millis() as u64}),
            )
            .await
    }

    /// Stop an instance this module built, and forget it. Its data stays.
    pub async fn stop(&self, instance: &str) -> Result<Value, RpcError> {
        self.cx
            .call("harnix.instances.stop", json!({"instance": instance}))
            .await
    }
}

/// One thing a tap reads.
#[derive(Debug, Clone)]
pub enum TapEvent {
    /// A session frame, exactly as a client receives it: `{method, params}`
    /// with `params.seq` on `harnix.event`.
    Frame(Value),
    /// Frames after `last_seq` were not delivered. Call [`Tap::catch_up`].
    Lagged { last_seq: u64 },
}

/// How many frames a tap holds before it counts as behind.
pub const TAP_CAPACITY: usize = 512;

type Reattach =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send>> + Send + Sync>;

/// A server-side watcher of one session.
pub struct Tap {
    pub session: SessionRef,
    rx: mpsc::Receiver<Value>,
    lagged: Arc<AtomicBool>,
    /// Settlement frames that arrived while the tap was full: kept apart, so a tap that fell
    /// behind still learns how each turn ended ([`Tap::next_settled`]).
    settled: Settled,
    last_seq: u64,
    pending: VecDeque<Value>,
    reattach: Reattach,
    on_drop: Option<Box<dyn FnOnce() + Send>>,
}

impl Tap {
    /// The pair the kernel builds a tap from: the sink the hub writes to,
    /// and the reading end. Not for modules.
    #[doc(hidden)]
    pub fn channel(session: SessionRef, capacity: usize) -> (Arc<dyn FrameSink>, TapParts) {
        let (tx, rx) = mpsc::channel(capacity);
        let lagged = Arc::new(AtomicBool::new(false));
        let settled = Settled::default();
        let sink = Arc::new(TapSink {
            tx,
            lagged: lagged.clone(),
            session_id: session.session_id.clone(),
            settled: settled.clone(),
        });
        (sink, TapParts { session, rx, lagged, settled })
    }

    /// Finish a tap from its parts and the attach that registered it.
    #[doc(hidden)]
    pub fn new(
        parts: TapParts,
        attached: Value,
        reattach: Reattach,
        on_drop: Box<dyn FnOnce() + Send>,
    ) -> Self {
        let mut tap = Self {
            session: parts.session,
            rx: parts.rx,
            lagged: parts.lagged,
            settled: parts.settled,
            last_seq: 0,
            pending: VecDeque::new(),
            reattach,
            on_drop: Some(on_drop),
        };
        tap.take_replay(&attached);
        tap
    }

    /// The next frame, or that frames were missed. `None` when the session
    /// is gone from the hub.
    ///
    /// The gap is reported as soon as it happens, before anything still
    /// queued: frames queued after the overflow are past the gap, and
    /// reading them first would make the missed ones look old to
    /// [`Tap::catch_up`].
    pub async fn next(&mut self) -> Option<TapEvent> {
        loop {
            if let Some(frame) = self.pending.pop_front() {
                if self.fresh(&frame) {
                    return Some(TapEvent::Frame(frame));
                }
                continue;
            }
            if self.lagged.swap(false, Ordering::AcqRel) {
                return Some(TapEvent::Lagged {
                    last_seq: self.last_seq,
                });
            }
            let frame = self.rx.recv().await?;
            if self.lagged.load(Ordering::Acquire) {
                self.pending.push_back(frame);
                continue;
            }
            if self.fresh(&frame) {
                return Some(TapEvent::Frame(frame));
            }
        }
    }

    /// Fill in what was missed from the hub's buffer, then carry on with
    /// what is queued. Returns the attach it caught up from: once a turn has
    /// settled into the transcript the buffer no longer holds it, and
    /// `history_total` says how far `session.history` covers instead — the
    /// same split a refreshing client draws.
    pub async fn catch_up(&mut self) -> Result<Value, RpcError> {
        let attached = (self.reattach)().await?;
        let held: Vec<Value> = self.pending.drain(..).collect();
        self.take_replay(&attached);
        self.pending.extend(held);
        Ok(attached)
    }

    /// [`Tap::catch_up`], and the settlements that were held apart while the tap was full go into
    /// what [`Tap::next`] reads, each where it belongs (ipdrun-41): after the replayed events of
    /// its own turn when the hub's buffer still holds them, otherwise before the replay — those
    /// turns ended earlier, and the buffer only ever holds the latest one. A caller that reads
    /// frame by frame then sees every `harnix.turn_settled` of the gap, with its `outcome`, and
    /// does not need [`Tap::next_settled`] next to its loop.
    pub async fn catch_up_settled(&mut self) -> Result<Value, RpcError> {
        let attached = (self.reattach)().await?;
        let held: Vec<Value> = self.pending.drain(..).collect();
        let replay = self.replay_frames(&attached);
        let mut settled = Vec::new();
        while let Some(frame) = self.settled.pop() {
            settled.push(frame);
        }
        let turn_of = |frame: &Value| {
            frame
                .get("params")
                .and_then(|p| p.get("turn_id"))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let (mut late, early): (Vec<Value>, Vec<Value>) = settled.into_iter().partition(|s| {
            let turn = turn_of(s);
            turn.is_some() && replay.iter().any(|f| turn_of(f) == turn)
        });
        self.pending.extend(early);
        for (i, frame) in replay.iter().enumerate() {
            self.pending.push_back(frame.clone());
            let turn = turn_of(frame);
            if replay[i + 1..].iter().all(|f| turn_of(f) != turn) {
                let (theirs, rest): (Vec<Value>, Vec<Value>) =
                    late.into_iter().partition(|s| turn_of(s) == turn);
                self.pending.extend(theirs);
                late = rest;
            }
        }
        self.pending.extend(held);
        Ok(attached)
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    /// The next `harnix.turn_settled`, skipping every other frame: its params — `turn_id`, then
    /// `result` or `error`, and `outcome` (stop reason, usage, structured output, a hook's message)
    /// when the engine said how the turn ended. `None` when the session is gone from the hub.
    ///
    /// A tap that fell behind still has its settlements: they are held apart while the tap is
    /// full (up to [`SETTLED_HELD`]), so a caller that only wants how each turn ended needs no
    /// [`Tap::catch_up`]. The frames it skips are gone from this tap, as if read.
    pub async fn next_settled(&mut self) -> Option<Value> {
        loop {
            // What is queued is older than anything held apart: read it first.
            while let Some(frame) = self.pending.pop_front() {
                if let Some(settled) = self.settled_params(&frame) {
                    return Some(settled);
                }
            }
            while let Ok(frame) = self.rx.try_recv() {
                if let Some(settled) = self.settled_params(&frame) {
                    return Some(settled);
                }
            }
            if let Some(frame) = self.settled.pop() {
                return self.settled_params(&frame);
            }
            let frame = self.rx.recv().await?;
            self.pending.push_back(frame);
        }
    }

    fn settled_params(&mut self, frame: &Value) -> Option<Value> {
        self.fresh(frame);
        (frame.get("method").and_then(Value::as_str) == Some("harnix.turn_settled"))
            .then(|| frame.get("params").cloned().unwrap_or(Value::Null))
    }

    fn take_replay(&mut self, attached: &Value) {
        let frames = self.replay_frames(attached);
        self.pending.extend(frames);
    }

    fn replay_frames(&self, attached: &Value) -> Vec<Value> {
        let instance = &self.session.instance;
        let session_id = &self.session.session_id;
        attached
            .get("replay")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|entry| {
                let mut params = entry.clone();
                if let Some(obj) = params.as_object_mut() {
                    obj.insert("instance".into(), json!(instance));
                    obj.insert("session_id".into(), json!(session_id));
                }
                json!({"method": "harnix.event", "params": params})
            })
            .collect()
    }

    /// Whether this frame is past what was already read. A frame without a
    /// seq (settlement, queue) is always news.
    fn fresh(&mut self, frame: &Value) -> bool {
        match frame
            .get("params")
            .and_then(|p| p.get("seq"))
            .and_then(Value::as_u64)
        {
            Some(seq) if seq <= self.last_seq => false,
            Some(seq) => {
                self.last_seq = seq;
                true
            }
            None => true,
        }
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        if let Some(f) = self.on_drop.take() {
            f();
        }
    }
}

#[doc(hidden)]
pub struct TapParts {
    session: SessionRef,
    rx: mpsc::Receiver<Value>,
    lagged: Arc<AtomicBool>,
    settled: Settled,
}

/// How many settlement frames a full tap holds apart before it drops those too.
pub const SETTLED_HELD: usize = 64;

/// Settlement frames a full tap could not queue.
#[derive(Clone, Default)]
struct Settled(Arc<std::sync::Mutex<VecDeque<Value>>>);

impl Settled {
    fn hold(&self, frame: Value) {
        let mut held = lock(&self.0);
        if held.len() < SETTLED_HELD {
            held.push_back(frame);
        }
    }

    fn pop(&self) -> Option<Value> {
        lock(&self.0).pop_front()
    }
}

/// The hub's side of a tap. It never waits: a full channel marks the tap
/// behind and the frame is dropped (design §6.1).
struct TapSink {
    tx: mpsc::Sender<Value>,
    lagged: Arc<AtomicBool>,
    session_id: String,
    settled: Settled,
}

#[async_trait::async_trait]
impl FrameSink for TapSink {
    async fn deliver(&self, frame: Frame) -> bool {
        if frame.session_id.as_deref() != Some(self.session_id.as_str()) {
            return true;
        }
        let Ok(value) = serde_json::from_str::<Value>(&frame.payload) else {
            return true;
        };
        match self.tx.try_send(value) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(value)) => {
                self.lagged.store(true, Ordering::Release);
                if value.get("method").and_then(Value::as_str) == Some("harnix.turn_settled") {
                    self.settled.hold(value);
                }
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    fn wants(&self, channel: Channel) -> bool {
        channel == Channel::SessionEvents
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(session: &str, method: &str, params: Value) -> Frame {
        Frame::session(
            session,
            json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string(),
        )
    }

    fn event(seq: u64, turn: &str) -> Value {
        json!({"session_id": "s", "seq": seq, "turn_id": turn, "event": {"kind": "text_delta", "text": "x"}})
    }

    fn settled(turn: &str) -> Value {
        json!({"session_id": "s", "turn_id": turn, "outcome": {"stop_reason": "end_turn"}})
    }

    fn label(frame: &Value) -> String {
        let p = &frame["params"];
        match frame["method"].as_str() {
            Some("harnix.turn_settled") => format!("settled:{}", p["turn_id"].as_str().unwrap()),
            _ => format!("{}#{}", p["turn_id"].as_str().unwrap(), p["seq"]),
        }
    }

    async fn drain(tap: &mut Tap) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(Some(TapEvent::Frame(f))) =
            tokio::time::timeout(std::time::Duration::from_millis(50), tap.next()).await
        {
            out.push(label(&f));
        }
        out
    }

    /// A tap of capacity 1 that is never read while three turns go by: the second and third
    /// settle in the gap; the hub's buffer holds only the third turn's events.
    async fn behind() -> Tap {
        let session = SessionRef {
            instance: "system".into(),
            session_id: "s".into(),
        };
        let (sink, parts) = Tap::channel(session, 1);
        let attached = json!({"replay": [event(5, "t3"), event(6, "t3")]});
        let reattach: Reattach = Arc::new(move || {
            let attached = attached.clone();
            Box::pin(async move { Ok(attached) })
        });
        let mut tap = Tap::new(parts, json!({}), reattach, Box::new(|| {}));
        for (method, params) in [
            ("harnix.event", event(1, "t1")),
            ("harnix.turn_settled", settled("t1")),
            ("harnix.event", event(2, "t2")),
            ("harnix.turn_settled", settled("t2")),
            ("harnix.event", event(5, "t3")),
            ("harnix.event", event(6, "t3")),
            ("harnix.turn_settled", settled("t3")),
        ] {
            sink.deliver(frame("s", method, params)).await;
        }
        assert!(matches!(tap.next().await, Some(TapEvent::Lagged { .. })));
        tap
    }

    #[test]
    fn send_options_carry_attachments_and_references_in_the_engine_shape() {
        let session = SessionRef {
            instance: "i".into(),
            session_id: "s".into(),
        };
        let options = SendOptions {
            on_busy: OnBusy::Steer,
            attachments: vec![
                Attachment::file("docs/a.md"),
                Attachment::text("note", "hello"),
                Attachment::image("image/png", "AAAA"),
            ],
            references: vec![SessionRef {
                instance: "i".into(),
                session_id: "other".into(),
            }],
        };
        assert_eq!(
            options.params(&session, "look"),
            json!({
                "instance": "i", "session_id": "s", "message": "look", "on_busy": "steer",
                "attachments": [
                    {"kind": "file", "path": "docs/a.md"},
                    {"kind": "text", "path": "note", "content": "hello"},
                    {"kind": "image", "media_type": "image/png", "data": "AAAA"},
                ],
                "references": [{"instance": "i", "session_id": "other"}],
            })
        );
        let plain = SendOptions::default().params(&session, "hi");
        assert_eq!(
            plain,
            json!({"instance": "i", "session_id": "s", "message": "hi", "on_busy": "queue"}),
            "nothing extra: exactly what send sends"
        );
    }

    #[tokio::test]
    async fn catch_up_settled_puts_each_held_settlement_where_it_belongs() {
        let mut tap = behind().await;
        tap.catch_up_settled().await.unwrap();
        assert_eq!(
            drain(&mut tap).await,
            ["settled:t1", "settled:t2", "t3#5", "t3#6", "settled:t3"],
            "t1 and t2 ended before the buffered turn; t3's settlement follows its events"
        );
    }

    #[tokio::test]
    async fn catch_up_still_leaves_the_held_settlements_to_next_settled() {
        let mut tap = behind().await;
        tap.catch_up().await.unwrap();
        assert_eq!(drain(&mut tap).await, ["t3#5", "t3#6"]);
        assert_eq!(tap.next_settled().await.unwrap()["turn_id"], "t1");
    }
}
