/**
 * Entries (design §7.6): what a module writes into a session's log for people — a card, a
 * decision, a job's result. It is in the log, so a refresh, a replay and a fork all see it; it is
 * never the model's; a module writes only under its own namespace, and a device writes none.
 *
 * `app.probe.note` (crates/testapp) writes one as the `probe` module and lists the module's own.
 */

import { REFUSED } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:entries", "@layer:api"] };

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

/** A session with one replayed turn: the user's message and the reply. */
async function oneTurn(): Promise<{ instance: string; session_id: string }> {
  const created = await api.call("session.create", { options: { recorder: { name: "says-ready" } } });
  const s = { instance: created.instance, session_id: created.session_id };
  await api.call("harnix.attach", s);
  await api.call("harnix.send", { ...s, message: "Say exactly: fixture ready" });
  await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id, {
    timeout: 60_000,
    describe: "the turn to settle",
  });
  return s;
}

test("a module's entry lands where it was written, under its own name, and watchers see it at once", TAGS, async () => {
  const s = await oneTurn();
  const { written, mine } = await api.call("app.probe.note", { ...s, event: "job_done", payload: { title: "entry-marker-41" } });
  expect(written).toMatchObject({ ns: "app.probe", event: "job_done", at: 2, payload: { title: "entry-marker-41" } });
  expect(mine.map((e: any) => e.id)).toEqual([written.id]);

  // The attached client was told, outside any turn.
  const frame = await api.waitFor(
    (f) => f.method === "harnix.event" && f.params.session_id === s.session_id && f.params.event?.kind === "extension_entry",
    { describe: "the entry to be pushed" },
  );
  expect(frame.params.event.entry.id).toBe(written.id);

  const { entries } = await api.call("session.entries", s);
  expect(entries.map((e: any) => [e.ns, e.event, e.at])).toEqual([["app.probe", "job_done", 2]]);
});

test("the model never sees an entry", TAGS, async () => {
  const s = await oneTurn();
  await api.call("app.probe.note", { ...s, event: "secret_card", payload: { title: "entry-marker-77" } });
  const history = await api.call("session.history", s);
  expect(history.total).toBe(2);
  expect(JSON.stringify(history.messages)).not.toContain("entry-marker-77");
  const context = await api.call("harnix.system.context", s);
  expect(context.messages.count).toBe(2);
  expect(JSON.stringify(context)).not.toContain("entry-marker-77");
});

test("a fork of the whole session keeps its entries; one cut before them does not", TAGS, async () => {
  const s = await oneTurn();
  await api.call("app.probe.note", { ...s, event: "after_the_turn", payload: {} });
  const whole = await api.call("session.fork", s);
  const kept = await api.call("session.entries", { instance: s.instance, session_id: whole.session_id });
  expect(kept.entries.map((e: any) => e.event)).toEqual(["after_the_turn"]);
  const cut = await api.call("session.fork", { ...s, at_message: 1 });
  const dropped = await api.call("session.entries", { instance: s.instance, session_id: cut.session_id });
  expect(dropped.entries).toEqual([]);
});

test("an entry outlives the session leaving memory, and a fresh client reads it back", TAGS, async () => {
  const s = await oneTurn();
  await api.call("app.probe.note", { ...s, event: "before_close", payload: {} });
  await api.call("session.close", s);
  const other = await connect(backend);
  try {
    await other.call("harnix.attach", s);
    const { entries } = await other.call("session.entries", s);
    expect(entries.map((e: any) => e.event)).toEqual(["before_close"]);
  } finally {
    other.close();
  }
});

test("a device writes no entries, and a module none outside its namespace", TAGS, async () => {
  const s = await oneTurn();
  const error = await api.refused("harnix.entries.append", { ...s, event: "forged", payload: {} });
  expect(error?.code).toBe(REFUSED);
  // Even naming one: the hub decides the namespace, not the caller.
  await api.call("app.probe.call", {
    method: "harnix.entries.append",
    params: { ...s, ns: "app.other", event: "claimed", payload: {} },
  });
  const { entries } = await api.call("session.entries", s);
  expect(entries.map((e: any) => [e.ns, e.event])).toEqual([["app.probe", "claimed"]]);
});
