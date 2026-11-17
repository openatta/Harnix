/**
 * The flow module (xops-01) in `harnix-testapp`: runs of the two demonstration flows, driven
 * through `flow.*` and watched on `flow.run {run}`.
 *
 * Session stations replay `flow-analyze` and `flow-report`: each session calls `flow_submit`
 * with a fixed output and ends its turn, so what is under test is the module — dispatch, the
 * native tools, joins, skips, confirmations, leases, rework and force, and carrying on after
 * a restart — never the model.
 */

import { existsSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:flow", "@layer:api"] };
const REFUSED = -31000;
const INVALID = -32602;

type View = { record: any; unmet: Record<string, any[]> };

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(async () => { await backend?.dispose(); });
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

async function start(flow: string, extra: Record<string, unknown> = {}): Promise<string> {
  const { run } = await api.call("flow.start", { flow, subject: "repo", ...extra });
  expect(run).toMatch(new RegExp(`^${flow}-`));
  return run;
}

/** The run's view once `ok` holds, from the `flow.run` topic. */
async function until(run: string, ok: (v: View) => boolean, describe: string): Promise<View> {
  const sub = await api.call("harnix.topics.subscribe", { topic: "flow.run", params: { run } });
  if (sub.value && ok(sub.value)) return sub.value;
  const frame = await api.waitFor(
    (f) =>
      f.method === "harnix.topic" &&
      f.params.topic === "flow.run" &&
      f.params.params?.run === run &&
      ok(f.params.value),
    { timeout: 60_000, describe: `${run}: ${describe}` },
  );
  return frame.params.value;
}

const station = (v: View, id: string) => v.record.stations[id];
const status = (id: string, s: string) => (v: View) => station(v, id)?.status === s;

test("the module says what it offers: flows, a definition with its parts, the catalog, schemas", TAGS, async () => {
  const defs = await api.call("flow.defs");
  expect(defs.map((d: any) => d.name).sort()).toEqual([
    "demo-agent", "demo-asked", "demo-bypassed", "demo-guarded", "demo-linear", "demo-main", "demo-misplaced", "demo-outside",
    "demo-outside-strict", "demo-placed", "demo-propose", "demo-review", "demo-shared", "demo-task",
  ]);
  // Where each comes from and whether it is valid (xops-02 §3): all built in here.
  expect(defs.every((d: any) => d.origin === "builtin" && d.valid === true)).toBe(true);
  const info = await api.call("flow.def", { name: "demo-review" });
  expect(info).toMatchObject({ origin: "builtin", valid: true, problems: [], writable: true });
  expect(info.def.stations.map((s: any) => s.id)).toEqual(["collect", "analyze", "lint", "self-check", "review", "report"]);
  expect(info.yaml).toContain("name: demo-review");
  expect(Object.keys(info.parts.skills).sort()).toEqual(["analyze", "report"]);
  expect(Object.keys(info.parts.rules)).toEqual(["review-rules"]);
  expect(Object.keys(info.parts.gates).sort()).toEqual(["lint", "open-high"]);
  expect(info.stations.analyze.rules).toEqual(["review-rules"]);
  expect(info.protocol.analyze).toContain("### Review rules");
  expect(info.protocol.analyze).toContain("flow_submit");
  expect(info.digest).toMatch(/^[0-9a-f]{64}$/);

  const catalog = await api.call("flow.catalog");
  // The wrapped scenes (flow_submit) and the application's own, for its templates' sessions
  // (structured output, xops-02 §2).
  expect(catalog.scenes).toEqual(["flow-code", "flow-harnix", "flow-mini", "harnix"]);
  // `guard: auto` (the default): no approvals inside the project, nothing outside it.
  expect(info.protocol.analyze).toContain("work freely inside the project");
  expect(info.protocol.analyze).not.toContain("Refused unless a permission allows them");
  // `guard: strict`: the guarded tools are refused unless a permission allows them.
  const strict = await api.call("flow.def", { name: "demo-propose" });
  expect(strict.def.guard).toBe("strict");
  expect(strict.protocol.propose).toContain("Refused unless a permission allows them: Bash, PowerShell, REPL, Write, Edit, NotebookEdit, TodoWrite");
  expect(catalog.steps).toEqual(["collect", "self-check"]);
  expect(catalog).toMatchObject({ checks: ["lint"], tools: ["open_high"], command_gates: true });
  expect(catalog.parts).toMatchObject({
    skill: ["analyze", "report", "write-notes", "write-outside"], rule: ["review-rules"], policy: ["ask-todo"],
    gate: ["lint", "open-high", "self-test", "task-check"], hook: [],
  });
  const def = await api.call("flow.schema", { of: "definition" });
  expect(def.properties.stations).toBeTruthy();
  const run = await api.call("flow.schema", { of: "run" });
  expect(run.properties.stations).toBeTruthy();

  expect(await api.call("flow.validate", { yaml: info.yaml })).toEqual([]);
  const problems = await api.call("flow.validate", {
    yaml: "name: bad\nstations: [{id: a, executor: {kind: program, step: nowhere}, exit: [{kind: output}]}]",
  });
  expect(problems.length).toBeGreaterThan(0);
  expect(JSON.stringify(problems)).toContain("nowhere");
  const unwrapped = await api.call("flow.validate", {
    yaml: "name: bare\nstations: [{id: a, executor: {kind: session, scene: mini}, skill: analyze, output: {type: object}, exit: [{kind: output}]}]",
  });
  expect(JSON.stringify(unwrapped)).toContain("scene mini");
});

test("lease tokens stay with their claimants, and push tasks are the flow's to hand in", TAGS, async () => {
  const run = await start("demo-review");
  const review = await until(run, status("review", "working"), "review waits");
  const text = JSON.stringify(review);
  expect(text).not.toContain('"token"');
  expect(JSON.stringify(await api.call("flow.get", { run }))).not.toContain('"token"');
  await api.call("flow.confirm", { run, station: "review", pass: true });
  await until(run, (v) => station(v, "report")?.tasks?.some((t: any) => t.id === "sign"), "report entered");
  const { token } = await api.call("flow.claim", { run, station: "report", task: "sign" });
  const after = await api.call("flow.get", { run });
  expect(JSON.stringify(after)).not.toContain(token);
  // The summary is a session task: a device holding no token, or any token, cannot hand it in.
  const denied = await api.refused("flow.submit", { run, station: "report", task: "summary", token, output: {} });
  expect(denied?.data?.flow).toMatch(/denied|lease/);
  // A lease past the cap is cut to a day, not overflowed.
  const hb = await api.call("flow.heartbeat", { run, station: "report", task: "sign", token, lease_ms: Number.MAX_SAFE_INTEGER });
  expect(hb.until).toBeLessThanOrEqual(Date.now() + 24 * 3600 * 1000 + 60_000);
  await api.call("flow.cancel", { run, reason: "done" });
});

test("a held station waits for its release; an external task that failed is retried and handed in", TAGS, async () => {
  await api.call("flow.save", {
    yaml: [
      "name: held-external",
      "stations:",
      "  - id: sign",
      "    hold: true",
      "    executor: {kind: external, who: reviewer}",
      "    output: {type: object}",
      "    exit: [{kind: output}]",
    ].join("\n"),
  });
  const run = await start("held-external");
  await until(run, status("sign", "held"), "sign is held");
  // Nothing can be claimed until the station is let in, and only a held one can be.
  expect(await api.refused("flow.claim", { run, station: "sign", task: "sign" })).toBeTruthy();
  await api.call("flow.release", { run, station: "sign" });
  await until(run, status("sign", "working"), "sign is let in");
  expect((await api.refused("flow.release", { run, station: "sign" }))?.code).toBe(INVALID);

  // A task that has not failed cannot be retried; one claimed under another token cannot be failed.
  expect((await api.refused("flow.retry", { run, station: "sign", task: "sign" }))?.code).toBe(INVALID);
  const first = await api.call("flow.claim", { run, station: "sign", task: "sign" });
  expect(await api.refused("flow.fail", { run, station: "sign", task: "sign", token: "nope", error: "x" })).toBeTruthy();
  await api.call("flow.fail", { run, station: "sign", task: "sign", token: first.token, error: "the signer was away" });
  const failed = await until(run, (v) => station(v, "sign")?.tasks?.[0]?.status === "failed", "the task failed");
  expect(JSON.stringify(station(failed, "sign").tasks[0])).toContain("the signer was away");

  await api.call("flow.retry", { run, station: "sign", task: "sign" });
  await until(run, (v) => station(v, "sign")?.tasks?.[0]?.status === "ready", "the task is ready again");
  const second = await api.call("flow.claim", { run, station: "sign", task: "sign" });
  await api.call("flow.submit", { run, station: "sign", task: "sign", token: second.token, output: { signed: true } });
  await until(run, (v) => v.record.status === "finished", "the run is finished");
  await api.call("flow.delete", { name: "held-external" });
});

test("each run method refuses what it cannot do, and says which run, station or task", TAGS, async () => {
  // Nothing of an unknown run.
  const nowhere = { run: "no-such-run", station: "report", task: "sign" };
  for (const [method, params] of [
    ["flow.get", { run: nowhere.run }],
    ["flow.history", { run: nowhere.run }],
    ["flow.claim", nowhere],
    ["flow.heartbeat", { ...nowhere, token: "t" }],
    ["flow.cancel", { run: nowhere.run, reason: "x" }],
    ["flow.pause", { run: nowhere.run }],
    ["flow.resume", { run: nowhere.run }],
    ["flow.confirm", { run: nowhere.run, station: "review", pass: true }],
    ["flow.rework", { run: nowhere.run, to: "analyze", reason: "x" }],
    ["flow.sessions", { run: nowhere.run }],
    ["flow.dry_run.step", { run: nowhere.run }],
    ["flow.dry_run.run", { run: nowhere.run }],
  ] as const) {
    const e = await api.refused(method, params);
    expect(e, `${method} answered for a run that does not exist`).toBeTruthy();
    expect(JSON.stringify(e), method).toMatch(/no-such-run|not found|unknown/i);
  }
  // Nor an unknown flow, part or guide language.
  expect(await api.refused("flow.start", { flow: "no-such-flow", subject: "x" })).toBeTruthy();
  expect(await api.refused("flow.def", { name: "no-such-flow" })).toBeTruthy();
  expect(await api.refused("flow.part", { kind: "skill", name: "no-such-skill" })).toBeTruthy();
  expect(await api.refused("flow.part", { kind: "nothing", name: "x" })).toBeTruthy();
  expect(await api.refused("flow.part.delete", { kind: "skill", name: "analyze" }), "a built-in part is not deleted").toBeTruthy();

  // What a live run is not ready for.
  const run = await start("demo-review");
  await until(run, status("review", "working"), "review waits");
  expect((await api.refused("flow.resume", { run }))?.code, "a run that is not paused").toBe(INVALID);
  expect(await api.refused("flow.confirm", { run, station: "report", pass: true }), "a station not entered").toBeTruthy();
  expect(await api.refused("flow.claim", { run, station: "analyze", task: "nope" }), "a task that does not exist").toBeTruthy();
  expect(await api.refused("flow.rework", { run, to: "no-such-station", reason: "x" })).toBeTruthy();
  expect(await api.refused("flow.cancel", { run }), "a cancel says why").toBeTruthy();
  await api.call("flow.pause", { run });
  expect((await api.refused("flow.pause", { run }))?.code, "paused twice").toBe(INVALID);
  await api.call("flow.resume", { run });
  await api.call("flow.cancel", { run, reason: "done" });
  expect((await api.refused("flow.confirm", { run, station: "review", pass: true }))?.code, "a cancelled run").toBe(INVALID);
});

test("a client that unsubscribes is told no more; one still subscribed is", TAGS, async () => {
  const run = await start("demo-review");
  await until(run, status("review", "working"), "review waits");
  const watcher = await connect(backend);
  try {
    await watcher.call("harnix.topics.subscribe", { topic: "flow.run", params: { run } });
    await watcher.call("harnix.topics.subscribe", { topic: "flow.runs" });
    await watcher.call("harnix.topics.unsubscribe", { topic: "flow.run", params: { run } });
    expect((await watcher.refused("harnix.topics.unsubscribe", {}))?.code, "a topic is named").toBe(INVALID);
    const mark = watcher.events.length;
    await api.call("flow.cancel", { run, reason: "unsubscribed" });
    // Both topics change with the cancel: the one still subscribed says so, the other stays quiet.
    await watcher.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === "flow.runs"
        && JSON.stringify(f.params.value).includes(run) && JSON.stringify(f.params.value).includes("cancelled"),
      { describe: "the runs list to show the cancel" },
    );
    await until(run, (v) => v.record.status === "cancelled", "the run is cancelled");
    const late = watcher.events.slice(mark).filter((f) => f.method === "harnix.topic" && f.params.topic === "flow.run");
    expect(late, "a frame of the topic unsubscribed from").toEqual([]);
  } finally {
    watcher.close();
  }
});

test("a session's write no permission allows is refused, its proposal is accepted once", TAGS, async () => {
  const run = await start("demo-propose");
  const waiting = await until(run, status("approve", "working"), "approve waits");
  const task = station(waiting, "propose").tasks[0];
  expect(task.status).toBe("submitted");
  const [instance, session_id] = task.session.split("/");
  const history = JSON.stringify((await api.call("engine.session.history", { instance, session_id })).messages);
  expect(history).toContain("Refused unless a permission allows them");
  expect(history).toContain("station propose of the flow does not allow TodoWrite");

  const { entries } = await api.call("engine.session.entries", { instance, session_id });
  const proposal = entries.find((e: any) => e.ns === "flow" && e.event === "proposal");
  expect(proposal.payload).toMatchObject({ run, station: "propose", kind: "rework", to: "propose", session: { instance, session_id } });
  const ref = { instance, session_id, entry: proposal.id };
  expect((await api.call("flow.proposal", ref)).handled).toBeNull();

  await api.call("flow.rework", { run, to: "propose", reason: proposal.payload.reason, proposal: ref });
  const again = await api.call("flow.rework", { run, to: "propose", reason: proposal.payload.reason, proposal: ref });
  expect(again.already).toMatchObject({ entry: proposal.id, action: "rework", by: { kind: "device" } });
  expect((await api.call("flow.proposal", ref)).handled).toMatchObject({ action: "rework" });
  const hist = await api.call("flow.history", { run });
  expect(hist.filter((h: any) => h.op === "rework")).toHaveLength(1);
  // The second round runs in a new session and hands in again.
  await until(run, (v) => station(v, "propose").round === 2 && station(v, "approve").status === "working", "the second round");
  await api.call("flow.cancel", { run, reason: "done" });
});

test("a linear run: a program, a session that hands in through flow_submit, a confirmation", TAGS, async () => {
  const run = await start("demo-linear");
  const waiting = await until(run, status("approve", "working"), "approve waits for a confirmation");
  expect(station(waiting, "collect").tasks[0].output).toEqual({ modules: ["core", "ui"] });
  const analyze = station(waiting, "analyze");
  expect(analyze.status).toBe("passed");
  const task = analyze.tasks[0];
  expect(task.status).toBe("submitted");
  expect(task.output.findings).toHaveLength(1);
  // The session it ran in is an ordinary one, in the flow space, and it called the tool.
  expect(task.session).toMatch(/^main\//);
  const session_id = task.session.split("/")[1];
  const { sessions } = await api.call("harnix.sessions.list");
  expect(sessions.find((s: any) => s.session_id === session_id)?.space).toBe("flow");
  const history = await api.call("engine.session.history", { session_id });
  const text = JSON.stringify(history.messages);
  expect(text).toContain("flow_submit");
  expect(text).toContain("Flow protocol");

  const hist = await api.call("flow.history", { run });
  const submit = hist.find((h: any) => h.op === "submit" && h.station === "analyze");
  expect(submit.by).toEqual({ kind: "agent", id: task.session });

  await api.call("flow.confirm", { run, station: "approve", pass: true });
  const done = await until(run, (v) => v.record.status === "finished", "the run finishes");
  expect(station(done, "approve").confirms[0].by.kind).toBe("device");
  const { runs } = { runs: (await api.call("harnix.topics.subscribe", { topic: "flow.runs" })).value };
  expect(runs.find((r: any) => r.id === run)?.status).toBe("finished");
});

test("a review: foreach sessions, a parallel gate joined later, a skipped station, a confirmation, an external task", TAGS, async () => {
  const run = await start("demo-review");
  const review = await until(run, status("review", "working"), "review waits for a confirmation");
  // foreach: one task per collected module, each handed in by its own session.
  const analyze = station(review, "analyze");
  expect(analyze.tasks.map((t: any) => t.id)).toEqual(["core", "ui"]);
  expect(analyze.tasks.every((t: any) => t.status === "submitted")).toBe(true);
  expect(new Set(analyze.tasks.map((t: any) => t.session)).size).toBe(2);
  // The gate-only checkpoint ran beside analyze and was joined at review; self-check was skipped.
  expect(station(review, "lint").status).toBe("passed");
  expect(station(review, "lint").checks[0]).toMatchObject({ kind: "gate", status: "pass" });
  expect(station(review, "self-check").status).toBe("skipped");
  expect(review.unmet.review.map((c: any) => c.kind)).toEqual(["confirm"]);

  await api.call("flow.confirm", { run, station: "review", pass: true, reason: "looks fine" });
  const report = await until(
    run,
    (v) => station(v, "report")?.tasks?.find((t: any) => t.id === "summary")?.status === "submitted",
    "the summary is handed in",
  );
  const sign = station(report, "report").tasks.find((t: any) => t.id === "sign");
  expect(sign.status).toBe("ready");

  // The external task: claimed with a lease, handed in under its token; a wrong token is refused.
  const { token, until: leased } = await api.call("flow.claim", { run, station: "report", task: "sign" });
  expect(leased).toBeGreaterThan(Date.now());
  const wrong = await api.refused("flow.submit", { run, station: "report", task: "sign", token: "nope", output: {} });
  expect(wrong?.data?.flow).toBe("lease");
  await api.call("flow.heartbeat", { run, station: "report", task: "sign", token });
  await api.call("flow.submit", { run, station: "report", task: "sign", token, output: { signed: true } });
  const done = await until(run, (v) => v.record.status === "finished", "the run finishes");
  expect(station(done, "report").status).toBe("passed");
});

test("force and rework: a station passed without its confirmation, and work sent back upstream", TAGS, async () => {
  const run = await start("demo-review");
  await until(run, status("review", "working"), "review waits for a confirmation");

  // Rework to analyze: it runs again (new sessions), review and what follows go stale; lint stays.
  await api.call("flow.rework", { run, to: "analyze", reason: "look again" });
  const again = await until(
    run,
    (v) => station(v, "analyze").round === 2 && station(v, "review").status === "working",
    "analyze's second round reaches review again",
  );
  expect(station(again, "lint").round).toBe(1);
  expect(station(again, "analyze").tasks.every((t: any) => t.status === "submitted")).toBe(true);

  const missing = await api.refused("flow.force", { run, station: "review", reason: "" });
  expect(missing?.data?.flow).toBe("invalid");
  await api.call("flow.force", { run, station: "review", reason: "the reviewer is away" });
  const forced = await until(run, status("review", "passed"), "review passes by force");
  expect(station(forced, "review").forced).toMatchObject({ by: { kind: "device" }, reason: "the reviewer is away" });
  expect(station(forced, "review").forced.unmet.map((c: any) => c.kind)).toEqual(["confirm"]);

  await api.call("flow.cancel", { run, reason: "done looking" });
  await until(run, (v) => v.record.status === "cancelled", "the run is cancelled");
});

test("tasks sharing the run's session take turns in it, each handing in under its own claim", TAGS, async () => {
  const run = await start("demo-shared");
  const done = await until(run, (v) => v.record.status === "finished", "both stations hand in");
  const [first, second] = [station(done, "first").tasks[0], station(done, "second").tasks[0]];
  expect(first.output).toEqual({ step: 1 });
  expect(second.output).toEqual({ step: 2 });
  expect(first.session).toBe(second.session);
  // The run's key, and the main session every run has (xops-03), apart from it.
  // Each bound with what it was opened with: the shared one with its station's scene.
  expect(done.record.session_keys).toMatchObject({ run: { id: first.session, opened_with: { scene: expect.any(String) } } });
  expect(done.record.session_keys["@main"].id).toBeTruthy();
  expect(done.record.session_keys["@main"].id).not.toBe(first.session);
  expect(done.record.session_keys["@main"].opened_with).toMatchObject({ main: true });
  // Every submission landed where it belonged: none was refused for the other's contract.
  const hist = await api.call("flow.history", { run });
  expect(hist.filter((h: any) => h.op === "submit").map((h: any) => h.station)).toEqual(["first", "second"]);
  expect(hist.some((h: any) => h.op === "fail")).toBe(false);
});

test("a task: a run in a project carries its root, workspace, output directory and trigger, and runs list by project", TAGS, async () => {
  const project = realpathSync(mkdtempSync(join(tmpdir(), "harnix-task-")));
  try {
    // Every flow on offer says what a run of it is started with, as a JSON Schema a form is drawn from.
    const defs = await api.call("flow.defs");
    for (const d of defs) {
      expect(d.inputs, d.name).toMatchObject({ type: "object", properties: expect.any(Object), required: expect.any(Array) });
    }

    const task = await start("demo-review", { project_root: `${project}/`, output_dir: "flows/${flow}-${date}${seq}" });
    const plain = await start("demo-review");
    const rec = (await api.call("flow.get", { run: task })).record;
    // The root as given, but for its trailing `/`; the project is where it works.
    expect(rec.project_root).toBe(project);
    expect(rec.workspace).toBe("project");
    // The template resolved and made at start, inside the project: the first of the day is `01`.
    expect(rec.output_dir).toMatch(new RegExp(`^${project}/flows/demo-review-\\d{8}01$`));
    expect(existsSync(rec.output_dir)).toBe(true);
    expect(readFileSync(join(project, ".harnix", ".gitignore"), "utf8")).toBe("runs/\nworktrees/\n");
    expect(rec.trigger).toMatchObject({ kind: "manual", by: { kind: "device" } });
    const other = (await api.call("flow.get", { run: plain })).record;
    expect(other.project_root).toBeUndefined();
    expect(other.workspace).toBe("scratch");
    expect(other.trigger.kind).toBe("manual");

    // `flow.list {project}`: that project's runs alone; the topic's rows carry the same fields.
    const listed = await api.call("flow.list", { project });
    expect(listed.map((r: any) => r.id)).toEqual([task]);
    expect(listed[0]).toMatchObject({ project_root: project, workspace: "project", trigger: { kind: "manual" } });
    expect(listed[0].output_dir).toBe(rec.output_dir);
    const rows = (await api.call("harnix.topics.subscribe", { topic: "flow.runs" })).value;
    expect(rows.find((r: any) => r.id === task)).toMatchObject({ project_root: project, workspace: "project" });
    expect(rows.find((r: any) => r.id === plain)).toMatchObject({ workspace: "scratch" });

    // A root that is not an existing absolute directory, a workspace in a project without one, a
    // device saying a schedule started it: refused, nothing started.
    const file = join(project, "file");
    writeFileSync(file, "");
    for (const project_root of ["relative/dir", join(project, "missing"), file]) {
      expect((await api.refused("flow.start", { flow: "demo-review", subject: "repo", project_root }))?.code, project_root).toBe(INVALID);
    }
    expect((await api.refused("flow.start", { flow: "demo-review", subject: "repo", workspace: "worktree" }))?.code).toBe(INVALID);
    const scheduled = await api.refused("flow.start", {
      flow: "demo-review", subject: "repo", project_root: project, trigger: { kind: "schedule", schedule: "s1", fire_at: 1 },
    });
    expect(scheduled?.code).toBe(REFUSED);
    expect((await api.call("flow.list", { project })).length).toBe(1);

    await api.call("flow.cancel", { run: task, reason: "done" });
    await api.call("flow.cancel", { run: plain, reason: "done" });
  } finally {
    // Retried: the engine may still be writing under the project (its part of `.harnix/`) as it goes.
    rmSync(project, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  }
});

test("only a device may drive a flow: another module is refused", TAGS, async () => {
  const start = await api.refused("probe.call", {
    method: "flow.start",
    params: { flow: "demo-linear", subject: "repo" },
  });
  expect(start?.code).toBe(REFUSED);
  const read = await api.refused("probe.call", { method: "flow.list", params: {} });
  expect(read?.code).toBe(REFUSED);
});

test("a run carries on after a restart", TAGS, async () => {
  const run = await start("demo-review");
  await until(run, status("review", "working"), "review waits for a confirmation");

  api.close();
  backend = await backend.restart();
  api = await connect(backend);

  // The run is where it was, and published again for whoever subscribes.
  const back = await until(run, status("review", "working"), "the run is back");
  expect(station(back, "analyze").status).toBe("passed");
  await api.call("flow.confirm", { run, station: "review", pass: true });
  await until(
    run,
    (v) => station(v, "report")?.tasks?.find((t: any) => t.id === "summary")?.status === "submitted",
    "the summary is handed in after the restart",
  );
  const { token } = await api.call("flow.claim", { run, station: "report", task: "sign" });
  await api.call("flow.submit", { run, station: "report", task: "sign", token, output: {} });
  await until(run, (v) => v.record.status === "finished", "the run finishes");
});
