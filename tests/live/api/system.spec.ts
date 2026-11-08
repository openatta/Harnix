/**
 * The system space's backend (design §9): any instance's sessions inspected and acted on through
 * `harnix.system.*` and the hub's own methods claiming the system space — and none of it there
 * when the profile turns the space off: the methods leave the authorization table, the space
 * stops existing, and nothing can claim it.
 *
 * Runs on `harnix-testapp`, whose `fleet` module builds an instance (`fleet-side`) for a space of
 * its own: the session there is another space's, which is the case the system space is for.
 */

import { cpSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { INVALID_PARAMS, METHOD_NOT_FOUND, REFUSED, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, RECORDINGS, type TestClient, connect, expect, test } from "@harnix/testkit";

import { until } from "./instances";
import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:system", "@layer:api"] };

const SYSTEM_METHODS = [
  "harnix.system.context",
  "harnix.system.calls",
  "harnix.system.turns",
  "harnix.system.tasks.stop",
  "harnix.system.tasks.output",
  "harnix.system.messages.drop",
  "harnix.system.messages.edit",
  "harnix.system.compact",
  "harnix.system.clear",
  "harnix.system.model",
  "harnix.system.rename",
];

/** A session in `space` that has had one turn, the user's message first. */
async function oneTurn(api: TestClient, space: string): Promise<{ instance: string; session_id: string }> {
  const created = await api.call("session.create", { space, options: { recorder: { name: "says-ready" } } });
  const s = { instance: created.instance, session_id: created.session_id };
  await api.call("harnix.attach", { ...s, space });
  await api.call("harnix.send", { ...s, space, message: "Say exactly: fixture ready" });
  const settled = await api.waitFor(
    (f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id,
    { timeout: 60_000, describe: "the turn to settle" },
  );
  expect(settled.params.error).toBeUndefined();
  return s;
}

const text = (message: any): string =>
  typeof message.content === "string"
    ? message.content
    : message.content.map((b: any) => (b.type === "text" ? b.text : "")).join("");

test.describe("on", () => {
  let backend: Backend;
  let api: TestClient;

  test.describe.configure({ mode: "serial" });
  test.beforeAll(async () => {
    backend = await startTestApp();
  });
  test.afterAll(async () => { await backend?.dispose(); });
  test.beforeEach(async () => {
    api = await connect(backend);
  });
  test.afterEach(() => api?.close());

  test("the methods are offered", TAGS, async () => {
    const { methods } = await api.call("harnix.reachable");
    expect(methods).toEqual(expect.arrayContaining(SYSTEM_METHODS));
  });

  test("the context of another space's session, and its messages rewritten and removed", TAGS, async () => {
    const s = await oneTurn(api, "fleet");
    const context = await api.call("harnix.system.context", s);
    expect(context.basis).toBe("next_step_without_new_input");
    expect(context.system.length).toBeGreaterThan(0);
    const history = await api.call("session.history", { ...s, space: "system" });
    // The turn's two messages, after what the engine put at the start of the conversation.
    const at = history.messages.findIndex((m: any) => m.origin === "user");
    expect(at, "the user's message is in the history").toBeGreaterThanOrEqual(0);
    expect(history.messages.slice(0, at).every((m: any) => m.origin === "injected")).toBe(true);
    expect(context.messages.count).toBe(at + 2);
    expect(await api.call("harnix.system.messages.edit", { ...s, at, text: "Say exactly: edited", total: history.total })).toEqual({ edited: at });
    let after = await api.call("session.history", { ...s, space: "system" });
    expect(text(after.messages[at])).toBe("Say exactly: edited");

    // A count that is not the session's any more is a view that went stale.
    await api.refused("harnix.system.messages.drop", { ...s, at: [at], total: history.total + 1 });
    expect(await api.call("harnix.system.messages.drop", { ...s, at: [at], total: history.total })).toEqual({ dropped: [at] });
    after = await api.call("session.history", { ...s, space: "system" });
    expect(text(after.messages[at])).toContain("removed by the host");
    // The model's own message cannot be taken out.
    await api.refused("harnix.system.messages.drop", { ...s, at: [at + 1], total: history.total });
  });

  test("operations on another space's session take effect where it belongs, and are audited", TAGS, async () => {
    const s = await oneTurn(api, "fleet");
    await api.call("harnix.system.rename", { ...s, name: "renamed from system" });
    const row = (await api.call("harnix.sessions", { space: "fleet" })).sessions.find((r: any) => r.session_id === s.session_id);
    expect(row.name).toBe("renamed from system");

    expect(await api.call("harnix.system.tasks.stop", { ...s, task_id: "no-such-task" })).toEqual({ stopped: false });
    const unknown = await api.refused("harnix.system.tasks.output", { ...s, task_id: "no-such-task" });
    expect(unknown?.code, String(unknown)).toBe(-32018);
    await api.call("harnix.system.compact", s).catch(() => undefined);
    await api.call("harnix.system.clear", s);
    const context = await api.call("harnix.system.context", s);
    expect(context.messages.count).toBe(0);

    const { entries } = await api.call("harnix.audit", { limit: 200 });
    for (const method of ["harnix.system.rename", "harnix.system.clear"]) {
      expect(entries.some((e: any) => e.method === method && e.decision === "allow"), method).toBe(true);
    }
    // And what it asked of the engine, as the system module.
    expect(entries.some((e: any) => e.method === "session.clear" && JSON.stringify(e.subject).includes("system"))).toBe(true);
  });

  test("a session's model calls: its envelopes, every call with its tool calls, one in full", TAGS, async () => {
    const s = await oneTurn(api, "sessions");
    // Nothing recorded yet is an answer, not an error.
    expect(await api.call("harnix.system.trace", s)).toEqual({ recording: false, calls: [] });
    expect((await api.call("harnix.system.calls", s)).recording).toBe(false);

    // A replayed session writes no recording of its own: give it the one its fixture was made from,
    // where `system`'s engine keeps them.
    cpSync(join(RECORDINGS, "spawns-agent"), join(backend.scratch, "agix", "recordings", s.session_id), { recursive: true });
    const trace = await api.call("harnix.system.trace", s);
    expect(trace.recording).toBe(true);
    expect(trace.calls.length).toBeGreaterThan(1);
    expect(trace.calls.map((c: any) => c.index)).toEqual(trace.calls.map((_: any, i: number) => i));
    const agent = trace.calls.find((c: any) => c.tools.some((t: any) => t.name === "Agent"));
    expect(agent, "the call that started the sub-agent").toBeTruthy();
    expect(agent.output.tool_uses).toBeGreaterThan(0);

    const full = await api.call("harnix.system.trace.call", { ...s, index: agent.index });
    expect(full.index).toBe(agent.index);
    expect(full.input.messages.length).toBe(agent.input.messages);
    expect(full.input.model).toBe(agent.model);
    expect((await api.refused("harnix.system.trace.call", s))?.code).toBe(INVALID_PARAMS);
    expect(await api.refused("harnix.system.trace.call", { ...s, index: 10_000 })).toBeTruthy();

    const calls = await api.call("harnix.system.calls", s);
    expect(calls.recording).toBe(true);
    expect(calls.headers[0].reason).toBe("initial");
    expect(calls.calls).toBeGreaterThanOrEqual(calls.headers.length);
  });

  test("the context says what the provider counted for the last call, apart from its estimates", TAGS, async () => {
    // Before any call there is nothing measured, only estimates.
    const fresh = await api.call("session.create", { space: "sessions", options: { recorder: { name: "says-ready" } } });
    const before = await api.call("harnix.system.context", { instance: fresh.instance, session_id: fresh.session_id });
    expect(before.measured).toBeUndefined();
    expect(before.total_tokens).toBeGreaterThan(0);

    const s = await oneTurn(api, "sessions");
    const context = await api.call("harnix.system.context", s);
    const info = await api.call("session.get", { ...s, space: "system" });
    expect(info.context?.tokens, "the turn was counted").toBeGreaterThan(0);
    expect(context.measured).toMatchObject({ tokens: info.context.tokens });
    // The estimates stay what they were: per block, for the next call.
    expect(context.system.length).toBeGreaterThan(0);
  });

  test("a fork cut inside a turn keeps exactly the messages asked for", TAGS, async () => {
    const created = await api.call("session.create", { space: "sessions", options: { recorder: { name: "calls-a-tool" } } });
    const s = { instance: created.instance, session_id: created.session_id };
    await api.call("harnix.attach", { ...s, space: "sessions" });
    await api.call("harnix.send", { ...s, space: "sessions", message: "Use the Bash tool to run exactly: echo harnix-fixture" });
    await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id, { timeout: 60_000, describe: "the turn to settle" });
    const history = await api.call("session.history", { ...s, space: "system" });
    // Up to and including the model's tool call, before its result.
    const call = history.messages.findIndex((m: any) => m.role === "assistant" && JSON.stringify(m.content).includes("tool_use"));
    expect(call, "the turn called a tool").toBeGreaterThan(0);
    const forked = await api.call("session.fork", { ...s, at_message: call + 1, space: "system" });
    const kept = await api.call("session.history", { instance: forked.instance ?? s.instance, session_id: forked.session_id, space: "system" });
    expect(kept.messages.slice(0, call + 1).map((m: any) => m.role)).toEqual(history.messages.slice(0, call + 1).map((m: any) => m.role));
    expect(JSON.stringify(kept.messages.at(call)?.content)).toContain("tool_use");
    // What the cut left open is closed, not left dangling: nothing after it is the original's.
    expect(JSON.stringify(kept.messages.slice(call + 1))).not.toContain("harnix-fixture\\n");
  });

  test("each call's time to first token and output rate, and a turn's model and tool time, read off the recording", TAGS, async () => {
    const s = await oneTurn(api, "sessions");
    cpSync(join(RECORDINGS, "calls-a-tool"), join(backend.scratch, "agix", "recordings", s.session_id), { recursive: true });
    // What the recording says, worked out here independently: each call's send, first chunk, last chunk.
    const records = readFileSync(join(RECORDINGS, "calls-a-tool", "calls.jsonl"), "utf8").trim().split("\n").map((l) => JSON.parse(l));
    const sent = records.filter((r) => r.type === "call");
    const times = sent.map((call) => {
      const chunks = records.filter((r) => r.call === call.seq && r.type !== "end");
      const starts = chunks.map((r) => r.ts0 ?? r.ts);
      const ends = chunks.map((r) => (r.ts0 != null ? r.ts0 + (r.dt ?? []).reduce((a: number, b: number) => a + b, 0) : r.ts));
      const end = records.find((r) => r.type === "end" && r.call === call.seq);
      return { sent: call.ts, first: Math.min(...starts), last: Math.max(...ends), duration: end.duration_ms };
    });

    const { calls } = await api.call("harnix.system.trace", s);
    expect(calls).toHaveLength(times.length);
    calls.forEach((call: any, i: number) => {
      expect(call.timing.sent_at, `call ${i}`).toBe(times[i]!.sent);
      expect(call.timing.ttft_ms, `call ${i}`).toBe(times[i]!.first - times[i]!.sent);
      expect(call.timing.generate_ms, `call ${i}`).toBe(times[i]!.last - times[i]!.first);
    });

    const { turns } = await api.call("harnix.system.turns", s);
    expect(turns).toHaveLength(1);
    const [turn] = turns;
    expect(turn.calls).toEqual([0, 1]);
    expect(turn.model_ms).toBe(times[0]!.duration + times[1]!.duration);
    expect(turn.tool_ms).toBe(times[1]!.sent - (times[0]!.sent + times[0]!.duration));
    expect(turn.ttft_ms).toBe(calls[0].timing.ttft_ms);
    const out = calls.reduce((n: number, c: any) => n + (c.usage?.output_tokens ?? 0), 0);
    expect(turn.output_tokens).toBe(out);
    expect(turn.cache_hit === null || (turn.cache_hit >= 0 && turn.cache_hit <= 1)).toBe(true);
    expect((await api.refused("harnix.system.turns", {}))?.code).toBe(INVALID_PARAMS);
  });

  test("a session's model is switched from the system space, and says so", TAGS, async () => {
    const s = await oneTurn(api, "fleet");
    await api.call("harnix.system.model", { ...s, model: "claude-haiku-5-5" });
    // What the fleet module's own session handle reads back: the engine's model for the session.
    expect((await api.call("app.fleet.model", s)).model).toBe("claude-haiku-5-5");
    expect((await api.refused("harnix.system.model", { ...s, session_id: "no-such-session", model: "x" }))).toBeTruthy();
  });

  test("the system space lists every space's sessions and builds from any template", TAGS, async () => {
    const fleet = await oneTurn(api, "fleet");
    const all = (await api.call("harnix.sessions", { space: "system" })).sessions.map((r: any) => r.session_id);
    expect(all).toContain(fleet.session_id);
    const { templates } = await api.call("harnix.instances.templates", { space: "system" });
    const fromSessions = (await api.call("harnix.instances.templates", {})).templates;
    expect(templates.length).toBeGreaterThanOrEqual(fromSessions.length);
  });

  test("a session made in the system space is the system space's", TAGS, async () => {
    const created = await api.call("session.create", { space: "system", options: { recorder: { name: "says-ready" } } });
    expect(created.space).toBe("system");
    const listed = (await api.call("harnix.sessions", { space: "sessions" })).sessions.map((r: any) => r.session_id);
    expect(listed).not.toContain(created.session_id);
  });

  test("an instance restarts from where it was built; `system` does not", TAGS, async () => {
    expect((await api.refused("harnix.instances.restart", { instance: "system", space: "system" }))?.code).toBe(REFUSED);
    // Not the interface's to restart from anywhere but the system space.
    expect((await api.refused("harnix.instances.restart", { instance: "fleet-side" }))?.code).toBe(REFUSED);
    const restarted = await api.call("harnix.instances.restart", { instance: "fleet-side", space: "system" });
    expect(restarted.id).toBe("fleet-side");
    await until(api, "fleet-side", "ready");
    await oneTurn(api, "fleet");
    const { entries } = await api.call("harnix.audit", { limit: 200 });
    expect(entries.some((e: any) => e.method === "harnix.instances.restart" && e.decision === "allow")).toBe(true);
  });
});

test.describe("off", () => {
  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    backend = await startTestApp({ profile: "\n[shell]\nspaces = [\"sessions\", \"projects\"]\n" });
  });
  test.afterAll(async () => { await backend?.dispose(); });
  test.beforeEach(async () => {
    api = await connect(backend);
  });
  test.afterEach(() => api?.close());

  test("the methods leave the authorization table, and the space cannot be claimed", TAGS, async () => {
    const { methods } = await api.call("harnix.reachable");
    for (const method of SYSTEM_METHODS) expect(methods, method).not.toContain(method);
    const created = await api.call("session.create", { space: "sessions" });
    const s = { instance: created.instance, session_id: created.session_id };
    const error = await api.refused("harnix.system.context", s);
    expect([METHOD_NOT_FOUND, REFUSED]).toContain(error?.code);

    expect((await api.refused("harnix.sessions", { space: "system" }))?.code).toBe(SPACE_MISMATCH);
    expect((await api.refused("harnix.attach", { ...s, space: "system" }))?.code).toBe(SPACE_MISMATCH);
    expect((await api.refused("harnix.instances.restart", { instance: "fleet-side", space: "system" }))?.code).toBe(SPACE_MISMATCH);
  });
});
