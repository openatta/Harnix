/**
 * `harnix.inbox` (harnix-ext.md §21.3, harnix-app.md §3.3): every ask a session is waiting on a
 * person for, with the session's space, name and project and the ask as the engine put it; the
 * same as a topic, pushed again when it is answered; within the space a call claims, unless that
 * one reaches across spaces. Replayed (`asks-permission`): the turn waits on one Bash permission.
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

const mine = (value: any, session_id: string) => ((value?.items ?? []) as any[]).filter((i) => i.session_id === session_id);

test("a waiting permission is in the inbox with its session, held to the space claimed, and gone once answered", TAGS, async () => {
  const { session_id, space } = await api.call("harnix.session.create", {
    project_root: api.project,
    options: { recorder: { name: "asks-permission" }, permission_mode: "default" },
  });
  try {
    const before = await api.call("harnix.topics.subscribe", { topic: "harnix.inbox" });
    expect(mine(before.value, session_id)).toEqual([]);
    await api.call("harnix.session.attach", { session_id });
    await api.call("harnix.session.send", { session_id, message: "Use the Bash tool to run exactly: touch harnix-permission-fixture" });
    const ask = (
      await api.waitFor((f) => f.method === "harnix.event" && f.params.session_id === session_id && f.params.event.kind === "prompt", { timeout: 30_000, describe: "the ask" })
    ).params.event;

    // Pushed to whoever watches the topic, and answered by the method alike.
    await api.waitFor((f) => f.method === "harnix.topic" && f.params.topic === "harnix.inbox" && mine(f.params.value, session_id).length === 1, {
      timeout: 10_000,
      describe: "the inbox to list the ask",
    });
    const [item] = mine(await api.call("harnix.inbox", {}), session_id);
    expect(item).toMatchObject({ instance: "main", space, waiting: "approval", project_root: api.project, started_by: "user" });
    expect(item.prompt.prompt_id).toBe(ask.prompt_id);
    expect(item.prompt.tool_name).toBe("Bash");
    expect(item.trust, "the product's own engine").toBeUndefined();

    // Its own space sees it, the system space reaches across, another space does not.
    expect(mine(await api.call("harnix.inbox", { space }), session_id)).toHaveLength(1);
    expect(mine(await api.call("harnix.inbox", { space: "system" }), session_id)).toHaveLength(1);
    expect(mine(await api.call("harnix.inbox", { space: "flow" }), session_id)).toHaveLength(0);

    // Answered: it waits on nobody, and the topic says so.
    await api.call("harnix.session.respond", { session_id, prompt_id: ask.prompt_id, decision: { type: "permit" } });
    await api.waitFor((f) => f.method === "harnix.topic" && f.params.topic === "harnix.inbox" && mine(f.params.value, session_id).length === 0, {
      timeout: 15_000,
      describe: "the inbox to drop the answered ask",
    });
    expect(mine(await api.call("harnix.inbox", {}), session_id)).toEqual([]);
  } finally {
    await finish(api, session_id);
  }
});
