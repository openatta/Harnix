/**
 * The parts of a flow pack besides flows (xops-04), in `harnix-testapp`: rules, policies and
 * their permissions, gates, hooks.
 *
 * `demo-asked`: one station in the run's main session, under the policy `ask-todo` (a person is
 * asked before `TodoWrite`) and the rule `review-rules`; its exit is the tool gate `open-high`,
 * and the next station's the command gate `self-test`. The main session replays `flow-asked`:
 * `TodoWrite`, then `flow_submit {edited: true}`.
 *
 * Hooks: a webhook put in the pack is told a run's events, filtered by event and by flow, while
 * no flow names it.
 */

import { createServer, type IncomingHttpHeaders, type Server } from "node:http";
import type { AddressInfo } from "node:net";

import { type Backend, type TestClient, connect, expect, sleep, test } from "@harnix/testkit";

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

async function until(run: string, ok: (v: View) => boolean, describe: string): Promise<View> {
  const sub = await api.call("harnix.subscribe", { topic: "app.flow.run", params: { run } });
  if (sub.value && ok(sub.value)) return sub.value;
  const frame = await api.waitFor(
    (f) => f.method === "harnix.topic" && f.params.topic === "app.flow.run" && f.params.params?.run === run && ok(f.params.value),
    { timeout: 60_000, describe: `${run}: ${describe}` },
  );
  return frame.params.value;
}

const station = (v: View, id: string) => v.record.stations[id];

test("a call a policy asks about waits for a person, the run waits for approval, then the gates judge", TAGS, async () => {
  const { run } = await api.call("app.flow.start", { flow: "demo-asked", subject: "repo" });
  const held = await until(run, (v) => !!station(v, "edit")?.tasks?.[0]?.asking, "the TodoWrite call is held");
  expect(held.record.stations.edit.tasks[0].asking).toMatchObject({
    tool: "TodoWrite",
    reason: "the todo list is a person's call here",
    since: expect.any(Number),
  });
  // The policy and gates were pinned with the run.
  expect(Object.keys(held.record.parts.policies)).toEqual(["ask-todo"]);
  expect(Object.keys(held.record.parts.gates).sort()).toEqual(["open-high", "self-test"]);
  const summary = (await api.call("app.flow.list", { flow: "demo-asked" })).find((s: any) => s.id === run);
  expect(summary.phase).toMatchObject({ state: "waiting", reason: "approval", stations: ["edit"] });

  // The person answers in the main session.
  const session = held.record.session_keys["@main"] as string;
  const [instance, session_id] = session.split("/");
  const attached = await api.call("harnix.attach", { session_id });
  const ask = attached.pending_prompts.find((p: any) => p.tool_name === "TodoWrite");
  expect(ask, JSON.stringify(attached.pending_prompts)).toBeTruthy();
  await api.call("session.respondToPrompt", { session_id, prompt_id: ask.prompt_id, decision: { type: "permit" } });

  const done = await until(run, (v) => v.record.status === "finished", "the run finishes");
  expect(station(done, "edit").tasks[0].asking).toBeUndefined();
  expect(station(done, "edit").tasks[0].output).toEqual({ edited: true });
  // The tool gate passed on the spot; the command gate ran in the run's workspace.
  const gate = (id: string) => station(done, id).checks.find((c: any) => c.kind === "gate");
  expect(gate("edit")).toMatchObject({ status: "pass", label: "gate open-high" });
  expect(gate("test")).toMatchObject({ status: "pass" });
  expect(gate("test").detail).toContain("self-test passed");
  // The call went through: its result is no error. The rule was in the task's message.
  const { messages } = await api.call("session.history", { instance, session_id });
  const text = JSON.stringify(messages);
  expect(text).toContain("### Review rules (`review-rules`)");
  const result = messages
    .flatMap((m: any) => (Array.isArray(m.content) ? m.content : []))
    .find((b: any) => b.type === "tool_result" && String(b.tool_use_id).includes("TodoWrite"));
  expect(result?.is_error ?? false, JSON.stringify(result)).toBe(false);
  const history = await api.call("app.flow.history", { run });
  expect(history.map((h: any) => h.op)).toContain("finish");
});

test("the parts are read and saved by kind; a part a flow needs cannot go without the flow saying so", TAGS, async () => {
  const gates = await api.call("app.flow.parts", { kind: "gate" });
  expect(gates.map((g: any) => g.name)).toEqual(["lint", "open-high", "self-test"]);
  expect(gates.find((g: any) => g.name === "open-high")).toMatchObject({ title: "No open high findings", origin: "builtin" });
  expect(gates.find((g: any) => g.name === "open-high").flows.sort()).toEqual(["demo-asked", "demo-review"]);
  const policy = await api.call("app.flow.part", { kind: "policy", name: "ask-todo" });
  expect(policy).toMatchObject({ kind: "policy", origin: "builtin", problems: [], flows: ["demo-asked"] });
  expect(policy.parsed.permissions[0]).toMatchObject({ effect: "ask", tools: ["TodoWrite"] });
  const rule = await api.call("app.flow.part", { kind: "rule", name: "review-rules" });
  expect(rule.title).toBe("Review rules");

  // A part with problems is not saved.
  const bad = await api.refused("app.flow.part.save", { kind: "gate", name: "broken", text: "name: broken\nkind: command\n" });
  expect(bad?.message).toContain("needs its command");
  const host = await api.refused("app.flow.part.save", { kind: "gate", name: "nobody", text: "name: nobody\nkind: host\n" });
  expect(host?.message).toContain("no check named nobody");
  // A flow on a part of its own: the part goes, the flow is said to be broken.
  await api.call("app.flow.part.save", { kind: "rule", name: "mine", text: "# Mine\n\nKeep it short." });
  await api.call("app.flow.save", { yaml: "name: ruled\nrules: [mine]\nstations: [{id: a, exit: [{kind: confirm}]}]" });
  const gone = await api.call("app.flow.part.delete", { kind: "rule", name: "mine" });
  expect(gone.broken).toEqual([{ flow: "ruled", problems: ["rule mine is not in the packs"] }]);
  expect((await api.call("app.flow.defs")).find((d: any) => d.name === "ruled").valid).toBe(false);
  await api.call("app.flow.delete", { name: "ruled" });
});

test("a command gate runs with none of the application's secrets, the run's context in a file", TAGS, async () => {
  // The test application is started with HARNIX_CREDENTIAL_PROBE_KEY in its environment.
  await api.call("app.flow.part.save", {
    kind: "gate",
    name: "clean-env",
    text: [
      "name: clean-env",
      "kind: command",
      "cwd: sub",
      `command: 'test -z "$HARNIX_CREDENTIAL_PROBE_KEY" && test -s "$FLOW_CONTEXT_FILE" && grep -q clean-subject "$FLOW_CONTEXT_FILE" && test "$(basename "$PWD")" = sub && echo clean'`,
      "",
    ].join("\n"),
  });
  await api.call("app.flow.save", { yaml: "name: env-gated\nstations: [{id: a, exit: [{kind: gate, gate: clean-env}]}]" });
  try {
    const { run } = await api.call("app.flow.start", { flow: "env-gated", subject: "clean-subject" });
    const done = await until(run, (v) => v.record.status === "finished" || station(v, "a")?.status === "unmet", "the gate is judged");
    const check = station(done, "a").checks.find((c: any) => c.kind === "gate");
    expect(check, JSON.stringify(station(done, "a"))).toMatchObject({ status: "pass" });
    expect(check.detail).toContain("clean");
  } finally {
    await api.call("app.flow.delete", { name: "env-gated" }).catch(() => {});
    await api.call("app.flow.part.delete", { kind: "gate", name: "clean-env" }).catch(() => {});
  }
});

type Delivery = { headers: IncomingHttpHeaders; body: any };

test("a hook put in the pack is told a run's events, filtered by event and flow; no flow names it", TAGS, async () => {
  const got: Delivery[] = [];
  const server: Server = createServer((req, res) => {
    let data = "";
    req.on("data", (c) => (data += c));
    req.on("end", () => {
      got.push({ headers: req.headers, body: JSON.parse(data) });
      res.writeHead(204).end();
    });
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  const port = (server.address() as AddressInfo).port;
  try {
    await api.call("app.flow.save", { yaml: "name: hooked\nstations: [{id: a, exit: [{kind: confirm}]}, {id: b, exit: [{kind: confirm}]}]" });
    await api.call("app.flow.part.save", {
      kind: "hook",
      name: "live-hook",
      text: `name: live-hook\non: ["run.*", station.passed]\nflows: [hooked]\nkind: webhook\naction: http://127.0.0.1:${port}/flow\n`,
    });
    // Another flow's run is not this hook's.
    const other = await api.call("app.flow.start", { flow: "demo-misplaced", subject: "elsewhere" });
    const { run } = await api.call("app.flow.start", { flow: "hooked", subject: "repo" });
    await until(run, (v) => station(v, "a")?.status === "working", "a waits");
    await api.call("app.flow.confirm", { run, station: "a", pass: true });
    await until(run, (v) => station(v, "b")?.status === "working", "b waits");
    await api.call("app.flow.confirm", { run, station: "b", pass: true });
    for (let i = 0; i < 100 && !got.some((d) => d.body.event.kind === "run.finished"); i++) await sleep(100);

    const kinds = got.map((d) => `${d.body.event.kind}${d.body.event.station ? `:${d.body.event.station}` : ""}`);
    expect(kinds).toEqual(["run.created", "station.passed:a", "station.passed:b", "run.finished"]);
    for (const d of got) {
      expect(d.body).toMatchObject({ hook: "live-hook", event: { run, flow: "hooked" }, run: { id: run, flow: "hooked" } });
      expect(d.headers["x-harnix-event"]).toBe(d.body.event.kind);
      expect(d.headers["x-harnix-delivery"]).toBe(d.body.delivery);
      expect(Number(d.headers["x-harnix-timestamp"])).toBeGreaterThan(Date.now() - 600_000);
      expect(d.headers["x-harnix-signature"]).toBeUndefined();
    }
    expect(new Set(got.map((d) => d.body.delivery)).size).toBe(got.length);
    expect(got.at(-1)!.body.run).toMatchObject({ status: "finished", phase: { state: "finished" } });

    const hooks = await api.call("app.flow.hooks");
    expect(hooks.find((h: any) => h.name === "live-hook")).toMatchObject({
      kind: "webhook", signed: false, last: { status: "delivered", code: 204, event: "run.finished", run, tries: 1 },
    });
    await api.call("app.flow.cancel", { run: other.run, reason: "done" });
  } finally {
    server.close();
    await api.call("app.flow.part.delete", { kind: "hook", name: "live-hook" }).catch(() => {});
    await api.call("app.flow.delete", { name: "hooked" }).catch(() => {});
  }
});
