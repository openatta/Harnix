//! The scheduler on a clock the test moves, against fake sessions and a fake flow extension: what
//! is due is delivered once, late times follow the schedule's missed policy, deliveries keep a
//! minute apart, a task whose last run still goes is skipped, a queued turn takes the next one in,
//! a new session is written down before its prompt goes and reused after a restart, one Harnix
//! stopped while opening is opened again under the same id rather than a second time, edits name
//! their revision, and every time leaves a receipt.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use harnix_ext::{codes, RpcError};
use harnix_ext_schedule::core::{new_session_id, Clock, Host, Scheduler, MIN_INTERVAL_MS};
use harnix_ext_schedule::wire::*;
use serde_json::{json, Value};

const MIN: u64 = 60_000;
const DAY: u64 = 86_400_000;
/// 2026-06-01T00:00:00Z.
const T0: u64 = 1_780_272_000_000;

#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<String>>,
    /// What `send` answers as the queued item, when set.
    queued_as: Mutex<Option<String>>,
    still_queued: AtomicBool,
    run_active: AtomicBool,
    fail_send: AtomicBool,
    /// New sessions opened.
    sessions: AtomicU64,
    /// The sessions there are, by id, and whether each has been sent anything.
    made: Mutex<Vec<(String, bool)>>,
    /// `create_session` fails, as Harnix stopping right after the engine made the session would
    /// leave it: the session there, the scheduler not told.
    drop_create: AtomicBool,
    runs: AtomicU64,
    /// Where the ledger is, to read what was written when a prompt is sent.
    file: Mutex<Option<PathBuf>>,
    /// The ledger as it was when each prompt was sent.
    seen_at_send: Mutex<Vec<Value>>,
    published: AtomicU64,
}

impl Fake {
    fn log(&self, s: String) {
        self.calls.lock().unwrap().push(s);
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn sent(&self) -> Vec<String> {
        self.calls().into_iter().filter(|c| c.starts_with("send ")).collect()
    }
}

#[async_trait]
impl Host for Fake {
    async fn default_space(&self) -> String {
        "sessions".into()
    }
    async fn session_project(&self, space: &str, s: &SessionKey) -> Result<Option<String>, RpcError> {
        match (space, s.session_id.as_str()) {
            (_, "gone") => Err(RpcError::new(-32000, "session not found")),
            ("sessions", _) => Ok(Some("/work/p".into())),
            _ => Err(RpcError::new(codes::SPACE_MISMATCH, "another space's")),
        }
    }
    async fn send(&self, space: &str, s: &SessionKey, prompt: &str) -> Result<Option<String>, RpcError> {
        if let Some(file) = self.file.lock().unwrap().clone() {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            self.seen_at_send.lock().unwrap().push(serde_json::from_str(&text).unwrap_or(Value::Null));
        }
        self.log(format!("send {space} {}/{} {prompt}", s.instance, s.session_id));
        if self.fail_send.load(Ordering::SeqCst) {
            return Err(RpcError::new(-32000, "session not found"));
        }
        if let Some(made) = self.made.lock().unwrap().iter_mut().find(|(id, _)| *id == s.session_id) {
            made.1 = true;
        }
        Ok(self.queued_as.lock().unwrap().clone())
    }
    async fn still_queued(&self, _: &str, _: &SessionKey, item: &str, _: &str) -> bool {
        self.log(format!("queued? {item}"));
        self.still_queued.load(Ordering::SeqCst)
    }
    async fn create_session(
        &self,
        space: &str,
        t: &NewSessionTarget,
        session_id: &str,
    ) -> Result<(SessionKey, bool), RpcError> {
        self.log(format!("create {space} {} {}", t.project_root.as_deref().unwrap_or("-"), t.model.as_deref().unwrap_or("-")));
        let existed = {
            let mut made = self.made.lock().unwrap();
            let existed = made.iter().any(|(id, _)| id == session_id);
            if !existed {
                made.push((session_id.to_string(), false));
                self.sessions.fetch_add(1, Ordering::SeqCst);
            }
            existed
        };
        if self.drop_create.load(Ordering::SeqCst) {
            return Err(RpcError::new(codes::INSTANCE_UNAVAILABLE, "the connection went"));
        }
        Ok((SessionKey { instance: "main".into(), session_id: session_id.to_string() }, existed))
    }
    async fn session_started(&self, _: &str, s: &SessionKey) -> Result<bool, RpcError> {
        self.log(format!("started? {}", s.session_id));
        Ok(self.made.lock().unwrap().iter().any(|(id, sent)| *id == s.session_id && *sent))
    }
    async fn rename(&self, _: &str, s: &SessionKey, title: &str) -> Result<(), RpcError> {
        self.log(format!("rename {} {title}", s.session_id));
        Ok(())
    }
    async fn start_task(&self, t: &TaskTarget, schedule: &str, fire_at: u64) -> Result<String, RpcError> {
        let n = self.runs.fetch_add(1, Ordering::SeqCst) + 1;
        self.log(format!("start {} {} {schedule} {fire_at}", t.flow, t.project_root));
        Ok(format!("run-{n}"))
    }
    async fn run_active(&self, run: &str) -> Result<bool, RpcError> {
        self.log(format!("active? {run}"));
        Ok(self.run_active.load(Ordering::SeqCst))
    }
    async fn model_jobs(&self, _: Option<&str>) -> (Vec<ModelJob>, Vec<String>) {
        let job = ModelJob {
            instance: "main".into(),
            id: "j1".into(),
            session_id: "s1".into(),
            cron: "0 9 * * *".into(),
            prompt: "standup".into(),
            recurring: true,
            created_ms: None,
            next_fire: None,
        };
        (vec![job], vec!["flow-x".into()])
    }
    async fn delete_model_job(&self, _: Option<&str>, instance: &str, id: &str) -> Result<bool, RpcError> {
        self.log(format!("delete job {instance}/{id}"));
        Ok(true)
    }
    async fn publish(&self, _: ScheduleList) {
        self.published.fetch_add(1, Ordering::SeqCst);
    }
}

struct Rig {
    fake: Arc<Fake>,
    clock: Arc<AtomicU64>,
    s: Arc<Scheduler>,
    dir: tempfile::TempDir,
    project: String,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(Fake::default());
        *fake.file.lock().unwrap() = Some(dir.path().join("schedules.json"));
        let clock = Arc::new(AtomicU64::new(T0));
        let s = Self::open(&dir, &fake, &clock);
        let project = dir.path().display().to_string();
        Rig { fake, clock, s, dir, project }
    }

    fn open(dir: &tempfile::TempDir, fake: &Arc<Fake>, clock: &Arc<AtomicU64>) -> Arc<Scheduler> {
        let c = clock.clone();
        let now: Clock = Arc::new(move || c.load(Ordering::SeqCst));
        Scheduler::open(Some(dir.path().join("schedules.json")), fake.clone(), now)
    }

    /// The same files and fakes, opened again: a restart.
    fn restart(&mut self) {
        self.s = Self::open(&self.dir, &self.fake, &self.clock);
    }

    fn at(&self, ms: u64) {
        self.clock.store(ms, Ordering::SeqCst);
    }

    async fn create(&self, timing: Timing, target: Target) -> Schedule {
        self.s.create("device:d1", params(timing, target)).await.unwrap()
    }

    fn history(&self, id: &str) -> Vec<Receipt> {
        self.s.history(&HistoryParams { id: id.into(), space: None, limit: Some(500) }).unwrap().receipts
    }
}

fn params(timing: Timing, target: Target) -> CreateParams {
    CreateParams {
        space: None,
        title: "Standup".into(),
        timing,
        time_zone: Some("UTC".into()),
        target,
        missed: None,
        concurrency: None,
        enabled: None,
    }
}

fn session(id: &str) -> Target {
    Target::Session(SessionTarget { instance: "main".into(), session_id: id.into(), prompt: "say ready".into() })
}

fn new_session(project: &str, mode: NewMode) -> Target {
    Target::NewSession(NewSessionTarget {
        mode,
        project_root: Some(project.into()),
        scene: Some("code".into()),
        instance: None,
        provider: None,
        model: Some("m1".into()),
        prompt: "check the build".into(),
        session: None,
    })
}

fn task(project: &str) -> Target {
    Target::Task(TaskTarget {
        flow: "example-review".into(),
        subject: "nightly".into(),
        inputs: json!({}),
        project_root: project.into(),
        title: None,
        output_dir: None,
        workspace: Some("worktree".into()),
        shown_in: None,
    })
}

fn daily(time: &str) -> Timing {
    Timing::Daily { time: time.into() }
}

fn id(s: &Schedule) -> IdParams {
    IdParams { id: s.id.clone(), space: None }
}

#[tokio::test]
async fn every_kind_is_made_with_its_next_time_and_bad_ones_are_refused() {
    let r = Rig::new();
    let once = r.create(Timing::Once { at: "2026-06-01T09:00".into() }, session("s1")).await;
    assert_eq!(once.next_fire, Some(T0 + 9 * 60 * MIN));
    assert_eq!((once.space.as_str(), once.project.as_deref(), once.revision), ("sessions", Some("/work/p"), 1));
    assert_eq!(once.created_by, "device:d1");
    let after = r.create(Timing::After { seconds: 90, from: Some(1) }, session("s1")).await;
    // Counted from when it was made, whatever the call said.
    assert_eq!(after.timing, Timing::After { seconds: 90, from: Some(T0) });
    assert_eq!(after.next_fire, Some(T0 + 90_000));
    let every = r.create(Timing::Every { minutes: 30, from: None }, new_session(&r.project, NewMode::NewEach)).await;
    assert_eq!(every.next_fire, Some(T0 + 30 * MIN));
    assert_eq!(every.project.as_deref(), Some(r.project.as_str()));
    let d = r.create(daily("09:00"), task(&r.project)).await;
    assert_eq!(d.next_fire, Some(T0 + 9 * 60 * MIN));
    assert_eq!(d.concurrency, Concurrency::Skip);
    assert_eq!(d.missed, Missed::CatchUpLatest);
    // 2026-06-01 is a Monday: the next Saturday.
    let w = r.create(Timing::Weekly { weekdays: vec![6], time: "10:00".into() }, session("s1")).await;
    assert_eq!(w.next_fire, Some(T0 + 5 * DAY + 10 * 60 * MIN));
    let c = r.create(Timing::Cron { expr: "15 * * * *".into() }, session("s1")).await;
    assert_eq!(c.next_fire, Some(T0 + 15 * MIN));
    // In another zone: 09:00 in Shanghai is 01:00 UTC.
    let mut p = params(daily("09:00"), session("s1"));
    p.time_zone = Some("Asia/Shanghai".into());
    assert_eq!(r.s.create("device:d1", p).await.unwrap().next_fire, Some(T0 + 60 * MIN));
    // Absent, the zone is this machine's.
    let mut p = params(daily("09:00"), session("s1"));
    p.time_zone = None;
    assert!(!r.s.create("device:d1", p).await.unwrap().time_zone.is_empty());

    let refused = |p: CreateParams| {
        let s = r.s.clone();
        async move { s.create("device:d1", p).await.unwrap_err() }
    };
    let past = refused(params(Timing::Once { at: "2026-05-31T09:00".into() }, session("s1"))).await;
    assert_eq!(past.code, codes::INVALID_PARAMS);
    let mut zone = params(daily("09:00"), session("s1"));
    zone.time_zone = Some("Nowhere/Else".into());
    assert_eq!(refused(zone).await.code, codes::INVALID_PARAMS);
    assert_eq!(refused(params(daily("25:00"), session("s1"))).await.code, codes::INVALID_PARAMS);
    assert_eq!(refused(params(Timing::Every { minutes: 0, from: None }, session("s1"))).await.code, codes::INVALID_PARAMS);
    assert_eq!(refused(params(Timing::Cron { expr: "0 0 31 2 *".into() }, session("s1"))).await.code, codes::INVALID_PARAMS);
    let mut untitled = params(daily("09:00"), session("s1"));
    untitled.title = "  ".into();
    assert_eq!(refused(untitled).await.code, codes::INVALID_PARAMS);
    assert_eq!(refused(params(daily("09:00"), task("relative/dir"))).await.code, codes::INVALID_PARAMS);
    assert_eq!(refused(params(daily("09:00"), session("gone"))).await.code, -32000);
    // A session of another space than the one claimed.
    let mut other = params(daily("09:00"), session("s1"));
    other.space = Some("flow".into());
    assert_eq!(refused(other).await.code, codes::SPACE_MISMATCH);

    assert_eq!(r.s.ids().len(), 8);
}

#[tokio::test]
async fn what_is_due_is_delivered_once_and_the_next_time_moves_on() {
    let r = Rig::new();
    let s = r.create(daily("09:00"), session("s1")).await;
    assert_eq!(r.s.tick().await, Some(T0 + 9 * 60 * MIN));
    assert!(r.fake.sent().is_empty());

    r.at(T0 + 9 * 60 * MIN + 500);
    assert_eq!(r.s.tick().await, Some(T0 + DAY + 9 * 60 * MIN));
    assert_eq!(r.fake.sent(), ["send sessions main/s1 say ready"]);
    // Looked at again in the same minute: nothing more.
    r.s.tick().await;
    assert_eq!(r.fake.sent().len(), 1);

    let got = r.s.get(&id(&s)).unwrap();
    assert_eq!(got.next_fire, Some(T0 + DAY + 9 * 60 * MIN));
    let last = got.last_delivery.unwrap();
    assert_eq!((last.outcome, last.scheduled_at, last.delivered_at), (Outcome::Delivered, T0 + 9 * 60 * MIN, Some(T0 + 9 * 60 * MIN + 500)));
    assert_eq!(last.target.unwrap().session_id.as_deref(), Some("s1"));
    // The revision is the people's: delivering does not move it.
    assert_eq!(got.revision, 1);
    assert!(r.fake.published.load(Ordering::SeqCst) > 0);

    // Off: not delivered; on again, next due after now — what fell due meanwhile is not made up.
    r.s.set_enabled(&id(&s), false).await.unwrap();
    r.at(T0 + 3 * DAY);
    r.s.tick().await;
    assert_eq!(r.fake.sent().len(), 1);
    let on = r.s.set_enabled(&id(&s), true).await.unwrap();
    assert_eq!(on.next_fire, Some(T0 + 3 * DAY + 9 * 60 * MIN));
    assert_eq!(on.revision, 3);
    r.s.tick().await;
    assert_eq!(r.fake.sent().len(), 1);
}

#[tokio::test]
async fn missed_times_are_caught_up_once_or_skipped() {
    let r = Rig::new();
    let catch_up = r.create(daily("09:00"), session("s1")).await;
    let mut p = params(daily("09:00"), session("s2"));
    p.missed = Some(Missed::Skip);
    let skip = r.s.create("device:d1", p).await.unwrap();

    // Three days on, as after Harnix was not running.
    r.restart_at(T0 + 3 * DAY + 12 * 60 * MIN).await;
    assert_eq!(r.fake.sent(), ["send sessions main/s1 say ready"]);
    let delivered = r.history(&catch_up.id);
    assert_eq!(delivered.len(), 1);
    // The latest of them, once.
    assert_eq!(delivered[0].scheduled_at, T0 + 3 * DAY + 9 * 60 * MIN);
    let skipped = r.history(&skip.id);
    assert_eq!((skipped.len(), skipped[0].outcome, skipped[0].reason.as_deref()), (1, Outcome::Skipped, Some("missed")));
    assert_eq!(skipped[0].scheduled_at, T0 + 3 * DAY + 9 * 60 * MIN);
    // Both are next due tomorrow.
    assert_eq!(r.s.get(&id(&skip)).unwrap().next_fire, Some(T0 + 4 * DAY + 9 * 60 * MIN));

    // A minute late at most is on time, even for one that skips what it missed.
    r.at(T0 + 4 * DAY + 9 * 60 * MIN + 59_000);
    r.s.tick().await;
    assert_eq!(r.history(&skip.id)[0].outcome, Outcome::Delivered);
}

impl Rig {
    async fn restart_at(&self, ms: u64) {
        self.at(ms);
        self.s.tick().await;
    }
}

#[tokio::test]
async fn deliveries_keep_a_minute_apart_run_now_included() {
    let r = Rig::new();
    let s = r.create(Timing::Every { minutes: 1, from: None }, session("s1")).await;
    let first = r.s.run_now(&id(&s)).await.unwrap();
    assert_eq!((first.outcome, first.manual, first.scheduled_at), (Outcome::Delivered, true, T0));
    // Run now does not move the next time.
    assert_eq!(r.s.get(&id(&s)).unwrap().next_fire, Some(T0 + MIN));
    let again = r.s.run_now(&id(&s)).await.unwrap();
    assert_eq!((again.outcome, again.reason.as_deref()), (Outcome::Skipped, Some("min_interval")));
    // Due 59 s after the last delivery: still too soon.
    r.at(T0 + MIN - 1_000);
    r.s.tick().await;
    r.at(T0 + MIN);
    r.s.tick().await;
    assert_eq!(r.history(&s.id)[0].reason.as_deref(), None);
    assert_eq!(r.fake.sent().len(), 2);
    r.at(T0 + MIN + 30_000);
    let soon = r.s.run_now(&id(&s)).await.unwrap();
    assert_eq!(soon.reason.as_deref(), Some("min_interval"));
    r.at(T0 + MIN + MIN_INTERVAL_MS);
    assert_eq!(r.s.run_now(&id(&s)).await.unwrap().outcome, Outcome::Delivered);
    // Off, it still runs by hand.
    r.s.set_enabled(&id(&s), false).await.unwrap();
    r.at(T0 + 10 * MIN);
    assert_eq!(r.s.run_now(&id(&s)).await.unwrap().outcome, Outcome::Delivered);
    assert_eq!(r.fake.sent().len(), 4);
}

#[tokio::test]
async fn a_turn_still_queued_takes_the_next_one_in() {
    let r = Rig::new();
    *r.fake.queued_as.lock().unwrap() = Some("q-1".into());
    let s = r.create(Timing::Every { minutes: 5, from: None }, session("s1")).await;
    r.at(T0 + 5 * MIN);
    r.s.tick().await;
    assert_eq!(r.history(&s.id)[0].target.as_ref().unwrap().item.as_deref(), Some("q-1"));
    r.fake.still_queued.store(true, Ordering::SeqCst);
    r.at(T0 + 10 * MIN);
    r.s.tick().await;
    let merged = &r.history(&s.id)[0];
    assert_eq!((merged.outcome, merged.reason.as_deref()), (Outcome::Skipped, Some("merged")));
    assert_eq!(r.fake.sent().len(), 1);
    // Once it has left the queue, the next one is sent.
    r.fake.still_queued.store(false, Ordering::SeqCst);
    r.at(T0 + 15 * MIN);
    r.s.tick().await;
    assert_eq!(r.fake.sent().len(), 2);
    assert!(r.fake.calls().contains(&"queued? q-1".to_string()));
}

#[tokio::test]
async fn a_task_starts_a_run_said_to_be_the_schedules_and_skips_while_the_last_one_goes() {
    let r = Rig::new();
    let s = r.create(Timing::Every { minutes: 10, from: None }, task(&r.project)).await;
    r.at(T0 + 10 * MIN);
    r.s.tick().await;
    assert_eq!(r.fake.calls(), [format!("start example-review {} {} {}", r.project, s.id, T0 + 10 * MIN)]);
    assert_eq!(r.history(&s.id)[0].target.as_ref().unwrap().run.as_deref(), Some("run-1"));

    r.fake.run_active.store(true, Ordering::SeqCst);
    r.at(T0 + 20 * MIN);
    r.s.tick().await;
    let skipped = &r.history(&s.id)[0];
    assert_eq!((skipped.outcome, skipped.reason.as_deref()), (Outcome::Skipped, Some("previous_run")));
    assert!(r.fake.calls().contains(&"active? run-1".to_string()));

    // In parallel, it starts anyway.
    let got = r.s.get(&id(&s)).unwrap();
    let up = UpdateParams {
        id: s.id.clone(),
        revision: got.revision,
        space: None,
        title: None,
        timing: None,
        time_zone: None,
        target: None,
        missed: None,
        concurrency: Some(Concurrency::Parallel),
    };
    r.s.update(up).await.unwrap();
    r.at(T0 + 30 * MIN);
    r.s.tick().await;
    assert_eq!(r.history(&s.id)[0].target.as_ref().unwrap().run.as_deref(), Some("run-2"));
}

#[tokio::test]
async fn a_new_session_is_written_down_before_its_prompt_and_reused_after_a_restart() {
    let mut r = Rig::new();
    let s = r.create(Timing::Every { minutes: 10, from: None }, new_session(&r.project, NewMode::NewOnceThenReuse)).await;
    r.at(T0 + 10 * MIN);
    r.s.tick().await;
    let calls = r.fake.calls();
    let new1 = new_session_id(&s.id, T0 + 10 * MIN);
    assert_eq!(calls[0], format!("create sessions {} m1", r.project));
    assert_eq!(calls[1], format!("rename {new1} Standup · 2026-06-01 00:10"));
    assert_eq!(calls[2], format!("send sessions main/{new1} check the build"));
    // When the prompt went, the session was already in the file: kept on the schedule, and on
    // the pending receipt.
    let seen = r.fake.seen_at_send.lock().unwrap()[0].clone();
    let kept = &seen["schedules"][0];
    assert_eq!(kept["schedule"]["target"]["session"]["session_id"], json!(new1));
    assert_eq!(kept["history"][0]["outcome"], "pending");
    assert_eq!(kept["history"][0]["target"]["session_id"], json!(new1));
    let first = &r.history(&s.id)[0];
    assert!(first.target.as_ref().unwrap().created);

    r.restart();
    r.at(T0 + 20 * MIN);
    r.s.tick().await;
    assert_eq!(r.fake.sessions.load(Ordering::SeqCst), 1, "{:?}", r.fake.calls());
    assert_eq!(r.fake.sent().last().unwrap(), &format!("send sessions main/{new1} check the build"));

    // Gone: that time fails, and the next opens another.
    r.fake.fail_send.store(true, Ordering::SeqCst);
    r.at(T0 + 30 * MIN);
    r.s.tick().await;
    assert_eq!(r.history(&s.id)[0].outcome, Outcome::Failed);
    r.fake.fail_send.store(false, Ordering::SeqCst);
    r.at(T0 + 40 * MIN);
    r.s.tick().await;
    assert_eq!(r.fake.sessions.load(Ordering::SeqCst), 2);

    // Each time a new one, when it says so.
    let each = r.create(Timing::Every { minutes: 10, from: None }, new_session(&r.project, NewMode::NewEach)).await;
    for n in 1..=2 {
        r.at(T0 + 40 * MIN + n * 10 * MIN);
        r.s.tick().await;
    }
    assert_eq!(r.fake.sessions.load(Ordering::SeqCst), 4);
    assert!(matches!(r.s.get(&id(&each)).unwrap().target, Target::NewSession(NewSessionTarget { session: None, .. })));
}

#[tokio::test]
async fn a_new_session_harnix_stopped_while_opening_is_opened_again_not_twice() {
    let mut r = Rig::new();
    let s = r.create(Timing::Every { minutes: 10, from: None }, new_session(&r.project, NewMode::NewEach)).await;
    let due = T0 + 10 * MIN;
    let sid = new_session_id(&s.id, due);
    // The engine made the session, and the answer never came back: Harnix stopped there.
    r.fake.drop_create.store(true, Ordering::SeqCst);
    r.at(due);
    r.s.tick().await;
    assert_eq!(r.fake.sessions.load(Ordering::SeqCst), 1);
    // The id is the schedule's and the time's, in the engine's form, and was on the receipt
    // before the session was asked for.
    assert_eq!(sid, new_session_id(&s.id, due));
    assert_ne!(sid, new_session_id(&s.id, due + 10 * MIN));
    assert!((21..=22).contains(&sid.len()) && sid.chars().all(|c| c.is_ascii_alphanumeric()), "{sid}");
    assert_eq!(r.history(&s.id)[0].target.as_ref().unwrap().session_id.as_deref(), Some(sid.as_str()));
    let path = r.dir.path().join("schedules.json");
    // Harnix stopped there: the receipt still pending.
    let mut ledger: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let pending = json!({"id": "dlv-x", "scheduled_at": due, "outcome": "pending", "target": {"session_id": sid, "created": true}});
    ledger["schedules"][0]["history"] = json!([pending]);
    std::fs::write(&path, ledger.to_string()).unwrap();
    r.fake.drop_create.store(false, Ordering::SeqCst);

    r.restart();
    assert_eq!(r.history(&s.id)[0].outcome, Outcome::Pending);
    r.at(due + MIN);
    r.s.tick().await;
    assert_eq!(r.fake.sessions.load(Ordering::SeqCst), 1, "{:?}", r.fake.calls());
    assert_eq!(r.fake.sent(), vec![format!("send sessions main/{sid} check the build")]);
    let h = r.history(&s.id);
    assert_eq!((h.len(), h[0].outcome, h[0].scheduled_at), (1, Outcome::Delivered, due));
    assert_eq!(h[0].target.as_ref().unwrap().session_id.as_deref(), Some(sid.as_str()));

    // Stopped again after the prompt went: opened again, found sent, and not sent twice.
    let mut ledger: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    ledger["schedules"][0]["history"] = json!([pending]);
    std::fs::write(&path, ledger.to_string()).unwrap();
    r.restart();
    r.s.tick().await;
    assert_eq!(r.fake.sessions.load(Ordering::SeqCst), 1);
    assert_eq!(r.fake.sent().len(), 1, "{:?}", r.fake.calls());
    assert_eq!(r.history(&s.id)[0].outcome, Outcome::Delivered);
}

#[tokio::test]
async fn a_delivery_harnix_stopped_in_the_middle_of_is_failed_and_not_tried_again() {
    let mut r = Rig::new();
    let s = r.create(daily("09:00"), session("s1")).await;
    // Written as it is while a delivery is under way, and the process gone.
    let path = r.dir.path().join("schedules.json");
    let mut ledger: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    ledger["schedules"][0]["history"] = json!([{ "id": "dlv-x", "scheduled_at": T0, "outcome": "pending" }]);
    ledger["schedules"][0]["schedule"]["next_fire"] = json!(T0 + 9 * 60 * MIN);
    std::fs::write(&path, ledger.to_string()).unwrap();
    r.restart();
    let h = r.history(&s.id);
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].outcome, Outcome::Failed);
    assert!(h[0].reason.as_deref().unwrap().starts_with("interrupted"));
    r.s.tick().await;
    assert!(r.fake.sent().is_empty());
    // And the file says so too.
    let back: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(back["schedules"][0]["history"][0]["outcome"], "failed");
}

#[tokio::test]
async fn an_edit_names_the_revision_it_read() {
    let r = Rig::new();
    let s = r.create(daily("09:00"), session("s1")).await;
    let edit = |revision: u64, title: &str| UpdateParams {
        id: s.id.clone(),
        revision,
        space: None,
        title: Some(title.into()),
        timing: None,
        time_zone: None,
        target: None,
        missed: None,
        concurrency: None,
    };
    let one = r.s.update(edit(1, "From one device")).await.unwrap();
    assert_eq!((one.title.as_str(), one.revision), ("From one device", 2));
    // Another device still holds revision 1.
    let stale = r.s.update(edit(1, "From another")).await.unwrap_err();
    assert_eq!(stale.code, codes::CONFLICT);
    assert_eq!(stale.data.unwrap()["revision"], 2);
    assert_eq!(r.s.get(&id(&s)).unwrap().title, "From one device");

    // A new timing moves the next time; the same one sent back changes nothing.
    let mut up = edit(2, "From one device");
    up.timing = Some(daily("10:30"));
    let retimed = r.s.update(up).await.unwrap();
    assert_eq!(retimed.next_fire, Some(T0 + 10 * 60 * MIN + 30 * MIN));
    let every = r.create(Timing::Every { minutes: 10, from: None }, session("s1")).await;
    r.at(T0 + 3 * MIN);
    let mut same = edit(1, "Every ten");
    same.id = every.id.clone();
    same.timing = Some(Timing::Every { minutes: 10, from: None });
    assert_eq!(r.s.update(same).await.unwrap().next_fire, Some(T0 + 10 * MIN));
    let mut other = edit(2, "Every five");
    other.id = every.id.clone();
    other.timing = Some(Timing::Every { minutes: 5, from: None });
    assert_eq!(r.s.update(other).await.unwrap().next_fire, Some(T0 + 8 * MIN));

    // Gone: not found.
    assert!(r.s.delete(&id(&s)).await.unwrap().deleted);
    assert_eq!(r.s.get(&id(&s)).unwrap_err().code, codes::NOT_FOUND);
    assert_eq!(r.s.update(edit(3, "x")).await.unwrap_err().code, codes::NOT_FOUND);
    assert!(!r.s.delete(&id(&s)).await.unwrap().deleted);
}

#[tokio::test]
async fn a_space_reaches_its_own_schedules_only() {
    let r = Rig::new();
    let s = r.create(daily("09:00"), session("s1")).await;
    let mine = |space: &str| IdParams { id: s.id.clone(), space: Some(space.into()) };
    assert!(r.s.get(&mine("sessions")).is_ok());
    assert_eq!(r.s.get(&mine("flow")).unwrap_err().code, codes::SPACE_MISMATCH);
    assert_eq!(r.s.run_now(&mine("flow")).await.unwrap_err().code, codes::SPACE_MISMATCH);
    assert_eq!(r.s.delete(&mine("flow")).await.unwrap_err().code, codes::SPACE_MISMATCH);
    assert_eq!(r.s.set_enabled(&mine("flow"), false).await.unwrap_err().code, codes::SPACE_MISMATCH);

    let listed = r.s.list(ListParams { space: Some("flow".into()), project: None }).await;
    assert!(listed.schedules.is_empty());
    let all = r.s.list(ListParams::default()).await;
    assert_eq!(all.schedules.len(), 1);
    // The engines' own jobs beside them, and the instances that did not answer.
    assert_eq!(all.model[0].prompt, "standup");
    assert_eq!(all.unreachable, ["flow-x"]);
    let by_project = r.s.list(ListParams { space: None, project: Some("/work/p".into()) }).await;
    assert_eq!(by_project.schedules.len(), 1);
    assert!(by_project.model.is_empty());
    assert!(r.s.list(ListParams { space: None, project: Some("/elsewhere".into()) }).await.schedules.is_empty());

    assert!(r.s.delete_model_job(&ModelDeleteParams { instance: "main".into(), id: "j1".into(), space: None }).await.unwrap().deleted);
    assert!(r.fake.calls().contains(&"delete job main/j1".to_string()));
}

#[tokio::test]
async fn receipts_are_read_newest_first() {
    let r = Rig::new();
    let s = r.create(Timing::Every { minutes: 1, from: None }, session("s1")).await;
    for n in 1..=5 {
        r.at(T0 + n * MIN);
        r.s.tick().await;
    }
    let h = r.s.history(&HistoryParams { id: s.id.clone(), space: None, limit: Some(3) }).unwrap();
    assert_eq!(h.total, 5);
    assert_eq!(h.receipts.iter().map(|r| r.scheduled_at).collect::<Vec<_>>(), [T0 + 5 * MIN, T0 + 4 * MIN, T0 + 3 * MIN]);
    assert!(h.receipts.iter().all(|r| r.outcome == Outcome::Delivered && r.delivered_at.is_some()));
    // Kept across a restart.
    let mut r = r;
    r.restart();
    assert_eq!(r.history(&s.id).len(), 5);
}

#[tokio::test(start_paused = true)]
async fn the_runner_wakes_for_what_is_due() {
    let r = Rig::new();
    // The runner sleeps on tokio's clock; this test moves both clocks together.
    let s = r.create(Timing::After { seconds: 120, from: None }, session("s1")).await;
    let runner = tokio::spawn(r.s.clone().run());
    tokio::task::yield_now().await;
    assert!(r.fake.sent().is_empty());
    for _ in 0..4 {
        let now = r.clock.load(Ordering::SeqCst);
        r.at(now + 30_000);
        tokio::time::advance(std::time::Duration::from_millis(30_000)).await;
        tokio::task::yield_now().await;
    }
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(r.fake.sent().len(), 1, "{:?}", r.fake.calls());
    assert_eq!(r.s.get(&id(&s)).unwrap().next_fire, None);
    runner.abort();
}
