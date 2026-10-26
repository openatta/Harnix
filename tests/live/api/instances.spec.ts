/**
 * The instance layer (design §2, §5): `system`, an instance in this process
 * and one in a daemon, side by side — each with sessions of its own and
 * nothing of the others'.
 *
 * Instances come from two places only. The profile declares some; the rest
 * are built from templates, and the interface may build only from a
 * template opened to it, filling only the parameters it opens. What was
 * built is in the ledger, and a restart builds it again.
 */

import { INVALID_PARAMS, REFUSED } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { INSTANCES_PROFILE, instances, runOn, until } from "./instances";
import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:instances", "@layer:api"] };

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startTestApp({ profile: INSTANCES_PROFILE });
});
test.afterAll(() => backend?.stop());
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

test("the instances are listed with where they came from", TAGS, async () => {
  await until(api, "ci", "ready");
  const all = await instances(api);
  expect(all.system).toMatchObject({ carrier: "in_process", owner: "kernel", template: null });
  expect(all.side).toMatchObject({ carrier: "in_process", owner: "kernel", scenes: ["chat"] });
  expect(all.ci).toMatchObject({ carrier: "spawn", owner: "kernel" });
  expect(all.ci.pid).toBeGreaterThan(0);
  expect(all.ci.protocol_version).toBe(all.system.protocol_version);
  // The module's own, built from its template at start.
  expect(all["fleet-side"]).toMatchObject({ carrier: "in_process", owner: "module:fleet", template: "fleet.side" });
  const roots = Object.values(all).map((i: any) => i.data_root);
  expect(new Set(roots).size, "every instance has a data root of its own").toBe(roots.length);

  const hello = await api.call("harnix.hello");
  expect(hello.permission_prompt_timeout_secs).toBeGreaterThan(0);
  const ids = hello.instances.map((i: any) => i.id);
  expect(ids).toEqual(expect.arrayContaining(["system", "side", "ci", "fleet-side"]));
  for (const i of hello.instances) expect(i.protocol_version, i.id).toBe(all.system.protocol_version);
});

test("each instance runs its own sessions and sees none of the others'", TAGS, async () => {
  await until(api, "ci", "ready");
  const ran: Record<string, string> = {};
  for (const [instance, scene] of [["system", "coding"], ["side", "chat"], ["ci", "coding"]]) {
    ran[instance] = await runOn(api, instance, scene);
    expect(api.text(ran[instance])).toMatch(/fixture ready/i);
  }
  for (const [instance, mine] of Object.entries(ran)) {
    const { sessions } = await api.call("harnix.instances.sessions", { instance });
    const ids = sessions.map((s: any) => s.session_id);
    expect(ids, instance).toContain(mine);
    for (const [other, theirs] of Object.entries(ran)) {
      if (other === instance) continue;
      expect(ids, `${instance} lists ${other}'s session`).not.toContain(theirs);
      const error = await api.refused("session.get", { instance, session_id: theirs });
      expect(error, `${instance} answers for ${other}'s session`).not.toBeNull();
    }
  }
  // Configuration is the instance's too: `side` runs only the scene its
  // profile entry names.
  const sideScenes = (await api.call("scene.list", { instance: "side" })).scenes.map((s: any) => s.id ?? s);
  const systemScenes = (await api.call("scene.list", { instance: "system" })).scenes.map((s: any) => s.id ?? s);
  expect(JSON.stringify(sideScenes)).not.toEqual(JSON.stringify(systemScenes));
});

test("the interface builds only from a template opened to it", TAGS, async () => {
  const { templates } = await api.call("harnix.instances.templates");
  expect(templates.map((t: any) => t.id)).toEqual(["worker"]);

  let error = await api.refused("harnix.instances.create", { template: "fleet.side", name: "mine" });
  expect(error?.code).toBe(REFUSED);
  error = await api.refused("harnix.instances.create", {
    template: "worker", name: "w0", params: { team: "a", program: "/bin/sh" },
  });
  expect(error?.code).toBe(INVALID_PARAMS);
  expect(error.message).toContain("program");
  error = await api.refused("harnix.instances.create", { template: "worker", name: "w0" });
  expect(error?.code).toBe(INVALID_PARAMS);
  error = await api.refused("harnix.instances.create", { template: "worker", name: "../w0", params: { team: "a" } });
  expect(error?.code).toBe(INVALID_PARAMS);
  error = await api.refused("harnix.instances.create", { template: "worker", name: "w0", params: { team: ".." } });
  expect(error?.code).toBe(INVALID_PARAMS);

  const built = await api.call("harnix.instances.create", { template: "worker", name: "w1", params: { team: "a" } });
  expect(built).toMatchObject({ id: "w1", owner: "space:sessions", template: "worker", carrier: "spawn" });
  expect(built.data_root).toMatch(/workers\/a\/w1$/);
  await api.call("harnix.instances.create", { template: "worker", name: "w2", params: { team: "b" } });
  error = await api.refused("harnix.instances.create", { template: "worker", name: "w3", params: { team: "c" } });
  expect(error?.code, "the template allows two").toBe(REFUSED);
  await until(api, "w1", "ready");
  await runOn(api, "w1");
});

test("a device stops what it built, and nothing it did not", TAGS, async () => {
  for (const id of ["system", "side", "ci", "fleet-side"]) {
    const error = await api.refused("harnix.instances.stop", { instance: id });
    expect(error?.code, id).toBe(REFUSED);
  }
  await api.call("harnix.instances.stop", { instance: "w2" });
  expect(Object.keys(await instances(api))).not.toContain("w2");
});

test("the instance tree is pushed whole", TAGS, async () => {
  const first = await api.call("harnix.subscribe", { topic: "harnix.instances" });
  expect(first.value.instances.map((i: any) => i.id)).toContain("system");
  const sessions = await api.call("harnix.subscribe", { topic: "harnix.instances.sessions", params: { instance: "side" } });
  expect(sessions.value.instance).toBe("side");
  const status = await api.call("harnix.subscribe", { topic: "harnix.sessions.status" });
  expect(status.value.sessions).toEqual([]);

  const sessionId = await runOn(api, "side", "chat");
  await api.waitFor(
    (f) => f.method === "harnix.topic" && f.params.topic === "harnix.instances.sessions"
      && f.params.value.sessions.some((s: any) => s.session_id === sessionId),
    { describe: "side's session list to include the new session" },
  );
  const running = api.events.filter(
    (f) => f.method === "harnix.topic" && f.params.topic === "harnix.sessions.status"
      && f.params.value.sessions.some((s: any) => s.session_id === sessionId && s.state === "running"),
  );
  expect(running.length, "the session was shown running while its turn ran").toBeGreaterThan(0);
  await api.waitFor(
    (f) => f.method === "harnix.topic" && f.params.topic === "harnix.sessions.status"
      && !f.params.value.sessions.some((s: any) => s.session_id === sessionId),
    { describe: "the session to be shown idle again" },
  );

  await api.call("harnix.instances.create", { template: "worker", name: "w2", params: { team: "b" } });
  await api.waitFor(
    (f) => f.method === "harnix.topic" && f.params.topic === "harnix.instances"
      && f.params.value.instances.some((i: any) => i.id === "w2"),
    { describe: "the instance list to include w2" },
  );
});

test("a restart builds again what the ledger records", TAGS, async () => {
  const before = await runOn(api, "side", "chat");
  api.close();
  backend = await backend.restart();
  api = await connect(backend);
  const all = await instances(api);
  expect(all.w1).toMatchObject({ owner: "space:sessions", template: "worker", params: { team: "a" } });
  expect(all.w2).toMatchObject({ owner: "space:sessions", template: "worker", params: { team: "b" } });
  expect(all["fleet-side"]).toMatchObject({ owner: "module:fleet" });
  await until(api, "w1", "ready");
  // Sessions are in the instance's data root, so they are still there.
  const { sessions } = await api.call("harnix.instances.sessions", { instance: "side" });
  expect(sessions.map((s: any) => s.session_id)).toContain(before);
  await api.call("harnix.attach", { instance: "side", session_id: before });
});
