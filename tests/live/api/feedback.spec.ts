/**
 * Feedback on an answer (harnix-app.md §2, harnix-ext.md §7.3): a device's thumbs up or down on a
 * turn, with a category and a note, filed by the hub as an entry of its own (`harnix.feedback` /
 * `rating`). Changing it or taking it back files another; the last one stands and the history stays.
 * It is in the session's log, so a fresh client and a whole fork read it back and the export
 * carries it; the model never sees it. Nobody hears of it outside the log unless the product granted
 * an extension `feedback.read`: the topic is no device's, and no ungranted extension's.
 *
 * Replayed: `says-ready`.
 */

import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { INVALID_PARAMS, REFUSED, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { readZip } from "./packages-host";
import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };
const FEEDBACK = ["harnix.feedback"];

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

/** A session of the session space with one replayed turn behind it, and that turn's id. */
async function oneTurn(): Promise<{ s: { instance: string; session_id: string; space: string }; turn: string }> {
  const created = await api.call("harnix.session.create", { space: "sessions", options: { recorder: { name: "says-ready" } } });
  const s = { instance: created.instance, session_id: created.session_id, space: "sessions" };
  await api.call("harnix.session.attach", s);
  await api.call("harnix.session.send", { ...s, message: "Say exactly: fixture ready" });
  const settled = await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id, {
    timeout: 60_000,
    describe: "the turn to settle",
  });
  expect(settled.params.error).toBeUndefined();
  return { s, turn: settled.params.turn_id };
}

const given = async (s: { session_id: string }) =>
  (await api.call("engine.session.entries", { ...s, namespaces: FEEDBACK })).entries as any[];

test("an answer rated, changed and taken back: each an entry the watcher is told of, the last one standing", TAGS, async () => {
  const { s, turn } = await oneTurn();
  const before = (await api.call("engine.session.history", s)).total;

  const filed = await api.call("harnix.session.feedback", { ...s, turn_id: turn, rating: "down", category: "inaccurate", note: "feedback-marker-23" });
  expect(filed.feedback).toMatchObject({ turn_id: turn, rating: "down", category: "inaccurate", note: "feedback-marker-23" });
  expect(filed.feedback.by).toMatch(/^device:/);
  expect(filed.entry).toMatchObject({ ns: "harnix.feedback", event: "rating", payload: filed.feedback });
  // Pushed to whoever watches, outside any turn, as any entry is.
  const pushed = await api.waitFor(
    (f) => f.method === "harnix.event" && f.params.session_id === s.session_id && f.params.event?.kind === "extension_entry"
      && f.params.event.entry?.ns === "harnix.feedback",
    { describe: "the feedback entry to be pushed" },
  );
  expect(pushed.params.event.entry.id).toBe(filed.entry.id);

  await api.call("harnix.session.feedback", { ...s, turn_id: turn, rating: "up" });
  // Taken back: what comes with it is dropped.
  const back = await api.call("harnix.session.feedback", { ...s, turn_id: turn, rating: null, note: "ignored" });
  expect(back.feedback).toEqual({ turn_id: turn, rating: null, at: back.feedback.at, by: filed.feedback.by });

  const entries = await given(s);
  expect(entries.map((e) => e.payload.rating)).toEqual(["down", "up", null]);
  expect(entries.every((e) => e.payload.turn_id === turn)).toBe(true);

  // Never the model's: no message more, the note nowhere in the transcript.
  const history = await api.call("engine.session.history", s);
  expect(history.total).toBe(before);
  expect(JSON.stringify(history.messages)).not.toContain("feedback-marker-23");
});

test("what stands outlives the page: a fresh client reads it back, a whole fork keeps it", TAGS, async () => {
  const { s, turn } = await oneTurn();
  await api.call("harnix.session.feedback", { ...s, turn_id: turn, rating: "up", note: "kept" });
  await api.call("harnix.session.close", s);

  const other = await connect(backend);
  try {
    await other.call("harnix.session.attach", s);
    const { entries } = await other.call("engine.session.entries", { ...s, namespaces: FEEDBACK });
    expect(entries.map((e: any) => [e.payload.turn_id, e.payload.rating, e.payload.note])).toEqual([[turn, "up", "kept"]]);
  } finally {
    other.close();
  }
  const fork = await api.call("harnix.session.fork", s);
  const kept = await api.call("engine.session.entries", { instance: s.instance, session_id: fork.session_id, space: "sessions", namespaces: FEEDBACK });
  expect(kept.entries.map((e: any) => e.payload.rating)).toEqual(["up"]);
});

test("every export carries it: under the answer in markdown, among the bundle's entries, in the engine's archive", TAGS, async () => {
  const { s, turn } = await oneTurn();
  await api.call("harnix.session.feedback", { ...s, turn_id: turn, rating: "up" });
  await api.call("harnix.session.feedback", { ...s, turn_id: turn, rating: "down", category: "slow", note: "feedback-marker-41" });

  const md = await api.call("harnix-app.sessions.export", { ...s, format: "markdown" });
  // The last word only, under the answer.
  expect(md.content).toMatch(/\*\*Assistant\*\*\n\n[^\n]*fixture ready[^\n]*\n\n> \*\*Feedback\*\*: down \(slow\) — feedback-marker-41\n/i);
  expect(md.content.match(/\*\*Feedback\*\*/g)).toHaveLength(1);

  const bundle = await api.call("harnix-app.sessions.export", { ...s, format: "bundle" });
  const inBundle = bundle.content.entries.filter((e: any) => e.ns === "harnix.feedback");
  expect(inBundle.map((e: any) => e.payload.rating)).toEqual(["up", "down"]);

  const archive = await api.call("harnix-app.sessions.export", { ...s, format: "archive" });
  const dir = mkdtempSync(join(tmpdir(), "harnix-feedback-"));
  try {
    const path = join(dir, archive.file);
    writeFileSync(path, Buffer.from(archive.content, "base64"));
    const files = readZip(path);
    const entries = files.get(`sessions/${s.session_id}/entries.json`);
    expect(entries, `no entries in ${[...files.keys()].join(", ")}`).toBeTruthy();
    const listed = JSON.parse(entries!.toString("utf8"));
    expect(listed.filter((e: any) => e.ns === "harnix.feedback").map((e: any) => e.payload.note ?? null)).toEqual([null, "feedback-marker-41"]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a device rates only what it can see; the topic is no device's, nor an ungranted extension's", TAGS, async () => {
  const { s, turn } = await oneTurn();
  expect((await api.refused("harnix.session.feedback", { ...s, space: "fleet", turn_id: turn, rating: "up" }))?.code).toBe(SPACE_MISMATCH);
  expect((await api.refused("harnix.session.feedback", { ...s, turn_id: turn, rating: "sideways" }))?.code).toBe(INVALID_PARAMS);
  expect((await api.refused("harnix.session.feedback", { ...s, rating: "up" }))?.code).toBe(INVALID_PARAMS);
  // A person's judgment: an extension does not give it.
  expect((await api.refused("probe.call", { method: "harnix.session.feedback", params: { ...s, turn_id: turn, rating: "up" } }))?.code).toBe(REFUSED);
  expect(await given(s)).toEqual([]);

  // Every piece of feedback from every space, note and all: only for an extension granted `feedback.read`.
  expect((await api.refused("harnix.topics.subscribe", { topic: "harnix.feedback" }))?.code).toBe(REFUSED);
  expect((await api.refused("probe.call", { method: "harnix.topics.subscribe", params: { topic: "harnix.feedback" } }))?.code).toBe(REFUSED);
  const reachable = await api.call("harnix.reachable", {});
  expect(reachable.methods).toContain("harnix.session.feedback");
  expect(reachable.topics).not.toContain("harnix.feedback");
});
