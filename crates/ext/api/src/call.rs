//! The one call a handler is answering.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use harnix_contract::s1::{ExtId, Subject};
use serde_json::Value;

use crate::context::ExtensionContext;
use crate::internal::CallInner;
use crate::{codes, Rpc, RpcError};

/// When a call stops being worth answering: milliseconds since the epoch, by the wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Deadline(u64);

impl Deadline {
    pub fn at_ms(ms: u64) -> Self {
        Self(ms)
    }

    pub fn as_ms(self) -> u64 {
        self.0
    }

    /// How long until it passes; zero once it has.
    pub fn remaining(self) -> Duration {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        Duration::from_millis(self.0.saturating_sub(now))
    }
}

/// One call: who makes it, and the extension answering it.
///
/// Cancellation and deadlines are cooperative: nothing drops a handler half-way, because state
/// a handler leaves half-written is state nobody can trust. A handler that waits on something
/// long waits in [`Call::run`], or watches [`Call::cancelled`].
#[derive(Clone)]
pub struct Call {
    inner: CallInner,
    ext: ExtensionContext,
}

impl Call {
    pub(crate) fn new(inner: CallInner, ext: ExtensionContext) -> Self {
        Self { inner, ext }
    }

    pub fn subject(&self) -> &Subject {
        self.inner.subject()
    }

    /// The paired device behind this call, when it came from one.
    pub fn device(&self) -> Option<&str> {
        match self.subject() {
            Subject::Device { id } => Some(id),
            _ => None,
        }
    }

    /// The extension whose page made this device's call, through the shell's bridge.
    pub fn on_behalf_of(&self) -> Option<&ExtId> {
        self.inner.on_behalf_of()
    }

    /// The context of the extension answering.
    pub fn ext(&self) -> &ExtensionContext {
        &self.ext
    }

    /// What the extension provided for its handlers ([`ExtensionContext::provide`]).
    pub fn state<T: Send + Sync + 'static>(&self) -> Rpc<Arc<T>> {
        self.ext.state::<T>()
    }

    /// Done when the call is cancelled: its caller went away, or the call it was made in was.
    pub async fn cancelled(&self) {
        self.inner.cancelled().await
    }

    /// What this call says of itself to another process or a host component: its id, the call it
    /// was made in, the extensions waiting on it, its deadline. Each a hint to whoever receives
    /// it.
    pub fn meta(&self) -> crate::CallMeta {
        self.inner.meta()
    }

    pub fn deadline(&self) -> Option<Deadline> {
        self.inner.deadline()
    }

    /// `f`, raced against cancellation and the deadline: `CANCELLED` or `DEADLINE_EXCEEDED` when
    /// either comes first, and `f` is dropped.
    pub async fn run<T>(&self, f: impl Future<Output = Rpc<T>>) -> Rpc<T> {
        let expired = async {
            match self.deadline() {
                Some(deadline) => tokio::time::sleep(deadline.remaining()).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            result = f => result,
            _ = self.cancelled() => Err(RpcError::new(codes::CANCELLED, "the call was cancelled")),
            _ = expired => Err(RpcError::new(codes::DEADLINE_EXCEEDED, "the call ran past its deadline")),
        }
    }

    /// A call as this extension, made while answering this one: decided and audited like any of
    /// its calls, on the record as made within this one, cancelled with it and by its deadline.
    pub async fn call(&self, method: &str, params: Value) -> Rpc<Value> {
        self.inner.child(&self.ext, method, params).await
    }
}
