/**
 * The empty application, `harnix`, has the flow module (xops-02 §6): an example flow built in,
 * a writable layer in the module's data directory, the flow space whose conversations run
 * `flow-author`, and no programs, gates or tools of an application's.
 *
 * What a client can tell at start: `harnix.reachable` lists `app.flow.*`, and
 * `harnix.spaces.list` has the flow space with `flow-author` as its scene.
 */

import { type Backend, type TestClient, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:flow", "@layer:api"] };

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(() => backend?.stop());
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

test("the empty application says it has flows: its methods, its space, its example", TAGS, async () => {
  const { methods } = await api.call("harnix.reachable");
  for (const m of ["app.flow.defs", "app.flow.save", "app.flow.focus", "app.flow.dry_run.start"]) {
    expect(methods).toContain(m);
  }
  const { spaces } = await api.call("harnix.spaces.list");
  expect(spaces.find((s: any) => s.id === "flow")).toMatchObject({ instance: "system", scene: "flow-author" });

  expect(await api.call("app.flow.defs")).toEqual([
    expect.objectContaining({ name: "example-review", origin: "builtin", valid: true }),
  ]);
  const catalog = await api.call("app.flow.catalog");
  expect(catalog).toMatchObject({ scenes: ["flow-chat", "flow-coding"], steps: [], checks: [], tools: [], command_gates: true, writable: true });
  // The example's parts: its skill, the rule it keeps, the policy it works under.
  expect(catalog.parts).toMatchObject({ skill: ["review"], rule: ["review-honest"], policy: ["read-only"], gate: [], hook: [] });
  // A flow that needs an application's program does not validate here.
  const problems = await api.call("app.flow.validate", {
    yaml: "name: x\nstations: [{id: a, executor: {kind: program, step: build}, output: {type: object}, exit: [{kind: output}]}]",
  });
  expect(JSON.stringify(problems)).toContain("step build is not in the catalog");
});

test("its example dry-runs to the approval and finishes once approved; a saved copy overrides it", TAGS, async () => {
  const { run } = await api.call("app.flow.dry_run.start", { flow: "example-review" });
  expect((await api.call("app.flow.dry_run.run", { run })).status).toBe("active");
  const view = await api.call("app.flow.get", { run });
  // The review is a turn of the run's main session (xops-03), on flow-coding.
  expect(view.record.stations.review.tasks[0].preview).toMatchObject({
    scene: "flow-coding",
    delivery: "flow_submit",
    session_key: "@main",
  });
  expect(view.record.stations.approve.status).toBe("working");
  await api.call("app.flow.confirm", { run, station: "approve", pass: true });
  expect((await api.call("app.flow.get", { run })).record.status).toBe("finished");

  const { yaml } = await api.call("app.flow.def", { name: "example-review" });
  const saved = await api.call("app.flow.save", { yaml: yaml.replace("Example: review and approve", "Mine now") });
  expect(saved).toMatchObject({ name: "example-review", origin: "override" });
  expect((await api.call("app.flow.defs"))[0]).toMatchObject({ title: "Mine now", origin: "override" });
  expect(await api.call("app.flow.delete", { name: "example-review" })).toMatchObject({ origin: "builtin" });
});
