/**
 * Tasks (concept §3): work that belongs to a session and runs beyond it —
 * here, a sub-agent. A sub-agent is its parent's task, listed by
 * `harnix.session.tasks` and pushed whole on the topic of that name; it is
 * not a session of its own in the instance's list.
 *
 * Replayed in every mode: the subject is where the sub-agent shows up, and a
 * live model decides for itself whether to spawn one.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { type Backend, expect, launchFor, startHarnix, connect, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:instances", "@layer:api"] };

let backend: Backend;
test.beforeAll(async () => {
  // Teams are a `code`-scene tool: activated beside the default.
  // Teams are the operator's switch, in the global tier: a project's own tier cannot turn them
  // on, so it is written before the engine reads its settings.
  const seed = (scratch: string) => {
    mkdirSync(join(scratch, "engine"), { recursive: true });
    writeFileSync(join(scratch, "engine", "settings.json"), JSON.stringify({ team: { enabled: true } }));
  };
  backend = await startHarnix(launchFor("replay", { profile: '\n[engine]\nscenes = ["code"]\n', seed }));
});
test.afterAll(async () => { await backend?.dispose(); });

test("a sub-agent is a task of its session, not a session in the list", TAGS, async () => {
  const api = await connect(backend);
  try {
    const created = await api.call("harnix.session.create", {
      scene: "harnix",
      project_root: api.project,
      options: { recorder: { name: "spawns-agent" } },
    });
    const s = { instance: "main", session_id: created.session_id };
    const first = await api.call("harnix.topics.subscribe", { topic: "harnix.session.tasks", params: s });
    expect(first.value.tasks).toEqual([]);

    await api.call("harnix.session.attach", s);
    await api.call("harnix.session.send", {
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

    const { tasks, teams } = await api.call("harnix.session.tasks", s);
    // No team ran here: the list is there, and empty.
    expect(teams).toEqual([]);
    const agents = tasks.filter((t: any) => t.kind === "agent");
    expect(agents, JSON.stringify(tasks)).toHaveLength(1);
    expect(agents[0].status).toBe("done");
    expect(agents[0].detail.description).toBe("list files");

    // What it produced, read through observability (Agix `engine.session.taskOutput`): the answer its
    // parent got back. A replay records only the parent's calls, so the sub-agent may have had no
    // answer to give — then there is nothing, and the parent was told it failed.
    const out = await api.call("observability.tasks.output", { ...s, task_id: agents[0].task_id });
    expect(out.status).toBe("done");
    const returned = api.eventsOfKind("tool_result", s.session_id).find((e) => e.id === agents[0].detail.tool_use_id);
    expect(returned, "the Agent call's result").toBeDefined();
    expect(out.output, JSON.stringify({ out, returned })).toBe(returned.is_error ? "" : textOf(returned.content));

    // The topic said so too, without being asked.
    await api.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === "harnix.session.tasks"
        && f.params.value.tasks.some((t: any) => t.kind === "agent"),
      { describe: "the task to be pushed on harnix.session.tasks" },
    );

    const { sessions } = await api.call("harnix.sessions.list", { instance: "main" });
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

test("a team's run is on its session's tasks: each team's stage and members, as the hub last heard them", TAGS, async () => {
  const api = await connect(backend);
  try {
    const created = await api.call("harnix.session.create", { scene: "code", project_root: api.project, options: { recorder: { name: "runs-a-team" } } });
    const s = { instance: "main", session_id: created.session_id };
    await api.call("harnix.topics.subscribe", { topic: "harnix.session.tasks", params: s });
    await api.call("harnix.session.attach", s);
    await api.call("harnix.session.send", { ...s, message: "Run a team of one: alpha says it is here" });
    const settled = await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id, {
      timeout: 60_000,
      describe: "the team's turn to settle",
    });
    expect(settled.params.error).toBeUndefined();
    expect(api.eventsOfKind("tool_use", s.session_id).some((e) => e.name === "TeamCreate")).toBe(true);
    // Pushed on the topic, and answered when asked: the one stage, finished, its one member.
    const pushed = await api.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === "harnix.session.tasks" && f.params.value.teams?.some((t: any) => t.status === "completed"),
      { timeout: 15_000, describe: "the team to be pushed finished" },
    );
    const [team] = pushed.params.value.teams;
    expect(team).toMatchObject({ team: "ui-team", stage_index: 0, stage_count: 1, status: "completed", members: ["alpha"], failed: [] });
    expect((await api.call("harnix.session.tasks", s)).teams).toEqual(pushed.params.value.teams);
  } finally {
    api.close();
  }
});
