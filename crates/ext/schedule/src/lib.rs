//! Schedules: at set times, a turn in a session, a turn in a new session, or a task — a run of
//! a flow in a project. Harnix runs them, not the engine: the engine's own schedule sends to a
//! session that exists, counts in UTC and opens nothing (`agix/docs/daemon_rpc_protocol.md`
//! §6.7). What a model set up there (`CronCreate`) is listed beside these, and can be deleted.
//!
//! Harnix not running, nothing is delivered; when it starts again, what fell due meanwhile is
//! delivered once (the latest of it) or skipped, as the schedule says. See
//! `docs/harnix-app.md` §9.
//!
//! ```text
//! wire.rs    the rows and params (`schedule.*`, the topic `schedule.list`)
//! timing.rs  when a timing is due, in its zone
//! store.rs   schedules.json: written whole, revisioned; receipts kept
//! core.rs    the scheduler: create, update, deliver, the timer
//! host.rs    what it reaches through the extension's context
//! ```

pub mod core;
pub mod host;
pub mod store;
pub mod timing;
pub mod wire;

use std::future::Future;
use std::sync::{Arc, Mutex};

use harnix_ext::{
    caps, Access, Call, Capability, CapabilitySpec, Extension, ExtensionContext, Manifest, Registrar, RowScope, Rpc,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

pub use crate::core::{Host, Scheduler};
use crate::wire::{CreateParams, HistoryParams, IdParams, ListParams, ModelDeleteParams, UpdateParams};

/// The extension's id: its methods are `schedule.*`.
pub const ID: &str = "schedule";

/// The topic carrying every schedule: `schedule.list`.
pub const TOPIC: &str = "list";

/// What another extension holds to manage schedules. A device needs nothing: a person manages
/// their own.
pub const MANAGE: Capability = Capability::new("schedule.manage");

/// What the schedule extension is granted: reading the engines' own jobs (`engine.inspect`) and
/// deleting one (`engine.control`).
pub const GRANTS: [Capability; 2] = [caps::ENGINE_INSPECT, caps::ENGINE_CONTROL];

/// The schedule extension (`schedule`).
#[derive(Default)]
pub struct Schedules {
    runner: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Schedules {
    pub fn new() -> Self {
        Self::default()
    }
}

/// `f` answered from the scheduler, the call's subject handed in.
fn method<C, P, R, F, Fut>(r: &mut Registrar<'_, C>, name: &str, describe: &str, f: F)
where
    P: DeserializeOwned + Send + 'static,
    R: Serialize + Send + 'static,
    F: Fn(Arc<Scheduler>, Call, P) -> Fut + Send + Sync + Copy + 'static,
    Fut: Future<Output = Rpc<R>> + Send + 'static,
{
    r.method(name)
        .access(Access::device_or_capability(MANAGE).scoped(RowScope::Space))
        .describe(describe)
        .handler(move |call: Call, p: P| async move {
            let scheduler = call.state::<Scheduler>()?;
            f(scheduler, call, p).await
        });
}

#[async_trait::async_trait]
impl Extension for Schedules {
    type Config = ();

    fn manifest(&self) -> Manifest {
        Manifest::new(ID, env!("CARGO_PKG_VERSION"))
            .title("Schedules")
            .disclosure(
                "定时：到点替定时的主人往会话里发一轮、在项目里新建会话、或起一个任务（flow.start）；\
                 读、删模型在引擎里建的定时（engine.schedule.*）",
            )
            .defines(CapabilitySpec::new("schedule.manage", "建、改、删、暂停、立即执行定时，读它们的投递记录"))
            // Without them the engines' own jobs are neither listed nor deleted.
            .optional(GRANTS)
    }

    fn register(&self, r: &mut Registrar<'_, ()>) {
        method(r, "list", "schedules, soonest first, and the engines' own jobs: {space?, project?}", |s, _, p: ListParams| async move {
            Ok(s.list(p).await)
        });
        method(r, "get", "one schedule: {id}", |s, _, p: IdParams| async move { s.get(&p) });
        method(
            r,
            "create",
            "a schedule: {space?, title, timing, time_zone?, target, missed?, concurrency?, enabled?}",
            |s, call, p: CreateParams| async move { s.create(&call.subject().label(), p).await },
        );
        method(
            r,
            "update",
            "change a schedule: {id, revision, title?, timing?, time_zone?, target?, missed?, concurrency?}; CONFLICT when the revision moved on",
            |s, _, p: UpdateParams| async move { s.update(p).await },
        );
        method(r, "delete", "delete a schedule and its receipts: {id}", |s, _, p: IdParams| async move { s.delete(&p).await });
        method(r, "enable", "switch a schedule on: next due after now", |s, _, p: IdParams| async move {
            s.set_enabled(&p, true).await
        });
        method(r, "disable", "switch a schedule off", |s, _, p: IdParams| async move { s.set_enabled(&p, false).await });
        method(
            r,
            "runNow",
            "deliver a schedule now, on or off, its next time unchanged; a minute after the last delivery at the soonest",
            |s, _, p: IdParams| async move { s.run_now(&p).await },
        );
        method(r, "history", "a schedule's receipts, newest first: {id, limit?}", |s, _, p: HistoryParams| async move {
            s.history(&p)
        });
        method(
            r,
            "model.delete",
            "delete a job a model made in the engine: {instance, id}",
            |s, _, p: ModelDeleteParams| async move { s.delete_model_job(&p).await },
        );
        r.topic(TOPIC).access(Access::device_or_capability(MANAGE));
    }

    async fn start(&self, cx: ExtensionContext) -> anyhow::Result<()> {
        let host = Arc::new(host::Hub { cx: cx.clone() });
        let scheduler = Scheduler::open(Some(cx.data_dir().join("schedules.json")), host, core::system_clock());
        cx.provide_arc(scheduler.clone());
        scheduler.publish().await;
        let runner = tokio::spawn(scheduler.run());
        *self.runner.lock().unwrap_or_else(|p| p.into_inner()) = Some(runner);
        Ok(())
    }

    async fn stop(&self) {
        if let Some(runner) = self.runner.lock().unwrap_or_else(|p| p.into_inner()).take() {
            runner.abort();
        }
    }
}
