/**
 * The system space's backend (design §9): any instance's sessions inspected and acted on through
 * `harnix.system.*` and the hub's own methods claiming the system space — and none of it there
 * when the profile turns the space off: the methods leave the authorization table, the space
 * stops existing, and nothing can claim it.
 *
 * Runs on `harnix-testapp`, whose `fleet` module builds an instance (`fleet-side`) for a space of
 * its own: the session there is another space's, which is the case the system space is for.
 */

import { METHOD_NOT_FOUND, REFUSED, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { until } from "./instances";
import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:system", "@layer:api"] };

const SYSTEM_METHODS = [
  "harnix.system.context",
  "harnix.system.calls",
  "harnix.system.tasks.stop",
  "harnix.system.messages.drop",
  "harnix.system.messages.edit",
  "harnix.system.compact",
  "harnix.system.clear",
  "harnix.system.model",
  "harnix.system.rename",
];

/** A session in `space` that has had one turn, the user's message first. */
async function oneTurn(api: TestClient, space: string): Promise<{ instance: string; session_id: string }> {
  const created = await api.call("session.create", { space, options: { recorder: { name: "says-ready" } } });
  const s = { instance: created.instance, session_id: created.session_id };
  await api.call("harnix.attach", { ...s, space });
  await api.call("harnix.send", { ...s, space, message: "Say exactly: fixture ready" });
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
  test.afterAll(() => backend?.stop());
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
    const context = await api.call("harnix.system.context", s);
    expect(context.basis).toBe("next_step_without_new_input");
    expect(context.system.length).toBeGreaterThan(0);
    expect(context.messages.count).toBe(2);

    const history = await api.call("session.history", { ...s, space: "system" });
    expect(history.messages[0].origin).toBe("user");
    expect(await api.call("harnix.system.messages.edit", { ...s, at: 0, text: "Say exactly: edited", total: history.total })).toEqual({ edited: 0 });
    let after = await api.call("session.history", { ...s, space: "system" });
    expect(text(after.messages[0])).toBe("Say exactly: edited");

    // A count that is not the session's any more is a view that went stale.
    await api.refused("harnix.system.messages.drop", { ...s, at: [0], total: history.total + 1 });
    expect(await api.call("harnix.system.messages.drop", { ...s, at: [0], total: history.total })).toEqual({ dropped: [0] });
    after = await api.call("session.history", { ...s, space: "system" });
    expect(text(after.messages[0])).toContain("removed by the host");
    // The model's own message cannot be taken out.
    await api.refused("harnix.system.messages.drop", { ...s, at: [1], total: history.total });
  });

  test("operations on another space's session take effect where it belongs, and are audited", TAGS, async () => {
    const s = await oneTurn(api, "fleet");
    await api.call("harnix.system.rename", { ...s, name: "renamed from system" });
    const row = (await api.call("harnix.sessions", { space: "fleet" })).sessions.find((r: any) => r.session_id === s.session_id);
    expect(row.name).toBe("renamed from system");

    expect(await api.call("harnix.system.tasks.stop", { ...s, task_id: "no-such-task" })).toEqual({ stopped: false });
    await api.call("harnix.system.compact", s).catch(() => undefined);
    await api.call("harnix.system.clear", s);
    const context = await api.call("harnix.system.context", s);
    expect(context.messages.count).toBe(0);

    const { entries } = await api.call("harnix.audit", { limit: 200 });
    for (const method of ["harnix.system.rename", "harnix.system.clear"]) {
      expect(entries.some((e: any) => e.method === method && e.decision === "allow"), method).toBe(true);
    }
    // And what it asked of the engine, as the system module.
    expect(entries.some((e: any) => e.method === "session.clear" && JSON.stringify(e.subject).includes("system"))).toBe(true);
  });

  test("the system space lists every space's sessions and builds from any template", TAGS, async () => {
    const fleet = await oneTurn(api, "fleet");
    const all = (await api.call("harnix.sessions", { space: "system" })).sessions.map((r: any) => r.session_id);
    expect(all).toContain(fleet.session_id);
    const { templates } = await api.call("harnix.instances.templates", { space: "system" });
    const fromSessions = (await api.call("harnix.instances.templates", {})).templates;
    expect(templates.length).toBeGreaterThanOrEqual(fromSessions.length);
  });

  test("a session made in the system space is the system space's", TAGS, async () => {
    const created = await api.call("session.create", { space: "system", options: { recorder: { name: "says-ready" } } });
    expect(created.space).toBe("system");
    const listed = (await api.call("harnix.sessions", { space: "sessions" })).sessions.map((r: any) => r.session_id);
    expect(listed).not.toContain(created.session_id);
  });

  test("an instance restarts from where it was built; `system` does not", TAGS, async () => {
    expect((await api.refused("harnix.instances.restart", { instance: "system", space: "system" }))?.code).toBe(REFUSED);
    // Not the interface's to restart from anywhere but the system space.
    expect((await api.refused("harnix.instances.restart", { instance: "fleet-side" }))?.code).toBe(REFUSED);
    const restarted = await api.call("harnix.instances.restart", { instance: "fleet-side", space: "system" });
    expect(restarted.id).toBe("fleet-side");
    await until(api, "fleet-side", "ready");
    await oneTurn(api, "fleet");
    const { entries } = await api.call("harnix.audit", { limit: 200 });
    expect(entries.some((e: any) => e.method === "harnix.instances.restart" && e.decision === "allow")).toBe(true);
  });
});

test.describe("off", () => {
  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    backend = await startTestApp({ profile: "\n[system_space]\nenabled = false\n" });
  });
  test.afterAll(() => backend?.stop());
  test.beforeEach(async () => {
    api = await connect(backend);
  });
  test.afterEach(() => api?.close());

  test("the methods leave the authorization table, and the space cannot be claimed", TAGS, async () => {
    const { methods } = await api.call("harnix.reachable");
    for (const method of SYSTEM_METHODS) expect(methods, method).not.toContain(method);
    const created = await api.call("session.create", { space: "sessions" });
    const s = { instance: created.instance, session_id: created.session_id };
    const error = await api.refused("harnix.system.context", s);
    expect([METHOD_NOT_FOUND, REFUSED]).toContain(error?.code);

    expect((await api.refused("harnix.sessions", { space: "system" }))?.code).toBe(SPACE_MISMATCH);
    expect((await api.refused("harnix.attach", { ...s, space: "system" }))?.code).toBe(SPACE_MISMATCH);
    expect((await api.refused("harnix.instances.restart", { instance: "fleet-side", space: "system" }))?.code).toBe(SPACE_MISMATCH);
  });
});
