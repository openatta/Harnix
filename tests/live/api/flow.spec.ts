/**
 * The flow module (xops-01) in `harnix-testapp`: runs of the two demonstration flows, driven
 * through `app.flow.*` and watched on `app.flow.run {run}`.
 *
 * Session stations replay `flow-analyze` and `flow-report`: each session calls `flow_submit`
 * with a fixed output and ends its turn, so what is under test is the module — dispatch, the
 * native tools, joins, skips, confirmations, leases, rework and force, and carrying on after
 * a restart — never the model.
 */

import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:flow", "@layer:api"] };
const REFUSED = -31000;

type View = { record: any; unmet: Record<string, any[]> };

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(() => backend?.stop());
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

async function start(flow: string, extra: Record<string, unknown> = {}): Promise<string> {
  const { run } = await api.call("app.flow.start", { flow, subject: "repo", ...extra });
  expect(run).toMatch(new RegExp(`^${flow}-`));
  return run;
}

/** The run's view once `ok` holds, from the `app.flow.run` topic. */
async function until(run: string, ok: (v: View) => boolean, describe: string): Promise<View> {
  const sub = await api.call("harnix.subscribe", { topic: "app.flow.run", params: { run } });
  if (sub.value && ok(sub.value)) return sub.value;
  const frame = await api.waitFor(
    (f) =>
      f.method === "harnix.topic" &&
      f.params.topic === "app.flow.run" &&
      f.params.params?.run === run &&
      ok(f.params.value),
    { timeout: 60_000, describe: `${run}: ${describe}` },
  );
  return frame.params.value;
}

const station = (v: View, id: string) => v.record.stations[id];
const status = (id: string, s: string) => (v: View) => station(v, id)?.status === s;

test("the module says what it offers: flows, a definition with its skills, the catalog, schemas", TAGS, async () => {
  const defs = await api.call("app.flow.defs");
  expect(defs.map((d: any) => d.name).sort()).toEqual([
    "demo-guarded", "demo-linear", "demo-misplaced", "demo-placed", "demo-propose", "demo-review", "demo-shared",
  ]);
  // Where each comes from and whether it is valid (xops-02 §3): all built in here.
  expect(defs.every((d: any) => d.origin === "builtin" && d.valid === true)).toBe(true);
  const info = await api.call("app.flow.def", { name: "demo-review" });
  expect(info).toMatchObject({ origin: "builtin", valid: true, problems: [], writable: true });
  expect(info.def.stations.map((s: any) => s.id)).toEqual(["collect", "analyze", "lint", "self-check", "review", "report"]);
  expect(info.yaml).toContain("name: demo-review");
  expect(Object.keys(info.skills).sort()).toEqual(["analyze", "report"]);
  expect(info.protocol.analyze).toContain("flow_submit");
  expect(info.digest).toMatch(/^[0-9a-f]{64}$/);

  const catalog = await api.call("app.flow.catalog");
  // The wrapped scene (flow_submit) and the application's own (structured output, xops-02 §2).
  expect(catalog.scenes).toEqual(["flow-chat", "structured-chat"]);
  expect(info.protocol.analyze).toContain("refuses these as writes: Bash, PowerShell, REPL, Write, Edit, NotebookEdit, TodoWrite");
  expect(catalog.steps).toEqual(["collect", "self-check"]);
  expect(catalog.skills).toEqual(["analyze", "report"]);
  const def = await api.call("app.flow.schema", { of: "definition" });
  expect(def.properties.stations).toBeTruthy();
  const run = await api.call("app.flow.schema", { of: "run" });
  expect(run.properties.stations).toBeTruthy();

  expect(await api.call("app.flow.validate", { yaml: info.yaml })).toEqual([]);
  const problems = await api.call("app.flow.validate", {
    yaml: "name: bad\nstations: [{id: a, executor: {kind: program, step: nowhere}, exit: [{kind: output}]}]",
  });
  expect(problems.length).toBeGreaterThan(0);
  expect(JSON.stringify(problems)).toContain("nowhere");
  const unwrapped = await api.call("app.flow.validate", {
    yaml: "name: bare\nstations: [{id: a, executor: {kind: session, scene: chat}, skill: analyze, output: {type: object}, exit: [{kind: output}]}]",
  });
  expect(JSON.stringify(unwrapped)).toContain("scene chat");
});

test("lease tokens stay with their claimants, and push tasks are the flow's to hand in", TAGS, async () => {
  const run = await start("demo-review");
  const review = await until(run, status("review", "working"), "review waits");
  const text = JSON.stringify(review);
  expect(text).not.toContain('"token"');
  expect(JSON.stringify(await api.call("app.flow.get", { run }))).not.toContain('"token"');
  await api.call("app.flow.confirm", { run, station: "review", pass: true });
  await until(run, (v) => station(v, "report")?.tasks?.some((t: any) => t.id === "sign"), "report entered");
  const { token } = await api.call("app.flow.claim", { run, station: "report", task: "sign" });
  const after = await api.call("app.flow.get", { run });
  expect(JSON.stringify(after)).not.toContain(token);
  // The summary is a session task: a device holding no token, or any token, cannot hand it in.
  const denied = await api.refused("app.flow.submit", { run, station: "report", task: "summary", token, output: {} });
  expect(denied?.data?.flow).toMatch(/denied|lease/);
  // A lease past the cap is cut to a day, not overflowed.
  const hb = await api.call("app.flow.heartbeat", { run, station: "report", task: "sign", token, lease_ms: Number.MAX_SAFE_INTEGER });
  expect(hb.until).toBeLessThanOrEqual(Date.now() + 24 * 3600 * 1000 + 60_000);
  await api.call("app.flow.cancel", { run, reason: "done" });
});

test("a session's write its station does not allow is refused, its proposal is accepted once", TAGS, async () => {
  const run = await start("demo-propose");
  const waiting = await until(run, status("approve", "working"), "approve waits");
  const task = station(waiting, "propose").tasks[0];
  expect(task.status).toBe("submitted");
  const [instance, session_id] = task.session.split("/");
  const history = JSON.stringify((await api.call("session.history", { instance, session_id })).messages);
  expect(history).toContain("allows no writes");
  expect(history).toContain("TodoWrite");

  const { entries } = await api.call("session.entries", { instance, session_id });
  const proposal = entries.find((e: any) => e.ns === "app.flow" && e.event === "proposal");
  expect(proposal.payload).toMatchObject({ run, station: "propose", kind: "rework", to: "propose", session: { instance, session_id } });
  const ref = { instance, session_id, entry: proposal.id };
  expect((await api.call("app.flow.proposal", ref)).handled).toBeNull();

  await api.call("app.flow.rework", { run, to: "propose", reason: proposal.payload.reason, proposal: ref });
  const again = await api.call("app.flow.rework", { run, to: "propose", reason: proposal.payload.reason, proposal: ref });
  expect(again.already).toMatchObject({ entry: proposal.id, action: "rework", by: { kind: "device" } });
  expect((await api.call("app.flow.proposal", ref)).handled).toMatchObject({ action: "rework" });
  const hist = await api.call("app.flow.history", { run });
  expect(hist.filter((h: any) => h.op === "rework")).toHaveLength(1);
  // The second round runs in a new session and hands in again.
  await until(run, (v) => station(v, "propose").round === 2 && station(v, "approve").status === "working", "the second round");
  await api.call("app.flow.cancel", { run, reason: "done" });
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
  expect(task.session).toMatch(/^system\//);
  const session_id = task.session.split("/")[1];
  const { sessions } = await api.call("harnix.sessions");
  expect(sessions.find((s: any) => s.session_id === session_id)?.space).toBe("flow");
  const history = await api.call("session.history", { session_id });
  const text = JSON.stringify(history.messages);
  expect(text).toContain("flow_submit");
  expect(text).toContain("Flow protocol");

  const hist = await api.call("app.flow.history", { run });
  const submit = hist.find((h: any) => h.op === "submit" && h.station === "analyze");
  expect(submit.by).toEqual({ kind: "agent", id: task.session });

  await api.call("app.flow.confirm", { run, station: "approve", pass: true });
  const done = await until(run, (v) => v.record.status === "finished", "the run finishes");
  expect(station(done, "approve").confirms[0].by.kind).toBe("device");
  const { runs } = { runs: (await api.call("harnix.subscribe", { topic: "app.flow.runs" })).value };
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

  await api.call("app.flow.confirm", { run, station: "review", pass: true, reason: "looks fine" });
  const report = await until(
    run,
    (v) => station(v, "report")?.tasks?.find((t: any) => t.id === "summary")?.status === "submitted",
    "the summary is handed in",
  );
  const sign = station(report, "report").tasks.find((t: any) => t.id === "sign");
  expect(sign.status).toBe("ready");

  // The external task: claimed with a lease, handed in under its token; a wrong token is refused.
  const { token, until: leased } = await api.call("app.flow.claim", { run, station: "report", task: "sign" });
  expect(leased).toBeGreaterThan(Date.now());
  const wrong = await api.refused("app.flow.submit", { run, station: "report", task: "sign", token: "nope", output: {} });
  expect(wrong?.data?.flow).toBe("lease");
  await api.call("app.flow.heartbeat", { run, station: "report", task: "sign", token });
  await api.call("app.flow.submit", { run, station: "report", task: "sign", token, output: { signed: true } });
  const done = await until(run, (v) => v.record.status === "finished", "the run finishes");
  expect(station(done, "report").status).toBe("passed");
});

test("force and rework: a station passed without its confirmation, and work sent back upstream", TAGS, async () => {
  const run = await start("demo-review");
  await until(run, status("review", "working"), "review waits for a confirmation");

  // Rework to analyze: it runs again (new sessions), review and what follows go stale; lint stays.
  await api.call("app.flow.rework", { run, to: "analyze", reason: "look again" });
  const again = await until(
    run,
    (v) => station(v, "analyze").round === 2 && station(v, "review").status === "working",
    "analyze's second round reaches review again",
  );
  expect(station(again, "lint").round).toBe(1);
  expect(station(again, "analyze").tasks.every((t: any) => t.status === "submitted")).toBe(true);

  const missing = await api.refused("app.flow.force", { run, station: "review", reason: "" });
  expect(missing?.data?.flow).toBe("invalid");
  await api.call("app.flow.force", { run, station: "review", reason: "the reviewer is away" });
  const forced = await until(run, status("review", "passed"), "review passes by force");
  expect(station(forced, "review").forced).toMatchObject({ by: { kind: "device" }, reason: "the reviewer is away" });
  expect(station(forced, "review").forced.unmet.map((c: any) => c.kind)).toEqual(["confirm"]);

  await api.call("app.flow.cancel", { run, reason: "done looking" });
  await until(run, (v) => v.record.status === "cancelled", "the run is cancelled");
});

test("tasks sharing the run's session take turns in it, each handing in under its own claim", TAGS, async () => {
  const run = await start("demo-shared");
  const done = await until(run, (v) => v.record.status === "finished", "both stations hand in");
  const [first, second] = [station(done, "first").tasks[0], station(done, "second").tasks[0]];
  expect(first.output).toEqual({ step: 1 });
  expect(second.output).toEqual({ step: 2 });
  expect(first.session).toBe(second.session);
  expect(done.record.session_keys).toEqual({ run: first.session });
  // Every submission landed where it belonged: none was refused for the other's contract.
  const hist = await api.call("app.flow.history", { run });
  expect(hist.filter((h: any) => h.op === "submit").map((h: any) => h.station)).toEqual(["first", "second"]);
  expect(hist.some((h: any) => h.op === "fail")).toBe(false);
});

test("only a device may drive a flow: another module is refused", TAGS, async () => {
  const start = await api.refused("app.probe.call", {
    method: "app.flow.start",
    params: { flow: "demo-linear", subject: "repo" },
  });
  expect(start?.code).toBe(REFUSED);
  const read = await api.refused("app.probe.call", { method: "app.flow.list", params: {} });
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
  await api.call("app.flow.confirm", { run, station: "review", pass: true });
  await until(
    run,
    (v) => station(v, "report")?.tasks?.find((t: any) => t.id === "summary")?.status === "submitted",
    "the summary is handed in after the restart",
  );
  const { token } = await api.call("app.flow.claim", { run, station: "report", task: "sign" });
  await api.call("app.flow.submit", { run, station: "report", task: "sign", token, output: {} });
  await until(run, (v) => v.record.status === "finished", "the run finishes");
});
