/**
 * The system space's backend (concept §5.5): any instance's sessions inspected and acted on through
 * `system.*` and the hub's own methods claiming the system space — and none of it there
 * when the profile turns the space off: the methods leave the authorization table, the space
 * stops existing, and nothing can claim it.
 *
 * Runs on `harnix-testapp`, whose `fleet` module builds an instance (`fleet-side`) for a space of
 * its own: the session there is another space's, which is the case the system space is for.
 */

import { cpSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { INVALID_PARAMS, METHOD_NOT_FOUND, NOT_FOUND, REFUSED, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, RECORDINGS, type TestClient, connect, expect, test } from "@harnix/testkit";

import { until } from "./instances";
import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:system", "@layer:api"] };

const SYSTEM_METHODS = [
  "system.context",
  "observability.calls",
  "observability.turns",
  "observability.usage",
  "system.goal",
  "system.goal.set",
  "system.goal.update",
  "system.tasks.stop",
  "system.tasks.message",
  "system.tasks.output",
  "system.messages.drop",
  "system.messages.edit",
  "system.compact",
  "system.clear",
  "system.model",
  "system.rename",
  "system.instances.stop",
  "system.instances.restart",
  "system.instances.create",
  "system.instances.templates",
];

/** A session in `space` that has had one turn, the user's message first. */
async function oneTurn(api: TestClient, space: string): Promise<{ instance: string; session_id: string }> {
  const created = await api.call("harnix.session.create", { space, options: { recorder: { name: "says-ready" } } });
  const s = { instance: created.instance, session_id: created.session_id };
  await api.call("harnix.session.attach", { ...s, space });
  await api.call("harnix.session.send", { ...s, space, message: "Say exactly: fixture ready" });
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
    const context = await api.call("system.context", s);
    expect(context.basis).toBe("next_step_without_new_input");
    expect(context.system.length).toBeGreaterThan(0);
    const history = await api.call("engine.session.history", { ...s, space: "system" });
    // The turn's two messages, after what the engine put at the start of the conversation.
    const at = history.messages.findIndex((m: any) => m.origin === "user");
    expect(at, "the user's message is in the history").toBeGreaterThanOrEqual(0);
    expect(history.messages.slice(0, at).every((m: any) => m.origin === "injected")).toBe(true);
    expect(context.messages.count).toBe(at + 2);
    expect(await api.call("system.messages.edit", { ...s, at, text: "Say exactly: edited", total: history.total })).toEqual({ edited: at });
    let after = await api.call("engine.session.history", { ...s, space: "system" });
    expect(text(after.messages[at])).toBe("Say exactly: edited");

    // A count that is not the session's any more is a view that went stale.
    await api.refused("system.messages.drop", { ...s, at: [at], total: history.total + 1 });
    expect(await api.call("system.messages.drop", { ...s, at: [at], total: history.total })).toEqual({ dropped: [at] });
    after = await api.call("engine.session.history", { ...s, space: "system" });
    expect(text(after.messages[at])).toContain("removed by the host");
    // The model's own message cannot be taken out.
    await api.refused("system.messages.drop", { ...s, at: [at + 1], total: history.total });
  });

  test("operations on another space's session take effect where it belongs, and are audited", TAGS, async () => {
    const s = await oneTurn(api, "fleet");
    await api.call("system.rename", { ...s, name: "renamed from system" });
    const row = (await api.call("harnix.sessions.list", { space: "fleet" })).sessions.find((r: any) => r.session_id === s.session_id);
    expect(row.name).toBe("renamed from system");

    expect(await api.call("system.tasks.stop", { ...s, task_id: "no-such-task" })).toEqual({ stopped: false });
    const unknown = await api.refused("system.tasks.output", { ...s, task_id: "no-such-task" });
    expect(unknown?.code, String(unknown)).toBe(-32018);
    await api.call("system.compact", s);
    await api.call("system.clear", s);
    const context = await api.call("system.context", s);
    expect(context.messages.count).toBe(0);

    const { entries } = await api.call("harnix.audit", { limit: 200 });
    for (const method of ["system.rename", "system.compact", "system.clear"]) {
      expect(entries.some((e: any) => e.method === method && e.decision === "allow"), method).toBe(true);
    }
    // And what it asked of the engine, as the system module.
    expect(entries.some((e: any) => e.method === "engine.session.clear" && JSON.stringify(e.subject).includes("system"))).toBe(true);
  });

  test("observability.usage: what each Agent of a session used this run — the session first, then its sub-agents with their parents", TAGS, async () => {
    const s = await oneTurn(api, "sessions");
    const usage = async () => (await api.call("observability.usage", { instance: s.instance, session_ids: [s.session_id] })).sessions[s.session_id];
    // A replayed session writes no recording of its own.
    expect(await usage()).toEqual({ recording: false, agents: [] });

    // One written by hand: the session's own two turns, a sub-agent's call, and a title call beside them.
    const dir = join(backend.scratch, "agix", "recordings", s.session_id);
    mkdirSync(dir, { recursive: true });
    const lines = [
      { type: "recording", version: 1, session_id: s.session_id },
      { type: "call", seq: 1, ts: 1000, session_id: s.session_id, turn: 0, step: 0, params: { model: "big" } },
      { type: "end", seq: 2, ts: 1400, call: 1, outcome: { status: "ok" }, usage: { input_tokens: 10, cache_read_input_tokens: 90, output_tokens: 5 }, duration_ms: 400 },
      { type: "call", seq: 3, ts: 1500, session_id: "SUB", parent_session_id: s.session_id, turn: 0, step: 0, params: { model: "small" } },
      { type: "end", seq: 4, ts: 1600, call: 3, outcome: { status: "ok" }, usage: { input_tokens: 20, output_tokens: 3 }, duration_ms: 100 },
      { type: "call", seq: 5, ts: 2000, session_id: s.session_id, turn: 1, step: 0, params: { model: "big" } },
      { type: "end", seq: 6, ts: 2300, call: 5, outcome: { status: "error" }, usage: {}, duration_ms: 300 },
      { type: "call", seq: 7, ts: 2400, session_id: s.session_id, turn: 1, purpose: "title", params: { model: "small" } },
    ];
    const write = (records: object[]) => writeFileSync(join(dir, "calls.jsonl"), records.map((r) => JSON.stringify(r)).join("\n") + "\n");
    write(lines);
    const first = await usage();
    expect(first.recording).toBe(true);
    expect(first.agents.map((a: any) => [a.session_id, a.parent_session_id])).toEqual([[s.session_id, null], ["SUB", s.session_id]]);
    expect(first.agents[0]).toMatchObject({
      calls: 2, auxiliary: 1, turns: 2, models: ["big", "small"],
      input_tokens: 10, cache_read_tokens: 90, cache_write_tokens: 0, output_tokens: 5,
      model_ms: 700, errors: 1, first_at: 1000, last_at: 2400,
    });
    expect(first.agents[1]).toMatchObject({ calls: 1, turns: 1, models: ["small"], input_tokens: 20, output_tokens: 3, model_ms: 100, errors: 0 });

    // The recording grew: read again, not served from before.
    write([...lines, { type: "call", seq: 8, ts: 3000, session_id: "SUB", parent_session_id: s.session_id, turn: 1, params: { model: "small" } },
      { type: "end", seq: 9, ts: 3050, call: 8, outcome: { status: "ok" }, usage: { output_tokens: 7 }, duration_ms: 50 }]);
    expect((await usage()).agents[1]).toMatchObject({ calls: 2, output_tokens: 10, model_ms: 150 });

    expect((await api.refused("observability.usage", { instance: s.instance }))?.code).toBe(INVALID_PARAMS);
    const many = Array.from({ length: 201 }, (_, i) => `s${i}`);
    expect((await api.refused("observability.usage", { instance: s.instance, session_ids: many }))?.code).toBe(INVALID_PARAMS);
  });

  test("a session's row says who started it — a person, or Harnix itself — and what it waits on a person for", TAGS, async () => {
    const row = async (instance: string, id: string) =>
      (await api.call("harnix.sessions.list", { instance })).sessions.find((r: any) => r.session_id === id);
    const mine = await oneTurn(api, "sessions");
    expect(await row(mine.instance, mine.session_id)).toMatchObject({ started_by: "user", waiting: null });

    const { started: [own] } = await api.call("fleet.headless", { count: 1, project_root: api.project, recording: "says-ready" });
    expect((await row(own.instance, own.session_id))?.started_by).toBe("harnix");

    // A turn held on its permission ask waits on a person; answered, it does not.
    const created = await api.call("harnix.session.create", { options: { recorder: { name: "asks-permission" } } });
    const s = { instance: created.instance, session_id: created.session_id };
    await api.call("harnix.session.attach", s);
    const sent = await api.call("harnix.session.send", { ...s, message: "Use the Bash tool to run exactly: touch harnix-permission-fixture" });
    const ask = await api.waitFor(
      (f) => f.method === "harnix.event" && f.params.session_id === s.session_id && f.params.event.kind === "prompt",
      { timeout: 30_000, describe: "the permission ask" },
    );
    await expect.poll(async () => (await row(s.instance, s.session_id))?.waiting, { timeout: 10_000 }).toBe("approval");
    await api.call("harnix.session.respond", { ...s, prompt_id: ask.params.event.prompt_id, decision: { type: "deny", reason: "not in this test" } });
    await expect.poll(async () => (await row(s.instance, s.session_id))?.waiting ?? null, { timeout: 10_000 }).toBeNull();
    await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.turn_id === sent.turn_id, { timeout: 60_000, describe: "the turn to settle" });
  });

  test("system.process counts the operating system's threads in the process, and is pushed while watched", TAGS, async () => {
    const process = await api.call("system.process", {});
    expect(process.threads).toBeGreaterThan(1);
    const first = await api.call("harnix.topics.subscribe", { topic: "system.process" });
    expect(first.value.threads).toBeGreaterThan(1);
    // Nothing asked again: the hub pushes it on its own clock.
    const pushed = await api.waitFor((f) => f.method === "harnix.topic" && f.params.topic === "system.process", {
      timeout: 15_000,
      describe: "system.process to be pushed",
    });
    expect(pushed.params.value.uptime_secs).toBeGreaterThanOrEqual(first.value.uptime_secs);
    await api.call("harnix.topics.unsubscribe", { topic: "system.process" });
  });

  test("a session's model calls: its envelopes, every call with its tool calls, one in full", TAGS, async () => {
    const s = await oneTurn(api, "sessions");
    // Nothing recorded yet is an answer, not an error.
    expect(await api.call("observability.trace", s)).toEqual({ recording: false, calls: [] });
    expect((await api.call("observability.calls", s)).recording).toBe(false);

    // A replayed session writes no recording of its own: give it the one its fixture was made from,
    // where `main`'s engine keeps them.
    cpSync(join(RECORDINGS, "spawns-agent"), join(backend.scratch, "agix", "recordings", s.session_id), { recursive: true });
    const trace = await api.call("observability.trace", s);
    expect(trace.recording).toBe(true);
    expect(trace.calls.length).toBeGreaterThan(1);
    expect(trace.calls.map((c: any) => c.index)).toEqual(trace.calls.map((_: any, i: number) => i));
    const agent = trace.calls.find((c: any) => c.tools.some((t: any) => t.name === "Agent"));
    expect(agent, "the call that started the sub-agent").toBeTruthy();
    expect(agent.output.tool_uses).toBeGreaterThan(0);

    const full = await api.call("observability.trace.call", { ...s, index: agent.index });
    expect(full.index).toBe(agent.index);
    expect(full.input.messages.length).toBe(agent.input.messages);
    expect(full.input.model).toBe(agent.model);
    expect((await api.refused("observability.trace.call", s))?.code).toBe(INVALID_PARAMS);
    // A thing that is not there is NOT_FOUND, not "there is no such method".
    expect((await api.refused("observability.trace.call", { ...s, index: 10_000 }))?.code).toBe(NOT_FOUND);
    const fresh = await api.call("harnix.session.create", { space: "sessions" });
    const unrecorded = { instance: fresh.instance, session_id: fresh.session_id };
    expect((await api.refused("observability.trace.call", { ...unrecorded, index: 0 }))?.code, "no recording yet").toBe(NOT_FOUND);
    expect((await api.call("observability.trace", unrecorded)).recording).toBe(false);

    const calls = await api.call("observability.calls", s);
    expect(calls.recording).toBe(true);
    expect(calls.headers[0].reason).toBe("initial");
    expect(calls.calls).toBeGreaterThanOrEqual(calls.headers.length);
  });

  test("the context says what the provider counted for the last call, apart from its estimates", TAGS, async () => {
    // Before any call there is nothing measured, only estimates.
    const fresh = await api.call("harnix.session.create", { space: "sessions", options: { recorder: { name: "says-ready" } } });
    const before = await api.call("system.context", { instance: fresh.instance, session_id: fresh.session_id });
    expect(before.measured).toBeUndefined();
    expect(before.total_tokens).toBeGreaterThan(0);

    const s = await oneTurn(api, "sessions");
    const context = await api.call("system.context", s);
    const info = await api.call("engine.session.get", { ...s, space: "system" });
    expect(info.context?.tokens, "the turn was counted").toBeGreaterThan(0);
    expect(context.measured).toMatchObject({ tokens: info.context.tokens });
    // The estimates stay what they were: per block, for the next call.
    expect(context.system.length).toBeGreaterThan(0);
  });

  test("a fork cut inside a turn keeps exactly the messages asked for", TAGS, async () => {
    const created = await api.call("harnix.session.create", { space: "sessions", options: { recorder: { name: "calls-a-tool" } } });
    const s = { instance: created.instance, session_id: created.session_id };
    await api.call("harnix.session.attach", { ...s, space: "sessions" });
    await api.call("harnix.session.send", { ...s, space: "sessions", message: "Use the Bash tool to run exactly: echo harnix-fixture" });
    await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id, { timeout: 60_000, describe: "the turn to settle" });
    const history = await api.call("engine.session.history", { ...s, space: "system" });
    // Up to and including the model's tool call, before its result.
    const call = history.messages.findIndex((m: any) => m.role === "assistant" && JSON.stringify(m.content).includes("tool_use"));
    expect(call, "the turn called a tool").toBeGreaterThan(0);
    const forked = await api.call("harnix.session.fork", { ...s, at_message: call + 1, space: "system" });
    const kept = await api.call("engine.session.history", { instance: forked.instance ?? s.instance, session_id: forked.session_id, space: "system" });
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

    const { calls } = await api.call("observability.trace", s);
    expect(calls).toHaveLength(times.length);
    calls.forEach((call: any, i: number) => {
      expect(call.timing.sent_at, `call ${i}`).toBe(times[i]!.sent);
      expect(call.timing.ttft_ms, `call ${i}`).toBe(times[i]!.first - times[i]!.sent);
      expect(call.timing.generate_ms, `call ${i}`).toBe(times[i]!.last - times[i]!.first);
    });

    const { turns } = await api.call("observability.turns", s);
    expect(turns).toHaveLength(1);
    const [turn] = turns;
    expect(turn.calls).toEqual([0, 1]);
    expect(turn.model_ms).toBe(times[0]!.duration + times[1]!.duration);
    expect(turn.tool_ms).toBe(times[1]!.sent - (times[0]!.sent + times[0]!.duration));
    expect(turn.ttft_ms).toBe(calls[0].timing.ttft_ms);
    // The turn's own calls: a recording also holds the calls beside it (its title, the memory extraction).
    const out = turn.calls.reduce((n: number, i: number) => n + (calls[i].usage?.output_tokens ?? 0), 0);
    expect(turn.output_tokens).toBe(out);
    expect(turn.cache_hit === null || (turn.cache_hit >= 0 && turn.cache_hit <= 1)).toBe(true);
    expect((await api.refused("observability.turns", {}))?.code).toBe(INVALID_PARAMS);
  });

  test("a session exported as markdown to read, as a bundle or the engine's archive to keep, the recording only when asked", TAGS, async () => {
    const s = await oneTurn(api, "sessions");
    const own = { ...s, space: "sessions" };
    const md = await api.call("harnix-app.sessions.export", { ...own, format: "markdown" });
    expect(md.format).toBe("markdown");
    expect(md.file).toMatch(/\.md$/);
    expect(md.content).toContain("**User**\n\nSay exactly: fixture ready");
    expect(md.content).toMatch(/\*\*Assistant\*\*\n\n[\s\S]*fixture ready/i);

    const history = await api.call("engine.session.history", { ...s, space: "system" });
    const plain = await api.call("harnix-app.sessions.export", { ...own, format: "bundle" });
    expect(plain.file).toMatch(/\.json$/);
    expect(plain.content).toMatchObject({ kind: "harnix-session", version: 1 });
    expect(plain.content.messages).toHaveLength(history.total);
    expect(plain.content.recording, "no recording unless asked").toBeUndefined();

    // A recording comes with the engine's archive; the bundle is Harnix's own and has none.
    expect((await api.refused("harnix-app.sessions.export", { ...own, format: "bundle", recording: true }))?.code).toBe(REFUSED);

    // As the engine packs it (Agix #37): a zip, its manifest and the session's own files in it.
    const archive = await api.call("harnix-app.sessions.export", { ...own, format: "archive" });
    expect(archive).toMatchObject({ format: "archive", encoding: "base64", recording: false });
    expect(archive.file).toMatch(/\.zip$/);
    const zip = Buffer.from(archive.content, "base64");
    expect(zip.length).toBe(archive.bytes);
    expect(zip.subarray(0, 4).toString("binary")).toBe("PK\x03\x04");
    expect(zip.includes("manifest.json")).toBe(true);
    expect(zip.includes(`sessions/${s.session_id}/history.json`)).toBe(true);
    expect(archive.sessions).toBeGreaterThanOrEqual(1);

    // A session of another space is not this caller's to export; nor a format that is not one.
    expect((await api.refused("harnix-app.sessions.export", { ...s, space: "fleet", format: "markdown" }))?.code).toBe(SPACE_MISMATCH);
    expect((await api.refused("harnix-app.sessions.export", { ...own, format: "pdf" }))?.code).toBe(INVALID_PARAMS);
    const audit = await api.call("harnix.audit", { limit: 50 });
    expect(audit.entries.some((e: any) => e.method === "harnix-app.sessions.export")).toBe(true);
  });

  test("a session's model is switched from the system space, and says so", TAGS, async () => {
    const s = await oneTurn(api, "fleet");
    await api.call("system.model", { ...s, model: "claude-haiku-5-5" });
    // What the fleet module's own session handle reads back: the engine's model for the session.
    expect((await api.call("fleet.model", s)).model).toBe("claude-haiku-5-5");
    expect((await api.refused("system.model", { ...s, session_id: "no-such-session", model: "x" }))).toBeTruthy();
  });

  test("the system space lists every space's sessions and builds from any template", TAGS, async () => {
    const fleet = await oneTurn(api, "fleet");
    const all = (await api.call("harnix.sessions.list", { space: "system" })).sessions.map((r: any) => r.session_id);
    expect(all).toContain(fleet.session_id);
    // Any template, through the system module, which holds `instances.admin`; a device by itself
    // only the interface's own (`ui`), whatever space it claims.
    const { templates } = await api.call("system.instances.templates", { space: "system" });
    const fromSessions = (await api.call("harnix.instances.templates", {})).templates;
    expect(templates.length).toBeGreaterThanOrEqual(fromSessions.length);
    expect((await api.call("harnix.instances.templates", { space: "system" })).templates).toEqual(fromSessions);

    // One the interface may not build from: the system module builds it, and stops it again.
    expect((await api.refused("harnix.instances.create", { template: "fleet.side", name: "from-system", space: "system" }))?.code).toBe(REFUSED);
    const built = await api.call("system.instances.create", { template: "fleet.side", name: "from-system", space: "system" });
    expect(built).toMatchObject({ id: "from-system", template: "fleet.side" });
    await until(api, "from-system", "ready");
    await api.call("system.instances.stop", { instance: "from-system", space: "system" });
    const { instances } = await api.call("harnix.instances.list");
    expect(instances.map((i: any) => i.id)).not.toContain("from-system");
  });

  test("a session made in the system space is the system space's", TAGS, async () => {
    const created = await api.call("harnix.session.create", { space: "system", options: { recorder: { name: "says-ready" } } });
    expect(created.space).toBe("system");
    const listed = (await api.call("harnix.sessions.list", { space: "sessions" })).sessions.map((r: any) => r.session_id);
    expect(listed).not.toContain(created.session_id);
  });

  test("an instance restarts from where it was built; `main` does not", TAGS, async () => {
    expect((await api.refused("system.instances.restart", { instance: "main", space: "system" }))?.code).toBe(REFUSED);
    // Not the interface's to restart, from any space: a device holds no capability. The system
    // module, holding `instances.admin`, restarts it for the system space.
    expect((await api.refused("harnix.instances.restart", { instance: "fleet-side" }))?.code).toBe(REFUSED);
    expect((await api.refused("harnix.instances.restart", { instance: "fleet-side", space: "system" }))?.code).toBe(REFUSED);
    const restarted = await api.call("system.instances.restart", { instance: "fleet-side", space: "system" });
    expect(restarted.id).toBe("fleet-side");
    await until(api, "fleet-side", "ready");
    await oneTurn(api, "fleet");
    const { entries } = await api.call("harnix.audit", { limit: 200 });
    // On the record as the system module's, made while answering the device's call.
    const made = entries.find((e: any) => e.method === "harnix.instances.restart" && e.decision === "allow");
    expect(made).toMatchObject({ subject: "ext:system" });
    expect(made.parent).toBe(entries.find((e: any) => e.method === "system.instances.restart" && e.decision === "allow")?.call_id);
  });
});

test.describe("off", () => {
  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    backend = await startTestApp({ profile: "\n[extensions]\noff = [\"system\"]\n" });
  });
  test.afterAll(async () => { await backend?.dispose(); });
  test.beforeEach(async () => {
    api = await connect(backend);
  });
  test.afterEach(() => api?.close());

  test("the methods leave the authorization table, and the space cannot be claimed", TAGS, async () => {
    const { methods } = await api.call("harnix.reachable");
    // The inspector's tabs are what the recordings show, which outlive the system space.
    const inspector = ["observability.calls", "observability.trace", "observability.trace.call", "observability.turns", "observability.usage"];
    for (const method of SYSTEM_METHODS.filter((m) => !inspector.includes(m))) expect(methods, method).not.toContain(method);
    const created = await api.call("harnix.session.create", { space: "sessions" });
    const s = { instance: created.instance, session_id: created.session_id };
    const error = await api.refused("system.context", s);
    expect([METHOD_NOT_FOUND, REFUSED]).toContain(error?.code);

    expect((await api.refused("harnix.sessions.list", { space: "system" }))?.code).toBe(SPACE_MISMATCH);
    expect((await api.refused("harnix.session.attach", { ...s, space: "system" }))?.code).toBe(SPACE_MISMATCH);
    expect((await api.refused("harnix.instances.restart", { instance: "fleet-side", space: "system" }))?.code).toBe(SPACE_MISMATCH);
  });
});
