//! The extension interface's side of the seam with the kernel: the one module that names it.
//!
//! The host binds an extension by handing its descriptor a seat ([`Seated`]); from then on the
//! descriptor's methods answer as router rows ([`Handler`]), its computed topics are computed for
//! the hub ([`TopicSource`]), and its context reaches the process through the seat's services.
//! None of it is public: an extension sees only the context, its calls and its streams.

use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use harnix_contract::s1::{Capability, ExtId, Subject};
use harnix_contract::{Channel, Frame, FrameSink, SessionRef};
use harnix_engine_api::readers::{Recordings, Transcripts};
use harnix_engine_api::Secret;
use harnix_kernel::seat::{DeviceAdmin, InstanceView, Listening, Seat, Seated};
use harnix_kernel::{CallContext, Handler, TopicSource};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::call::{Call, Deadline};
use crate::context::{ExtensionContext, GrantsView, Listen};
use crate::credentials::Credentials;
use crate::descriptor::{ExtensionDescriptor, MethodDecl, TopicDecl, TopicKind};
use crate::sessions::{Reattach, Tap};
use crate::topics::{TopicStream, TopicValue};
use crate::{internal_error, Rpc, RpcError};

/// How many values a topic stream holds before newer ones are dropped: a whole value replaces
/// the last, so a reader that falls behind loses nothing it would not have overwritten.
const STREAM_CAPACITY: usize = 64;

impl Seated for ExtensionDescriptor {
    fn seat(&self, seat: Seat) {
        if self.slot.get().is_some() {
            return;
        }
        let grants = GrantsView::new(
            seat.grants.capabilities(&seat.id),
            seat.grants.trust(&seat.id).unwrap_or_else(|| self.trust.clone()),
        );
        let topics = self.topics.iter().map(|t| (t.name.clone(), t.params.clone())).collect();
        let cx = ExtensionContext::assemble(
            seat.id.clone(),
            seat.config.clone(),
            seat.data_dir.clone(),
            grants,
            Ports::new(seat),
            topics,
        );
        let _ = self.slot.set(cx);
    }
}

#[async_trait::async_trait]
impl Handler for MethodDecl {
    async fn call(&self, cx: CallContext, params: Value) -> Rpc<Value> {
        let Some(ext) = self.slot.get().cloned() else {
            return Err(RpcError::not_found(format!("method `{}` is not served here (yet)", self.name)));
        };
        self.handler.call(Call::new(CallInner(cx), ext), params).await
    }
}

#[async_trait::async_trait]
impl TopicSource for TopicDecl {
    async fn compute(&self, params: &Value) -> Rpc<Value> {
        let (TopicKind::Computed { compute, .. }, Some(ext)) = (&self.kind, self.slot.get()) else {
            return Err(internal_error(format!("`{}` is not computed here", self.name)));
        };
        compute.compute(ext, params).await
    }

    async fn on_subscribe(&self, params: &Value) {
        if let (TopicKind::Computed { compute, .. }, Some(ext)) = (&self.kind, self.slot.get()) {
            compute.on_subscribe(ext, params).await;
        }
    }

    fn every(&self) -> Option<Duration> {
        TopicDecl::every(self)
    }
}

/// The call a handler answers, as the kernel made it.
#[derive(Clone)]
pub(crate) struct CallInner(CallContext);

impl CallInner {
    pub(crate) fn meta(&self) -> harnix_contract::s1::CallMeta {
        self.0.meta()
    }

    pub(crate) fn subject(&self) -> &Subject {
        &self.0.subject
    }

    pub(crate) fn on_behalf_of(&self) -> Option<&ExtId> {
        self.0.on_behalf_of.as_ref()
    }

    pub(crate) async fn cancelled(&self) {
        self.0.cancel.cancelled().await
    }

    pub(crate) fn deadline(&self) -> Option<Deadline> {
        self.0.deadline.map(|d| Deadline::at_ms(d.0))
    }

    /// A call `ext` makes while answering this one.
    pub(crate) async fn child(&self, ext: &ExtensionContext, method: &str, params: Value) -> Rpc<Value> {
        let cx = self.0.child(ext.id(), false);
        ext.ports().seat.dispatch.call(cx, method, params).await
    }
}

/// An instance as a context sees it.
#[derive(Clone)]
pub(crate) struct View(pub(crate) InstanceView);

impl View {
    pub(crate) fn recordings(&self) -> Option<Arc<dyn Recordings>> {
        self.0.recordings.as_ref()?.downcast_ref::<Arc<dyn Recordings>>().cloned()
    }

    pub(crate) fn transcripts(&self) -> Option<Arc<dyn Transcripts>> {
        self.0.transcripts.as_ref()?.downcast_ref::<Arc<dyn Transcripts>>().cloned()
    }
}

/// What a context reaches the process through.
pub(crate) struct Ports {
    seat: Seat,
    credentials: SeatCredentials,
}

impl Ports {
    fn new(seat: Seat) -> Self {
        let credentials = SeatCredentials(seat.credentials.clone());
        Self { seat, credentials }
    }

    pub(crate) fn credentials(&self) -> &dyn Credentials {
        &self.credentials
    }

    pub(crate) async fn call_root(&self, subject: Subject, method: &str, params: Value) -> Rpc<Value> {
        self.seat.dispatch.call(CallContext::root(subject, None), method, params).await
    }

    pub(crate) fn holds(&self, ext: &ExtId, cap: &Capability) -> bool {
        self.seat.grants.holds(ext, cap)
    }

    pub(crate) async fn publish(&self, topic: &str, params: Value, value: Value) {
        self.seat.topics.publish(topic, params, value).await
    }

    pub(crate) fn changed(&self, topic: &str, params: Value) {
        self.seat.topics.changed(topic, params)
    }

    pub(crate) fn instance(&self, id: &str) -> Option<View> {
        self.seat.instances.view(id).map(View)
    }

    pub(crate) fn validate_provider(&self, entry: &Value) -> Result<(), String> {
        self.seat.instances.validate_provider(entry)
    }

    pub(crate) fn process(&self) -> Value {
        self.seat.process.process()
    }

    pub(crate) fn bulk(&self, ext: &ExtId, name: Option<&str>, max_bytes: usize) -> Rpc<(String, PathBuf)> {
        self.seat.bulk.issue(ext, name, max_bytes)
    }

    pub(crate) fn ui(&self, ext: &ExtId, entries: Vec<Value>) {
        self.seat.ui.publish(ext, entries)
    }

    pub(crate) fn devices(&self) -> &dyn DeviceAdmin {
        self.seat.devices.as_ref()
    }

    pub(crate) fn service(&self, ext: &ExtId, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        self.seat.services.service(ext, name)
    }

    pub(crate) fn projects_dir(&self) -> &std::path::Path {
        &self.seat.projects_dir
    }

    pub(crate) fn listen_now(&self) -> Option<Listen> {
        let rx = self.seat.listen.as_ref()?;
        let listening = rx.borrow().clone();
        listening.map(listen)
    }

    pub(crate) async fn listening(&self) -> Listen {
        if let Some(mut rx) = self.seat.listen.clone() {
            loop {
                if let Some(listening) = rx.borrow_and_update().clone() {
                    return listen(listening);
                }
                if rx.changed().await.is_err() {
                    break;
                }
            }
        }
        std::future::pending().await
    }

    /// A tap is a client of the hub like any other, registered as a watcher by the same attach,
    /// made through the admission point as the extension: the same table decides whether it may.
    pub(crate) async fn tap(&self, subject: Subject, session: &SessionRef, capacity: usize) -> Rpc<Tap> {
        let (sink, parts) = Tap::channel(session.clone(), capacity);
        let clients = self.seat.clients.clone();
        let client = clients.add_client(sink).await;
        let params = json!({"instance": session.instance, "session_id": session.session_id});
        let attach = {
            let dispatch = self.seat.dispatch.clone();
            move |params: Value| {
                let dispatch = dispatch.clone();
                let cx = CallContext::root(subject.clone(), Some(client));
                async move { dispatch.call(cx, "harnix.session.attach", params).await }
            }
        };
        let attached = match attach(params.clone()).await {
            Ok(v) => v,
            Err(e) => {
                clients.remove_client(client).await;
                return Err(e);
            }
        };
        let reattach: Reattach = Arc::new(move || Box::pin(attach(params.clone())));
        Ok(Tap::new(parts, attached, reattach, forget(clients, client)))
    }

    /// One client of the hub per subscription: what the hub pushes it is the stream's.
    pub(crate) async fn subscribe(&self, subject: Subject, topic: &str, params: Value) -> Rpc<TopicStream> {
        let (tx, rx) = mpsc::channel(STREAM_CAPACITY);
        let clients = self.seat.clients.clone();
        let client = clients.add_client(Arc::new(TopicSink { topic: topic.to_string(), tx })).await;
        let cx = CallContext::root(subject, Some(client));
        let answer = self
            .seat
            .dispatch
            .call(cx, "harnix.topics.subscribe", json!({"topic": topic, "params": params}))
            .await;
        match answer {
            Ok(answer) => {
                let current = answer.get("value").cloned().unwrap_or(Value::Null);
                Ok(TopicStream::new(current, rx, forget(clients, client)))
            }
            Err(e) => {
                clients.remove_client(client).await;
                Err(e)
            }
        }
    }
}

/// Forgetting a client when what holds it is dropped — which may be where there is no runtime
/// left, and then there is no hub left to tell either.
fn forget(clients: Arc<dyn harnix_kernel::ClientRegistry>, client: harnix_kernel::ClientId) -> Box<dyn FnOnce() + Send> {
    Box::new(move || {
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move { clients.remove_client(client).await });
        }
    })
}

fn listen(listening: Listening) -> Listen {
    Listen::new(listening.addrs, listening.url)
}

struct SeatCredentials(Arc<dyn harnix_kernel::seat::CredentialLookup>);

impl Credentials for SeatCredentials {
    fn get(&self, reference: &str) -> Result<Secret, String> {
        self.0.secret(reference).map(Secret::new)
    }
}

/// What the hub pushes one subscription's client, as the stream reads it.
struct TopicSink {
    topic: String,
    tx: mpsc::Sender<TopicValue>,
}

#[async_trait::async_trait]
impl FrameSink for TopicSink {
    async fn deliver(&self, frame: Frame) -> bool {
        let Ok(note) = serde_json::from_str::<Value>(&frame.payload) else {
            return true;
        };
        let p = &note["params"];
        if p["topic"].as_str() != Some(self.topic.as_str()) {
            return true;
        }
        let said = match note["method"].as_str() {
            Some("harnix.topic") => TopicValue::Value(p["value"].clone()),
            Some("harnix.topic_ended") => {
                TopicValue::Ended { reason: p["reason"].as_str().unwrap_or_default().to_string() }
            }
            _ => return true,
        };
        !matches!(self.tx.try_send(said), Err(mpsc::error::TrySendError::Closed(_)))
    }

    fn wants(&self, channel: Channel) -> bool {
        channel == Channel::HostEvents
    }
}
