/**
 * Spaces (concept §5.4): a session belongs to one space from its creation, and a call that
 * claims a space is checked against that — on the server, not only in the interface. No space of
 * the product reaches across. The home page is a space that holds no sessions (harnix-app.md §10).
 *
 * `fleet` is a space the testapp declares, on its own instance (`fleet-side`).
 */

import { INSTANCE_NOT_FOUND, INVALID_PARAMS, REFUSED, SPACE_MISMATCH } from "@harnix/core/protocol";
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
  expect(mine.instance).toBe("main");

  const inFleet = (await api.call("harnix.sessions.list", { space: "fleet" })).sessions.map((s: any) => s.session_id);
  const inSessions = (await api.call("harnix.sessions.list", { space: "sessions" })).sessions.map((s: any) => s.session_id);
  expect(inFleet).toContain(fleet.session_id);
  expect(inFleet).not.toContain(mine.session_id);
  expect(inSessions).toContain(mine.session_id);
  expect(inSessions).not.toContain(fleet.session_id);
});

test("the kernel has no space of its own: the session space is the Harnix app's, and the default", TAGS, async () => {
  const { spaces } = await api.call("harnix.spaces.list");
  expect(spaces.filter((s: any) => s.default).map((s: any) => s.id)).toEqual(["sessions"]);
  expect(spaces.filter((s: any) => s.cross_space).map((s: any) => s.id), "no space reaches across").toEqual([]);
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

test("a module moves a session to another space, where it is listed alone; a space it cannot go to is refused (ipdrun-51)", TAGS, async () => {
  const mine = await createIn("sessions");
  const move = (params: Record<string, unknown>) =>
    api.call("probe.call", { method: "harnix.session.move", params: { ...mine, ...params } });
  expect(await move({ space: "projects" })).toEqual({ space: "projects", from: "sessions", moved: true });
  const listed = async (space: string) =>
    (await api.call("harnix.sessions.list", { space })).sessions.map((r: any) => r.session_id);
  expect(await listed("projects")).toContain(mine.session_id);
  expect(await listed("sessions")).not.toContain(mine.session_id);
  expect((await api.refused("engine.session.get", { ...mine, space: "sessions" }))?.code).toBe(SPACE_MISMATCH);
  await api.call("engine.session.get", { ...mine, space: "projects" });
  // Moving it where it already is changes nothing.
  expect(await move({ space: "projects" })).toMatchObject({ moved: false });

  // Not to the home page, which holds none, nor to a name no space can have (a well-formed one is
  // a space — one may live only in the interface).
  for (const space of ["home", "not a space!"]) {
    const refused = await api.refused("probe.call", { method: "harnix.session.move", params: { ...mine, space } });
    expect(refused?.code, space).toBe(SPACE_MISMATCH);
  }
  expect(await api.refused("probe.call", { method: "harnix.session.move", params: { ...mine, session_id: "no-such-session", space: "sessions" } })).toBeTruthy();
});

test("a module exposes its session to another space: looked at there, talked to only when it says so, its life still its own; listed there saying where it is from; taken back", TAGS, async () => {
  // The probe's own session, in the session space: a device's session is nobody's to expose.
  const made = await api.call("probe.call", { method: "harnix.session.create", params: { space: "sessions", options: { recorder: { name: "says-ready" } } } });
  const mine = { instance: made.instance, session_id: made.session_id };
  const expose = (params: Record<string, unknown>) => api.call("probe.call", { method: "harnix.session.expose", params: { ...mine, ...params } });
  const refusedExpose = (params: Record<string, unknown>) => api.refused("probe.call", { method: "harnix.session.expose", params: { ...mine, ...params } });
  expect((await api.refused("engine.session.get", { ...mine, space: "fleet" }))?.code).toBe(SPACE_MISMATCH);

  // To look at.
  expect(await expose({ to: "fleet" })).toEqual({ ...mine, space: "sessions", to: "fleet", exposed: true, interact: false, changed: true });
  await api.call("engine.session.get", { ...mine, space: "fleet" });
  await api.call("engine.session.history", { ...mine, space: "fleet" });
  await api.call("harnix.session.attach", { ...mine, space: "fleet" });
  const looked = await api.refused("harnix.session.interrupt", { ...mine, space: "fleet" });
  expect(looked?.code).toBe(SPACE_MISMATCH);
  expect(looked?.message).toContain("to look at, not to talk to");
  expect(looked?.data?.space).toBe("sessions");
  // Listed in the space it is shown in, saying where it is from; in its own as ever, saying where it is shown.
  const rows = async (space: string) => (await api.call("harnix.sessions.list", { space })).sessions as any[];
  expect((await rows("fleet")).find((r) => r.session_id === mine.session_id)).toMatchObject({ space: "sessions", exposed_from: "sessions", exposed_to: [{ space: "fleet", interact: false }] });
  expect((await rows("sessions")).find((r) => r.session_id === mine.session_id)).toMatchObject({ space: "sessions", exposed_to: [{ space: "fleet", interact: false }] });
  expect((await rows("sessions")).find((r) => r.session_id === mine.session_id).exposed_from).toBeUndefined();

  // To talk to as well: the same call again changes it.
  expect(await expose({ to: "fleet", interact: true })).toMatchObject({ interact: true, changed: true });
  expect(await expose({ to: "fleet", interact: true })).toMatchObject({ changed: false });
  await api.call("harnix.session.interrupt", { ...mine, space: "fleet" });
  // Its life and identity stay the session space's, whatever the exposure.
  for (const method of ["harnix.session.close", "harnix.session.rename", "harnix.session.fork"]) {
    const refused = await api.refused(method, { ...mine, space: "fleet", name: "x" });
    expect(refused?.code, method).toBe(SPACE_MISMATCH);
    expect(refused?.message, method).toContain("is done from there");
  }
  await api.call("engine.session.get", { ...mine, space: "sessions" });

  // Not to its own space, nor a name no space can have.
  for (const to of ["sessions", "not a space!"]) {
    expect((await refusedExpose({ to }))?.code, to).toBe(SPACE_MISMATCH);
  }
  expect((await refusedExpose({}))?.code, "no `to`").toBe(INVALID_PARAMS);

  // Taken back: nothing reaches it from there, and the list no longer has it.
  expect(await api.call("probe.call", { method: "harnix.session.unexpose", params: { ...mine, to: "fleet" } })).toEqual({ ...mine, space: "sessions", to: "fleet", exposed: false, interact: false, changed: true });
  expect(await api.call("probe.call", { method: "harnix.session.unexpose", params: { ...mine, to: "fleet" } })).toMatchObject({ changed: false });
  expect((await api.refused("engine.session.get", { ...mine, space: "fleet" }))?.code).toBe(SPACE_MISMATCH);
  expect((await rows("fleet")).some((r) => r.session_id === mine.session_id)).toBe(false);
});

test("exposing a session is its maker's: a device is refused, and so is a session the module did not make", TAGS, async () => {
  const theirs = await createIn("sessions");
  const error = await api.refused("harnix.session.expose", { ...theirs, to: "fleet" });
  expect(error, "a device cannot expose a session").not.toBeNull();
  const nobodys = await api.refused("probe.call", { method: "harnix.session.expose", params: { ...theirs, to: "fleet" } });
  expect(nobodys?.code).toBe(REFUSED);
  expect(nobodys?.message).toContain("nobody's to expose");
  expect((await api.refused("engine.session.get", { ...theirs, space: "fleet" }))?.code).toBe(SPACE_MISMATCH);
});

test("moving a session to another space is the application's: a device is refused (ipdrun-51)", TAGS, async () => {
  const mine = await createIn("sessions");
  const error = await api.refused("harnix.session.move", { ...mine, space: "fleet" });
  expect(error, "a device cannot rebind a session").not.toBeNull();
  await api.call("engine.session.get", { ...mine, space: "sessions" });
});

test("the home page is a space that holds no sessions: none is created, imported or moved there, and it lists none", TAGS, async () => {
  const { spaces } = await api.call("harnix.spaces.list");
  expect(spaces.find((s: any) => s.id === "home")).toMatchObject({ no_sessions: true, default: false });
  expect(spaces.find((s: any) => s.id === "sessions").no_sessions, "absent where it holds them").toBeUndefined();
  const listed = async (space: string) => (await api.call("harnix.sessions.list", { space })).sessions.map((r: any) => r.session_id);
  const before = await listed("sessions");
  const holdsNone = (error: any, what: string) => {
    expect(error?.code, what).toBe(SPACE_MISMATCH);
    expect(error?.message, what).toContain("holds no sessions");
  };

  // Created there, by a device or a module.
  holdsNone(await api.refused("harnix.session.create", { space: "home" }), "a device's create");
  holdsNone(await api.refused("probe.call", { method: "harnix.session.create", params: { space: "home" } }), "a module's create");

  // Imported there: the bundle is not even read, the space is refused first.
  const grant = await api.call("harnix-app.sessions.importUpload", { name: "home.zip", space: "home" });
  const sent = await fetch(`${backend.url.replace(/\/$/, "")}${grant.url}`, { method: "POST", body: new Uint8Array(Buffer.from("not read")) });
  expect(sent.ok).toBe(true);
  holdsNone(await api.refused("harnix-app.sessions.import", { token: grant.token, space: "home" }), "an import");

  // Moved there: to a move, it is no space a session can go to.
  const mine = await createIn("sessions");
  const moved = await api.refused("probe.call", { method: "harnix.session.move", params: { ...mine, space: "home" } });
  expect(moved?.code).toBe(SPACE_MISMATCH);
  expect(moved?.message).toContain("no space `home` to move a session to");
  expect(await listed("sessions")).toEqual(expect.arrayContaining([...before, mine.session_id]));

  // It lists none, and nothing was left anywhere: the session space has only the one made here.
  expect(await listed("home")).toEqual([]);
  expect((await listed("sessions")).filter((id: string) => !before.includes(id))).toEqual([mine.session_id]);
});
