/**
 * How a turn ended, on its settlement (ipdrun-17).
 *
 * `harnix.turn_settled` carries `outcome` — the stop reason, the usage, a structured output or a
 * hook's message when there is one — taken from the engine's own `turn_complete`. A caller that
 * missed that frame still knows: a module's tap that fell behind gets its settlements from
 * `next_settled()` without catching up.
 */

import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(async () => {
  await backend?.dispose();
});
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

test("a settled turn says how it ended, as its turn_complete did", TAGS, async () => {
  const created = await api.call("session.create", {
    project_root: api.project,
    options: { recorder: { name: "says-ready" } },
  });
  const sessionId = created.session_id;
  await api.call("harnix.attach", { session_id: sessionId });
  const sent = await api.call("harnix.send", { session_id: sessionId, message: "Say exactly: fixture ready" });
  const settled = await api.waitFor(
    (f) => f.method === "harnix.turn_settled" && f.params.turn_id === sent.turn_id,
    { timeout: 60_000, describe: "the turn to settle" },
  );
  const complete = api.eventsOfKind("turn_complete", sessionId).at(-1);
  expect(complete, "no turn_complete frame").toBeTruthy();
  expect(settled.params.outcome.stop_reason).toBe("end_turn");
  expect(settled.params.outcome.stop_reason).toBe(complete.stop_reason);
  expect(settled.params.outcome.usage).toEqual(complete.usage);
  // `result` is what it always was.
  expect(settled.params.result.turn_id).toBeTruthy();
});

test("a tap that fell behind still has the settlement, with its outcome", TAGS, async () => {
  const answer = await api.call("app.fleet.settle_behind", {
    project_root: api.project,
    recording: "says-ready",
    message: "Say exactly: fixture ready",
  });
  expect(answer.lagged, "the small tap did not fall behind").toBe(true);
  expect(answer.narrow.turn_id).toBe(answer.wide.turn_id);
  expect(answer.narrow.outcome.stop_reason).toBe("end_turn");
  expect(answer.narrow.outcome).toEqual(answer.wide.outcome);
});
