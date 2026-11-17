//! Carrying out what the kernel hands back.
//!
//! Every engine result comes through [`apply`]: its events are broadcast, the runs it touched
//! are told changed (`Runner::changes`, with their process directories written and the bindings
//! of claims that are over dropped), the hooks are told, and each work item is started on its own
//! task — a caller answers without waiting for a session. A work item's own engine calls come
//! back through [`apply`] again. A program keeps its claim alive while it runs; a session task is
//! `session.rs`'s, a gate `gate.rs`'s.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::session::{session_task, SessionJob};
use super::state::TaskKey;
use super::Runner;
use crate::kernel::def::{Executor, MAIN_SESSION_KEY};
use crate::kernel::host::{Delivery, Outcome, WorkItem};
use crate::kernel::run::{Actor, RunQuery, RunStatus};
use crate::kernel::FlowError;

pub(crate) fn dispatcher() -> Actor {
    Actor::flow("dispatch")
}

fn run_of(w: &WorkItem) -> Option<&str> {
    match w {
        WorkItem::Task { run, .. } | WorkItem::Gate { run, .. } => Some(run),
        WorkItem::CloseSession { .. } => None,
    }
}

/// Carry out an engine result.
pub(crate) async fn apply(rn: &Arc<Runner>, hint: Option<&str>, o: Outcome) {
    for e in &o.events {
        let _ = rn.events.send(e.clone());
        if matches!(e.kind.as_str(), "run.finished" | "run.cancelled") {
            let (rn2, run) = (rn.clone(), e.run.clone());
            rn.spawn(async move { rn2.host().run_over(&run).await });
        }
        // Every run has its main session from the start, whatever its first station is: the
        // conversation a person opens to watch and talk to it.
        if e.kind == "run.created" {
            let (rn2, run) = (rn.clone(), e.run.clone());
            rn.spawn(async move {
                if let Err(why) = super::main::ensure_main(&rn2, &run).await {
                    tracing::warn!(run, why, "no main session");
                }
            });
        }
    }
    let mut runs: BTreeSet<String> = o.events.iter().map(|e| e.run.clone()).collect();
    runs.extend(o.work.iter().filter_map(run_of).map(String::from));
    if let Some(h) = hint {
        runs.insert(h.to_string());
    }
    rn.changed(runs, !o.bookkeeping).await;
    super::hooks::notify(rn, &o.events).await;
    for w in o.work {
        spawn_work(rn, w, false);
    }
}

/// [`settle`] for bookkeeping: what it changed is told, a refusal only logged. Boxed: it is
/// reached from work [`apply`] starts, and the future must not contain itself.
pub(crate) fn settle_quiet<'a>(
    rn: &'a Arc<Runner>,
    run: &'a str,
    r: crate::Result<Outcome>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        settle(rn, run, "bookkeeping", r).await;
    })
}

/// An engine call's result: carried out when it succeeded, logged when not.
pub(crate) async fn settle(rn: &Arc<Runner>, run: &str, what: &str, r: crate::Result<Outcome>) -> bool {
    match r {
        Ok(o) => {
            apply(rn, Some(run), o).await;
            true
        }
        Err(e) => {
            tracing::debug!(run, what, error = %e, "not applied");
            false
        }
    }
}

/// After a restart: whatever was handed out and not answered is carried on with — a session
/// task waits on the session it had, a program runs again, a gate still waiting runs again.
pub(crate) async fn recover(rn: &Arc<Runner>) {
    let engine = rn.engine();
    let active = match engine.list(RunQuery { status: Some(RunStatus::Active), ..Default::default() }).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "runs to recover could not be listed");
            return;
        }
    };
    let now = engine.clock().now_ms();
    for s in active {
        let Ok(rec) = engine.record(&s.id).await else {
            continue;
        };
        for w in engine.dispatched_work(&rec, now) {
            spawn_work(rn, w, true);
        }
        // Its main session watched again (opened, if it never was).
        let (rn2, run) = (rn.clone(), s.id.clone());
        rn.spawn(async move {
            if let Err(why) = super::main::ensure_main(&rn2, &run).await {
                tracing::warn!(run, why, "no main session after a restart");
            }
        });
    }
}

/// Lease expiry, once a tick.
pub(crate) async fn tick_loop(rn: Arc<Runner>) {
    let mut every = tokio::time::interval(Duration::from_millis(rn.cfg.tick_ms));
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = every.tick() => {}
            _ = rn.cancelled().cancelled() => return,
        }
        match rn.engine().tick().await {
            Ok(outcomes) => {
                for o in outcomes {
                    apply(&rn, None, o).await;
                }
            }
            Err(e) => tracing::warn!(error = %e, "tick failed"),
        }
    }
}

pub(crate) fn spawn_work(rn: &Arc<Runner>, w: WorkItem, resume: bool) {
    let rn2 = rn.clone();
    rn.spawn(async move { run_work(rn2, w, resume).await });
}

async fn run_work(rn: Arc<Runner>, w: WorkItem, resume: bool) {
    if rn.stopping() {
        return;
    }
    match w {
        WorkItem::Task { .. } => {
            let recording = rn.cfg.recording.as_ref().and_then(|f| f(&w));
            let WorkItem::Task {
                run,
                station,
                round,
                task,
                token,
                executor,
                skill,
                skill_digest,
                rules,
                permissions,
                item,
                protocol,
                session_key,
                output_schema,
                placement,
                delivery,
                scene,
                ..
            } = w
            else {
                return;
            };
            let key = TaskKey { run: run.clone(), station: station.clone(), round, task: task.clone() };
            if !rn.state.begin(&key, &token) {
                return;
            }
            match executor {
                Executor::Program { step } => program_task(&rn, &key, &token, &step, item.as_ref()).await,
                Executor::Session { .. } | Executor::Agent { .. } => {
                    let main = session_key.as_deref() == Some(MAIN_SESSION_KEY);
                    let scene = match (scene, executor) {
                        (Some(s), _) => s,
                        (None, Executor::Session { scene, .. }) if !scene.is_empty() => scene,
                        _ => {
                            fail(&rn, &key, &token, "the flow names no main scene and the runner has none").await;
                            rn.state.end(&key, &token);
                            return;
                        }
                    };
                    let job = SessionJob {
                        main,
                        key: key.clone(),
                        token: token.clone(),
                        scene,
                        skill,
                        skill_digest,
                        rules,
                        permissions,
                        protocol: protocol.unwrap_or_default(),
                        session_key,
                        recording,
                        resume,
                        output_schema,
                        placement: placement.unwrap_or_default(),
                        delivery: delivery.unwrap_or(Delivery::StructuredOutput),
                    };
                    session_task(&rn, job).await
                }
                Executor::External { .. } | Executor::Subflow { .. } => {}
            }
            rn.state.end(&key, &token);
        }
        WorkItem::Gate { run, station, round, gate } => super::gate::run_gate(&rn, run, station, round, gate).await,
        WorkItem::CloseSession { session } => {
            rn.state.unbind_session(&session);
            let host = rn.host();
            let _ = host.interrupt(&session).await;
            if let Err(e) = host.close(&session).await {
                tracing::debug!(session, error = %e, "session not closed");
            }
        }
    }
}

/// How often a claim being worked on is renewed: a third of the lease.
pub(crate) fn renew_every(rn: &Runner) -> Duration {
    Duration::from_millis((rn.cfg.lease_ms / 3).max(500))
}

/// Renew a claim (claimed or handed in); `false` when it is no longer ours.
pub(crate) async fn renew(rn: &Arc<Runner>, key: &TaskKey, token: &str) -> bool {
    let r = rn
        .engine()
        .heartbeat(&key.run, &key.station, &key.task, token, rn.cfg.lease_ms, dispatcher())
        .await;
    settle(rn, &key.run, "heartbeat", r).await
}

async fn program_task(rn: &Arc<Runner>, key: &TaskKey, token: &str, step: &str, item: Option<&Value>) {
    let engine = rn.engine();
    let TaskKey { run, station, task, .. } = key;
    let result = match (&rn.cfg.programs, engine.get(run).await) {
        (Some(p), Ok(view)) => {
            // The claim is kept alive while the program runs; one that is lost abandons it.
            let running = p.run(step, &view.record, item);
            tokio::pin!(running);
            let every = renew_every(rn);
            loop {
                tokio::select! {
                    r = &mut running => break r,
                    _ = tokio::time::sleep(every) => {
                        if !renew(rn, key, token).await {
                            return;
                        }
                    }
                    // Abandoned, not failed: the claim lapses and the next start takes it up.
                    _ = rn.cancelled().cancelled() => return,
                }
            }
        }
        (None, _) => Err(format!("no programs are registered to run {step}")),
        (_, Err(e)) => {
            tracing::warn!(run, error = %e, "program not run");
            return;
        }
    };
    match result {
        Ok(output) => match engine.submit(run, station, task, token, output, dispatcher()).await {
            Ok(o) => apply(rn, Some(run), o).await,
            // Only a contract failure is the program's: a station no longer open, a claim gone,
            // is not a failed attempt.
            Err(FlowError::Contract(m)) => fail(rn, key, token, &m).await,
            Err(e) => tracing::debug!(run, station, task, error = %e, "program output not taken"),
        },
        Err(e) => fail(rn, key, token, &e).await,
    }
}

pub(crate) async fn fail(rn: &Arc<Runner>, key: &TaskKey, token: &str, error: &str) {
    let r = rn.engine().fail(&key.run, &key.station, &key.task, token, error, dispatcher()).await;
    settle(rn, &key.run, "fail", r).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::def::SessionBinding;

    #[test]
    fn work_names_its_run() {
        let w = WorkItem::Task {
            run: "r-1".into(),
            station: "a".into(),
            round: 1,
            task: "a".into(),
            token: "t".into(),
            attempt: 1,
            executor: Executor::Session {
                scene: "flow-harnix".into(),
                session: Some(SessionBinding::New),
                model: None,
                provider: None,
                effort: None,
                instance: None,
                template: None,
                params: Default::default(),
                space: None,
            },
            skill: None,
            skill_digest: None,
            rules: vec![],
            permissions: vec![],
            item: None,
            output_schema: None,
            protocol: None,
            session_key: None,
            session: None,
            placement: None,
            scene: None,
            delivery: None,
        };
        assert_eq!(run_of(&w), Some("r-1"));
        assert_eq!(run_of(&WorkItem::CloseSession { session: "s".into() }), None);
    }
}
