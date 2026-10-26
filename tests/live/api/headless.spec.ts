/**
 * Sessions driven from the server side (design §6.1): a module creates
 * sessions, each with a model of its own, and sends into them through the
 * hub — the same queue, settlement and broadcast a client gets.
 *
 * A headless session is an ordinary one: it is in its instance's session
 * list, a device can attach and see everything it did, and it is still
 * there after a restart. A tap — the module's own watcher — is bounded: one
 * nobody reads never holds the session up, and when it is read it says what
 * it missed and catches up.
 */

import { type Backend, type TestClient, connect, expect, sleep, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:headless", "@layer:api"] };

/** Wait until none of these sessions is running a turn. */
async function idle(api: TestClient, sessions: { instance: string; session_id: string }[]) {
  const deadline = Date.now() + 60_000;
  for (;;) {
    const states = await Promise.all(
      sessions.map((s) => api.call("session.get", { instance: s.instance, session_id: s.session_id })),
    );
    if (states.every((s) => s.turn_state === "idle")) return;
    if (Date.now() > deadline) throw new Error(`still running: ${JSON.stringify(states.map((s) => s.turn_state))}`);
    await sleep(100);
  }
}

let backend: Backend;
let api: TestClient;
let started: { instance: string; session_id: string }[] = [];

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(() => backend?.stop());
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

test("a module runs three headless sessions, each with its own model", TAGS, async () => {
  const result = await api.call("app.fleet.headless", {
    count: 3,
    project_root: api.project,
    recording: "says-ready",
    message: "Say exactly: fixture ready",
  });
  started = result.started.map((s: any) => ({ instance: s.instance, session_id: s.session_id }));
  expect(started).toHaveLength(3);
  for (const s of result.started) expect(s.sent.turn_id, "each send started a turn").toBeTruthy();
  await idle(api, started);

  const models = await Promise.all(started.map((s) => api.call("app.fleet.model", s)));
  expect(models.map((m) => m.model)).toEqual(["headless-model-0", "headless-model-1", "headless-model-2"]);

  const { sessions } = await api.call("harnix.instances.sessions", { instance: "system" });
  const listed = sessions.map((s: any) => s.session_id);
  for (const s of started) expect(listed).toContain(s.session_id);
});

test("a device attaching to a headless session sees all of it", TAGS, async () => {
  for (const s of started) {
    const attached = await api.call("harnix.attach", s);
    expect(attached.instance).toBe("system");
    expect(attached.running_turn).toBeNull();
    expect(attached.history_total).toBeGreaterThanOrEqual(2);
    const history = await api.call("session.history", s);
    const text = JSON.stringify(history.messages);
    expect(text).toContain("Say exactly: fixture ready");
    expect(text).toMatch(/fixture ready/i);
  }
});

test("a tap nobody reads holds nothing up, and catches up when read", TAGS, async () => {
  const { started: [session] } = await api.call("app.fleet.headless", {
    count: 1,
    project_root: api.project,
    recording: "says-ready",
  });
  const s = { instance: session.instance, session_id: session.session_id };
  // Two frames' room: a turn sends many more.
  await api.call("app.fleet.tap", { ...s, capacity: 2 });
  await api.call("harnix.attach", s);
  await api.call("harnix.send", { ...s, message: "Say exactly: fixture ready" });
  const settled = await api.waitFor(
    (f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id,
    { timeout: 60_000, describe: "the turn to settle with the tap unread" },
  );
  expect(settled.params.error).toBeUndefined();
  const sentToClient = api.events.filter((f) => f.method === "harnix.event" && f.params.session_id === s.session_id);
  expect(sentToClient.length, "the turn sent more than the tap holds").toBeGreaterThan(2);

  const drained = await api.call("app.fleet.drain", { catch_up: true });
  expect(drained.lagged.length, "the tap was told it fell behind").toBeGreaterThan(0);
  expect(drained.lagged[0]).toBeLessThan(sentToClient.at(-1).params.seq);
  // What the buffer no longer has, the transcript does: the turn settled,
  // and history covers it.
  expect(drained.history_total).toBeGreaterThanOrEqual(2);
  const history = await api.call("session.history", s);
  expect(JSON.stringify(history.messages)).toMatch(/fixture ready/i);
  started.push(s);
});

test("headless sessions are still there after a restart", TAGS, async () => {
  api.close();
  backend = await backend.restart();
  api = await connect(backend);
  const { sessions } = await api.call("app.fleet.sessions");
  expect(sessions.map((s: any) => s.session_id).sort()).toEqual(started.map((s) => s.session_id).sort());
  for (const s of started) {
    const attached = await api.call("harnix.attach", s);
    expect(attached.history_total).toBeGreaterThanOrEqual(2);
  }
});
