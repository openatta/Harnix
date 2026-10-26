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

    pub async fn interrupt(&self, session: &SessionRef) -> Result<Value, RpcError> {
        self.cx
            .call("session.interrupt", key(session))
            .await
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
        let sink = Arc::new(TapSink {
            tx,
            lagged: lagged.clone(),
            session_id: session.session_id.clone(),
        });
        (sink, TapParts { session, rx, lagged })
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

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    fn take_replay(&mut self, attached: &Value) {
        let instance = self.session.instance.clone();
        let session_id = self.session.session_id.clone();
        for entry in attached
            .get("replay")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let mut params = entry.clone();
            if let Some(obj) = params.as_object_mut() {
                obj.insert("instance".into(), json!(instance));
                obj.insert("session_id".into(), json!(session_id));
            }
            self.pending
                .push_back(json!({"method": "harnix.event", "params": params}));
        }
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
}

/// The hub's side of a tap. It never waits: a full channel marks the tap
/// behind and the frame is dropped (design §6.1).
struct TapSink {
    tx: mpsc::Sender<Value>,
    lagged: Arc<AtomicBool>,
    session_id: String,
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
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.lagged.store(true, Ordering::Release);
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    fn wants(&self, channel: Channel) -> bool {
        channel == Channel::SessionEvents
    }
}
