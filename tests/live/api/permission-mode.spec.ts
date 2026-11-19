/**
 * A session's permission mode, changed while it lives (`harnix.session.setPermissionMode`,
 * harnix-app.md §4): the engine takes it at once, holds it to the settings, says the mode the
 * session is in and whether it narrowed it, and tells every watcher; the hub keeps what it heard
 * on the session's row and its attach. No model call is made: nothing here needs a recording.
 */

import { INVALID_PARAMS, SPACE_MISMATCH } from "@harnix/core/protocol";
import { type Backend, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

test.describe.configure({ mode: "serial" });

let backend: Backend;
test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => {
  await backend?.dispose();
});

/** The session's row in its instance's list. */
async function row(api: Awaited<ReturnType<typeof connect>>, s: { instance: string; session_id: string }) {
  const listed = await api.call("harnix.sessions.list", { instance: s.instance });
  return listed.sessions.find((r: { session_id: string }) => r.session_id === s.session_id);
}

test("a device changes its own session's mode, and every watcher hears it", TAGS, async () => {
  const api = await connect(backend);
  const watcher = await connect(backend);
  try {
    expect((await api.call("harnix.reachable")).methods).toContain("harnix.session.setPermissionMode");
    const created = await api.call("harnix.session.create", { space: "sessions", options: { permission_mode: "default" } });
    const s = { instance: created.instance, session_id: created.session_id, space: "sessions" };
    // Opened: the mode it is in, read from the engine.
    const attached = await watcher.call("harnix.session.attach", s);
    expect(attached.session.permission_mode).toBe("default");

    const set = await api.call("harnix.session.setPermissionMode", { ...s, mode: "plan" });
    expect(set).toEqual({ instance: s.instance, session_id: s.session_id, mode: "plan", requested: "plan", clamped: false });
    const heard = await watcher.waitFor(
      (f) => f.method === "harnix.event" && f.params.session_id === s.session_id && f.params.event?.kind === "permission_mode_changed",
      { describe: "the change, told to a watcher" },
    );
    expect(heard.params.event).toMatchObject({ mode: "plan", source: "host" });
    expect((await row(api, s))?.permission_mode).toBe("plan");
    // Opened again: what the hub heard.
    expect((await api.call("harnix.session.attach", s)).session.permission_mode).toBe("plan");

    // Out of plan again, back to asking.
    expect((await api.call("harnix.session.setPermissionMode", { ...s, mode: "default" })).mode).toBe("default");
  } finally {
    api.close();
    watcher.close();
  }
});

test("a mode wider than the settings allow is held to them, and the answer says so", TAGS, async () => {
  const api = await connect(backend);
  try {
    await api.call("settings.write", { tier: "global", changes: { permission_mode: "default", allow_client_permission_override: false } });
    const created = await api.call("harnix.session.create", { space: "sessions" });
    const s = { instance: created.instance, session_id: created.session_id, space: "sessions" };
    const set = await api.call("harnix.session.setPermissionMode", { ...s, mode: "bypassPermissions" });
    expect(set).toMatchObject({ mode: "default", requested: "bypassPermissions", clamped: true });
    expect((await row(api, s))?.permission_mode).toBe("default");
  } finally {
    await api.call("settings.write", { tier: "global", changes: { permission_mode: null, allow_client_permission_override: null } }).catch(() => undefined);
    api.close();
  }
});

test("only a session of the space claimed, and only a mode the engine takes from a host", TAGS, async () => {
  const api = await connect(backend);
  try {
    const created = await api.call("harnix.session.create", { space: "sessions" });
    const s = { instance: created.instance, session_id: created.session_id };
    expect((await api.refused("harnix.session.setPermissionMode", { ...s, space: "flow", mode: "plan" }))?.code).toBe(SPACE_MISMATCH);
    expect((await api.refused("harnix.session.setPermissionMode", { ...s, space: "sessions" }))?.code).toBe(INVALID_PARAMS);
    // The engine's own: a program's mode, not a host's to set.
    expect((await api.refused("harnix.session.setPermissionMode", { ...s, space: "sessions", mode: "bubble" }))?.code).toBe(INVALID_PARAMS);
    expect((await api.refused("harnix.session.setPermissionMode", { ...s, space: "sessions", mode: "plann" }))?.code).toBe(INVALID_PARAMS);
  } finally {
    api.close();
  }
});
