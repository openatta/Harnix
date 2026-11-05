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
  expect(defs.map((d: any) => d.name).sort()).toEqual(["demo-linear", "demo-review"]);
  const info = await api.call("app.flow.def", { name: "demo-review" });
  expect(info.def.stations.map((s: any) => s.id)).toEqual(["collect", "analyze", "lint", "self-check", "review", "report"]);
  expect(info.yaml).toContain("name: demo-review");
  expect(Object.keys(info.skills).sort()).toEqual(["analyze", "report"]);
  expect(info.protocol.analyze).toContain("flow_submit");
  expect(info.digest).toMatch(/^[0-9a-f]{64}$/);

  const catalog = await api.call("app.flow.catalog");
  expect(catalog.scenes).toContain("flow-chat");
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
