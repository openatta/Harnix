//! The readers of what an engine writes down — its sessions' recordings and transcripts — through
//! its protocol: `session.calls`, `session.call`, `session.callBlob`, `session.search`. One per
//! instance, made with its link; what it read is kept until the link says the session may have
//! moved on ([`crate::activity`]).
//!
//! A recording covers the latest run of the session only: the engine starts it over when the
//! process running the session first calls the model, so what an earlier process recorded is not
//! in it. Every answer says which run it covers ([`harnix_engine_api::readers::RecordingRun`]).

mod calls;
mod days;
mod envelopes;
mod search;
mod time;
mod trace;
mod usage;
mod wire;

mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use harnix_contract::RpcError;
use harnix_engine_api::readers::{
    CallDetail, DayWindow, Envelopes, ReadError, Recordings, SearchLimits, SearchResult, SessionDays, SessionUsage, Trace, Transcripts,
    Turns,
};
use harnix_engine_link::Readers;
use serde_json::Value;

use crate::activity::Activity;
use crate::daemon_link::DaemonLink;
use crate::gate::WakeGate;
use crate::translate::Rpc;

pub(crate) struct ProtocolReader {
    rpc: Arc<dyn Rpc>,
    gate: Option<Arc<dyn WakeGate>>,
    activity: Arc<Activity>,
    calls: calls::Calls,
    envelopes: envelopes::Kept,
    days: days::Days,
}

impl DaemonLink {
    /// The readers of this instance, through this link. Made once per instance: what they keep is
    /// theirs.
    pub fn readers(&self) -> Readers {
        let reader = Arc::new(ProtocolReader::new(self.rpc(), self.gate(), self.activity()));
        Readers::new(reader.clone(), reader)
    }
}

impl ProtocolReader {
    pub(crate) fn new(rpc: Arc<dyn Rpc>, gate: Option<Arc<dyn WakeGate>>, activity: Arc<Activity>) -> Self {
        Self { rpc, gate, activity, calls: calls::Calls::new(calls::FRESH), envelopes: envelopes::Kept::default(), days: days::Days::default() }
    }

    /// Up for a call: a sleeping instance woken first (what the person opened is worth a start).
    async fn send(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        if let Some(gate) = &self.gate {
            gate.awake().await?;
        }
        self.rpc.call(method, params).await
    }

    /// [`ProtocolReader::send`], its answer read as `T`.
    async fn ask<T: serde::de::DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, ReadError> {
        let answer = self.send(method, params).await.map_err(wire::read_error)?;
        wire::parse(answer, method)
    }

    /// Whether the instance sleeps now: `usage` then answers from what it kept, waking nothing.
    fn asleep(&self) -> bool {
        self.gate.as_ref().is_some_and(|g| g.asleep_list().is_some())
    }
}

#[async_trait::async_trait]
impl Recordings for ProtocolReader {
    async fn calls(&self, session: &str) -> Result<Trace, ReadError> {
        let root = self.calls.root_of(self, session, true).await?;
        let kept = self.calls.read(self, &root, true).await?;
        Ok(trace::trace(&kept, session))
    }

    async fn call(&self, session: &str, index: usize) -> Result<CallDetail, ReadError> {
        self.detail(session, index).await
    }

    async fn turns(&self, session: &str) -> Result<Turns, ReadError> {
        let root = self.calls.root_of(self, session, true).await?;
        let kept = self.calls.read(self, &root, true).await?;
        Ok(trace::turns(&kept, session))
    }

    async fn envelopes(&self, session: &str) -> Result<Envelopes, ReadError> {
        self.envelopes_of(session).await
    }

    async fn usage(&self, sessions: &[String]) -> Result<BTreeMap<String, SessionUsage>, ReadError> {
        self.usage_of(sessions).await
    }

    async fn usage_by_day(&self, sessions: &[String], window: &DayWindow) -> Result<BTreeMap<String, SessionDays>, ReadError> {
        self.usage_days_of(sessions, window).await
    }
}

#[async_trait::async_trait]
impl Transcripts for ProtocolReader {
    async fn search(&self, sessions: &[String], query: &str, limits: &SearchLimits) -> Result<SearchResult, ReadError> {
        self.search_of(sessions, query, limits).await
    }
}
