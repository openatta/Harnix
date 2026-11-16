//! The links this layer hands the hub (concept §3.1.1).
//!
//! An engine in this process, a daemon in another, or nothing at all: the engine's links
//! (`harnix_engine`), wrapped so that what they create is recorded — or a [`DownLink`], for an
//! instance that could not be built. The hub holds each as an `EngineLink` and has no branch on
//! which it is; the registry keeps only its `EngineControl` half.

use std::sync::Arc;

use harnix_contract::RpcError;
use harnix_engine_link::link::{EngineControl, EngineIdentity, EngineLink, EventReceiver, InstanceCarrier, LinkStatus};
use harnix_engine_link::ops::{Answer, EngineOps, Entry, HistoryPage, Said, SessionCreated, SessionInfo, SessionSetup, Subscribed, Tasks, TurnInput};
use serde_json::Value;
use tokio::sync::watch;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// An instance that could not be built: a ledger entry whose template is gone or whose owner is
/// disabled, or a carrier that failed to start. It is listed `Down` with the reason, so the
/// system space can say what happened to it; its data root is not touched.
pub struct DownLink {
    status: watch::Sender<LinkStatus>,
    carrier: InstanceCarrier,
}

impl DownLink {
    pub fn new(carrier: InstanceCarrier, reason: String) -> Arc<Self> {
        let (status, _) = watch::channel(LinkStatus::Down { reason });
        Arc::new(Self { status, carrier })
    }

    /// One whose template is gone, and with it what carrier it had: listed as in-process.
    pub fn without_template(reason: String) -> Arc<Self> {
        Self::new(InstanceCarrier::InProcess, reason)
    }

    fn unavailable(&self) -> RpcError {
        self.status.borrow().unavailable()
    }
}

#[async_trait::async_trait]
impl EngineControl for DownLink {
    fn status(&self) -> watch::Receiver<LinkStatus> {
        self.status.subscribe()
    }
    fn identity(&self) -> EngineIdentity {
        EngineIdentity::default()
    }
    fn carrier(&self) -> InstanceCarrier {
        self.carrier
    }
    async fn reload_settings(&self) -> Result<(), RpcError> {
        Err(self.unavailable())
    }
    async fn shutdown(&self) {}
}

#[async_trait::async_trait]
impl EngineLink for DownLink {
    fn ops(&self) -> &dyn EngineOps {
        self
    }
    fn take_events(&self) -> Option<EventReceiver> {
        None
    }
    async fn mark(&self, _id: u64) {}
}

/// Every call answers that the instance is down.
#[async_trait::async_trait]
impl EngineOps for DownLink {
    async fn create(&self, _: Value, _: SessionSetup) -> Answer<SessionCreated> {
        Err(self.unavailable())
    }
    async fn get(&self, _: &str) -> Answer<SessionInfo> {
        Err(self.unavailable())
    }
    async fn list(&self) -> Answer<Vec<SessionInfo>> {
        Err(self.unavailable())
    }
    async fn subscribe(&self, _: &str) -> Answer<Option<Subscribed>> {
        Err(self.unavailable())
    }
    async fn resume(&self, _: &str, _: &SessionInfo) -> Answer<()> {
        Err(self.unavailable())
    }
    async fn history_len(&self, _: &str) -> Answer<u64> {
        Err(self.unavailable())
    }
    async fn transcript(&self, _: &str, _: usize) -> Answer<Vec<Said>> {
        Err(self.unavailable())
    }
    async fn history(&self, _: &str, _: u64, _: usize) -> Answer<HistoryPage> {
        Err(self.unavailable())
    }
    async fn tasks(&self, _: &str) -> Answer<Tasks> {
        Err(self.unavailable())
    }
    async fn run_turn(&self, _: &str, _: TurnInput) -> Answer<Value> {
        Err(self.unavailable())
    }
    async fn interrupt(&self, _: &str) -> Answer<Value> {
        Err(self.unavailable())
    }
    async fn close(&self, _: Value, _: bool) -> Answer<Value> {
        Err(self.unavailable())
    }
    async fn fork(&self, _: Value) -> Answer<SessionCreated> {
        Err(self.unavailable())
    }
    async fn append_entry(&self, _: &str, _: Entry) -> Answer<Value> {
        Err(self.unavailable())
    }
    async fn respond(&self, _: Value) -> Answer<Value> {
        Err(self.unavailable())
    }
    async fn rename(&self, _: &str, _: &str) -> Answer<Value> {
        Err(self.unavailable())
    }
    async fn set_model(&self, _: &str, _: &str, _: Option<&str>) -> Answer<Value> {
        Err(self.unavailable())
    }
    async fn set_prompt(&self, _: &str, _: Option<Option<String>>, _: Option<Option<String>>) -> Answer<Value> {
        Err(self.unavailable())
    }
    async fn set_skill_dirs(&self, _: &str, _: &[std::path::PathBuf]) -> Answer<Value> {
        Err(self.unavailable())
    }
    async fn passthrough(&self, _: &str, _: Value) -> Answer<Value> {
        Err(self.unavailable())
    }
}
