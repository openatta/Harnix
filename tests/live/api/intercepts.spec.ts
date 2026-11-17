/**
 * What the hub does *after* an engine method answers: bookkeeping the authorization table cannot
 * show, since it is not a row but what a row's handler goes on to do. Three of them, each pinned
 * from the outside so that moving the handler somewhere else keeps them:
 *
 * - who answered a permission prompt is kept for the application's audit;
 * - a goal heard from the engine lands on the session's row;
 * - a method after which the session list reads differently pushes the list again.
 *
 * `probe.call` (crates/testapp) calls a method as the `probe` module.
 */

import { REFUSED } from "@harnix/core/protocol";
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

const row = async (s: { instance: string; session_id: string }) =>
  (await api.call("harnix.sessions.list", { instance: s.instance })).sessions.find((r: any) => r.session_id === s.session_id);

test("the device that answered a prompt is kept once for a module, and dropped from what the engine is sent", TAGS, async () => {
  const created = await api.call("harnix.session.create", {
    project_root: api.project,
    options: { recorder: { name: "runs-a-while" } },
  });
  const s = { instance: created.instance, session_id: created.session_id };
  await api.call("harnix.session.attach", s);
  await api.call("harnix.session.send", { ...s, message: "Wait four seconds" });
  const ask = await api.waitFor(
    (f) => f.method === "harnix.event" && f.params.session_id === s.session_id && f.params.event.kind === "prompt",
    { timeout: 30_000, describe: "the permission ask" },
  );
  const toolUse = ask.params.event.tool_use_id;
  expect(toolUse).toBeTruthy();
  // The engine takes the answer, with the extra field stripped, and the ask stops waiting.
  await api.call("harnix.session.respond", {
    ...s,
    prompt_id: ask.params.event.prompt_id,
    decision: { type: "permit" },
    tool_use_id: toolUse,
  });
  await expect.poll(async () => (await row(s))?.waiting ?? null, { timeout: 10_000 }).toBeNull();

  const read = () =>
    api.call("probe.call", {
      method: "harnix.session.answerer",
      params: { session_id: s.session_id, tool_use_id: toolUse },
    });
  expect((await read()).answerer).toMatch(/^device:./);
  expect((await read()).answerer, "forgotten once read").toBeNull();
  expect((await api.refused("harnix.session.answerer", { session_id: s.session_id, tool_use_id: toolUse }))?.code).toBe(REFUSED);
});

test("a goal the engine reports is on the session's row", TAGS, async () => {
  const created = await api.call("harnix.session.create", { project_root: api.project, options: { recorder: { name: "sets-a-goal" } } });
  const s = { instance: created.instance, session_id: created.session_id };
  await api.call("harnix.session.attach", s);
  expect((await row(s)).goal).toBeUndefined();

  // The model sets it with its goal tool; the engine says so, and the hub keeps it on the row.
  await api.call("harnix.session.send", { ...s, message: "Set the goal: keep the fixture tests green" });
  await expect.poll(async () => (await row(s))?.goal?.objective, { timeout: 30_000 }).toBe("Keep the fixture tests green");
});

test("a rename the engine makes is pushed to whoever watches the session list", TAGS, async () => {
  const created = await api.call("harnix.session.create", { project_root: api.project, options: { recorder: { name: "says-ready" } } });
  const s = { instance: created.instance, session_id: created.session_id };
  await api.call("harnix.session.attach", s);
  const first = await api.call("harnix.topics.subscribe", { topic: "harnix.instances.sessions", params: { instance: s.instance } });
  expect(first.value.sessions.find((r: any) => r.session_id === s.session_id)?.name).not.toBe("renamed by the engine");

  // The engine sends no event for a rename: the push is the hub's, after the answer.
  await api.call("harnix.session.rename", { ...s, title: "renamed by the engine" });
  await api.waitFor(
    (f) =>
      f.method === "harnix.topic" &&
      f.params.topic === "harnix.instances.sessions" &&
      f.params.value.sessions.some((r: any) => r.session_id === s.session_id && r.name === "renamed by the engine"),
    { describe: "the session list to be pushed with the new name" },
  );
  await api.call("harnix.topics.unsubscribe", { topic: "harnix.instances.sessions", params: { instance: s.instance } });
});
