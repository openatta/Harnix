/**
 * Template instances that see the settings page's providers (ipdrun-11).
 *
 * The settings page writes providers into the `system` instance's global tier. An instance built
 * from a template that says `inherit_providers` has them too — at once, without being built again —
 * and loses them when they are deleted; one whose template does not say so sees only its own.
 */

import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { until } from "./instances";
import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:settings", "@layer:api"] };

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startTestApp({
    profile: `
[[instance_templates]]
id = "spawn-heir"
carrier = "spawn"
ui = true
inherit_providers = true
`,
  });
});
test.afterAll(async () => {
  await backend?.dispose();
});
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

/** The provider ids an instance's engine has, as it says. */
async function providerIds(instance: string): Promise<string[]> {
  const answer = await api.call("app.fleet.providers", { instance });
  return Object.keys(answer.providers ?? {});
}

const CONFIG = { api_type: "anthropic", api_key: "sk-inherited", default_model: "m-1" };

test("an inheriting instance sees a provider the moment it is set, and loses it when deleted", TAGS, async () => {
  await api.call("app.fleet.ensure_many", { template: "fleet.heir", name: "heir-1", count: 1 });
  await api.call("app.fleet.ensure_many", { template: "fleet.side", name: "plain-1", count: 1 });
  expect(await providerIds("heir-1")).not.toContain("hx-check");

  const added = await api.call("harnix.settings.setProvider", { provider_id: "hx-check", config: CONFIG });
  expect(added.reloaded_instances).toContain("heir-1");
  expect(added.reloaded_instances).not.toContain("plain-1");
  expect(JSON.stringify(added)).not.toContain("sk-inherited");
  expect(await providerIds("heir-1")).toContain("hx-check");
  // Not inheriting: its own, which is none.
  expect(await providerIds("plain-1")).not.toContain("hx-check");

  const removed = await api.call("harnix.settings.setProvider", { provider_id: "hx-check", delete: true });
  expect(removed.reloaded_instances).toContain("heir-1");
  expect(await providerIds("heir-1")).not.toContain("hx-check");
});

test("an inheriting instance built later starts with what is there", TAGS, async () => {
  await api.call("harnix.settings.setProvider", { provider_id: "hx-early", config: CONFIG });
  await api.call("app.fleet.ensure_many", { template: "fleet.heir", name: "heir-2", count: 1 });
  expect(await providerIds("heir-2")).toContain("hx-early");
  await api.call("harnix.settings.setProvider", { provider_id: "hx-early", delete: true });
});

test("a spawned inheriting instance does too", TAGS, async () => {
  await api.call("harnix.instances.ensure", { template: "spawn-heir", name: "heir-spawn" });
  await until(api, "heir-spawn", "ready");
  const added = await api.call("harnix.settings.setProvider", { provider_id: "hx-spawn", config: CONFIG });
  expect(added.reloaded_instances).toContain("heir-spawn");
  expect(await providerIds("heir-spawn")).toContain("hx-spawn");
  await api.call("harnix.settings.setProvider", { provider_id: "hx-spawn", delete: true });
  expect(await providerIds("heir-spawn")).not.toContain("hx-spawn");
});
