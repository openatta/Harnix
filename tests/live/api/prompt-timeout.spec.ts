/**
 * `[engine] permission_prompt_timeout_secs`: an ask nobody answers is refused when it runs out,
 * and the turn goes on to its end rather than waiting for ever. Replayed (`asks-permission`) on a
 * backend of its own, with the timeout cut to two seconds.
 */

import { type Backend, type TestClient, connect, expect, finish, launchFor, startHarnix, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

let backend: Backend;
let api: TestClient;

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay", { profile: "\n[engine]\npermission_prompt_timeout_secs = 2\n" }));
  api = await connect(backend);
});
test.afterAll(async () => {
  api?.close();
  await backend?.dispose();
});

test("an unanswered ask is refused when its time runs out, and the turn settles", TAGS, async () => {
  const hello = await api.call("harnix.hello");
  expect(hello.permission_prompt_timeout_secs, "the client is told how long an ask lasts").toBe(2);
  const { session_id } = await api.call("session.create", {
    project_root: api.project,
    options: { recorder: { name: "asks-permission" }, permission_mode: "default" },
  });
  try {
    await api.call("harnix.attach", { session_id });
    const started = Date.now();
    const { turn_id } = await api.call("harnix.send", { session_id, message: "Use the Bash tool to run: echo permission-check" });
    const ask = await api.waitFor(
      (f) => f.method === "harnix.event" && f.params.session_id === session_id && f.params.event.kind === "prompt",
      { timeout: 30_000, describe: "the ask" },
    );
    // Nobody answers.
    const settled = await api.waitFor(
      (f) => f.method === "harnix.turn_settled" && f.params.session_id === session_id && f.params.turn_id === turn_id,
      { timeout: 30_000, describe: "the turn to settle once the ask ran out" },
    );
    expect(Date.now() - started, "it waited for the timeout").toBeGreaterThanOrEqual(1_500);
    const call = settled.params.outcome?.tool_calls?.find((c: any) => c.id === ask.params.event.tool_use_id);
    expect(call, `the asked-about call in ${JSON.stringify(settled.params.outcome)}`).toBeTruthy();
    expect(call.is_error, "the call was refused, not run").toBe(true);
    // Nothing is left to answer.
    const info = await api.call("harnix.attach", { session_id });
    expect(info.pending_prompts).toEqual([]);
  } finally {
    await finish(api, session_id);
  }
});
