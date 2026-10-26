/**
 * The protocol's shape from 0.4 on (design §5.6, roadmap M2-8): a session
 * is `(instance, session)`. Methods take `instance` (absent means `system`),
 * frames always carry it, and an instance that does not exist is refused by
 * name rather than quietly meaning `system`.
 */

import { INSTANCE_NOT_FOUND } from "@harnix/core/protocol";
import { type Backend, expect, finish, launchFor, startHarnix, connect, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

// Replaying in every mode: what is under test is the frames' shape, and a
// recorded turn produces every kind of frame a turn produces.
let backend: Backend;
test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(() => backend?.stop());

test("every frame of a turn names its instance", TAGS, async () => {
  const api = await connect(backend);
  const created = await api.call("session.create", {
    scene: "coding",
    project_root: api.project,
    options: { recorder: { name: "says-ready" } },
  });
  expect(created.instance).toBe("system");
  const sessionId = created.session_id;
  try {
    const attached = await api.call("harnix.attach", { instance: "system", session_id: sessionId });
    expect(attached.instance).toBe("system");
    await api.call("harnix.send", { session_id: sessionId, message: "Say exactly: fixture ready" });
    await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === sessionId, {
      timeout: 60_000,
      describe: "the replayed turn to settle",
    });
    const frames = api.events.filter((f) => f.params?.session_id === sessionId);
    expect(frames.length).toBeGreaterThan(2);
    for (const frame of frames) expect(frame.params.instance, frame.method).toBe("system");
    const { sessions } = await api.call("harnix.sessions");
    const row = sessions.find((s: any) => s.session_id === sessionId);
    expect(row.instance).toBe("system");
  } finally {
    await finish(api, sessionId);
    api.close();
  }
});

test("an instance that does not exist is refused by name", TAGS, async () => {
  const api = await connect(backend);
  try {
    for (const [method, params] of [
      ["harnix.sessions", { instance: "nowhere" }],
      ["harnix.attach", { instance: "nowhere", session_id: "s" }],
      ["harnix.send", { instance: "nowhere", session_id: "s", message: "hi" }],
      ["session.create", { instance: "nowhere" }],
      ["session.get", { instance: "nowhere", session_id: "s" }],
    ] as const) {
      const error = await api.refused(method, params);
      expect(error?.code, method).toBe(INSTANCE_NOT_FOUND);
      expect(error.message).toContain("nowhere");
    }
  } finally {
    api.close();
  }
});
