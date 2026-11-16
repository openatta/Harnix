/**
 * The asks beside permissions, answered through `harnix.session.respond`: the model's question
 * (`AskUserQuestion`, `prompt_type: question`) and a plan's review (`ExitPlanMode`, a permission
 * answered as approve or "request changes"). A person's session lists questions among the asks it
 * answers, so the `code` scene gives it the tool; whoever answers first is the answer, and every
 * watcher is told what it was (`ask_answered`).
 *
 * Replayed (`asks-a-question`, `reviews-a-plan`, made as `recorded.ts` says) on a backend with the
 * `code` scene beside the default. An MCP server's form (`mcp_elicitation`) needs a server that asks;
 * its answers are covered by the hub's and the interface's unit tests.
 */

import { type Backend, type TestClient, connect, expect, finish, launchFor, startHarnix, test } from "@harnix/testkit";

import { PLAN_FEEDBACK, QUESTION } from "./recorded";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

let backend: Backend;
let api: TestClient;

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay", { profile: '\n[engine]\nscenes = ["code"]\n' }));
  api = await connect(backend);
});
test.afterAll(async () => {
  api?.close();
  await backend?.dispose();
});

async function session(recording: string, options: Record<string, unknown> = {}): Promise<string> {
  const { session_id } = await api.call("harnix.session.create", {
    scene: "code",
    project_root: api.project,
    options: { recorder: { name: recording }, ...options },
  });
  await api.call("harnix.session.attach", { session_id });
  return session_id;
}

const asked = (client: TestClient, session_id: string, match: (event: any) => boolean = () => true) =>
  client.waitFor((f) => f.method === "harnix.event" && f.params.session_id === session_id && f.params.event.kind === "prompt" && match(f.params.event), {
    timeout: 30_000,
    describe: "an ask",
  });

test("a model's question reaches every watcher, the first answer wins, and every watcher is told it", TAGS, async () => {
  const session_id = await session("asks-a-question");
  const second = await connect(backend);
  try {
    await second.call("harnix.session.attach", { session_id });
    const { turn_id } = await api.call("harnix.session.send", { session_id, message: "Ask me which color to use." });
    const ask = (await asked(api, session_id)).params.event;
    expect(ask.prompt_type).toBe("question");
    expect(ask.prompt_id, "a question's id is its call's").toBe(ask.tool_use_id);
    expect(ask.options).toEqual(QUESTION.options);
    await asked(second, session_id, (e) => e.prompt_id === ask.prompt_id);

    // The session waits on an answer, and its row says so.
    const { sessions } = await api.call("harnix.sessions.list", { instance: "main" });
    expect(sessions.find((r: any) => r.session_id === session_id)?.waiting).toBe("answer");

    // A permission's answer is not a question's.
    const wrong = await api.refused("harnix.session.respond", { session_id, prompt_id: ask.prompt_id, prompt_type: "question", decision: { type: "permit" } });
    expect(wrong?.code, "refused as misshapen").toBe(-32602);

    const answer = { session_id, prompt_id: ask.prompt_id, prompt_type: "question", answer: { type: "answer", text: QUESTION.answer } };
    await second.call("harnix.session.respond", answer);
    const late = await api.refused("harnix.session.respond", { ...answer, answer: { type: "answer", text: "r" } });
    expect(late, "a later answer is a silent success").toBeNull();

    for (const client of [api, second]) {
      const told = await client.waitFor(
        (f) => f.method === "harnix.event" && f.params.session_id === session_id && f.params.event.kind === "ask_answered",
        { describe: "what the question was answered with" },
      );
      expect(told.params.event).toMatchObject({ prompt_id: ask.prompt_id, prompt_type: "question", answer: { type: "answer", text: QUESTION.answer } });
    }
    const settled = await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.turn_id === turn_id, { timeout: 30_000, describe: "the turn" });
    expect(api.eventsOfKind("ask_answered", session_id), "only the first answer is told").toHaveLength(1);
    const call = settled.params.outcome?.tool_calls?.find((c: any) => c.id === ask.tool_use_id);
    expect(call?.is_error, "the question's call came back with the answer").toBe(false);
    const result = api.eventsOfKind("tool_result", session_id).find((e) => e.id === ask.tool_use_id);
    expect(JSON.stringify(result?.content)).toContain("blue");
  } finally {
    second.close();
    await finish(api, session_id);
  }
});

test("a declined question goes back to the model as not answered", TAGS, async () => {
  const session_id = await session("asks-a-question");
  try {
    const { turn_id } = await api.call("harnix.session.send", { session_id, message: "Ask me which color to use." });
    const ask = (await asked(api, session_id)).params.event;
    await api.call("harnix.session.respond", { session_id, prompt_id: ask.prompt_id, prompt_type: "question", answer: { type: "decline" } });
    const told = await api.waitFor((f) => f.params?.session_id === session_id && f.params?.event?.kind === "ask_answered", { describe: "the decline told" });
    expect(told.params.event.answer).toEqual({ type: "decline" });
    await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.turn_id === turn_id, { timeout: 30_000, describe: "the turn" });
    const result = api.eventsOfKind("tool_result", session_id).find((e) => e.id === ask.tool_use_id);
    expect(JSON.stringify(result?.content), "the model is told nobody answered").toContain("has not answered");
  } finally {
    await finish(api, session_id);
  }
});

test("a plan is reviewed: changes asked for stay in plan mode, the next plan is approved", TAGS, async () => {
  const session_id = await session("reviews-a-plan", { permission_mode: "plan" });
  try {
    const { turn_id } = await api.call("harnix.session.send", { session_id, message: "Plan a file." });
    const first = (await asked(api, session_id, (e) => e.tool_name === "ExitPlanMode")).params.event;
    expect(first.prompt_type ?? "permission").toBe("permission");
    const { sessions } = await api.call("harnix.sessions.list", { instance: "main" });
    expect(sessions.find((r: any) => r.session_id === session_id)?.waiting).toBe("plan");
    // The plan is the reply before the call.
    expect(api.text(session_id)).toMatch(/plan/i);

    await api.call("harnix.session.respond", { session_id, prompt_id: first.prompt_id, tool_use_id: first.tool_use_id, decision: { type: "deny", reason: PLAN_FEEDBACK } });
    const told = await api.waitFor((f) => f.params?.event?.kind === "ask_answered" && f.params.event.prompt_id === first.prompt_id, { describe: "the review told" });
    expect(told.params.event.decision).toEqual({ type: "deny", reason: PLAN_FEEDBACK });

    const second = (await asked(api, session_id, (e) => e.tool_name === "ExitPlanMode" && e.prompt_id !== first.prompt_id)).params.event;
    await api.call("harnix.session.respond", { session_id, prompt_id: second.prompt_id, tool_use_id: second.tool_use_id, decision: { type: "permit" } });
    const settled = await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.turn_id === turn_id, { timeout: 30_000, describe: "the turn" });
    expect(settled.params.error).toBeUndefined();
    expect(settled.params.outcome?.text ?? api.text(session_id)).toMatch(/plan approved/i);
    const resumed = await api.call("harnix.session.attach", { session_id });
    expect(resumed.pending_prompts).toEqual([]);
  } finally {
    await finish(api, session_id);
  }
});
