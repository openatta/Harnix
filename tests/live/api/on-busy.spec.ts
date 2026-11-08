/**
 * What a send does to a session that is busy (`harnix.send {on_busy}`): `interrupt` stops the
 * running turn, drops what was queued behind it, and runs the new message next.
 *
 * The busy turn is `asks-permission` held at its ask, so it is still running when the second send
 * arrives — replayed on a backend of its own, whatever mode the run is in: against a live model
 * there may be no ask to hold it.
 */

import { type Backend, type TestClient, connect, expect, finish, launchFor, startHarnix, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => { await backend?.dispose(); });
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

test("interrupt stops the running turn, drops the queue behind it, and runs next", TAGS, async () => {
  const { session_id } = await api.call("session.create", {
    project_root: api.project,
    options: { recorder: { name: "asks-permission" }, permission_mode: "default" },
  });
  try {
    await api.call("harnix.attach", { session_id });
    const first = await api.call("harnix.send", { session_id, message: "Use the Bash tool to run: echo permission-check" });
    await api.waitFor(
      (f) => f.method === "harnix.event" && f.params.session_id === session_id && f.params.event.kind === "prompt",
      { timeout: 30_000, describe: "the first turn to stop at its ask" },
    );
    const queued = await api.call("harnix.send", { session_id, message: "queued behind it", on_busy: "queue" });
    expect(queued.queued).toBe(true);

    const mark = api.events.length;
    const next = await api.call("harnix.send", { session_id, message: "instead of all that", on_busy: "interrupt" });
    expect(next.queued).toBe(true);

    const settled = await api.waitFor(
      (f) => f.method === "harnix.turn_settled" && f.params.session_id === session_id && f.params.turn_id === first.turn_id,
      { timeout: 30_000, describe: "the interrupted turn to settle" },
    );
    expect(settled.params.turn_id).toBe(first.turn_id);
    // What was queued is gone; the interrupting message went in front and runs next.
    const snapshots = api.events.slice(mark).filter((f) => f.method === "harnix.queue" && f.params.session_id === session_id);
    expect(snapshots.length).toBeGreaterThan(0);
    expect(snapshots[0].params.items.map((i: any) => i.message ?? i.text)).not.toContain("queued behind it");
    const started = await api.waitFor(
      (f) => f.method === "harnix.event" && f.params.session_id === session_id
        && f.params.event.kind === "user_message" && f.params.event.text === "instead of all that",
      { timeout: 30_000, describe: "the interrupting message to start its turn" },
    );
    expect(started.params.turn_id).not.toBe(first.turn_id);
    expect(api.events.some((f) => f.method === "harnix.event" && f.params.event?.kind === "user_message"
      && f.params.event.text === "queued behind it"), "the dropped message ran").toBe(false);
  } finally {
    await finish(api, session_id);
  }
});

test("an on_busy that is not one of the four is refused before anything is sent", TAGS, async () => {
  const { session_id } = await api.call("session.create", {});
  try {
    const error = await api.refused("harnix.send", { session_id, message: "x", on_busy: "later" });
    expect(error?.message).toMatch(/queue, reject, interrupt or steer/);
  } finally {
    await finish(api, session_id);
  }
});
