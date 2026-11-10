/**
 * Spaces (concept §5.4): a session belongs to one space from its creation, and a call that
 * claims a space is checked against that — on the server, not only in the interface. The system
 * space is the exception, and each time it reaches across a space it is on the record.
 *
 * `fleet` is a space the testapp declares, on its own instance (`fleet-side`).
 */

import { INSTANCE_NOT_FOUND, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:spaces", "@layer:api"] };

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

/** A session created in a space, the way the interface creates one. */
async function createIn(space: string): Promise<{ instance: string; session_id: string }> {
  const created = await api.call("harnix.session.create", { space, options: { recorder: { name: "says-ready" } } });
  expect(created.space).toBe(space);
  return { instance: created.instance, session_id: created.session_id };
}

test("a space's new session lands on its default instance and is listed in it alone", TAGS, async () => {
  const { spaces } = await api.call("harnix.spaces.list");
  expect(spaces.find((s: any) => s.id === "fleet")).toMatchObject({ instance: "fleet-side", scene: "harnix" });
  const fleet = await createIn("fleet");
  expect(fleet.instance).toBe("fleet-side");
  const mine = await createIn("sessions");
  expect(mine.instance).toBe("system");

  const inFleet = (await api.call("harnix.sessions.list", { space: "fleet" })).sessions.map((s: any) => s.session_id);
  const inSessions = (await api.call("harnix.sessions.list", { space: "sessions" })).sessions.map((s: any) => s.session_id);
  expect(inFleet).toContain(fleet.session_id);
  expect(inFleet).not.toContain(mine.session_id);
  expect(inSessions).toContain(mine.session_id);
  expect(inSessions).not.toContain(fleet.session_id);
  const all = (await api.call("harnix.sessions.list", { space: "system" })).sessions.map((s: any) => s.session_id);
  expect(all).toEqual(expect.arrayContaining([fleet.session_id, mine.session_id]));
});

test("the kernel has no space of its own: the session space is the Harnix app's, and the default", TAGS, async () => {
  const { spaces } = await api.call("harnix.spaces.list");
  expect(spaces.filter((s: any) => s.default).map((s: any) => s.id)).toEqual(["sessions"]);
  expect(spaces.find((s: any) => s.id === "system")).toMatchObject({ cross_space: true, default: false });
  // A session created claiming no space is the default space's, and checked as it.
  const created = await api.call("harnix.session.create", { options: { recorder: { name: "says-ready" } } });
  const inSessions = (await api.call("harnix.sessions.list", { space: "sessions" })).sessions.map((s: any) => s.session_id);
  expect(inSessions).toContain(created.session_id);
  // The topic of the spaces is there whatever is linked.
  expect((await api.call("harnix.reachable")).topics).toContain("harnix.spaces");

  const without = await startTestApp({ profile: "\n[extensions]\noff = [\"harnix-app\"]\n" });
  try {
    const client = await connect(without);
    const listed = (await client.call("harnix.spaces.list")).spaces.map((s: any) => s.id);
    expect(listed).not.toContain("sessions");
    expect((await client.refused("harnix.sessions.list", { space: "sessions" }))?.code).toBe(SPACE_MISMATCH);
    client.close();
  } finally {
    await without.dispose();
  }
});

test("a call claiming the wrong space is refused, naming the right one", TAGS, async () => {
  const fleet = await createIn("fleet");
  for (const [method, extra] of [
    ["harnix.session.attach", {}],
    ["harnix.session.send", { message: "hello" }],
    ["engine.session.history", {}],
    ["harnix.session.delete", {}],
  ] as const) {
    const error = await api.refused(method, { ...fleet, ...extra, space: "sessions" });
    expect(error?.code, method).toBe(SPACE_MISMATCH);
    expect(error.data?.space, method).toBe("fleet");
    expect(error.message, method).toContain("fleet");
  }
  // Its own space reaches it; so does a caller that claims none.
  const attached = await api.call("harnix.session.attach", { ...fleet, space: "fleet" });
  expect(attached.instance).toBe("fleet-side");
  await api.call("engine.session.get", fleet);
});

test("the instance has to be the session's own", TAGS, async () => {
  const fleet = await createIn("fleet");
  const wrong = await api.refused("harnix.session.attach", { instance: "main", session_id: fleet.session_id, space: "fleet" });
  expect(wrong, "attached through another instance").not.toBeNull();
  const nowhere = await api.refused("harnix.session.attach", { instance: "nowhere", session_id: fleet.session_id, space: "fleet" });
  expect(nowhere?.code).toBe(INSTANCE_NOT_FOUND);
  const taken = await api.refused("harnix.session.create", { space: "sessions", instance: "fleet-side" });
  expect(taken, "an instance is usable by spaces other than the one that owns it only when no space owns it").toBeNull();
});

test("the system space reaches any space's session, and that is audited", TAGS, async () => {
  const fleet = await createIn("fleet");
  await api.call("harnix.session.attach", { ...fleet, space: "system" });
  const { entries } = await api.call("harnix.audit", { limit: 50 });
  const crossed = entries.find((e: any) => e.method === "harnix.session.attach" && e.reason?.includes("cross-space"));
  expect(crossed, JSON.stringify(entries.slice(0, 5))).toBeTruthy();
});

test("a session never moves: forking it keeps the space", TAGS, async () => {
  const fleet = await createIn("fleet");
  await api.call("harnix.session.send", { ...fleet, space: "fleet", message: "Say exactly: fixture ready" });
  await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === fleet.session_id, { timeout: 60_000 });
  const forked = await api.call("harnix.session.fork", { ...fleet, space: "fleet" });
  const inFleet = (await api.call("harnix.sessions.list", { space: "fleet" })).sessions.map((s: any) => s.session_id);
  expect(inFleet).toContain(forked.session_id);
});

test("@session references carry another space's conversation into a turn", TAGS, async () => {
  const fleet = await createIn("fleet");
  await api.call("harnix.session.attach", { ...fleet, space: "fleet" });
  await api.call("harnix.session.send", { ...fleet, space: "fleet", message: "Say exactly: fixture ready" });
  await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === fleet.session_id, { timeout: 60_000 });

  const mine = await createIn("sessions");
  await api.call("harnix.session.attach", { ...mine, space: "sessions" });
  const sent = await api.call("harnix.session.send", {
    ...mine,
    space: "sessions",
    message: "Say exactly: fixture ready",
    references: [fleet],
  });
  expect(sent.turn_id).toBeTruthy();
  const settled = await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === mine.session_id, { timeout: 60_000 });
  expect(settled.params.error).toBeUndefined();
  const bad = await api.refused("harnix.session.send", { ...mine, space: "sessions", message: "x", references: [{ instance: "main", session_id: "no-such" }] });
  expect(bad, "a reference to nothing is refused rather than dropped").not.toBeNull();
});

test("with the system space turned off, nobody may claim it", TAGS, async () => {
  const off = await startTestApp({ profile: "\n[extensions]\noff = [\"system\"]\n" });
  try {
    const client = await connect(off);
    const created = await client.call("harnix.session.create", { space: "sessions" });
    const error = await client.refused("harnix.session.attach", { instance: created.instance, session_id: created.session_id, space: "system" });
    expect(error?.code).toBe(SPACE_MISMATCH);
    const { spaces } = await client.call("harnix.spaces.list");
    expect(spaces.map((s: any) => s.id)).not.toContain("system");
    client.close();
  } finally {
    await off.dispose();
  }
});

test("a module moves a session to another space, where it is listed alone; a space it cannot go to is refused (ipdrun-51)", TAGS, async () => {
  const mine = await createIn("sessions");
  const move = (params: Record<string, unknown>) =>
    api.call("app.probe.call", { method: "harnix.session.move", params: { ...mine, ...params } });
  expect(await move({ space: "projects" })).toEqual({ space: "projects", from: "sessions", moved: true });
  const listed = async (space: string) =>
    (await api.call("harnix.sessions.list", { space })).sessions.map((r: any) => r.session_id);
  expect(await listed("projects")).toContain(mine.session_id);
  expect(await listed("sessions")).not.toContain(mine.session_id);
  expect((await api.refused("engine.session.get", { ...mine, space: "sessions" }))?.code).toBe(SPACE_MISMATCH);
  await api.call("engine.session.get", { ...mine, space: "projects" });
  // Moving it where it already is changes nothing.
  expect(await move({ space: "projects" })).toMatchObject({ moved: false });

  // Not to the system space, nor to a name no space can have (a well-formed one is a space — one
  // may live only in the interface).
  for (const space of ["system", "not a space!"]) {
    const refused = await api.refused("app.probe.call", { method: "harnix.session.move", params: { ...mine, space } });
    expect(refused?.code, space).toBe(SPACE_MISMATCH);
  }
  expect(await api.refused("app.probe.call", { method: "harnix.session.move", params: { ...mine, session_id: "no-such-session", space: "sessions" } })).toBeTruthy();
});

test("moving a session to another space is the application's: a device is refused (ipdrun-51)", TAGS, async () => {
  const mine = await createIn("sessions");
  const error = await api.refused("harnix.session.move", { ...mine, space: "fleet" });
  expect(error, "a device cannot rebind a session").not.toBeNull();
  await api.call("engine.session.get", { ...mine, space: "sessions" });
});
