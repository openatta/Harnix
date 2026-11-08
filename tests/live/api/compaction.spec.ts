/**
 * Compaction as a session's watchers see it: a session long enough to compact (eight replayed
 * turns), compacted from the system space; the engine says so as it happens (`compact_action`,
 * `compact`, with how many messages before and after) and the transcript keeps what replaced the
 * older messages as messages of their own (`origin: "compact"`).
 *
 * `many-turns` is one recorded call (`says-ready`'s) sixty times: whichever call comes next — a
 * turn, a memory extraction, a compaction's summary — the same answer stands for it.
 */

import { type Backend, type TestClient, connect, expect, finish, launchFor, startHarnix, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

let backend: Backend;
let api: TestClient;

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
  api = await connect(backend);
});
test.afterAll(async () => {
  api?.close();
  await backend?.dispose();
});

/** A session with `turns` replayed turns behind it. */
export async function longSession(client: TestClient, turns: number): Promise<string> {
  const { session_id } = await client.call("session.create", { project_root: client.project, options: { recorder: { name: "many-turns" } } });
  await client.call("harnix.attach", { session_id });
  for (let i = 0; i < turns; i += 1) {
    const { turn_id } = await client.call("harnix.send", { session_id, message: `Say exactly: fixture ready (${i})` });
    await client.waitFor((f) => f.method === "harnix.turn_settled" && f.params.turn_id === turn_id, { timeout: 30_000, describe: `turn ${i} to settle` });
  }
  return session_id;
}

test("a compaction is told as it happens, what replaced the older messages stays in the transcript, and a device opening it reads that", TAGS, async () => {
  const session_id = await longSession(api, 8);
  try {
    const before = await api.call("session.history", { session_id });
    expect(before.messages.some((m: any) => m.origin === "compact")).toBe(false);
    const mark = api.events.length;
    await api.call("harnix.system.compact", { instance: "system", session_id });

    const event = await api.waitFor(
      (f) => f.method === "harnix.event" && f.params.session_id === session_id && f.params.event?.kind === "compact" && api.events.indexOf(f) >= mark,
      { timeout: 30_000, describe: "the compaction to be told" },
    );
    const told = event.params.event;
    expect(told.messages_after).toBeLessThan(told.messages_before);
    expect(told.messages_before).toBe(before.total);
    expect(typeof told.strategy).toBe("string");

    const after = await api.call("session.history", { session_id });
    const compacted = after.messages.filter((m: any) => m.origin === "compact");
    expect(compacted.length, "what replaced the older messages is in the transcript").toBeGreaterThan(0);
    expect(after.total).toBe(told.messages_after);

    // A device that opens it now reads the compacted transcript, not only the compaction's turn:
    // the transcript got shorter, and that is it having caught up, not it lagging behind.
    const fresh = await connect(backend);
    try {
      const attached = await fresh.call("harnix.attach", { session_id });
      expect(attached.history_total).toBe(after.total);
      expect(attached.replay).toEqual([]);
    } finally {
      fresh.close();
    }
  } finally {
    await finish(api, session_id);
  }
});
