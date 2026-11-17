//! The conformance suite against the scripted host: the suite holds for the host every runner test
//! runs on, and a host that breaks the contract is caught by it.

use std::sync::Arc;

use harnix_flow::conformance::{run_all, Subject};
use harnix_flow::runner::{Decision, Frame, HostError, Note, OpenSpec, Opened, ScriptedHost, SessionHost, SessionId, SessionInfo, Turn, Watch};
use serde_json::json;

fn scripted() -> Arc<ScriptedHost> {
    let host = ScriptedHost::new();
    // What the suite sends, in order: the idle turn, then the asking turn.
    host.station(
        "conformance",
        vec![
            Turn::Ends { output: None },
            Turn::Asks { tool: "Write".into(), paths: vec!["/p/a.md".into()], input: json!({"file_path": "/p/a.md"}), then: Box::new(Turn::Ends { output: None }) },
        ],
    );
    host
}

#[tokio::test(start_paused = true)]
async fn the_scripted_host_conforms() {
    let host = scripted();
    let subject = Subject::new(host, "flow-harnix").idle_turn("Say hello and stop.").asking_turn("Write a.md", "Write").project("/p");
    let failures = run_all(&subject).await;
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A host that forgets the one thing it must not: the settlement of a turn.
struct DropsSettlements(Arc<ScriptedHost>);

#[async_trait::async_trait]
impl SessionHost for DropsSettlements {
    async fn open(&self, spec: OpenSpec) -> Result<Opened, HostError> {
        self.0.open(spec).await
    }
    async fn lookup(&self, id: &SessionId) -> Result<Option<SessionInfo>, HostError> {
        self.0.lookup(id).await
    }
    async fn watch(&self, id: &SessionId) -> Result<Watch, HostError> {
        use futures::StreamExt;
        let w = self.0.watch(id).await?;
        let frames = w.frames.filter(|f| std::future::ready(!matches!(f, Frame::TurnSettled { .. })));
        Ok(Watch { frames: Box::pin(frames), info: w.info })
    }
    async fn send(&self, id: &SessionId, text: &str) -> Result<(), HostError> {
        self.0.send(id, text).await
    }
    async fn interrupt(&self, id: &SessionId) -> Result<(), HostError> {
        self.0.interrupt(id).await
    }
    async fn close(&self, id: &SessionId) -> Result<(), HostError> {
        self.0.close(id).await
    }
    async fn answer(&self, id: &SessionId, ask: &str, decision: Decision) -> Result<(), HostError> {
        self.0.answer(id, ask, decision).await
    }
    async fn note(&self, id: &SessionId, note: Note) -> Result<(), HostError> {
        self.0.note(id, note).await
    }
    async fn project_trusted(&self, root: &std::path::Path) -> Result<bool, HostError> {
        self.0.project_trusted(root).await
    }
}

#[tokio::test(start_paused = true)]
async fn a_host_that_drops_settlements_fails_the_suite() {
    let host = Arc::new(DropsSettlements(scripted()));
    let subject = Subject::new(host, "flow-harnix").asking_turn("Write a.md", "Write").turn_timeout(std::time::Duration::from_secs(5));
    let failures = run_all(&subject).await;
    let names: Vec<&str> = failures.iter().map(|f| f.check).collect();
    assert!(names.contains(&"a_send_starts_one_turn_that_settles"), "{failures:#?}");
    assert!(names.contains(&"a_question_is_asked_and_answered"), "{failures:#?}");
    assert!(!names.contains(&"open_is_idempotent"), "{failures:#?}");
}
