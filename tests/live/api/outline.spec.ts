/**
 * `harnix.session.outline`: a session's turns as the hub reads them off its transcript — one row per turn,
 * each where its first message is in `engine.session.history`, with the person's words and the last reply cut
 * short — read again after each turn settles, and held to the space a call claims as history is.
 *
 * Turns are replayed from `many-turns` (`says-ready`'s one call sixty times over): every turn answers
 * "fixture ready". `fleet` is the testapp's space on its own instance.
 */

import { SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, finish, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startTestApp();
  api = await connect(backend);
});
test.afterAll(async () => {
  api?.close();
  await backend?.dispose();
});

/** Send `message` and wait for its turn to settle; its turn id. */
async function turn(session_id: string, message: string): Promise<string> {
  const { turn_id } = await api.call("harnix.session.send", { session_id, message });
  await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.turn_id === turn_id, { timeout: 30_000, describe: `"${message}" to settle` });
  return turn_id;
}

test("the outline is one row per turn, each where its first message is, and grows as turns settle", TAGS, async () => {
  const { session_id } = await api.call("harnix.session.create", { project_root: api.project, options: { recorder: { name: "many-turns" } } });
  await api.call("harnix.session.attach", { session_id });
  try {
    const empty = await api.call("harnix.session.outline", { session_id });
    expect(empty).toMatchObject({ instance: "main", session_id, turns: [] });

    const long = `Say exactly: fixture ready. ${"and then some more words ".repeat(10)}`;
    const ids = [await turn(session_id, "Say exactly: fixture ready (0)"), await turn(session_id, `${long}\n\nsecond paragraph`)];
    const outline = await api.call("harnix.session.outline", { session_id });
    expect(outline.turns.map((t: any) => t.turn_id)).toEqual(ids);
    const history = await api.call("engine.session.history", { session_id, offset: 0, limit: 500 });
    expect(outline.total).toBe(history.total);
    for (const row of outline.turns) {
      const first = history.messages.find((m: any) => m.seq === row.seq);
      expect(first, `history has the message at ${row.seq}`).toMatchObject({ role: "user", turn_id: row.turn_id });
      expect(row.reply).toMatch(/fixture ready/i);
      expect(typeof row.at).toBe("string");
      expect(row).not.toHaveProperty("marks");
    }
    expect(outline.turns[0].prompt).toBe("Say exactly: fixture ready (0)");
    // One line, cut to about a line's worth.
    expect(outline.turns[1].prompt).not.toContain("\n");
    expect([...outline.turns[1].prompt].length).toBe(100);
    expect(outline.turns[1].prompt.endsWith("…")).toBe(true);

    // The next turn is read on to; the rows before it are as they were.
    ids.push(await turn(session_id, "Say exactly: fixture ready (2)"));
    const grown = await api.call("harnix.session.outline", { session_id });
    expect(grown.turns.map((t: any) => t.turn_id)).toEqual(ids);
    expect(grown.turns.slice(0, 2)).toEqual(outline.turns);
    expect(grown.turns[2].seq).toBeGreaterThan(outline.turns[1].seq);
  } finally {
    await finish(api, session_id);
  }
});

test("the outline is held to the space a call claims, as the session's history is", TAGS, async () => {
  const fleet = await api.call("harnix.session.create", { space: "fleet", options: { recorder: { name: "says-ready" } } });
  const key = { instance: fleet.instance, session_id: fleet.session_id };
  try {
    expect((await api.call("harnix.session.outline", { ...key, space: "fleet" })).turns).toEqual([]);
    const refused = await api.refused("harnix.session.outline", { ...key, space: "sessions" });
    expect(refused?.code).toBe(SPACE_MISMATCH);
    expect(refused?.data?.space).toBe("fleet");
    expect((await api.call("harnix.reachable")).methods).toContain("harnix.session.outline");
  } finally {
    await api.call("harnix.session.delete", { ...key, space: "fleet" }).catch(() => {});
  }
});
