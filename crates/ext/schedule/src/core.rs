//! The scheduler: the schedules and their receipts, what is due, and delivering it. What it
//! reaches outside itself — sessions, flows, the engine's own jobs, the topic — is a [`Host`], so
//! the same code runs against the hub (`host.rs`) and against a fake in tests, on a clock it is
//! handed.
//!
//! Deliveries go one at a time. Each writes a pending receipt first; for a new session the
//! session's id is decided before it is opened — derived from the schedule and the time — and is
//! on that receipt before the session exists. A receipt still pending at start is one Harnix
//! stopped in the middle of. One that was opening a new session is delivered again: opening the
//! same id finds the session the first attempt made, and a session that already has been sent
//! something is not sent the prompt again, so no time opens two sessions. Any other is failed,
//! not tried again (whether its turn was sent or its run started is not known). Times are moved
//! on before they are delivered.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use harnix_ext::{codes, RpcError};
use jiff::tz::TimeZone;
use serde_json::json;

use crate::store::{File, Kept, Ledger};
use crate::timing::{self, Plan};
use crate::wire::*;

/// The least time between two deliveries of one schedule, «run now» included.
pub const MIN_INTERVAL_MS: u64 = 60_000;
/// How late a time may be delivered and still count as on time; later, it was missed.
pub const GRACE_MS: u64 = 60_000;
/// The longest the runner sleeps without looking again (a clock that jumps is seen within it).
pub const MAX_SLEEP_MS: u64 = 30_000;
/// Titles are at most this many characters.
pub const MAX_TITLE: usize = 120;

/// What a delivery reaches. Errors are as the hub answers them.
#[async_trait]
pub trait Host: Send + Sync {
    /// The space a call that claims none belongs to.
    async fn default_space(&self) -> String;
    /// A session of `space`, and the project it runs in. `SPACE_MISMATCH` when it is another
    /// space's; the engine's error when there is no such session.
    async fn session_project(&self, space: &str, session: &SessionKey) -> Result<Option<String>, RpcError>;
    /// Send `prompt` as a turn, behind a running one: the queued item's id when it waits.
    async fn send(&self, space: &str, session: &SessionKey, prompt: &str) -> Result<Option<String>, RpcError>;
    /// Whether `item` still waits in the session's queue.
    async fn still_queued(&self, space: &str, session: &SessionKey, item: &str, prompt: &str) -> bool;
    /// Open the session `session_id` (in the engine's form) in `space` as the target says — the
    /// one that exists under that id already, when there is one, with `true`.
    async fn create_session(
        &self,
        space: &str,
        target: &NewSessionTarget,
        session_id: &str,
    ) -> Result<(SessionKey, bool), RpcError>;
    /// Whether a session has been sent anything: a message in it, or a turn running.
    async fn session_started(&self, space: &str, session: &SessionKey) -> Result<bool, RpcError>;
    async fn rename(&self, space: &str, session: &SessionKey, title: &str) -> Result<(), RpcError>;
    /// `flow.start`, said to be by this schedule at `fire_at`: the run's id.
    async fn start_task(&self, task: &TaskTarget, schedule: &str, fire_at: u64) -> Result<String, RpcError>;
    /// Whether a run is still going.
    async fn run_active(&self, run: &str) -> Result<bool, RpcError>;
    /// The engines' own jobs (a space's sessions' only, when one is named), and the instances
    /// whose could not be read.
    async fn model_jobs(&self, space: Option<&str>) -> (Vec<ModelJob>, Vec<String>);
    async fn delete_model_job(&self, space: Option<&str>, instance: &str, id: &str) -> Result<bool, RpcError>;
    /// The topic's new value.
    async fn publish(&self, list: ScheduleList);
}

/// Milliseconds since the epoch, now.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// The system's clock.
pub fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
    })
}

pub struct Scheduler {
    ledger: Mutex<Ledger>,
    file: File,
    host: Arc<dyn Host>,
    now: Clock,
    /// Deliveries, one at a time: the runner's and «run now»'s.
    delivering: tokio::sync::Mutex<()>,
    /// Something changed that may move the next time.
    wake: tokio::sync::Notify,
    /// The engines' jobs as last read: what the topic carries.
    model: Mutex<(Vec<ModelJob>, Vec<String>)>,
    /// Deliveries to a new session found pending at start: done again on the first tick.
    interrupted: Mutex<Vec<(String, Receipt)>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn missing(id: &str) -> RpcError {
    RpcError::missing(format!("there is no schedule `{id}`"))
}

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::invalid_params(message.into())
}

/// A call that claims a space reaches that space's schedules only.
fn within(schedule: &Schedule, claimed: Option<&str>) -> Result<(), RpcError> {
    match claimed {
        Some(space) if !space.is_empty() && space != schedule.space => Err(RpcError::with_data(
            codes::SPACE_MISMATCH,
            format!("schedule `{}` belongs to the space `{}`, not `{space}`", schedule.id, schedule.space),
            json!({"space": schedule.space, "id": schedule.id}),
        )),
        _ => Ok(()),
    }
}

/// A project's directory as given: an existing absolute one.
fn project_dir(root: &str) -> Result<String, RpcError> {
    let root = root.trim();
    if !Path::new(root).is_absolute() {
        return Err(invalid(format!("project_root {root:?} is not an absolute path")));
    }
    match std::fs::metadata(root) {
        Ok(m) if m.is_dir() => Ok(root.to_string()),
        _ => Err(invalid(format!("project_root {root:?} is not an existing directory"))),
    }
}

fn title_of(title: &str) -> Result<String, RpcError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(invalid("a title is not empty"));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(invalid(format!("a title is at most {MAX_TITLE} characters")));
    }
    Ok(title.to_string())
}

/// Two timings that differ only in where an elapsed one counts from.
fn same_rule(a: &Timing, b: &Timing) -> bool {
    let bare = |t: &Timing| match t {
        Timing::After { seconds, .. } => Timing::After { seconds: *seconds, from: None },
        Timing::Every { minutes, .. } => Timing::Every { minutes: *minutes, from: None },
        other => other.clone(),
    };
    bare(a) == bare(b)
}

/// `timing` anchored at `now` when it counts elapsed time.
fn anchored(timing: Timing, now: u64) -> Timing {
    match timing {
        Timing::After { seconds, .. } => Timing::After { seconds, from: Some(now) },
        Timing::Every { minutes, .. } => Timing::Every { minutes, from: Some(now) },
        other => other,
    }
}

fn plan_of(s: &Schedule) -> Option<(Plan, TimeZone)> {
    let tz = timing::zone(&s.time_zone).ok()?;
    let plan = Plan::of(&s.timing, &tz, s.created_at).ok()?;
    Some((plan, tz))
}

impl Scheduler {
    /// The schedules kept at `file` (none: in memory). A receipt left pending is failed.
    pub fn open(file: Option<PathBuf>, host: Arc<dyn Host>, now: Clock) -> Arc<Self> {
        let file = File::at(file);
        let mut ledger = file.load();
        let at = now();
        let mut changed = false;
        let mut interrupted = Vec::new();
        for kept in &mut ledger.schedules {
            let pending: Vec<Receipt> =
                kept.history.iter().filter(|r| r.outcome == Outcome::Pending).cloned().collect();
            for mut r in pending {
                let opening = r.target.as_ref().is_some_and(|t| t.created && t.session_id.is_some());
                if opening && matches!(kept.schedule.target, Target::NewSession(_)) {
                    interrupted.push((kept.schedule.id.clone(), r));
                    continue;
                }
                r.outcome = Outcome::Failed;
                r.reason = Some("interrupted: Harnix stopped while it was being delivered".into());
                r.delivered_at.get_or_insert(at);
                kept.record(r, at);
                changed = true;
            }
        }
        if changed {
            if let Err(e) = file.save(&mut ledger) {
                tracing::warn!("schedules not written: {e}");
            }
        }
        Arc::new(Self {
            ledger: Mutex::new(ledger),
            file,
            host,
            now,
            delivering: tokio::sync::Mutex::new(()),
            wake: tokio::sync::Notify::new(),
            model: Mutex::new((Vec::new(), Vec::new())),
            interrupted: Mutex::new(interrupted),
        })
    }

    fn save(&self, ledger: &mut Ledger) {
        if let Err(e) = self.file.save(ledger) {
            tracing::warn!("schedules not written: {e}");
        }
    }

    /// Change one kept schedule and write the ledger.
    fn with_kept<T>(&self, id: &str, f: impl FnOnce(&mut Kept) -> Result<T, RpcError>) -> Result<T, RpcError> {
        let mut ledger = lock(&self.ledger);
        let kept = ledger.schedules.iter_mut().find(|k| k.schedule.id == id).ok_or_else(|| missing(id))?;
        let out = f(kept)?;
        self.save(&mut ledger);
        Ok(out)
    }

    fn kept(&self, id: &str) -> Result<Kept, RpcError> {
        lock(&self.ledger).schedules.iter().find(|k| k.schedule.id == id).cloned().ok_or_else(|| missing(id))
    }

    /// Soonest due first; those due no more after, newest first.
    fn sorted(mut all: Vec<Schedule>) -> Vec<Schedule> {
        all.sort_by(|a, b| match (a.next_fire, b.next_fire) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => b.created_at.cmp(&a.created_at),
        });
        all
    }

    /// The schedules, of a space and of a project when they are named.
    pub fn schedules(&self, space: Option<&str>, project: Option<&str>) -> Vec<Schedule> {
        let all = lock(&self.ledger)
            .schedules
            .iter()
            .map(|k| k.schedule.clone())
            .filter(|s| space.is_none_or(|sp| sp.is_empty() || s.space == sp))
            .filter(|s| project.is_none_or(|p| s.project.as_deref() == Some(p)))
            .collect();
        Self::sorted(all)
    }

    /// `schedule.list`: the schedules and the engines' jobs, read now.
    pub async fn list(&self, p: ListParams) -> ScheduleList {
        let schedules = self.schedules(p.space.as_deref(), p.project.as_deref());
        let (model, unreachable) = self.host.model_jobs(p.space.as_deref()).await;
        if p.space.is_none() {
            *lock(&self.model) = (model.clone(), unreachable.clone());
        }
        // A project's own: the engines' jobs are per session, and their project is not read here.
        let model = if p.project.is_some() { Vec::new() } else { model };
        ScheduleList { schedules, model, unreachable }
    }

    /// Push the topic's value: every schedule, and the engines' jobs as last read.
    pub async fn publish(&self) {
        let (model, unreachable) = lock(&self.model).clone();
        let list = ScheduleList { schedules: self.schedules(None, None), model, unreachable };
        self.host.publish(list).await;
    }

    /// Something changed: publish, and let the runner look again.
    async fn changed(&self) {
        self.wake.notify_one();
        self.publish().await;
    }

    pub fn get(&self, p: &IdParams) -> Result<Schedule, RpcError> {
        let kept = self.kept(&p.id)?;
        within(&kept.schedule, p.space.as_deref())?;
        Ok(kept.schedule)
    }

    pub fn history(&self, p: &HistoryParams) -> Result<History, RpcError> {
        let kept = self.kept(&p.id)?;
        within(&kept.schedule, p.space.as_deref())?;
        let total = kept.history.len();
        let receipts = kept.history.into_iter().rev().take(p.limit.unwrap_or(50).max(1)).collect();
        Ok(History { receipts, total })
    }

    /// A target as it will be kept: checked against what it names, its server-kept parts
    /// carried over from `before` when it still names the same thing. Answers it and its project.
    async fn checked_target(
        &self,
        space: &str,
        target: Target,
        before: Option<&Target>,
    ) -> Result<(Target, Option<String>), RpcError> {
        match target {
            Target::Session(t) => {
                if t.prompt.trim().is_empty() {
                    return Err(invalid("a prompt is not empty"));
                }
                let key = SessionKey { instance: t.instance.clone(), session_id: t.session_id.clone() };
                let project = self.host.session_project(space, &key).await?;
                Ok((Target::Session(t), project))
            }
            Target::NewSession(mut t) => {
                if t.prompt.trim().is_empty() {
                    return Err(invalid("a prompt is not empty"));
                }
                t.project_root = t.project_root.as_deref().map(project_dir).transpose()?;
                // The session it reuses is the scheduler's to say; kept while it would open the
                // same kind of session.
                t.session = match before {
                    Some(Target::NewSession(b))
                        if t.mode == NewMode::NewOnceThenReuse
                            && (b.project_root.as_ref(), b.scene.as_ref(), b.instance.as_ref())
                                == (t.project_root.as_ref(), t.scene.as_ref(), t.instance.as_ref()) =>
                    {
                        b.session.clone()
                    }
                    _ => None,
                };
                let project = t.project_root.clone();
                Ok((Target::NewSession(t), project))
            }
            Target::Task(mut t) => {
                if t.flow.trim().is_empty() {
                    return Err(invalid("a task names its flow"));
                }
                if t.subject.trim().is_empty() {
                    return Err(invalid("a task has a subject"));
                }
                if !(t.inputs.is_null() || t.inputs.is_object()) {
                    return Err(invalid("a task's inputs are an object"));
                }
                if let Some(w) = &t.workspace {
                    if !["project", "worktree"].contains(&w.as_str()) {
                        return Err(invalid(format!("workspace is project or worktree, not {w:?}")));
                    }
                }
                t.project_root = project_dir(&t.project_root)?;
                let project = Some(t.project_root.clone());
                Ok((Target::Task(t), project))
            }
        }
    }

    /// When it is next due from `now`, if it is on; checked to be due at all.
    fn next_fire(timing: &Timing, time_zone: &str, created_at: u64, now: u64) -> Result<Option<u64>, RpcError> {
        let tz = timing::zone(time_zone).map_err(invalid)?;
        let plan = Plan::of(timing, &tz, created_at).map_err(invalid)?;
        Ok(plan.next_after(&tz, now))
    }

    /// `schedule.create`, by `by` (a subject's label).
    pub async fn create(&self, by: &str, p: CreateParams) -> Result<Schedule, RpcError> {
        let now = (self.now)();
        let title = title_of(&p.title)?;
        let time_zone = p.time_zone.filter(|z| !z.trim().is_empty()).unwrap_or_else(timing::local_zone_name);
        let timing = anchored(p.timing, now);
        let next_fire = Self::next_fire(&timing, &time_zone, now, now)?;
        if next_fire.is_none() {
            return Err(invalid("it would never be due: a one-off time is in the future"));
        }
        let space = match p.space.filter(|s| !s.is_empty()) {
            Some(space) => space,
            None => self.host.default_space().await,
        };
        let (target, project) = self.checked_target(&space, p.target, None).await?;
        let enabled = p.enabled.unwrap_or(true);
        let schedule = Schedule {
            id: format!("sch-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
            title,
            enabled,
            timing,
            time_zone,
            target,
            missed: p.missed.unwrap_or_default(),
            concurrency: p.concurrency.unwrap_or_default(),
            space,
            project,
            created_by: by.to_string(),
            created_at: now,
            revision: 1,
            next_fire: next_fire.filter(|_| enabled),
            last_delivery: None,
        };
        {
            let mut ledger = lock(&self.ledger);
            ledger.schedules.push(Kept { schedule: schedule.clone(), history: Vec::new(), last_delivered_at: None });
            self.save(&mut ledger);
        }
        self.changed().await;
        Ok(schedule)
    }

    /// `schedule.update`: what is given changes; a revision other than the current is `CONFLICT`.
    pub async fn update(&self, p: UpdateParams) -> Result<Schedule, RpcError> {
        let now = (self.now)();
        let before = self.kept(&p.id)?.schedule;
        within(&before, p.space.as_deref())?;
        conflict(&before, p.revision)?;
        let title = p.title.as_deref().map(title_of).transpose()?;
        let (target, project) = match p.target {
            Some(target) => {
                let (t, project) = self.checked_target(&before.space, target, Some(&before.target)).await?;
                (Some(t), project)
            }
            None => (None, before.project.clone()),
        };
        let timing = match p.timing {
            Some(t) if same_rule(&t, &before.timing) => before.timing.clone(),
            Some(t) => anchored(t, now),
            None => before.timing.clone(),
        };
        let time_zone = p.time_zone.filter(|z| !z.trim().is_empty()).unwrap_or_else(|| before.time_zone.clone());
        let retimed = timing != before.timing || time_zone != before.time_zone;
        let next = if retimed {
            let next = Self::next_fire(&timing, &time_zone, before.created_at, now)?;
            if next.is_none() && matches!(timing, Timing::Once { .. }) && timing != before.timing {
                return Err(invalid("it would never be due: a one-off time is in the future"));
            }
            Some(next)
        } else {
            None
        };
        let updated = self.with_kept(&p.id, |kept| {
            // Checked again under the lock: the one written is the one read.
            conflict(&kept.schedule, p.revision)?;
            let s = &mut kept.schedule;
            if let Some(title) = title {
                s.title = title;
            }
            if let Some(target) = target {
                s.target = target;
            }
            s.project = project;
            s.timing = timing;
            s.time_zone = time_zone;
            if let Some(missed) = p.missed {
                s.missed = missed;
            }
            if let Some(c) = p.concurrency {
                s.concurrency = c;
            }
            if let Some(next) = next {
                s.next_fire = next.filter(|_| s.enabled);
            }
            s.revision += 1;
            Ok(s.clone())
        })?;
        self.changed().await;
        Ok(updated)
    }

    pub async fn delete(&self, p: &IdParams) -> Result<Deleted, RpcError> {
        let deleted = {
            let mut ledger = lock(&self.ledger);
            let Some(at) = ledger.schedules.iter().position(|k| k.schedule.id == p.id) else {
                return Ok(Deleted { deleted: false });
            };
            within(&ledger.schedules[at].schedule, p.space.as_deref())?;
            ledger.schedules.remove(at);
            self.save(&mut ledger);
            true
        };
        self.changed().await;
        Ok(Deleted { deleted })
    }

    /// `schedule.enable` / `disable`. Switched on, it is next due after now: what fell due while
    /// it was off is not made up.
    pub async fn set_enabled(&self, p: &IdParams, enabled: bool) -> Result<Schedule, RpcError> {
        let now = (self.now)();
        let s = self.with_kept(&p.id, |kept| {
            within(&kept.schedule, p.space.as_deref())?;
            let s = &mut kept.schedule;
            if s.enabled != enabled {
                s.enabled = enabled;
                s.revision += 1;
                s.next_fire = if enabled { plan_of(s).and_then(|(plan, tz)| plan.next_after(&tz, now)) } else { None };
            }
            Ok(s.clone())
        })?;
        self.changed().await;
        Ok(s)
    }

    /// `schedule.model.delete`.
    pub async fn delete_model_job(&self, p: &ModelDeleteParams) -> Result<Deleted, RpcError> {
        let deleted = self.host.delete_model_job(p.space.as_deref(), &p.instance, &p.id).await?;
        {
            let mut model = lock(&self.model);
            model.0.retain(|j| !(j.instance == p.instance && j.id == p.id));
        }
        self.publish().await;
        Ok(Deleted { deleted })
    }

    /// `schedule.runNow`: delivered now, whether it is on or not, and its next time stays.
    pub async fn run_now(&self, p: &IdParams) -> Result<Receipt, RpcError> {
        let kept = self.kept(&p.id)?;
        within(&kept.schedule, p.space.as_deref())?;
        let receipt = self.deliver(&p.id, (self.now)(), true).await;
        Ok(receipt)
    }

    /// Deliver what is due now, and say when the next thing is.
    pub async fn tick(&self) -> Option<u64> {
        let interrupted = std::mem::take(&mut *lock(&self.interrupted));
        for (id, receipt) in interrupted {
            self.deliver_again(&id, receipt).await;
        }
        let now = (self.now)();
        let mut due = Vec::new();
        let mut skipped = false;
        {
            let mut ledger = lock(&self.ledger);
            for kept in &mut ledger.schedules {
                let s = &mut kept.schedule;
                let Some(first) = s.next_fire.filter(|t| s.enabled && *t <= now) else { continue };
                let Some((plan, tz)) = plan_of(s) else {
                    s.next_fire = None;
                    continue;
                };
                let latest = plan.latest_until(&tz, first, now);
                s.next_fire = plan.next_after(&tz, now);
                let late = now.saturating_sub(latest) > GRACE_MS;
                if late && s.missed == Missed::Skip {
                    let receipt = Receipt {
                        id: receipt_id(),
                        scheduled_at: latest,
                        delivered_at: Some(now),
                        manual: false,
                        outcome: Outcome::Skipped,
                        reason: Some("missed".into()),
                        target: None,
                    };
                    kept.record(receipt, now);
                    skipped = true;
                } else {
                    due.push((s.id.clone(), latest));
                }
            }
            if !due.is_empty() || skipped {
                self.save(&mut ledger);
            }
        }
        if skipped {
            self.publish().await;
        }
        for (id, at) in due {
            self.deliver(&id, at, false).await;
        }
        lock(&self.ledger).schedules.iter().filter_map(|k| k.schedule.next_fire.filter(|_| k.schedule.enabled)).min()
    }

    /// The runner: one timer for every schedule, until the task is dropped.
    pub async fn run(self: Arc<Self>) {
        loop {
            let next = self.tick().await;
            let now = (self.now)();
            let wait = next.map(|t| t.saturating_sub(now)).unwrap_or(MAX_SLEEP_MS).clamp(1, MAX_SLEEP_MS);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
                _ = self.wake.notified() => {}
            }
        }
    }

    /// Record `receipt` for `id`, minding when it was last delivered; publish.
    async fn record(&self, id: &str, receipt: Receipt, f: impl FnOnce(&mut Kept)) {
        let now = (self.now)();
        {
            let mut ledger = lock(&self.ledger);
            if let Some(kept) = ledger.schedules.iter_mut().find(|k| k.schedule.id == id) {
                if receipt.outcome == Outcome::Delivered {
                    kept.last_delivered_at = Some(receipt.delivered_at.unwrap_or(now));
                }
                f(kept);
                kept.record(receipt, now);
                self.save(&mut ledger);
            }
        }
        self.publish().await;
    }

    /// Deliver `id` for the time `scheduled_at`: the receipt, whatever became of it.
    pub async fn deliver(&self, id: &str, scheduled_at: u64, manual: bool) -> Receipt {
        let _one_at_a_time = self.delivering.lock().await;
        let now = (self.now)();
        let mut receipt = Receipt {
            id: receipt_id(),
            scheduled_at,
            delivered_at: None,
            manual,
            outcome: Outcome::Pending,
            reason: None,
            target: None,
        };
        let Ok(kept) = self.kept(id) else {
            receipt.outcome = Outcome::Failed;
            receipt.reason = Some("the schedule was deleted".into());
            return receipt;
        };
        let s = kept.schedule.clone();
        let skip = |mut r: Receipt, why: &str| {
            r.outcome = Outcome::Skipped;
            r.reason = Some(why.to_string());
            r.delivered_at = Some(now);
            r
        };
        if kept.last_delivered_at.is_some_and(|t| now < t + MIN_INTERVAL_MS) {
            let r = skip(receipt, "min_interval");
            self.record(id, r.clone(), |_| {}).await;
            return r;
        }
        // The last delivery to a session that still waits in its queue takes this one in.
        let waiting = kept.history.iter().rev().find(|r| r.outcome == Outcome::Delivered).and_then(|r| r.target.clone());
        let reused = match &s.target {
            Target::Session(t) => Some((SessionKey { instance: t.instance.clone(), session_id: t.session_id.clone() }, &t.prompt)),
            Target::NewSession(t) if t.mode == NewMode::NewOnceThenReuse => t.session.clone().map(|k| (k, &t.prompt)),
            _ => None,
        };
        if let (Some((key, prompt)), Some(DeliveredTo { item: Some(item), instance: Some(i), session_id: Some(sid), .. })) =
            (&reused, &waiting)
        {
            if (i, sid) == (&key.instance, &key.session_id) && self.host.still_queued(&s.space, key, item, prompt).await {
                let r = skip(receipt, "merged");
                self.record(id, r.clone(), |_| {}).await;
                return r;
            }
        }
        if let (Target::Task(_), Concurrency::Skip) = (&s.target, s.concurrency) {
            if let Some(run) = waiting.as_ref().and_then(|w| w.run.clone()) {
                if self.host.run_active(&run).await.unwrap_or(false) {
                    let r = skip(receipt, "previous_run");
                    self.record(id, r.clone(), |_| {}).await;
                    return r;
                }
            }
        }
        // Pending, before anything is opened or sent.
        self.record(id, receipt.clone(), |_| {}).await;
        let delivered = match &s.target {
            Target::Session(t) => {
                let key = SessionKey { instance: t.instance.clone(), session_id: t.session_id.clone() };
                self.host.send(&s.space, &key, &t.prompt).await.map(|item| sent_to(&key, item, false))
            }
            Target::NewSession(t) => self.new_session(id, &s, t, &mut receipt, scheduled_at).await,
            Target::Task(t) => self
                .host
                .start_task(t, &s.id, scheduled_at)
                .await
                .map(|run| DeliveredTo { run: Some(run), ..DeliveredTo::default() }),
        };
        receipt.delivered_at = Some((self.now)());
        match delivered {
            Ok(to) => {
                receipt.outcome = Outcome::Delivered;
                receipt.target = Some(to);
            }
            Err(e) => {
                receipt.outcome = Outcome::Failed;
                receipt.reason = Some(e.message);
            }
        }
        self.record(id, receipt.clone(), |_| {}).await;
        receipt
    }

    /// A new session's delivery: the session it reuses, or one opened now and written down before
    /// the prompt goes.
    async fn new_session(
        &self,
        id: &str,
        s: &Schedule,
        t: &NewSessionTarget,
        receipt: &mut Receipt,
        scheduled_at: u64,
    ) -> Result<DeliveredTo, RpcError> {
        let reuse = t.mode == NewMode::NewOnceThenReuse;
        if let (true, Some(key)) = (reuse, &t.session) {
            return match self.host.send(&s.space, key, &t.prompt).await {
                Ok(item) => Ok(sent_to(key, item, false)),
                Err(e) => {
                    // Gone, most likely: the next time opens a session again.
                    let _ = self.with_kept(id, |kept| {
                        if let Target::NewSession(t) = &mut kept.schedule.target {
                            t.session = None;
                        }
                        Ok(())
                    });
                    Err(e)
                }
            };
        }
        // The id first, on the pending receipt: from here on a restart opens this one, no other.
        let session_id = new_session_id(&s.id, scheduled_at);
        receipt.target = Some(DeliveredTo {
            instance: t.instance.clone(),
            session_id: Some(session_id.clone()),
            created: true,
            ..DeliveredTo::default()
        });
        self.record(id, receipt.clone(), |_| {}).await;
        self.open_and_send(id, s, t, receipt, scheduled_at, &session_id).await
    }

    /// Open the new session `session_id` (the one there already, when an earlier attempt made it),
    /// write it down, name it and send it the prompt — unless it has been sent something already.
    async fn open_and_send(
        &self,
        id: &str,
        s: &Schedule,
        t: &NewSessionTarget,
        receipt: &mut Receipt,
        scheduled_at: u64,
        session_id: &str,
    ) -> Result<DeliveredTo, RpcError> {
        let reuse = t.mode == NewMode::NewOnceThenReuse;
        let (key, existed) = self.host.create_session(&s.space, t, session_id).await?;
        receipt.target = Some(sent_to(&key, None, true));
        let reserved = key.clone();
        self.record(id, receipt.clone(), move |kept| {
            if let (true, Target::NewSession(t)) = (reuse, &mut kept.schedule.target) {
                t.session = Some(reserved);
            }
        })
        .await;
        if existed && self.host.session_started(&s.space, &key).await? {
            return Ok(sent_to(&key, None, true));
        }
        let zone = timing::zone(&s.time_zone).unwrap_or(TimeZone::UTC);
        let title = format!("{} · {}", s.title, timing::local_minute(scheduled_at, &zone));
        if let Err(e) = self.host.rename(&s.space, &key, &title).await {
            tracing::warn!(schedule = %s.id, "the new session was not named: {}", e.message);
        }
        let item = self.host.send(&s.space, &key, &t.prompt).await?;
        Ok(sent_to(&key, item, true))
    }

    /// A delivery to a new session that Harnix stopped in the middle of, done again: the session
    /// its receipt names opened again, and the receipt finished.
    async fn deliver_again(&self, id: &str, mut receipt: Receipt) {
        let _one_at_a_time = self.delivering.lock().await;
        let Ok(s) = self.kept(id).map(|k| k.schedule) else { return };
        let session_id = receipt.target.as_ref().and_then(|t| t.session_id.clone()).unwrap_or_default();
        let scheduled_at = receipt.scheduled_at;
        let delivered = match &s.target {
            Target::NewSession(t) => self.open_and_send(id, &s, t, &mut receipt, scheduled_at, &session_id).await,
            _ => Err(RpcError::new(codes::INTERNAL_ERROR, "the schedule no longer opens a new session")),
        };
        receipt.delivered_at = Some((self.now)());
        match delivered {
            Ok(to) => {
                receipt.outcome = Outcome::Delivered;
                receipt.target = Some(to);
            }
            Err(e) => {
                receipt.outcome = Outcome::Failed;
                receipt.reason = Some(format!("interrupted, and not delivered again: {}", e.message));
            }
        }
        self.record(id, receipt, |_| {}).await;
    }

    /// The schedules' ids, for tests.
    pub fn ids(&self) -> BTreeSet<String> {
        lock(&self.ledger).schedules.iter().map(|k| k.schedule.id.clone()).collect()
    }
}

fn conflict(s: &Schedule, revision: u64) -> Result<(), RpcError> {
    if s.revision == revision {
        return Ok(());
    }
    Err(RpcError::with_data(
        codes::CONFLICT,
        format!("schedule `{}` changed since revision {revision}: read it again", s.id),
        json!({"revision": s.revision}),
    ))
}

/// The id of the session a schedule opens for the time `scheduled_at`: a v5 UUID of the two,
/// written as the engine writes its own ids (BASE58), so another attempt at the same time opens
/// the same session.
pub fn new_session_id(schedule: &str, scheduled_at: u64) -> String {
    let ns = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, b"schedule.harnix");
    base58(uuid::Uuid::new_v5(&ns, format!("{schedule}/{scheduled_at}").as_bytes()).as_bytes())
}

/// BASE58 (the Bitcoin alphabet), as the engine writes a UUID.
fn base58(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let zeros = bytes.iter().take_while(|b| **b == 0).count();
    let mut digits: Vec<u8> = vec![];
    for &b in &bytes[zeros..] {
        let mut carry = b as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut out = String::with_capacity(zeros + digits.len());
    out.extend(std::iter::repeat_n('1', zeros));
    out.extend(digits.iter().rev().map(|&d| ALPHABET[d as usize] as char));
    out
}

fn receipt_id() -> String {
    format!("dlv-{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}

fn sent_to(key: &SessionKey, item: Option<String>, created: bool) -> DeliveredTo {
    DeliveredTo {
        instance: Some(key.instance.clone()),
        session_id: Some(key.session_id.clone()),
        created,
        item,
        run: None,
    }
}
