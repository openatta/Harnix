// Who answered a permission prompt (helm-01): the shell sends the prompt's
// tool_use_id with its answer, the hub keeps the answering device and strips
// the field before the engine, and a module reads it once for its own audit.
//
// `app.probe.call` (crates/testapp) calls a method as the `probe` module.

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

test("a module learns once which device answered a tool's prompt; a device cannot ask (helm-01)", TAGS, async () => {
  const created = await api.call("session.create", {
    project_root: api.project,
    options: { recorder: { name: "runs-a-while" } },
  });
  const s = { instance: "system", session_id: created.session_id };
  await api.call("harnix.attach", s);
  await api.call("harnix.send", { ...s, message: "Wait four seconds" });
  const ask = await api.waitFor(
    (f) => f.method === "harnix.event" && f.params.session_id === s.session_id && f.params.event.kind === "prompt",
    { timeout: 30_000, describe: "the permission ask for the Bash call" },
  );
  const toolUse = ask.params.event.tool_use_id;
  expect(toolUse, "the ask names its tool call").toBeTruthy();
  // The engine takes the answer with the extra field stripped.
  await api.call("session.respondToPrompt", {
    ...s,
    prompt_id: ask.params.event.prompt_id,
    decision: { type: "permit" },
    tool_use_id: toolUse,
  });

  const read = () =>
    api.call("app.probe.call", {
      method: "harnix.prompts.answerer",
      params: { session_id: s.session_id, tool_use_id: toolUse },
    });
  expect((await read()).answerer).toMatch(/^device:./);
  expect((await read()).answerer, "forgotten once read").toBeNull();

  const refused = await api.refused("harnix.prompts.answerer", { session_id: s.session_id, tool_use_id: toolUse });
  expect(refused?.code).toBe(REFUSED);
});
