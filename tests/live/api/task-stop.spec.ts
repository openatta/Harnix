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

test("a task's output is followed by offset: each read starts where the last ended, and what was read is not read again", TAGS, async () => {
  const api = await connect(backend);
  try {
    const created = await api.call("session.create", {
      scene: "code",
      project_root: api.project,
      options: { recorder: { name: "streams-output" }, permission_mode: "default" },
    });
    const s = { instance: "system", session_id: created.session_id };
    await api.call("harnix.attach", s);
    await api.call("harnix.send", { ...s, message: "Start watching the counter" });
    const ask = await api.waitFor((f) => f.method === "harnix.event" && f.params.session_id === s.session_id && f.params.event.kind === "prompt", {
      timeout: 30_000,
      describe: "the Monitor call to ask",
    });
    await api.call("session.respondToPrompt", { ...s, prompt_id: ask.params.event.prompt_id, decision: { type: "permit" } });
    let monitor: any;
    await expect
      .poll(async () => {
        monitor = (await api.call("harnix.sessions.tasks", s)).tasks.find((t: any) => t.kind === "monitor");
        return monitor?.status;
      }, { timeout: 15_000 })
      .toBe("running");
    const read = (extra: object = {}) => api.call("harnix.system.tasks.output", { ...s, task_id: monitor.task_id, ...extra });

    // From the start, then onward from wherever the last read ended, three times over.
    let text = "";
    let next = 0;
    for (let i = 0; i < 3; i += 1) {
      const slice = await read({ from_byte: next });
      expect(slice.from_byte, `read ${i} starts where the last ended`).toBe(next);
      expect(slice.dropped).toBe(0);
      expect(slice.next_byte).toBe(slice.from_byte + Buffer.byteLength(slice.output));
      text += slice.output;
      next = slice.next_byte;
      await new Promise((r) => setTimeout(r, 600));
    }
    const lines = text.split("\n").filter(Boolean);
    expect(lines.length).toBeGreaterThan(2);
    // Put together, the pieces are the counter's lines in order: none twice, none missing.
    expect(lines).toEqual(lines.map((_, i) => `line-${i + 1}`));
    // Past the end is refused, and says where the end is.
    expect((await api.refused("harnix.system.tasks.output", { ...s, task_id: monitor.task_id, from_byte: next + 1_000_000 }))?.code).toBe(-32602);

    // Stopped, it winds down; once its output is final, nothing more comes.
    await api.call("harnix.system.tasks.stop", { ...s, task_id: monitor.task_id });
    let last: any;
    await expect
      .poll(async () => {
        last = await read({ from_byte: next });
        next = last.next_byte;
        return last.final;
      }, { timeout: 30_000, intervals: [500] })
      .toBe(true);
    expect((await read({ from_byte: next })).output).toBe("");

    // A session with no goal says so.
    expect(await api.call("harnix.system.goal", s)).toEqual({ goal: null });
  } finally {
    api.close();
  }
});
