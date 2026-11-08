/**
 * A task that outlives its turn (design §2): a Monitor printing a line every quarter second for ten
 * seconds (`streams-output`), started by a turn that ends at once. The session's row counts it as
 * running while no turn is, `harnix.system.tasks.stop` stops it, and both topics say so without
 * being asked — to a client that is no longer watching the conversation, as the system space is not.
 *
 * Monitor is a tool of the `code` scene, so this backend activates it beside the default.
 */

import { type Backend, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:instances", "@layer:api"] };

let backend: Backend;
test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay", { profile: '\n[engine]\nscenes = ["code"]\n' }));
});
test.afterAll(async () => {
  await backend?.dispose();
});

test("a task running after its turn is counted on its session's row, and stopped", TAGS, async () => {
  const api = await connect(backend);
  try {
    const created = await api.call("session.create", {
      scene: "code",
      project_root: api.project,
      options: { recorder: { name: "streams-output" }, permission_mode: "default" },
    });
    const s = { instance: "system", session_id: created.session_id };
    await api.call("harnix.attach", s);
    await api.call("harnix.subscribe", { topic: "harnix.sessions.tasks", params: s });
    await api.call("harnix.subscribe", { topic: "harnix.instances.sessions", params: { instance: "system" } });
    await api.call("harnix.send", { ...s, message: "Start watching the counter" });
    const ask = await api.waitFor((f) => f.method === "harnix.event" && f.params.event.kind === "prompt", {
      timeout: 30_000,
      describe: "the Monitor call to ask",
    });
    await api.call("session.respondToPrompt", { ...s, prompt_id: ask.params.event.prompt_id, decision: { type: "permit" } });
    await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id, {
      timeout: 30_000,
      describe: "the turn to settle",
    });
    await api.call("harnix.detach", s);

    const row = (frame: any) => frame.params.value.sessions.find((r: any) => r.session_id === s.session_id);
    const counted = await api.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === "harnix.instances.sessions" && row(f)?.tasks === 1,
      { timeout: 15_000, describe: "the row to count the running task" },
    );
    expect(row(counted).running, "no turn is running").toBe(false);

    const { tasks } = await api.call("harnix.sessions.tasks", s);
    const monitor = tasks.find((t: any) => t.kind === "monitor");
    expect(monitor?.status, JSON.stringify(tasks)).toBe("running");
    expect(await api.call("harnix.system.tasks.stop", { ...s, task_id: monitor.task_id })).toEqual({ stopped: true });

    await api.waitFor(
      (f) =>
        f.method === "harnix.topic" &&
        f.params.topic === "harnix.sessions.tasks" &&
        f.params.value.tasks.some((t: any) => t.task_id === monitor.task_id && t.status === "stopped"),
      { timeout: 5_000, describe: "the stop to be pushed on harnix.sessions.tasks" },
    );
    await api.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === "harnix.instances.sessions" && row(f)?.tasks === 0,
      { timeout: 5_000, describe: "the row to count no running task" },
    );
  } finally {
    api.close();
  }
});
