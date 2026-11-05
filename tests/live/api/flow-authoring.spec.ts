/**
 * Writing flows and trying them out (xops-02 §3–5), in `harnix-testapp`, whose flow packs have a
 * writable layer in the module's data directory over the demonstration flows.
 *
 * - In conversation: a session of the flow space runs `flow-author`, whose tools create, change
 *   and delete flows. Which flow it works on is the conversation's focus — what the panel beside
 *   it selected (`app.flow.focus`), or what `flow_create` just made. The recordings call the tools:
 *   `flow-author-unfocused` (`flow_update` with nothing selected), `flow-author-edit` (three turns:
 *   schema, validate, create · show, update, dry-run · delete; a filler between turns answers the
 *   engine's memory extraction). Tools that change something are asked about and permitted here.
 * - Through the methods: `save` / `delete` and starting what was saved.
 * - Dry runs: the kernel in a sandbox, everything outside mocked — no session is opened.
 */

import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:flow", "@layer:api"] };

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

/** A session of the flow space replaying `fixture`. */
async function author(fixture: string): Promise<string> {
  const created = await api.call("session.create", { space: "flow", options: { recorder: { name: fixture } } });
  expect(created.space).toBe("flow");
  expect((await api.call("session.get", { session_id: created.session_id })).scene).toBe("flow-author");
  await api.call("harnix.attach", { session_id: created.session_id });
  return created.session_id;
}

const permitted = new Set<string>();
const asks: any[] = [];

/** One turn, permitting every ask it raises; its settlement. */
async function turn(session_id: string, message: string): Promise<any> {
  const sent = await api.call("harnix.send", { session_id, message });
  for (;;) {
    const f = await api.waitFor(
      (f) =>
        (f.method === "harnix.turn_settled" && f.params.turn_id === sent.turn_id) ||
        (f.method === "harnix.event" &&
          f.params.session_id === session_id &&
          f.params.event?.kind === "prompt" &&
          !permitted.has(f.params.event.prompt_id)),
      { timeout: 60_000, describe: "the turn to settle, or an ask" },
    );
    if (f.method === "harnix.turn_settled") return f.params;
    permitted.add(f.params.event.prompt_id);
    asks.push(f.params.event);
    await api.call("session.respondToPrompt", {
      session_id,
      prompt_id: f.params.event.prompt_id,
      decision: { type: "permit" },
    });
  }
}

/** Every tool result of a session, by the tool that was called. */
async function results(session_id: string): Promise<Record<string, any[]>> {
  const { messages } = await api.call("session.history", { session_id });
  const blocks = messages.flatMap((m: any) => (Array.isArray(m.content) ? m.content : []));
  const names = new Map<string, string>(blocks.filter((b: any) => b.type === "tool_use").map((b: any) => [b.id, b.name]));
  const out: Record<string, any[]> = {};
  for (const b of blocks.filter((b: any) => b.type === "tool_result")) {
    const name = names.get(b.tool_use_id) ?? "?";
    (out[name] ??= []).push(b);
  }
  return out;
}

const topic = async (name: string, params: Record<string, unknown> = {}) =>
  (await api.call("harnix.subscribe", { topic: `app.flow.${name}`, params })).value;

test("without a selection, changing the current flow answers that the user must select one first", TAGS, async () => {
  const session = await author("flow-author-unfocused");
  await turn(session, "Change the flow's title.");
  const [update] = (await results(session)).flow_update;
  expect(update.is_error).toBe(true);
  expect(JSON.stringify(update.content)).toContain("No flow is selected");
  expect((await api.call("app.flow.defs")).some((d: any) => d.name === "authored")).toBe(false);
  // The panel's selection is the focus, and is published for the panel.
  const selected = await api.call("app.flow.focus", { instance: "system", session_id: session, flow: "demo-linear" });
  expect(selected).toEqual({ flow: "demo-linear" });
  expect(await topic("focus", { instance: "system", session_id: session })).toEqual({ flow: "demo-linear" });
  const missing = await api.refused("app.flow.focus", { instance: "system", session_id: session, flow: "nothing" });
  expect(missing?.data?.flow).toBe("not_found");
  // Only a flow-author conversation has a focus.
  const plain = await api.call("session.create", { options: { recorder: { name: "says-ready" } } });
  const notAuthor = await api.refused("app.flow.focus", { instance: plain.instance, session_id: plain.session_id, flow: "demo-linear" });
  expect(notAuthor?.message).toContain("not a flow-author conversation");
  const nobody = await api.refused("app.flow.focus", { instance: "system", session_id: "no-such-session", flow: "demo-linear" });
  expect(nobody?.message).toContain("no session");
  expect(await api.call("app.flow.focus", { instance: "system", session_id: session, flow: null })).toEqual({ flow: null });
});

test("a flow-author conversation creates a flow (and selects it), shows, changes and dry-runs it, and deletes it", TAGS, async () => {
  const session = await author("flow-author-edit");

  await turn(session, "Write a flow `authored`: a draft step, then an approval.");
  let r = await results(session);
  expect(r.flow_schema[0].is_error ?? false).toBe(false);
  expect(JSON.stringify(r.flow_validate[0].content)).toContain('\\"problems\\": []');
  expect(r.flow_create[0].is_error ?? false).toBe(false);
  let defs = await topic("defs");
  expect(defs.find((d: any) => d.name === "authored")).toMatchObject({ origin: "user", valid: true, title: "Authored in conversation" });
  // Created means selected: the panel is told.
  expect(await topic("focus", { instance: "system", session_id: session })).toEqual({ flow: "authored" });
  expect((await api.call("app.flow.def", { name: "authored" })).skills).toEqual({});
  expect((await api.call("app.flow.catalog")).skills).toContain("authored-note");

  await turn(session, "Show it, change its title, and try it.");
  r = await results(session);
  expect(JSON.stringify(r.flow_show[0].content)).toContain("Authored in conversation");
  expect(r.flow_update[0].is_error ?? false).toBe(false);
  expect((await api.call("app.flow.def", { name: "authored" })).def.title).toBe("Authored, then changed");
  const dry = JSON.parse(r.flow_dry_run[0].content.map?.((c: any) => c.text).join("") ?? r.flow_dry_run[0].content);
  expect(dry.run).toMatch(/^dry-authored-/);
  expect(dry.stations).toEqual({ draft: "passed", approve: "working" });
  expect((await api.call("app.flow.dry_run.list")).map((d: any) => d.id)).toContain(dry.run);

  await turn(session, "Delete it.");
  r = await results(session);
  expect(r.flow_delete[0].is_error ?? false).toBe(false);
  defs = await topic("defs");
  expect(defs.some((d: any) => d.name === "authored")).toBe(false);
  expect(await topic("focus", { instance: "system", session_id: session })).toEqual({ flow: null });
  // Each tool that changes something was asked about (create, update, delete here; the update
  // without a selection before): the person in the conversation decides.
  expect(permitted.size).toBeGreaterThanOrEqual(4);
  // Each ask is about a call that names its flow: what the person approves is that flow. (The
  // engine shows its own "Allow <tool>?", so the card reads the flow from the call's input.)
  expect(asks.map((a) => a.tool_name)).toEqual(expect.arrayContaining(["flow_create", "flow_update", "flow_delete"]));
  const { messages: all } = await api.call("session.history", { session_id: session });
  const calls = new Map<string, any>(
    all.flatMap((m: any) => (Array.isArray(m.content) ? m.content : [])).filter((b: any) => b.type === "tool_use").map((b: any) => [b.id, b]),
  );
  const named = asks
    .filter((a) => a.tool_name === "flow_update" || a.tool_name === "flow_delete")
    .map((a) => calls.get(a.tool_use_id))
    .filter(Boolean);
  expect(named).toHaveLength(2);
  for (const call of named) expect(call.input.flow).toBe("authored");
  // Nothing in the conversation confirms or forces: those tools do not exist.
  const { messages } = await api.call("session.history", { session_id: session });
  expect(JSON.stringify(messages)).not.toMatch(/flow_(confirm|force|rework|release|cancel)/);
});

test("a flow saved through the methods can be started; an override gives way to the built-in one", TAGS, async () => {
  const yaml = "name: saved-one\ntitle: Saved\nstations: [{id: approve, exit: [{kind: confirm}]}]";
  const bad = await api.refused("app.flow.save", {
    yaml: "name: saved-bad\nstations: [{id: a, executor: {kind: program, step: nowhere}, output: {type: object}, exit: [{kind: output}]}]",
  });
  expect(bad?.message).toContain("nowhere");
  expect((await api.call("app.flow.defs")).some((d: any) => d.name === "saved-bad")).toBe(false);

  // `dry-` is kept for dry runs: no flow may take it, so no real run can look like one.
  const dry = await api.refused("app.flow.save", { yaml: "name: dry-sneaky\nstations: [{id: a, exit: [{kind: confirm}]}]" });
  expect(dry?.message).toContain("kept for dry runs");

  expect(await api.call("app.flow.save", { yaml })).toMatchObject({ name: "saved-one", origin: "user" });
  const { run } = await api.call("app.flow.start", { flow: "saved-one", subject: "x" });
  await api.call("app.flow.confirm", { run, station: "approve", pass: true });
  expect((await api.call("app.flow.get", { run })).record.status).toBe("finished");

  const linear = (await api.call("app.flow.def", { name: "demo-linear" })).yaml as string;
  const changed = linear.replace('title: "Demo: linear"', 'title: "Demo: linear, changed"');
  expect(await api.call("app.flow.save", { yaml: changed })).toMatchObject({ origin: "override" });
  expect((await api.call("app.flow.def", { name: "demo-linear" })).def.title).toBe("Demo: linear, changed");
  expect(await api.call("app.flow.delete", { name: "demo-linear" })).toEqual({ name: "demo-linear", origin: "builtin" });
  expect((await api.call("app.flow.def", { name: "demo-linear" })).def.title).toBe("Demo: linear");
  const builtin = await api.refused("app.flow.delete", { name: "demo-linear" });
  expect(builtin?.message).toContain("built in");

  expect(await api.call("app.flow.skill.save", { name: "saved-skill", body: "Do it." })).toMatchObject({ name: "saved-skill" });
  expect(await api.call("app.flow.skill.delete", { name: "saved-skill" })).toEqual({ name: "saved-skill" });
  await api.call("app.flow.delete", { name: "saved-one" });
});

test("a dry run opens no session, runs to a person, takes their part through the same methods, and finishes", TAGS, async () => {
  const before = (await api.call("harnix.sessions", { space: "system" })).sessions.length;
  // The program's output is mocked (its contract's minimal value is no modules at all).
  const mocks = { stations: { collect: { modules: ["core", "ui"] } } };
  const { run } = await api.call("app.flow.dry_run.start", { flow: "demo-review", mocks });
  expect(run).toMatch(/^dry-demo-review-/);
  const one = await api.call("app.flow.dry_run.step", { run });
  expect(one.steps).toHaveLength(1);
  expect(one.steps[0]).toMatchObject({ kind: "task", station: "collect", result: "submitted" });
  const ran = await api.call("app.flow.dry_run.run", { run });
  expect(ran).toMatchObject({ pending: 0, status: "active" });

  const view = await api.call("app.flow.get", { run });
  const st = view.record.stations;
  expect(st.review.status).toBe("working");
  expect(st.lint.status).toBe("passed");
  // A mocked session task records the message its session would have been sent.
  expect(st.analyze.tasks.map((t: any) => t.id)).toEqual(["core", "ui"]);
  const preview = st.analyze.tasks[0].preview;
  expect(preview.message).toContain("Flow protocol");
  expect(preview.message).toContain("## Skill `analyze`");
  expect(preview).toMatchObject({ scene: "flow-chat", delivery: "flow_submit", round: 1 });
  // The same on its topic.
  expect((await topic("run", { run })).record.stations.analyze.tasks[0].preview.message).toBe(preview.message);

  // A person's part: the same methods, reaching the sandbox.
  await api.call("app.flow.confirm", { run, station: "review", pass: true });
  const report = (await api.call("app.flow.get", { run })).record.stations.report;
  expect(report.tasks.find((t: any) => t.id === "sign").status).toBe("ready");
  const { token } = await api.call("app.flow.claim", { run, station: "report", task: "sign" });
  await api.call("app.flow.submit", { run, station: "report", task: "sign", token, output: { signed: true } });
  expect((await api.call("app.flow.get", { run })).record.status).toBe("finished");
  expect((await api.call("app.flow.history", { run })).some((h: any) => h.op === "confirm")).toBe(true);

  // Not a run of the module's, and nothing was opened for it.
  expect((await api.call("app.flow.list", {})).some((r: any) => r.id === run)).toBe(false);
  expect((await topic("dry_runs")).find((d: any) => d.id === run)).toMatchObject({ flow: "demo-review", status: "finished" });
  expect((await api.call("harnix.sessions", { space: "system" })).sessions.length).toBe(before);
});

test("a gate mocked to fail stops the dry run at its station", TAGS, async () => {
  const { run } = await api.call("app.flow.dry_run.start", { flow: "demo-review", mocks: { gates: { lint: false } } });
  const ran = await api.call("app.flow.dry_run.run", { run });
  expect(ran.status).toBe("active");
  expect(ran.steps.find((s: any) => s.kind === "gate")).toMatchObject({ gate: "lint", result: "not passed" });
  const st = (await api.call("app.flow.get", { run })).record.stations;
  expect(st.lint.status).toBe("unmet");
  expect(st.review.status).toBe("pending");
  // An unsaved text runs too, with outputs mocked.
  const draft = await api.call("app.flow.dry_run.start", {
    yaml: "name: draft-only\nstations: [{id: a, executor: {kind: program, step: unknown-here}, output: {type: object, required: [n], properties: {n: {const: 3}}}, exit: [{kind: output}]}]",
  });
  expect((await api.call("app.flow.dry_run.run", { run: draft.run })).status).toBe("finished");
  expect((await api.call("app.flow.get", { run: draft.run })).record.stations.a.tasks[0].output).toEqual({ n: 3 });
});
