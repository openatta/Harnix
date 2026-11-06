/**
 * The run's main session — the flow's main Agent (xops-03), in `harnix-testapp`.
 *
 * `demo-main`: an external task, two session stations without a scene (turns of the main
 * session, which replays `flow-main`: each turn tries `TodoWrite`, then hands in with
 * `flow_submit`), and a confirmation. `draft` does not allow the write, `edit` does — the flow
 * answers the main session's question each time from the station being worked on.
 *
 * `demo-agent`: a sub-agent station; the main session is on `flow-coding` in a directory of the
 * run's, and replays `flow-agent` (an `Agent` call named by the task's tag, then `flow_submit`).
 *
 * Also: pause and resume, each station's time and tokens, a run's sessions, a flow's own
 * conversation and the general one, the guide.
 */

import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:flow", "@layer:api"] };

type View = { record: any; unmet: Record<string, any[]> };

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(() => backend?.stop());
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

async function start(flow: string): Promise<string> {
  const { run } = await api.call("app.flow.start", { flow, subject: "repo" });
  expect(run).toMatch(new RegExp(`^${flow}-`));
  return run;
}

/** The run's view once `ok` holds, from the `app.flow.run` topic. */
async function until(run: string, ok: (v: View) => boolean, describe: string): Promise<View> {
  const sub = await api.call("harnix.subscribe", { topic: "app.flow.run", params: { run } });
  if (sub.value && ok(sub.value)) return sub.value;
  const frame = await api.waitFor(
    (f) =>
      f.method === "harnix.topic" &&
      f.params.topic === "app.flow.run" &&
      f.params.params?.run === run &&
      ok(f.params.value),
    { timeout: 60_000, describe: `${run}: ${describe}` },
  );
  return frame.params.value;
}

const station = (v: View, id: string) => v.record.stations[id];
const main = (v: View) => v.record.session_keys?.["@main"] as string | undefined;
const ref = (session: string) => {
  const [instance, session_id] = session.split("/");
  return { instance, session_id };
};
const results = (messages: any[]) =>
  messages
    .flatMap((m: any) => (Array.isArray(m.content) ? m.content : []))
    .filter((b: any) => b.type === "tool_result");

test("a run has its main session from the start; paused, it lets the work in hand finish and starts nothing", TAGS, async () => {
  const run = await start("demo-main");
  const opened = await until(run, (v) => !!main(v), "the main session is opened");
  expect(opened.record.stations.wait.status).toBe("working");
  const { summary } = { summary: (await api.call("app.flow.list", { flow: "demo-main" })).find((s: any) => s.id === run) };
  expect(summary.phase).toMatchObject({ state: "waiting", reason: "external" });

  // Claimed before the pause, it may still be handed in; someone outside has it, so the run is
  // paused at once (only the flow's own work in hand keeps it pausing).
  const { token } = await api.call("app.flow.claim", { run, station: "wait", task: "wait" });
  await api.call("app.flow.pause", { run });
  expect((await api.call("app.flow.list", { flow: "demo-main" })).find((s: any) => s.id === run).phase.state).toBe("paused");
  await api.call("app.flow.submit", { run, station: "wait", task: "wait", token, output: {} });
  const paused = await until(run, (v) => station(v, "wait").status === "passed", "wait passes while paused");
  expect(paused.record.paused).toBeTruthy();
  expect(station(paused, "draft").status).toBe("pending");
  expect(paused.record.stations.wait.tasks[0]).toMatchObject({ started_at: expect.any(Number), ended_at: expect.any(Number) });
  const list = await api.call("app.flow.list", { flow: "demo-main" });
  expect(list.find((s: any) => s.id === run).phase.state).toBe("paused");

  await api.call("app.flow.resume", { run });
  const done = await until(run, (v) => station(v, "sign")?.status === "working", "both main-session stations hand in");
  expect(done.record.paused).toBeUndefined();
  // Both turns were the main session's.
  const session = main(done)!;
  expect(station(done, "draft").tasks[0].session).toBe(session);
  expect(station(done, "edit").tasks[0].session).toBe(session);
  expect(station(done, "draft").tasks[0].output).toEqual({ text: "a draft" });
  expect(station(done, "edit").tasks[0].output).toEqual({ edited: true });
  // The flow answered each TodoWrite from the station being worked on.
  const { messages } = await api.call("session.history", ref(session));
  const todo = results(messages).filter((b: any) => JSON.stringify(b).includes("flow-write-probe") || b.is_error);
  const refused = results(messages).find((b: any) => JSON.stringify(b).includes("station draft of the flow does not allow TodoWrite"));
  expect(refused, `draft's TodoWrite was not refused: ${JSON.stringify(results(messages)).slice(0, 800)}`).toBeTruthy();
  expect(refused.is_error).toBe(true);
  expect(results(messages).filter((b: any) => b.is_error).length, JSON.stringify(todo).slice(0, 800)).toBe(1);
  // Tokens: each station's turn, counted on its task once the turn is over (it handed in during it).
  const counted = await until(run, (v) => !!station(v, "edit")?.tasks?.[0]?.usage, "edit's turn is counted");
  expect(station(counted, "draft").tasks[0].usage.turns).toBeGreaterThanOrEqual(1);
  expect(station(counted, "edit").tasks[0].usage.input_tokens).toBeGreaterThan(0);
  const phase = (await api.call("app.flow.list", { flow: "demo-main" })).find((s: any) => s.id === run);
  expect(phase.phase).toMatchObject({ state: "waiting", reason: "confirm" });
  expect(phase.usage.input_tokens).toBeGreaterThan(0);

  // Its sessions: the main one, nothing of its own.
  const sessions = await api.call("app.flow.sessions", { run });
  expect(sessions.main).toEqual(ref(session));
  expect(sessions.own).toEqual([]);
  await api.call("app.flow.confirm", { run, station: "sign", pass: true });
  await until(run, (v) => v.record.status === "finished", "finished");
});

test("a sub-agent station: the main Agent hands the task to a sub-agent of the main session", TAGS, async () => {
  const run = await start("demo-agent");
  expect(run).toBe("demo-agent-1"); // the recording names the task's tag
  const v = await until(run, (v) => station(v, "check")?.status === "passed", "the sub-agent's result is handed in");
  expect(station(v, "check").tasks[0].output).toEqual({ files: ["README.md"] });
  const session = main(v)!;
  const info = await api.call("session.get", ref(session));
  expect(info.scene).toBe("flow-coding");
  const { messages } = await api.call("session.history", ref(session));
  expect(JSON.stringify(messages)).toContain("flow:demo-agent-1/check/check");
  const sessions = await api.call("app.flow.sessions", { run });
  expect(sessions.main).toEqual(ref(session));
  // The sub-agent was built and ran (the call's result is not an engine error), named by its tag.
  const call = results(messages).find((b: any) => String(b.tool_use_id).startsWith("call_Agent"));
  expect(call, JSON.stringify(results(messages)).slice(0, 600)).toBeTruthy();
  expect(JSON.stringify(call)).not.toContain("already registered");
  expect(call.is_error, JSON.stringify(call)).toBeFalsy();
  // The run lists it, tied to the flow task by its tag, with its own session to open.
  const agent = (await api.call("app.flow.sessions", { run })).agents[0];
  expect(agent, JSON.stringify(sessions)).toBeTruthy();
  expect(agent.agent_session_id).toEqual(expect.any(String));
  expect(agent.flow).toEqual({ station: "check", task: "check" });
  expect(agent.description).toBe("flow:demo-agent-1/check/check");
});

test("each flow has a conversation of its own, the general one is apart; the guide is there in two languages", TAGS, async () => {
  const general = await api.call("app.flow.session", {});
  expect(general.flow).toBeNull();
  expect(await api.call("app.flow.session", {})).toEqual(general);
  const mine = await api.call("app.flow.session", { flow: "demo-main" });
  expect(mine).toMatchObject({ flow: "demo-main" });
  expect(mine.session_id).not.toBe(general.session_id);
  expect(await api.call("app.flow.session", { flow: "demo-main" })).toEqual(mine);
  expect((await api.call("session.get", { instance: mine.instance, session_id: mine.session_id })).scene).toBe("flow-author");
  // Its flow does not change.
  const fixed = await api.refused("app.flow.focus", { instance: mine.instance, session_id: mine.session_id, flow: "demo-linear" });
  expect(fixed?.message).toMatch(/does not change/);
  expect((await api.refused("app.flow.session", { flow: "nope" }))?.message).toMatch(/flow nope/);

  const en = await api.call("app.flow.guide", { lang: "en" });
  expect(en.text).toContain("# Flow guide");
  const zh = await api.call("app.flow.guide", { lang: "zh-CN" });
  expect(zh).toMatchObject({ lang: "zh-CN" });
  expect(zh.text).toContain("主会话");
});
