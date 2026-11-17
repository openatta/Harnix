/**
 * What `observability` reads of a session and of this process (harnix-app.md §6.1, §10): a
 * session's context and its tasks' output, asked of the engine with the extension's own
 * `engine.inspect`, for the sessions of the space the caller claims and no other; its model calls,
 * turns and usage read back out of the engine's recording; this process's figures, a method and a
 * topic. Beside them, what a session's row says and what it is exported and imported as.
 *
 * Runs on `harnix-testapp`, whose `fleet` module builds an instance (`fleet-side`) for a space of
 * its own: a session there is another space's than the session space's. That backend replays in
 * every run, a live one too, and a replayed session records nothing: what reads a recording is
 * tested on a second backend whose sessions record (`recorder` below).
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { INVALID_PARAMS, NOT_FOUND, REFUSED, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, launchFor, modelMode, sleep, startHarnix, test, topology } from "@harnix/testkit";

import { startTestApp } from "./testapp";

/** A session's inside, as the right column and the trajectory read it. */
const TAGS = { tag: ["@feature:session-space", "@layer:api"] };
/** What a session's row says. */
const SESSIONS = { tag: ["@feature:sessions", "@layer:api"] };
/** This process's figures, as the home page shows them. */
const SHELL = { tag: ["@feature:shell", "@layer:api"] };

const METHODS = [
  "observability.context",
  "observability.tasks.output",
  "observability.calls",
  "observability.trace",
  "observability.trace.call",
  "observability.turns",
  "observability.usage",
  "observability.usage.daily",
  "observability.process",
  "observability.extensions.log",
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

/**
 * A session on `main` of `on` whose recording is `lines(id)`, put where `main`'s engine records it.
 * It runs no turn, so nothing writes over it (a recording is rewritten by the session's first call).
 */
async function planted(client: TestClient, on: Backend, lines: (id: string) => object[]): Promise<{ instance: string; session_id: string }> {
  const created = await client.call("harnix.session.create", { instance: "main", space: "sessions", scene: "harnix" });
  const s = { instance: created.instance, session_id: created.session_id };
  await client.call("harnix.session.attach", { ...s, space: "sessions" });
  const dir = join(on.scratch, "engine", "recordings", s.session_id);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "calls.jsonl"), lines(s.session_id).map((l) => JSON.stringify(l)).join("\n") + "\n");
  return s;
}

/** A model call as the engine's reader reads one: what it was sent is by reference. */
const modelCall = (seq: number, ts: number, session: string, model: string, parent?: string) => ({
  type: "call", seq, ts, session_id: session, ...(parent ? { parent_session_id: parent } : {}), turn: 0, step: 0,
  provider: "anthropic", api_type: "anthropic", params: { model, max_tokens: 2000, thinking_mode: "off" },
  system: [], tools: "0000000000000000", messages: [],
});

const text = (message: any): string =>
  typeof message.content === "string"
    ? message.content
    : message.content.map((b: any) => (b.type === "text" ? b.text : "")).join("");

test.describe("on", () => {
  let backend: Backend;
  let api: TestClient;
  /**
   * Where what reads a recording is tested: its sessions record. In a live run they call the model;
   * in any other nothing on it runs a turn (`launchFor("none")`), and a recording is `planted`.
   */
  let recorder: Backend;
  const live = modelMode() === "live";
  /** A client of `recorder` for `body`, closed after it. */
  const onRecorder = async (body: (rec: TestClient) => Promise<void>) => {
    const rec = await connect(recorder);
    try {
      await body(rec);
    } finally {
      rec.close();
    }
  };

  test.describe.configure({ mode: "serial" });
  test.beforeAll(async () => {
    backend = await startTestApp();
    recorder = await startHarnix(launchFor(live ? "live" : "none"));
  });
  test.afterAll(async () => {
    await backend?.dispose();
    await recorder?.dispose();
  });
  test.beforeEach(async () => {
    api = await connect(backend);
  });
  test.afterEach(() => api?.close());

  test("the methods are offered, and nothing of a system space is", TAGS, async () => {
    const { methods, topics } = await api.call("harnix.reachable");
    expect(methods).toEqual(expect.arrayContaining(METHODS));
    expect(topics).toContain("observability.process");
    expect(methods.filter((m: string) => m.startsWith("system.")), "no system.* method").toEqual([]);
    expect(topics.filter((t: string) => t.startsWith("system.")), "no system.* topic").toEqual([]);
    // Each extension's log is read through observability, never from the kernel by a device.
    expect(methods).not.toContain("harnix.extensions.log");
  });

  test("a session's context is read in its own space, and refused claiming another, naming its own", TAGS, async () => {
    const s = await oneTurn(api, "fleet");
    const context = await api.call("observability.context", { ...s, space: "fleet" });
    expect(context.basis).toBe("next_step_without_new_input");
    expect(context.system.length).toBeGreaterThan(0);
    const history = await api.call("engine.session.history", { ...s, space: "fleet" });
    // The turn's two messages, after what the engine put at the start of the conversation.
    const at = history.messages.findIndex((m: any) => m.origin === "user");
    expect(at, "the user's message is in the history").toBeGreaterThanOrEqual(0);
    expect(history.messages.slice(0, at).every((m: any) => m.origin === "injected")).toBe(true);
    expect(context.messages.count).toBe(at + 2);

    // Claiming another space, it is not the caller's: the hub says whose it is.
    const refused = await api.refused("observability.context", { ...s, space: "sessions" });
    expect(refused?.code).toBe(SPACE_MISMATCH);
    expect(refused?.data?.space).toBe("fleet");
    expect((await api.refused("observability.context", { instance: s.instance, space: "fleet" }))?.code).toBe(INVALID_PARAMS);

    // On the record: the device's call, and under it what observability asked the engine as itself.
    // The refused reads stop at the session's check and ask the engine nothing: the read that
    // answered is the one with the engine's inspection under it.
    const { entries } = await api.call("harnix.audit", { limit: 200 });
    const asked = entries.filter((e: any) => e.method === "observability.context" && e.decision === "allow");
    expect(asked.length, JSON.stringify(entries.slice(0, 5))).toBeGreaterThan(0);
    const inspected = entries.filter((e: any) => e.method === "engine.session.inspectContext" && asked.some((a: any) => a.call_id === e.parent));
    expect(inspected).toHaveLength(1);
    expect(inspected[0]).toMatchObject({ subject: "ext:observability", decision: "allow" });
  });

  test("a session's task output is read in its own space, and refused claiming another", TAGS, async () => {
    const s = await oneTurn(api, "fleet");
    // A task the session does not have is the engine's to say, in its own space.
    const unknown = await api.refused("observability.tasks.output", { ...s, space: "fleet", task_id: "no-such-task" });
    expect(unknown?.code, String(unknown)).toBe(-32018);
    // Claiming another space, the hub refuses it before the engine is asked.
    const across = await api.refused("observability.tasks.output", { ...s, space: "sessions", task_id: "no-such-task" });
    expect(across?.code).toBe(SPACE_MISMATCH);
    expect(across?.data?.space).toBe("fleet");
    expect((await api.refused("observability.tasks.output", { instance: s.instance, space: "fleet", task_id: "x" }))?.code).toBe(INVALID_PARAMS);
  });

  test("observability.usage: what each Agent of a session used this run — the session first, then its sub-agents with their parents", TAGS, async () => {
    const usage = async (client: TestClient, s: { instance: string; session_id: string }) =>
      (await client.call("observability.usage", { instance: s.instance, session_ids: [s.session_id] })).sessions[s.session_id];
    const s = await oneTurn(api, "sessions");
    // A replayed session writes no recording of its own.
    expect(await usage(api, s)).toMatchObject({ recording: false, agents: [] });

    await onRecorder(async (rec) => {
      // A recording read through the engine: the session's own figures first, then a sub-agent's
      // under it, and which run they cover. The rest of the folding (errors, auxiliary calls,
      // times) is a unit test of the reader.
      const p = await planted(rec, recorder, (id) => [
        { type: "recording", version: 1, name: id, session_id: id, created_at: 1000, engine_version: "test" },
        modelCall(1, 1000, id, "big"),
        { type: "end", seq: 2, ts: 1900, call: 1, outcome: { status: "ok" }, usage: { input_tokens: 600, cache_read_input_tokens: 400, output_tokens: 34 }, duration_ms: 900 },
        modelCall(3, 2000, "SUB-1", "small", id),
        { type: "end", seq: 4, ts: 2600, call: 3, outcome: { status: "ok" }, usage: { input_tokens: 150, output_tokens: 50 }, duration_ms: 600 },
      ]);
      const read = await usage(rec, p);
      expect(read.recording).toBe(true);
      expect(read.run.created_at).toBeTruthy();
      expect(read.agents.map((a: any) => [a.session_id, a.parent_session_id])).toEqual([[p.session_id, null], ["SUB-1", p.session_id]]);
      expect(read.agents[0]).toMatchObject({ calls: 1, models: ["big"], input_tokens: 600, cache_read_tokens: 400, output_tokens: 34, model_ms: 900 });
      expect(read.agents[1]).toMatchObject({ calls: 1, models: ["small"], input_tokens: 150, output_tokens: 50, model_ms: 600 });
      if (!live) return;

      // One the engine recorded as the model answered.
      const r = await oneTurn(rec, "sessions");
      const real = await usage(rec, r);
      expect(real.recording).toBe(true);
      expect(real.agents[0]).toMatchObject({ session_id: r.session_id, parent_session_id: null });
      expect(real.agents[0].calls).toBeGreaterThan(0);
      expect(real.agents[0].output_tokens).toBeGreaterThan(0);
    });

    expect((await api.refused("observability.usage", { instance: s.instance }))?.code).toBe(INVALID_PARAMS);
    const many = Array.from({ length: 201 }, (_, i) => `s${i}`);
    expect((await api.refused("observability.usage", { instance: s.instance, session_ids: many }))?.code).toBe(INVALID_PARAMS);
  });

  test("observability.usage.daily: a session's tree per day and model, the days of the window only, at the caller's offset", TAGS, async () => {
    const daily = async (client: TestClient, s: { instance: string; session_id: string }, extra: object = {}) =>
      await client.call("observability.usage.daily", { instance: s.instance, session_ids: [s.session_id], ...extra });
    // A replayed session records nothing: no days, and no error.
    const s = await oneTurn(api, "fleet");
    expect((await daily(api, s)).sessions[s.session_id]).toMatchObject({ recording: false, days: [] });

    await onRecorder(async (rec) => {
      const DAY = 86_400_000;
      const at = Date.now();
      const dayOf = (ms: number) => new Date(ms).toISOString().slice(0, 10);
      const p = await planted(rec, recorder, (id) => [
        { type: "recording", version: 1, name: id, session_id: id, created_at: at - 40 * DAY, engine_version: "test" },
        modelCall(1, at - 40 * DAY, id, "big"),
        { type: "end", seq: 2, ts: at - 40 * DAY + 900, call: 1, outcome: { status: "ok" }, usage: { input_tokens: 9999, output_tokens: 1 }, duration_ms: 900 },
        modelCall(3, at - 2 * DAY, id, "big"),
        { type: "end", seq: 4, ts: at - 2 * DAY + 900, call: 3, outcome: { status: "ok" }, usage: { input_tokens: 600, cache_read_input_tokens: 400, output_tokens: 34 }, duration_ms: 900 },
        modelCall(5, at - 2 * DAY + 1000, "SUB-1", "small", id),
        { type: "end", seq: 6, ts: at - 2 * DAY + 1600, call: 5, outcome: { status: "ok" }, usage: { input_tokens: 150, output_tokens: 50 }, duration_ms: 600 },
      ]);
      const read = await daily(rec, p, { days: 30, tz_offset_min: 0 });
      expect(read).toMatchObject({ instance: "main", days: 30 });
      expect(dayOf(read.since_ms)).toBe(dayOf(at - 29 * DAY));
      const mine = read.sessions[p.session_id];
      expect(mine.recording).toBe(true);
      expect(mine.run.created_at).toBeTruthy();
      // Forty days ago is outside the window; the sub-agent's call is under the session.
      expect(mine.days.map((d: any) => [d.day, d.model, d.calls])).toEqual([
        [dayOf(at - 2 * DAY), "big", 1],
        [dayOf(at - 2 * DAY), "small", 1],
      ]);
      expect(mine.days[0]).toMatchObject({ input_tokens: 600, cache_read_tokens: 400, output_tokens: 34, model_ms: 900 });
      // One day, today: the recording has nothing in it.
      expect((await daily(rec, p, { days: 1 })).sessions[p.session_id].days).toEqual([]);
      if (!live) return;

      // One the engine recorded as the model answered: today.
      const r = await oneTurn(rec, "sessions");
      const real = (await daily(rec, r, { tz_offset_min: 0 })).sessions[r.session_id];
      expect(real.recording).toBe(true);
      expect(real.days.map((d: any) => d.day)).toContain(dayOf(Date.now()));
    });

    for (const bad of [{}, { session_ids: Array.from({ length: 201 }, (_, i) => `s${i}`) }, { session_ids: [], days: 0 }, { session_ids: [], days: 400 }, { session_ids: [], tz_offset_min: 2000 }]) {
      expect((await api.refused("observability.usage.daily", { instance: s.instance, ...bad }))?.code, JSON.stringify(bad)).toBe(INVALID_PARAMS);
    }
  });

  test("a session's row says who started it — a person, or Harnix itself — and what it waits on a person for", SESSIONS, async () => {
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

  test("observability.process counts the operating system's threads in the process, and is pushed while watched", SHELL, async () => {
    const process = await api.call("observability.process", {});
    expect(process.threads).toBeGreaterThan(1);
    const first = await api.call("harnix.topics.subscribe", { topic: "observability.process" });
    expect(first.value.threads).toBeGreaterThan(1);
    // Nothing asked again: the hub pushes it on its own clock.
    const pushed = await api.waitFor((f) => f.method === "harnix.topic" && f.params.topic === "observability.process", {
      timeout: 15_000,
      describe: "observability.process to be pushed",
    });
    expect(pushed.params.value.uptime_secs).toBeGreaterThanOrEqual(first.value.uptime_secs);
    await api.call("harnix.topics.unsubscribe", { topic: "observability.process" });
  });

  test("a session's model calls: its envelopes, every call with its tool calls, one in full", TAGS, async () => {
    const s = await oneTurn(api, "sessions");
    const fresh = await api.call("harnix.session.create", { space: "sessions" });
    const unrecorded = { instance: fresh.instance, session_id: fresh.session_id };
    // Nothing recorded yet is an answer, not an error.
    expect(await api.call("observability.trace", unrecorded)).toMatchObject({ recording: false, calls: [], run: null });
    expect((await api.call("observability.calls", unrecorded)).recording).toBe(false);
    expect((await api.refused("observability.trace.call", s))?.code).toBe(INVALID_PARAMS);
    expect((await api.refused("observability.trace.call", { ...unrecorded, index: 0 }))?.code, "no recording yet").toBe(NOT_FOUND);

    // A replayed session writes no recording of its own; a live one does, through the engine, and
    // the reader reads it through the engine's protocol.
    expect((await api.call("observability.trace", s)).recording).toBe(false);
    if (!live) return;
    await onRecorder(async (rec) => {
      const s = await oneTurn(rec, "sessions");
      const trace = await rec.call("observability.trace", s);
      expect(trace.recording).toBe(true);
      expect(trace.calls.length).toBeGreaterThan(0);
      expect(trace.run.created_at).toBeTruthy();
      expect(trace.calls.every((c: any) => c.session_id === s.session_id)).toBe(true);
      // The index is the call's place in the recording of the whole tree: it rises, with gaps where a sub-agent called.
      const indexes = trace.calls.map((c: any) => c.index);
      expect(indexes).toEqual([...indexes].sort((a: number, b: number) => a - b));
      expect(new Set(indexes).size).toBe(indexes.length);
      const first = trace.calls[0];
      const full = await rec.call("observability.trace.call", { ...s, index: first.index });
      expect(full.index).toBe(first.index);
      expect(full.input.messages.length).toBe(first.input.messages);
      expect(full.input.model).toBe(first.model);
      expect(full.input.session_id).toBe(s.session_id);
      // A thing that is not there is NOT_FOUND, not "there is no such method".
      expect((await rec.refused("observability.trace.call", { ...s, index: 10_000 }))?.code).toBe(NOT_FOUND);

      const calls = await rec.call("observability.calls", s);
      expect(calls.recording).toBe(true);
      expect(calls.headers[0].reason).toBe("initial");
      expect(calls.calls).toBeGreaterThanOrEqual(calls.headers.length);
    });
  });

  test("the context says what the provider counted for the last call, apart from its estimates", TAGS, async () => {
    // Before any call there is nothing measured, only estimates.
    const fresh = await api.call("harnix.session.create", { space: "sessions", options: { recorder: { name: "says-ready" } } });
    const before = await api.call("observability.context", { instance: fresh.instance, session_id: fresh.session_id });
    expect(before.measured).toBeUndefined();
    expect(before.total_tokens).toBeGreaterThan(0);

    const s = await oneTurn(api, "sessions");
    const context = await api.call("observability.context", s);
    const info = await api.call("engine.session.get", { ...s, space: "sessions" });
    expect(info.context?.tokens, "the turn was counted").toBeGreaterThan(0);
    expect(context.measured).toMatchObject({ tokens: info.context.tokens });
    // The estimates stay what they were: per block, for the next call.
    expect(context.system.length).toBeGreaterThan(0);
  });

  test("a fork cut inside a turn keeps exactly the messages asked for", SESSIONS, async () => {
    const created = await api.call("harnix.session.create", { space: "sessions", options: { recorder: { name: "calls-a-tool" } } });
    const s = { instance: created.instance, session_id: created.session_id };
    await api.call("harnix.session.attach", { ...s, space: "sessions" });
    await api.call("harnix.session.send", { ...s, space: "sessions", message: "Use the Bash tool to run exactly: echo harnix-fixture" });
    await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id, { timeout: 60_000, describe: "the turn to settle" });
    const history = await api.call("engine.session.history", { ...s, space: "sessions" });
    // Up to and including the model's tool call, before its result.
    const call = history.messages.findIndex((m: any) => m.role === "assistant" && JSON.stringify(m.content).includes("tool_use"));
    expect(call, "the turn called a tool").toBeGreaterThan(0);
    const forked = await api.call("harnix.session.fork", { ...s, at_message: call + 1, space: "sessions" });
    const kept = await api.call("engine.session.history", { instance: forked.instance ?? s.instance, session_id: forked.session_id, space: "sessions" });
    expect(kept.messages.slice(0, call + 1).map((m: any) => m.role)).toEqual(history.messages.slice(0, call + 1).map((m: any) => m.role));
    expect(JSON.stringify(kept.messages.at(call)?.content)).toContain("tool_use");
    // What the cut left open is closed, not left dangling: nothing after it is the original's.
    expect(JSON.stringify(kept.messages.slice(call + 1))).not.toContain("harnix-fixture\\n");
  });

  test("each call's time to first token and output rate, and a turn's model and tool time, read off the recording", TAGS, async () => {
    const s = await oneTurn(api, "sessions");
    // The sums and times are worked out from the recording's rows by the reader's unit tests; here
    // only that a recorded run answers in that shape. A replayed session has no recording.
    expect((await api.call("observability.trace", s)).recording).toBe(false);
    expect((await api.call("observability.turns", s)).turns).toEqual([]);
    if (live) {
      await onRecorder(async (rec) => {
        const r = await oneTurn(rec, "sessions");
        const { calls } = await rec.call("observability.trace", r);
        const { turns } = await rec.call("observability.turns", r);
        expect(calls.length).toBeGreaterThan(0);
        for (const call of calls) {
          expect(call.timing.sent_at).toBeGreaterThan(0);
          // A call still streaming — one beside the turn, a title say — has no duration yet.
          if (call.duration_ms == null) {
            expect(call.status).toBe("in_flight");
            continue;
          }
          if (call.timing.generate_ms != null) expect(call.timing.generate_ms).toBeLessThanOrEqual(call.duration_ms);
          if (call.timing.ttft_ms != null) expect(call.timing.ttft_ms).toBeLessThanOrEqual(call.duration_ms);
        }
        expect(turns.length).toBeGreaterThan(0);
        expect(turns[0].model_ms).toBeGreaterThan(0);
      });
    }
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

    const history = await api.call("engine.session.history", own);
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

  test("an exported archive is imported into the session space: listed, its history the same, a bundle that is not one refused", { tag: ["@feature:session-space", "@layer:api"] }, async () => {
    const s = await oneTurn(api, "sessions");
    const own = { ...s, space: "sessions" };
    const archive = await api.call("harnix-app.sessions.export", { ...own, format: "archive" });
    const zip = Buffer.from(archive.content, "base64");
    const send = async (name: string, body: Buffer) => {
      const grant = await api.call("harnix-app.sessions.importUpload", { name, space: "sessions" });
      // A Node Buffer is not a `BodyInit` to the DOM's fetch types; its bytes, copied, are.
      const sent = await fetch(`${backend.url.replace(/\/$/, "")}${grant.url}`, { method: "POST", body: new Uint8Array(body) });
      expect(sent.ok).toBe(true);
      return grant.token as string;
    };

    const token = await send(archive.file, zip);
    const done = await api.call("harnix-app.sessions.import", { token, space: "sessions" });
    expect(done.root_session_id).toBeTruthy();
    expect(done.root_session_id, "a new session, not the original").not.toBe(s.session_id);
    expect(done.space).toBe("sessions");

    const listed = await api.call("harnix.sessions.list", { space: "sessions" });
    expect(listed.sessions.some((r: any) => r.session_id === done.root_session_id)).toBe(true);
    const imported = { instance: done.instance, session_id: done.root_session_id, space: "sessions" };
    const was = await api.call("engine.session.history", own);
    const now = await api.call("engine.session.history", imported);
    expect(now.total).toBe(was.total);
    expect(now.messages.map((m: any) => m.role)).toEqual(was.messages.map((m: any) => m.role));

    // A grant is spent by its import; a file that is not a bundle is the engine's to refuse, and
    // nothing is created.
    expect((await api.refused("harnix-app.sessions.import", { token, space: "sessions" }))?.code).toBe(INVALID_PARAMS);
    const before = (await api.call("harnix.sessions.list", { space: "sessions" })).sessions.length;
    const bad = await send("not-a-bundle.zip", Buffer.from("not a zip"));
    expect(await api.refused("harnix-app.sessions.import", { token: bad, space: "sessions" })).toBeTruthy();
    expect((await api.call("harnix.sessions.list", { space: "sessions" })).sessions.length).toBe(before);
    expect((await api.refused("harnix-app.sessions.importUpload", { name: "x.txt", space: "sessions" }))?.code).toBe(INVALID_PARAMS);
  });
});

test("observability.process: versions, uptime, peak memory, and every connected user", SHELL, async () => {
  const backend = await startHarnix(launchFor("replay"));
  try {
    const one = await connect(backend);
    const two = await connect(backend);
    const process = await one.call("observability.process");
    expect(process.harnix).toMatch(/^0\.4\.\d+-\d{10}$/);
    expect(typeof process.agix).toBe("string");
    expect(process.uptime_secs).toBeGreaterThanOrEqual(0);
    expect(process.connections.length).toBeGreaterThanOrEqual(2);
    for (const c of process.connections) expect(c).toMatchObject({ device: expect.any(String), topology: expect.any(String), since_ms: expect.any(Number) });
    two.close();
    if (topology() === "split_streams") {
      // A split-streams client says nothing when it goes: closing its streams looks the same as
      // streams that dropped and are being reopened (a proxy's timeout, a laptop asleep), and its
      // handshake has to outlive that or the page loses its subscriptions. So it is listed until
      // the transport's idle sweep ends it — streams closed and no `/rpc` for SPLIT_IDLE, five
      // minutes (crates/transport/src/session.rs, whose unit test covers the sweep).
      await sleep(500);
      expect((await one.call("observability.process")).connections.length).toBe(process.connections.length);
    } else {
      await expect.poll(async () => (await one.call("observability.process")).connections.length).toBe(process.connections.length - 1);
    }
    one.close();
  } finally {
    await backend.dispose();
  }
});
