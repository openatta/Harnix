/**
 * Tasks (design §2): work that belongs to a session and runs beyond it —
 * here, a sub-agent. A sub-agent is its parent's task, listed by
 * `harnix.sessions.tasks` and pushed whole on the topic of that name; it is
 * not a session of its own in the instance's list.
 *
 * Replayed in every mode: the subject is where the sub-agent shows up, and a
 * live model decides for itself whether to spawn one.
 */

import { type Backend, expect, launchFor, startHarnix, connect, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:instances", "@layer:api"] };

let backend: Backend;
test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(() => backend?.stop());

test("a sub-agent is a task of its session, not a session in the list", TAGS, async () => {
  const api = await connect(backend);
  try {
    const created = await api.call("session.create", {
      scene: "coding",
      project_root: api.project,
      options: { recorder: { name: "spawns-agent" } },
    });
    const s = { instance: "system", session_id: created.session_id };
    const first = await api.call("harnix.subscribe", { topic: "harnix.sessions.tasks", params: s });
    expect(first.value.tasks).toEqual([]);

    await api.call("harnix.attach", s);
    await api.call("harnix.send", {
      ...s,
      message:
        "Use the Agent tool exactly once to launch a general-purpose sub-agent with the description 'list files' and the prompt 'Run ls in the current directory and reply with the file names only.' When it returns, reply with one line: 'sub-agent found:' followed by the names.",
    });
    const settled = await api.waitFor(
      (f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id,
      { timeout: 120_000, describe: "the replayed turn to settle" },
    );
    expect(settled.params.error).toBeUndefined();
    expect(api.eventsOfKind("tool_use", s.session_id).some((e) => e.name === "Agent")).toBe(true);

    const { tasks } = await api.call("harnix.sessions.tasks", s);
    const agents = tasks.filter((t: any) => t.kind === "agent");
    expect(agents, JSON.stringify(tasks)).toHaveLength(1);
    expect(agents[0].status).toBe("done");
    expect(agents[0].detail.description).toBe("list files");

    // What it produced, read as the system module (Agix `session.taskOutput`): the answer its
    // parent got back. A replay records only the parent's calls, so the sub-agent may have had no
    // answer to give — then there is nothing, and the parent was told it failed.
    const out = await api.call("harnix.system.tasks.output", { ...s, task_id: agents[0].task_id });
    expect(out.status).toBe("done");
    const returned = api.eventsOfKind("tool_result", s.session_id).find((e) => e.id === agents[0].detail.tool_use_id);
    expect(returned, "the Agent call's result").toBeDefined();
    expect(out.output, JSON.stringify({ out, returned })).toBe(returned.is_error ? "" : textOf(returned.content));

    // The topic said so too, without being asked.
    await api.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === "harnix.sessions.tasks"
        && f.params.value.tasks.some((t: any) => t.kind === "agent"),
      { describe: "the task to be pushed on harnix.sessions.tasks" },
    );

    const { sessions } = await api.call("harnix.instances.sessions", { instance: "system" });
    expect(sessions.map((r: any) => r.session_id)).toEqual([s.session_id]);
    for (const row of sessions) expect(row.session_kind).toBe("primary");
  } finally {
    api.close();
  }
});

function textOf(content: unknown): string {
  if (typeof content === "string") return content;
  return Array.isArray(content) ? content.map((b: { text?: string }) => b.text ?? "").join("") : "";
}
