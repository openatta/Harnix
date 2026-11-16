/**
 * The system space's backend (concept §5.5): any instance's sessions inspected and acted on through
 * `system.*` and the hub's own methods claiming the system space — and none of it there
 * when the profile turns the space off: the methods leave the authorization table, the space
 * stops existing, and nothing can claim it.
 *
 * Runs on `harnix-testapp`, whose `fleet` module builds an instance (`fleet-side`) for a space of
 * its own: the session there is another space's, which is the case the system space is for. That
 * backend replays in every run, a live one too, and a replayed session records nothing: what reads
 * a recording is tested on a second backend whose sessions record (`recorder` below).
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { INVALID_PARAMS, METHOD_NOT_FOUND, NOT_FOUND, REFUSED, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, launchFor, modelMode, startHarnix, test } from "@harnix/testkit";

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
  "system.extensions.log",
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
  const dir = join(on.scratch, "agix", "recordings", s.session_id);
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

  test("a session's model is switched from the system space, and says so", TAGS, async () => {
    const s = await oneTurn(api, "fleet");
    // The engine's answer: the model the session's next turn runs; `fleet.model` reads the engine's
    // `engine.session.inspectContext`, which names the one chosen here before that turn.
    const chosen = await api.call("system.model", { ...s, model: "claude-haiku-5-5" });
    expect(chosen).toMatchObject({ session_id: s.session_id, model: "claude-haiku-5-5", applies: "next_turn" });
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
