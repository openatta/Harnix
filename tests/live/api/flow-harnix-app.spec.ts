/**
 * The Harnix app, `harnix`, has the flow module (xops-02 §6): an example flow built in,
 * a writable layer in the module's data directory, the flow space whose conversations run
 * `flow-author`, and no programs, gates or tools of an application's.
 *
 * What a client can tell at start: `harnix.reachable` lists `flow.*`, and
 * `harnix.spaces.list` has the flow space with `flow-author` as its scene.
 */

import { INVALID_PARAMS, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:flow", "@layer:api"] };

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => { await backend?.dispose(); });
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

test("the Harnix app says it has flows: its methods, its space, its example", TAGS, async () => {
  const { methods } = await api.call("harnix.reachable");
  for (const m of ["flow.defs", "flow.save", "flow.focus", "flow.dry_run.start"]) {
    expect(methods).toContain(m);
  }
  const { spaces } = await api.call("harnix.spaces.list");
  expect(spaces.find((s: any) => s.id === "flow")).toMatchObject({ instance: "main", scene: "flow-author" });

  expect(await api.call("flow.defs")).toEqual([
    expect.objectContaining({ name: "example-review", origin: "builtin", valid: true }),
  ]);
  const catalog = await api.call("flow.catalog");
  expect(catalog).toMatchObject({ scenes: ["flow-code", "flow-harnix", "flow-mini"], steps: [], checks: [], tools: [], command_gates: true, writable: true });
  // The example's parts: its skill, the rule it keeps, the policy it works under.
  expect(catalog.parts).toMatchObject({ skill: ["review"], rule: ["review-honest"], policy: ["read-only"], gate: [], hook: [] });
  // A flow that needs an application's program does not validate here.
  const problems = await api.call("flow.validate", {
    yaml: "name: x\nstations: [{id: a, executor: {kind: program, step: build}, output: {type: object}, exit: [{kind: output}]}]",
  });
  expect(JSON.stringify(problems)).toContain("step build is not in the catalog");
});

test("its example dry-runs to the approval and finishes once approved; a saved copy overrides it", TAGS, async () => {
  const { run } = await api.call("flow.dry_run.start", { flow: "example-review" });
  expect((await api.call("flow.dry_run.run", { run })).status).toBe("active");
  const view = await api.call("flow.get", { run });
  // The review is a turn of the run's main session (xops-03), on flow-harnix.
  expect(view.record.stations.review.tasks[0].preview).toMatchObject({
    scene: "flow-harnix",
    delivery: "flow_submit",
    session_key: "@main",
  });
  expect(view.record.stations.approve.status).toBe("working");
  await api.call("flow.confirm", { run, station: "approve", pass: true });
  expect((await api.call("flow.get", { run })).record.status).toBe("finished");

  const { yaml } = await api.call("flow.def", { name: "example-review" });
  const saved = await api.call("flow.save", { yaml: yaml.replace("Example: review and approve", "Mine now") });
  expect(saved).toMatchObject({ name: "example-review", origin: "override" });
  expect((await api.call("flow.defs"))[0]).toMatchObject({ title: "Mine now", origin: "override" });
  expect(await api.call("flow.delete", { name: "example-review" })).toMatchObject({ origin: "builtin" });
});

test("a task started with shown_in has its main session shown in that space, to be talked to, and flow.runOf finds the run from the session", TAGS, async () => {
  const { path } = await api.call("harnix-app.projects.create", { name: `shown-${Date.now()}` });
  const { run } = await api.call("flow.start", { flow: "example-review", subject: "the repo", project_root: path, shown_in: "sessions" });
  expect((await api.call("flow.get", { run })).record.shown_in).toBe("sessions");
  expect((await api.call("flow.list", { project: path })).find((s: any) => s.id === run)).toMatchObject({ shown_in: "sessions" });
  // The main session is opened a moment after the start.
  let main: string | undefined;
  for (let i = 0; i < 60 && !main; i++) {
    main = (await api.call("flow.get", { run })).record.session_keys?.["@main"]?.id;
    if (!main) await new Promise((r) => setTimeout(r, 500));
  }
  expect(main, "the main session is opened").toBeTruthy();
  const [instance, session_id] = main!.split("/");
  const mine = { instance, session_id };
  // Listed in the session space as lent from the flow's, and reached from there, to talk to.
  const row = (await api.call("harnix.sessions.list", { space: "sessions" })).sessions.find((r: any) => r.session_id === session_id);
  expect(row).toMatchObject({ space: "flow", exposed_from: "flow", exposed_to: [{ space: "sessions", interact: true }] });
  await api.call("engine.session.get", { ...mine, space: "sessions" });
  await api.call("harnix.session.attach", { ...mine, space: "sessions" });
  // Its life stays the flow's.
  const refused = await api.refused("harnix.session.close", { ...mine, space: "sessions" });
  expect(refused?.code).toBe(SPACE_MISMATCH);
  // The run from its session, and nothing for a session that is no run's.
  expect(await api.call("flow.runOf", mine)).toEqual({ run });
  expect(await api.call("flow.runOf", { instance, session_id: "no-such-session" })).toEqual({ run: null });
  // A start that names no space shows the session nowhere else.
  const { run: plain } = await api.call("flow.start", { flow: "example-review", subject: "again", project_root: path });
  expect((await api.call("flow.get", { run: plain })).record.shown_in).toBeUndefined();
  expect((await api.refused("flow.start", { flow: "example-review", subject: "x", project_root: path, shown_in: "not a space!" }))?.code).toBe(INVALID_PARAMS);
  await api.call("flow.cancel", { run, reason: "done" }).catch(() => undefined);
  await api.call("flow.cancel", { run: plain, reason: "done" }).catch(() => undefined);
});
