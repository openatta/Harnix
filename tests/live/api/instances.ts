/**
 * What the instance suites share: a profile with one instance of each
 * carrier and a template the interface may use, and ways to wait on them.
 */

import { type TestClient, expect, sleep } from "@harnix/testkit";

export const INSTANCES_PROFILE = `
[[instances]]
id = "side"
carrier = "in_process"
[instances.engine]
scene = "chat"

[[instances]]
id = "ci"
carrier = "spawn"

[[instance_templates]]
id = "worker"
carrier = "spawn"
ui = true
params = ["team"]
data_root = "workers/{team}/{name}"
max = 2
`;

/** Every instance, by id. */
export async function instances(api: TestClient): Promise<Record<string, any>> {
  const { instances } = await api.call("harnix.instances.list");
  return Object.fromEntries(instances.map((i: any) => [i.id, i]));
}

/** Wait until an instance is in a state, or fail saying which it was in. */
export async function until(api: TestClient, id: string, state: string, timeout = 60_000): Promise<any> {
  const deadline = Date.now() + timeout;
  let last: any;
  while (Date.now() < deadline) {
    last = (await instances(api))[id];
    if (last?.status?.state === state) return last;
    await sleep(100);
  }
  throw new Error(`instance ${id} did not become ${state}: ${JSON.stringify(last)}`);
}

/** Create a session on an instance, run the `says-ready` fixture, wait for it. */
export async function runOn(api: TestClient, instance: string, scene = "coding"): Promise<string> {
  const created = await api.call("session.create", {
    instance,
    scene,
    project_root: api.project,
    options: { recorder: { name: "says-ready" } },
  });
  expect(created.instance).toBe(instance);
  const sessionId = created.session_id;
  await api.call("harnix.attach", { instance, session_id: sessionId });
  await api.call("harnix.send", { instance, session_id: sessionId, message: "Say exactly: fixture ready" });
  const settled = await api.waitFor(
    (f) => f.method === "harnix.turn_settled" && f.params.session_id === sessionId && f.params.instance === instance,
    { timeout: 60_000, describe: `the turn on ${instance} to settle` },
  );
  expect(settled.params.error, JSON.stringify(settled.params.error)).toBeUndefined();
  return sessionId;
}
