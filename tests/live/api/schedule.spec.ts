/**
 * Schedules (harnix-app.md §9): the `schedule` extension in the Harnix app. Every kind of timing
 * is made with its next time and a bad one refused; «run now» sends the prompt to a session as a
 * turn and leaves a receipt, a minute apart at the soonest; a task target starts a run the
 * schedule is said to have started; an edit names the revision it read; a call that claims a
 * space reaches that space's schedules only; the engine's own jobs are no device's to reach
 * directly, and are listed beside the schedules.
 *
 * Replayed: `says-ready` (the turn «run now» sends). Nothing here waits for a time to come: the
 * timer and its policies are the crate's tests (`crates/ext/schedule/tests/scheduler.rs`).
 */

import { mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { CONFLICT, INVALID_PARAMS, REFUSED, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:schedule", "@layer:api"] };
const PROMPT = "Say exactly: fixture ready";

let backend: Backend;
let api: TestClient;
let project: string;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
  project = realpathSync(mkdtempSync(join(tmpdir(), "harnix-schedule-")));
});
test.afterAll(async () => {
  await backend?.dispose();
  rmSync(project, { recursive: true, force: true });
});
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

/** A session of the session space that replays `says-ready`, watched. */
async function aSession(): Promise<{ instance: string; session_id: string; space: string }> {
  const created = await api.call("harnix.session.create", { space: "sessions", options: { recorder: { name: "says-ready" } } });
  const s = { instance: created.instance, session_id: created.session_id, space: "sessions" };
  await api.call("harnix.session.attach", s);
  return s;
}

const toSession = (s: { instance: string; session_id: string }) => ({
  kind: "session",
  instance: s.instance,
  session_id: s.session_id,
  prompt: PROMPT,
});

test("every kind of timing is made with its next time; one that is never due is refused", TAGS, async () => {
  const s = await aSession();
  const timings = [
    { kind: "once", at: "2099-06-01T09:00" },
    { kind: "after", seconds: 3600 },
    { kind: "every", minutes: 15 },
    { kind: "daily", time: "09:00" },
    { kind: "weekly", weekdays: [1, 2, 3, 4, 5], time: "09:00" },
    { kind: "cron", expr: "0 9 1 * *" },
  ];
  const before = Date.now();
  for (const timing of timings) {
    const made = await api.call("schedule.create", { space: "sessions", title: `Kind ${timing.kind}`, timing, time_zone: "Asia/Shanghai", target: toSession(s) });
    expect(made, timing.kind).toMatchObject({
      title: `Kind ${timing.kind}`,
      enabled: true,
      time_zone: "Asia/Shanghai",
      space: "sessions",
      revision: 1,
      missed: "catch_up_latest",
      concurrency: "skip",
      target: { kind: "session", session_id: s.session_id },
    });
    expect(made.created_by).toMatch(/^device:/);
    expect(made.next_fire, timing.kind).toBeGreaterThan(before);
  }
  // Every N minutes counts from when it was made.
  const every = (await api.call("schedule.list", { space: "sessions" })).schedules.find((x: any) => x.title === "Kind every");
  expect(every.next_fire - every.timing.from).toBe(15 * 60_000);
  // A new session in a project, and a task: made, nothing sent.
  const opens = await api.call("schedule.create", {
    title: "Opens one",
    timing: { kind: "daily", time: "08:00" },
    target: { kind: "new_session", project_root: project, prompt: PROMPT },
  });
  expect(opens).toMatchObject({ space: "sessions", project, target: { kind: "new_session", mode: "new_once_then_reuse" } });
  expect(opens.time_zone).toBeTruthy();

  const refused = async (params: Record<string, unknown>) =>
    (await api.refused("schedule.create", { title: "Bad", timing: { kind: "daily", time: "09:00" }, target: toSession(s), ...params }))?.code;
  expect(await refused({ timing: { kind: "once", at: "2001-01-01T00:00:00Z" } })).toBe(INVALID_PARAMS);
  expect(await refused({ timing: { kind: "cron", expr: "0 0 31 2 *" } })).toBe(INVALID_PARAMS);
  expect(await refused({ timing: { kind: "every", minutes: 0 } })).toBe(INVALID_PARAMS);
  expect(await refused({ time_zone: "Nowhere/Else" })).toBe(INVALID_PARAMS);
  expect(await refused({ title: " " })).toBe(INVALID_PARAMS);
  expect(await refused({ target: { kind: "task", flow: "example-review", subject: "x", project_root: "relative/dir" } })).toBe(INVALID_PARAMS);
  // A session of another space than the one the call claims.
  expect(await refused({ space: "flow" })).toBe(SPACE_MISMATCH);
});

test("run now sends the prompt as a turn and leaves a receipt; a minute apart at the soonest", TAGS, async () => {
  const s = await aSession();
  const made = await api.call("schedule.create", {
    space: "sessions",
    title: "Ready check",
    timing: { kind: "daily", time: "09:00" },
    target: toSession(s),
  });
  const receipt = await api.call("schedule.runNow", { id: made.id, space: "sessions" });
  expect(receipt).toMatchObject({ outcome: "delivered", manual: true, target: { instance: s.instance, session_id: s.session_id } });
  const settled = await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id, {
    timeout: 60_000,
    describe: "the scheduled turn to settle",
  });
  expect(settled.params.error).toBeUndefined();
  const history = await api.call("engine.session.history", s);
  expect(JSON.stringify(history.messages)).toContain(PROMPT);

  // Its next time is where it was; the receipt is the last delivery and the history's first.
  const got = await api.call("schedule.get", { id: made.id });
  expect(got.next_fire).toBe(made.next_fire);
  expect(got.last_delivery).toMatchObject({ id: receipt.id, outcome: "delivered" });
  const kept = await api.call("schedule.history", { id: made.id });
  expect(kept.total).toBe(1);
  expect(kept.receipts[0]).toMatchObject({ id: receipt.id, scheduled_at: receipt.scheduled_at, outcome: "delivered" });
  expect(kept.receipts[0].delivered_at).toBeGreaterThanOrEqual(receipt.scheduled_at);

  const again = await api.call("schedule.runNow", { id: made.id });
  expect(again).toMatchObject({ outcome: "skipped", reason: "min_interval" });
  expect((await api.call("schedule.history", { id: made.id })).total).toBe(2);
});

test("a task target starts a run said to be the schedule's", TAGS, async () => {
  const made = await api.call("schedule.create", {
    title: "Nightly review",
    timing: { kind: "daily", time: "02:00" },
    target: { kind: "task", flow: "example-review", subject: "nightly", project_root: project },
  });
  expect(made).toMatchObject({ project, concurrency: "skip", target: { kind: "task", flow: "example-review" } });
  const receipt = await api.call("schedule.runNow", { id: made.id });
  expect(receipt.outcome).toBe("delivered");
  const run = receipt.target.run;
  expect(run).toMatch(/^example-review-/);
  const rec = (await api.call("flow.get", { run })).record;
  expect(rec.project_root).toBe(project);
  expect(rec.trigger).toEqual({ kind: "schedule", schedule: made.id, fire_at: receipt.scheduled_at });
  // The project's tasks list it.
  expect((await api.call("flow.list", { project })).map((r: any) => r.id)).toContain(run);
  await api.call("flow.cancel", { run, reason: "done" });
});

test("an edit names the revision it read; switched off and on, it is due after now", TAGS, async () => {
  const s = await aSession();
  const made = await api.call("schedule.create", { title: "Edited", timing: { kind: "daily", time: "09:00" }, target: toSession(s) });
  const first = await api.call("schedule.update", { id: made.id, revision: 1, title: "Edited once" });
  expect(first).toMatchObject({ title: "Edited once", revision: 2 });
  // Another device still holds revision 1.
  const stale = await api.refused("schedule.update", { id: made.id, revision: 1, title: "Edited twice" });
  expect(stale?.code).toBe(CONFLICT);
  expect((stale?.data as any)?.revision).toBe(2);
  expect((await api.call("schedule.get", { id: made.id })).title).toBe("Edited once");

  const retimed = await api.call("schedule.update", { id: made.id, revision: 2, timing: { kind: "cron", expr: "30 7 * * *" } });
  expect(retimed.revision).toBe(3);
  expect(retimed.next_fire).not.toBe(made.next_fire);

  const off = await api.call("schedule.disable", { id: made.id });
  expect(off).toMatchObject({ enabled: false, revision: 4 });
  expect(off.next_fire).toBeUndefined();
  const on = await api.call("schedule.enable", { id: made.id });
  expect(on).toMatchObject({ enabled: true, revision: 5 });
  expect(on.next_fire).toBeGreaterThan(Date.now());

  expect(await api.call("schedule.delete", { id: made.id })).toEqual({ deleted: true });
  expect(await api.call("schedule.delete", { id: made.id })).toEqual({ deleted: false });
});

test("a call that claims a space reaches that space's schedules only; the topic carries them all", TAGS, async () => {
  const s = await aSession();
  const sub = await api.call("harnix.topics.subscribe", { topic: "schedule.list" });
  expect(sub.value).toMatchObject({ schedules: expect.any(Array), model: expect.any(Array) });
  const made = await api.call("schedule.create", { space: "sessions", title: "Mine", timing: { kind: "every", minutes: 30 }, target: toSession(s) });
  const pushed = await api.waitFor(
    (f) => f.method === "harnix.topic" && f.params.topic === "schedule.list" && f.params.value.schedules.some((x: any) => x.id === made.id),
    { describe: "the new schedule on the topic" },
  );
  expect(pushed.params.value.schedules.find((x: any) => x.id === made.id).space).toBe("sessions");

  expect((await api.call("schedule.list", { space: "sessions" })).schedules.map((x: any) => x.id)).toContain(made.id);
  expect((await api.call("schedule.list", { space: "flow" })).schedules.map((x: any) => x.id)).not.toContain(made.id);
  expect((await api.call("schedule.list", {})).schedules.map((x: any) => x.id)).toContain(made.id);
  for (const method of ["schedule.get", "schedule.runNow", "schedule.disable", "schedule.delete", "schedule.history"]) {
    expect((await api.refused(method, { id: made.id, space: "flow" }))?.code, method).toBe(SPACE_MISMATCH);
  }
  expect((await api.refused("schedule.update", { id: made.id, revision: 1, space: "flow", title: "x" }))?.code).toBe(SPACE_MISMATCH);
  await api.call("schedule.delete", { id: made.id, space: "sessions" });
});

test("the engine's own jobs: listed beside the schedules, no device's to reach directly", TAGS, async () => {
  const listed = await api.call("schedule.list", {});
  // The model made none here; what it makes is listed under `model`.
  expect(listed.model).toEqual([]);
  expect(listed.unreachable).toEqual(expect.any(Array));
  // Deleted through the extension, which holds what it takes: a job that is not there is not.
  expect(await api.call("schedule.model.delete", { instance: "main", id: "no-such-job" })).toEqual({ deleted: false });
  expect((await api.refused("engine.schedule.list", {}))?.code).toBe(REFUSED);
  expect((await api.refused("engine.schedule.delete", { id: "j1" }))?.code).toBe(REFUSED);
  expect((await api.refused("engine.schedule.create", { session_id: "x", cron: "* * * * *", prompt: "x" }))?.code).toBeTruthy();
  // The schedules are a device's to manage, and reachable as such.
  const { methods, topics } = await api.call("harnix.reachable");
  for (const m of ["schedule.list", "schedule.create", "schedule.update", "schedule.runNow", "schedule.history", "schedule.model.delete"]) {
    expect(methods).toContain(m);
  }
  expect(topics).toContain("schedule.list");
  expect(methods).not.toContain("engine.schedule.list");
});
