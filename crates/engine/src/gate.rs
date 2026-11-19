//! A daemon that may be let go while idle: what its link asks before each call.
//!
//! An extension process's engine sleeps with its process. The link the hub holds is still the one
//! [`DaemonLink`](crate::DaemonLink); with a [`WakeGate`] it wakes the daemon before a call that
//! needs it, and answers the session list from what was listed before it went, waking nothing.

use std::sync::Arc;

use async_trait::async_trait;
use harnix_contract::wire::{RewindParams, RewindResult};
use harnix_contract::RpcError;
use harnix_engine_link::ops::{Answer, EngineOps, Entry, HistoryPage, PermissionModeChoice, Said, SessionCreated, SessionInfo, SessionSetup, Subscribed, Tasks, TurnInput};
use serde_json::Value;

use crate::translate::AgixOps;

/// What wakes a daemon let go while idle, and keeps its list while it sleeps: the instance
/// layer's, for an adopted engine.
#[async_trait]
pub trait WakeGate: Send + Sync {
    /// Up for a call: woken first when it sleeps, or why it could not be.
    async fn awake(&self) -> Result<(), RpcError>;

    /// What it listed last, while it sleeps; `None` while it is up.
    fn asleep_list(&self) -> Option<Vec<SessionInfo>>;

    /// What it listed now: the answer once it sleeps.
    fn listed(&self, rows: &[SessionInfo]);
}

/// A daemon link's calls, behind its gate when it has one.
pub(crate) struct GatedOps {
    pub(crate) agix: AgixOps,
    pub(crate) gate: Option<Arc<dyn WakeGate>>,
}

impl GatedOps {
    async fn awake(&self) -> Result<(), RpcError> {
        match &self.gate {
            Some(gate) => gate.awake().await,
            None => Ok(()),
        }
    }
}

/// Every call wakes a sleeping daemon but the list.
#[async_trait]
impl EngineOps for GatedOps {
    async fn create(&self, params: Value, setup: SessionSetup) -> Answer<SessionCreated> {
        self.awake().await?;
        self.agix.create(params, setup).await
    }
    async fn get(&self, session: &str) -> Answer<SessionInfo> {
        self.awake().await?;
        self.agix.get(session).await
    }
    async fn list(&self) -> Answer<Vec<SessionInfo>> {
        let Some(gate) = &self.gate else { return self.agix.list().await };
        if let Some(rows) = gate.asleep_list() {
            return Ok(rows);
        }
        let rows = self.agix.list().await?;
        gate.listed(&rows);
        Ok(rows)
    }
    async fn subscribe(&self, session: &str) -> Answer<Option<Subscribed>> {
        self.awake().await?;
        self.agix.subscribe(session).await
    }
    async fn resume(&self, session: &str, from: &SessionInfo) -> Answer<()> {
        self.awake().await?;
        self.agix.resume(session, from).await
    }
    async fn history_len(&self, session: &str) -> Answer<u64> {
        self.awake().await?;
        self.agix.history_len(session).await
    }
    async fn transcript(&self, session: &str, limit: usize) -> Answer<Vec<Said>> {
        self.awake().await?;
        self.agix.transcript(session, limit).await
    }
    async fn history(&self, session: &str, offset: u64, limit: usize) -> Answer<HistoryPage> {
        self.awake().await?;
        self.agix.history(session, offset, limit).await
    }
    async fn tasks(&self, session: &str) -> Answer<Tasks> {
        self.awake().await?;
        self.agix.tasks(session).await
    }
    async fn run_turn(&self, session: &str, input: TurnInput) -> Answer<Value> {
        self.awake().await?;
        self.agix.run_turn(session, input).await
    }
    async fn interrupt(&self, session: &str) -> Answer<Value> {
        self.awake().await?;
        self.agix.interrupt(session).await
    }
    async fn rename(&self, session: &str, title: &str) -> Answer<Value> {
        self.awake().await?;
        self.agix.rename(session, title).await
    }
    async fn set_model(&self, session: &str, choice: &harnix_contract::wire::ModelChoice) -> Answer<Value> {
        self.awake().await?;
        self.agix.set_model(session, choice).await
    }
    async fn set_prompt(&self, session: &str, append: Option<Option<String>>, override_text: Option<Option<String>>) -> Answer<Value> {
        self.awake().await?;
        self.agix.set_prompt(session, append, override_text).await
    }
    async fn set_skill_dirs(&self, session: &str, dirs: &[std::path::PathBuf]) -> Answer<Value> {
        self.awake().await?;
        self.agix.set_skill_dirs(session, dirs).await
    }
    async fn close(&self, params: Value, delete: bool) -> Answer<Value> {
        self.awake().await?;
        self.agix.close(params, delete).await
    }
    async fn fork(&self, params: Value) -> Answer<SessionCreated> {
        self.awake().await?;
        self.agix.fork(params).await
    }
    async fn append_entry(&self, session: &str, entry: Entry) -> Answer<Value> {
        self.awake().await?;
        self.agix.append_entry(session, entry).await
    }
    async fn respond(&self, params: Value) -> Answer<Value> {
        self.awake().await?;
        self.agix.respond(params).await
    }
    async fn passthrough(&self, method: &str, params: Value) -> Answer<Value> {
        self.awake().await?;
        self.agix.passthrough(method, params).await
    }
    async fn rewind(&self, session: &str, ask: &RewindParams) -> Answer<RewindResult> {
        self.awake().await?;
        self.agix.rewind(session, ask).await
    }
    async fn live_turns(&self, session: &str) -> Answer<Vec<String>> {
        self.awake().await?;
        self.agix.live_turns(session).await
    }
    async fn set_permission_mode(&self, session: &str, mode: &str) -> Answer<PermissionModeChoice> {
        self.awake().await?;
        self.agix.set_permission_mode(session, mode).await
    }
    async fn permission_mode(&self, session: &str, from: &SessionInfo) -> Answer<Option<String>> {
        self.awake().await?;
        self.agix.permission_mode(session, from).await
    }
    async fn session_skills(&self, session: &str) -> Answer<harnix_contract::wire::SessionSkills> {
        self.awake().await?;
        self.agix.session_skills(session).await
    }
    async fn settings_hooks(&self, session: &str) -> Answer<harnix_contract::wire::SessionHooks> {
        self.awake().await?;
        self.agix.settings_hooks(session).await
    }
}
