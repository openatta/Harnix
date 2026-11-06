/**
 * Where a flow station's session runs, on which model, how it hands in and what it may do
 * (xops-02 §1–2, xops-04 §3), in `harnix-testapp`.
 *
 * The flow module there has two templates of its own: `flow.local` (in process) and
 * `flow.remote` (a spawned daemon); both engines have `structured-chat`, Agix's chat scene with
 * `StructuredOutput` allowed, as a native contribution. A station names the template (and its
 * space, its model); the module makes the instance ready, opens the session there with the
 * station's contract as its `output_schema` and an `ask` permission rule for each guarded tool,
 * answers each question from the task's permissions, and submits each turn's structured output.
 *
 * Session stations replay `flow-<station>`: `flow-local` and `flow-remote` call `StructuredOutput`
 * once; `flow-guarded` first calls `TodoWrite` (guarded in this application), which no permission
 * of the station allows: the flow refuses it.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { type Backend, type TestClient, connect, expect, sleep, test } from "@harnix/testkit";

import { instances } from "./instances";
import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:flow", "@layer:api"] };

type View = { record: any; unmet: Record<string, any[]> };

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  // The instance `demo-bypassed` places its session on is set to bypass permissions.
  backend = await startTestApp({
    seed: (scratch) => {
      const root = join(scratch, "home", ".harnix", "instances", "bypass-box");
      mkdirSync(root, { recursive: true });
      writeFileSync(join(root, "settings.json"), JSON.stringify({ permission_mode: "bypassPermissions" }));
    },
  });
});
test.afterAll(() => backend?.stop());
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

async function start(flow: string): Promise<string> {
  const { run } = await api.call("app.flow.start", { flow, subject: "repo" });
  return run;
}

async function until(run: string, ok: (v: View) => boolean, describe: string): Promise<View> {
  const sub = await api.call("harnix.subscribe", { topic: "app.flow.run", params: { run } });
  if (sub.value && ok(sub.value)) return sub.value;
  const frame = await api.waitFor(
    (f) => f.method === "harnix.topic" && f.params.topic === "app.flow.run" && f.params.params?.run === run && ok(f.params.value),
    { timeout: 90_000, describe: `${run}: ${describe}` },
  );
  return frame.params.value;
}

const station = (v: View, id: string) => v.record.stations[id];

/** Wait until an instance is gone from the list. */
async function gone(name: string): Promise<void> {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (!(await instances(api))[name]) return;
    await sleep(100);
  }
  throw new Error(`instance ${name} is still there`);
}
const ref = (session: string) => {
  const [instance, session_id] = session.split("/");
  return { instance, session_id };
};

test("sessions on an in-process and a spawned template instance hand in by structured output; the run finishes", TAGS, async () => {
  const run = await start("demo-placed");
  const waiting = await until(run, (v) => station(v, "approve")?.status === "working", "approve waits");
  const local = station(waiting, "local").tasks[0];
  const remote = station(waiting, "remote").tasks[0];
  for (const t of [local, remote]) {
    expect(t.status).toBe("submitted");
    expect(t.output.findings).toHaveLength(1);
  }
  // On instances of the module's templates: named by the station's `name` parameter, or by the
  // module after the run and the template.
  expect(ref(local.session).instance).toBe(`local-${run}`);
  const made = ref(remote.session).instance;
  expect(made).toMatch(new RegExp(`^flow-${run}-[0-9a-f]{8}$`));
  const all = await instances(api);
  expect(all[`local-${run}`]).toMatchObject({ carrier: "in_process", owner: "module:flow", template: "flow.local" });
  expect(all[made]).toMatchObject({ carrier: "spawn", owner: "module:flow", template: "flow.remote" });

  // Handed in by the dispatcher from the turn's structured output, not by a native tool.
  const hist = await api.call("app.flow.history", { run });
  const submits = hist.filter((h: any) => h.op === "submit");
  expect(submits.map((h: any) => [h.station, h.by.kind])).toEqual([["local", "flow"], ["remote", "flow"]]);
  for (const t of [local, remote]) {
    const text = JSON.stringify((await api.call("session.history", ref(t.session))).messages);
    expect(text).toContain("StructuredOutput");
    expect(text).not.toContain("flow_submit");
  }

  // The local station's session is in the space it names, on the model it names.
  const inFleet = (await api.call("harnix.sessions", { space: "fleet" })).sessions.map((s: any) => s.session_id);
  expect(inFleet).toContain(ref(local.session).session_id);
  const inFlow = (await api.call("harnix.sessions", { space: "flow" })).sessions.map((s: any) => s.session_id);
  expect(inFlow).toContain(ref(remote.session).session_id);
  expect((await api.call("app.fleet.model", ref(local.session))).model).toBe("flow-demo-model");

  await api.call("app.flow.confirm", { run, station: "approve", pass: true });
  await until(run, (v) => v.record.status === "finished", "the run finishes");
  // The instance the module named for the run goes with it; the one the station named stays.
  await gone(made);
  expect((await instances(api))[`local-${run}`]).toBeTruthy();
});

test("a write no permission allows is refused on a template instance: the flow answers the session's question", TAGS, async () => {
  const run = await start("demo-guarded");
  const waiting = await until(run, (v) => station(v, "approve")?.status === "working", "approve waits");
  const task = station(waiting, "guarded").tasks[0];
  expect(task.output).toEqual({ done: true });
  const made = ref(task.session).instance;
  expect(made).toMatch(new RegExp(`^flow-${run}-[0-9a-f]{8}$`));
  const { messages } = await api.call("session.history", ref(task.session));
  // The TodoWrite call's result is an error: refused before it ran.
  const results = messages
    .flatMap((m: any) => (Array.isArray(m.content) ? m.content : []))
    .filter((b: any) => b.type === "tool_result");
  const refused = results.find((b: any) => JSON.stringify(b).includes("station guarded of the flow does not allow TodoWrite"));
  expect(refused, `no refused tool result in ${JSON.stringify(results).slice(0, 600)}`).toBeTruthy();
  expect(refused.is_error).toBe(true);
  expect(JSON.stringify(messages)).toContain("Refused unless a permission allows them");
  // A cancelled run's instance goes as well.
  await api.call("app.flow.cancel", { run, reason: "seen enough" });
  await gone(made);
});

test("on an instance set to bypass permissions, the session still keeps its station's rules", TAGS, async () => {
  const run = await start("demo-bypassed");
  const done = await until(run, (v) => v.record.status === "finished", "the run finishes");
  const task = station(done, "bypassed").tasks[0];
  expect(ref(task.session).instance).toBe("bypass-box");
  const { messages } = await api.call("session.history", ref(task.session));
  const refused = messages
    .flatMap((m: any) => (Array.isArray(m.content) ? m.content : []))
    .find((b: any) => b.type === "tool_result" && JSON.stringify(b).includes("station bypassed of the flow does not allow TodoWrite"));
  expect(refused?.is_error, `TodoWrite was not refused: ${JSON.stringify(messages).slice(0, 600)}`).toBe(true);
});

test("a station placed where there is nothing fails its task, saying why", TAGS, async () => {
  const run = await start("demo-misplaced");
  const failed = await until(
    run,
    (v) => station(v, "nowhere")?.tasks?.[0]?.status === "failed",
    "the task fails",
  );
  const task = station(failed, "nowhere").tasks[0];
  expect(task.error).toContain("flow.nowhere");
  expect(task.error).toMatch(/no template/);
  expect(task.session ?? null).toBeNull();
  expect(station(failed, "nowhere").status).toBe("unmet");
  await api.call("app.flow.cancel", { run, reason: "nowhere to go" });
});
