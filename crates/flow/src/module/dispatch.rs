//! Carrying out what the kernel hands back.
//!
//! Every engine result comes through [`apply`]: its events are broadcast, the runs it touched
//! are published on `run {run}` (lease tokens removed) and the list on `runs`, bindings of
//! claims that are over are dropped, and each work item is started on its own task — a method
//! answers without waiting for a session. A work item's own engine calls come back through
//! [`apply`] again. A program keeps its claim alive while it runs; a session task is
//! `session.rs`'s, a gate `gate.rs`'s.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::publish::publish_runs;
use super::session::{session_task, SessionJob};
use super::shared::{session_ref, Shared, TaskKey};
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
pub(crate) async fn apply(sh: &Arc<Shared>, hint: Option<&str>, o: Outcome) {
    for e in &o.events {
        let _ = sh.events.send(e.clone());
        if matches!(e.kind.as_str(), "run.finished" | "run.cancelled") {
            let (sh2, run) = (sh.clone(), e.run.clone());
            sh.spawn(async move { stop_run_instances(&sh2, &run).await });
        }
        // Every run has its main session from the start (xops-03 §1), whatever its first
        // station is: the conversation a person opens to watch and talk to it.
        if e.kind == "run.created" {
            let (sh2, run) = (sh.clone(), e.run.clone());
            sh.spawn(async move {
                if let Err(why) = super::main::ensure_main(&sh2, &run).await {
                    tracing::warn!(run, why, "flow: no main session");
                }
            });
        }
    }
    let mut runs: BTreeSet<String> = o.events.iter().map(|e| e.run.clone()).collect();
    runs.extend(o.work.iter().filter_map(run_of).map(String::from));
    if let Some(h) = hint {
        runs.insert(h.to_string());
    }
    publish_runs(sh, &runs, !o.bookkeeping).await;
    super::hooks::notify(sh, &o.events).await;
    for w in o.work {
        spawn_work(sh, w, false);
    }
}

/// [`settle`] for bookkeeping: what it changed is published, a refusal only logged. Boxed:
/// it is reached from work [`apply`] starts, and the future must not contain itself.
pub(crate) fn settle_quiet<'a>(
    sh: &'a Arc<Shared>,
    run: &'a str,
    r: crate::Result<Outcome>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        settle(sh, run, "bookkeeping", r).await;
    })
}

/// An engine call's result: carried out when it succeeded, logged when not.
pub(crate) async fn settle(sh: &Arc<Shared>, run: &str, what: &str, r: crate::Result<Outcome>) -> bool {
    match r {
        Ok(o) => {
            apply(sh, Some(run), o).await;
            true
        }
        Err(e) => {
            tracing::debug!(run, what, error = %e, "flow: not applied");
            false
        }
    }
}

/// Stop the instances the module made for `run` under its own names: a
/// spawned daemon per run would otherwise outlive it, and a template's `max` run out.
pub(crate) async fn stop_run_instances(sh: &Arc<Shared>, run: &str) {
    let Some(cx) = sh.cx.get() else { return };
    for name in sh.take_instances(run) {
        match cx.instances().stop(&name).await {
            Ok(_) => tracing::debug!(run, instance = name, "flow: the run's instance stopped"),
            Err(e) => {
                tracing::info!(run, instance = name, error = %e, "flow: the run's instance not stopped")
            }
        }
    }
}

/// After a restart: the instances of runs that ended while the module was down.
pub(crate) async fn stop_stale_instances(sh: &Arc<Shared>) {
    let Some(engine) = sh.engine.get().cloned() else {
        return;
    };
    for run in sh.instance_runs() {
        let over = match engine.record(&run).await {
            Ok(rec) => !rec.is_active(),
            Err(FlowError::NotFound(_)) => true,
            Err(_) => false,
        };
        if over {
            stop_run_instances(sh, &run).await;
        }
    }
}

/// After a restart: whatever was handed out and not answered is carried on with — a session
/// task waits on the session it had, a program runs again, a gate still waiting runs again.
pub(crate) async fn recover(sh: &Arc<Shared>) {
    let Some(engine) = sh.engine.get().cloned() else {
        return;
    };
    let active = match engine
        .list(RunQuery {
            status: Some(RunStatus::Active),
            ..Default::default()
        })
        .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "flow: runs to recover could not be listed");
            return;
        }
    };
    let now = engine.clock().now_ms();
    for s in active {
        let Ok(rec) = engine.record(&s.id).await else {
            continue;
        };
        for w in engine.dispatched_work(&rec, now) {
            spawn_work(sh, w, true);
        }
        // Its main session watched again (opened, if it never was).
        let (sh2, run) = (sh.clone(), s.id.clone());
        sh.spawn(async move {
            if let Err(why) = super::main::ensure_main(&sh2, &run).await {
                tracing::warn!(run, why, "flow: no main session after a restart");
            }
        });
    }
}

/// Lease expiry, once a tick.
pub(crate) async fn tick_loop(sh: Arc<Shared>) {
    let mut every = tokio::time::interval(Duration::from_millis(sh.cfg.tick_ms));
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = every.tick() => {}
            _ = sh.cancel.cancelled() => return,
        }
        let Some(engine) = sh.engine.get().cloned() else {
            continue;
        };
        match engine.tick().await {
            Ok(outcomes) => {
                for o in outcomes {
                    apply(&sh, None, o).await;
                }
            }
            Err(e) => tracing::warn!(error = %e, "flow: tick failed"),
        }
    }
}

pub(crate) fn spawn_work(sh: &Arc<Shared>, w: WorkItem, resume: bool) {
    let sh2 = sh.clone();
    sh.spawn(async move { run_work(sh2, w, resume).await });
}

async fn run_work(sh: Arc<Shared>, w: WorkItem, resume: bool) {
    if sh.stopping() {
        return;
    }
    if sh.engine().is_err() {
        return;
    }
    match w {
        WorkItem::Task { .. } => {
            let recording = sh.cfg.recording.as_ref().and_then(|f| f(&w));
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
            let key = TaskKey {
                run: run.clone(),
                station: station.clone(),
                round,
                task: task.clone(),
            };
            if !sh.begin(&key, &token) {
                return;
            }
            match executor {
                Executor::Program { step } => {
                    program_task(&sh, &key, &token, &step, item.as_ref()).await
                }
                Executor::Session { .. } | Executor::Agent { .. } => {
                    let main = session_key.as_deref() == Some(MAIN_SESSION_KEY);
                    let scene = match (scene, executor) {
                        (Some(s), _) => s,
                        (None, Executor::Session { scene, .. }) if !scene.is_empty() => scene,
                        _ => {
                            fail(
                                &sh,
                                &key,
                                &token,
                                "the flow names no main scene and the module has none",
                            )
                            .await;
                            sh.end(&key, &token);
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
                    session_task(&sh, job).await
                }
                Executor::External { .. } | Executor::Subflow { .. } => {}
            }
            sh.end(&key, &token);
        }
        WorkItem::Gate {
            run,
            station,
            round,
            gate,
        } => super::gate::run_gate(&sh, run, station, round, gate).await,
        WorkItem::CloseSession { session } => {
            let s = session_ref(&session);
            sh.unbind_session(&s.session_id);
            if let Some(cx) = sh.cx.get() {
                let _ = cx.sessions().interrupt(&s).await;
                if let Err(e) = cx.sessions().close(&s).await {
                    tracing::debug!(session, error = %e, "flow: session not closed");
                }
            }
        }
    }
}

/// How often a claim being worked on is renewed: a third of the lease.
pub(crate) fn renew_every(sh: &Shared) -> Duration {
    Duration::from_millis((sh.cfg.lease_ms / 3).max(500))
}

/// Renew a claim (claimed or handed in); `false` when it is no longer ours.
pub(crate) async fn renew(sh: &Arc<Shared>, key: &TaskKey, token: &str) -> bool {
    let Ok(engine) = sh.engine() else {
        return false;
    };
    let r = engine
        .heartbeat(
            &key.run,
            &key.station,
            &key.task,
            token,
            sh.cfg.lease_ms,
            dispatcher(),
        )
        .await;
    settle(sh, &key.run, "heartbeat", r).await
}

async fn program_task(
    sh: &Arc<Shared>,
    key: &TaskKey,
    token: &str,
    step: &str,
    item: Option<&Value>,
) {
    let Ok(engine) = sh.engine() else { return };
    let TaskKey {
        run, station, task, ..
    } = key;
    let result = match (&sh.cfg.programs, engine.get(run).await) {
        (Some(p), Ok(view)) => {
            // The claim is kept alive while the program runs; one that is lost abandons it.
            let running = p.run(step, &view.record, item);
            tokio::pin!(running);
            let every = renew_every(sh);
            loop {
                tokio::select! {
                    r = &mut running => break r,
                    _ = tokio::time::sleep(every) => {
                        if !renew(sh, key, token).await {
                            return;
                        }
                    }
                    // Abandoned, not failed: the claim lapses and the next start takes it up.
                    _ = sh.cancel.cancelled() => return,
                }
            }
        }
        (None, _) => Err(format!("no programs are registered to run {step}")),
        (_, Err(e)) => {
            tracing::warn!(run, error = %e, "flow: program not run");
            return;
        }
    };
    match result {
        Ok(output) => {
            match engine
                .submit(run, station, task, token, output, dispatcher())
                .await
            {
                Ok(o) => apply(sh, Some(run), o).await,
                // Only a contract failure is the program's: a station no longer open, a claim
                // gone, is not a failed attempt.
                Err(FlowError::Contract(m)) => fail(sh, key, token, &m).await,
                Err(e) => {
                    tracing::debug!(run, station, task, error = %e, "flow: program output not taken")
                }
            }
        }
        Err(e) => fail(sh, key, token, &e).await,
    }
}

pub(crate) async fn fail(sh: &Arc<Shared>, key: &TaskKey, token: &str, error: &str) {
    let Ok(engine) = sh.engine() else { return };
    let r = engine
        .fail(
            &key.run,
            &key.station,
            &key.task,
            token,
            error,
            dispatcher(),
        )
        .await;
    settle(sh, &key.run, "fail", r).await;
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
        assert_eq!(
            run_of(&WorkItem::CloseSession {
                session: "s".into()
            }),
            None
        );
    }
}
