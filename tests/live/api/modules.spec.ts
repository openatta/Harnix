/**
 * The module contract (design §7), through a real application binary.
 *
 * A module's methods are rows in the authorization table with the
 * principals it declared: nothing is open by default, a device reaches only
 * what was declared for devices, and one module reaches another only by
 * name. A module that fails anywhere is disabled alone.
 */

import { REFUSED, METHOD_NOT_FOUND, INVALID_PARAMS } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { moduleStatus, startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:modules", "@layer:api"] };

let backend: Backend;
let api: TestClient;

test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(() => backend?.stop());
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

test("a method declared for devices answers a device", TAGS, async () => {
  const answer = await api.call("app.probe.echo", { text: "hello" });
  expect(answer.text).toBe("hello");
  expect(answer.from.kind).toBe("device");
});

test("its params are checked against its type", TAGS, async () => {
  const error = await api.refused("app.probe.echo", { nope: 1 });
  expect(error?.code).toBe(INVALID_PARAMS);
});

test("a method declared only for the kernel is refused to a device", TAGS, async () => {
  const error = await api.refused("app.probe.internal", {});
  expect(error?.code).toBe(REFUSED);
  const { methods } = await api.call("harnix.reachable");
  expect(methods).toContain("app.probe.echo");
  expect(methods).not.toContain("app.probe.internal");
  expect(methods).not.toContain("app.probe.for_peer");
});

test("one module reaches another only where it was named", TAGS, async () => {
  // peer → probe.for_peer: declared for Module(peer).
  const allowed = await api.call("app.peer.relay", { method: "app.probe.for_peer" });
  expect(allowed.caller).toEqual({ kind: "module", id: "peer" });
  // peer → probe.internal: the kernel's only.
  const refused = await api.refused("app.peer.relay", { method: "app.probe.internal" });
  expect(refused?.code).toBe(REFUSED);
  // probe → probe.for_peer: declared for peer, not for probe itself.
  const self = await api.refused("app.probe.call", { method: "app.probe.for_peer", params: {} });
  expect(self?.code).toBe(REFUSED);
});

test("a module calls the kernel's methods through the same door", TAGS, async () => {
  const sessions = await api.call("app.probe.call", { method: "harnix.sessions", params: {} });
  expect(Array.isArray(sessions.sessions)).toBe(true);
  // And what the hub owns stays the hub's, for a module too.
  const turn = await api.refused("app.probe.call", { method: "session.run_turn", params: {} });
  expect(turn?.code).toBe(REFUSED);
  // A device may not write a provider; the settings module was granted it,
  // this one was not.
  const provider = await api.refused("app.probe.call", { method: "config.setProvider", params: {} });
  expect(provider?.code).toBe(REFUSED);
});

test("config comes from its section, secrets through the credential source", TAGS, async () => {
  const config = await api.call("app.probe.config");
  expect(config.greeting).toBe("hi there");
  expect(config.key).toEqual({ Ok: "12 chars" });
  expect(config.data_dir).toMatch(/modules[\\/]probe$/);
});

test("a failed start disables that module and nothing else", TAGS, async () => {
  const status = await moduleStatus(api, "fails-start");
  expect(status.enabled).toBe(false);
  expect(status.phase).toBe("start");
  expect(status.reason).toContain("on purpose");
  const error = await api.refused("app.fails-start.ping", {});
  expect(error?.code).toBe(METHOD_NOT_FOUND);
  // Everything registered after it is up.
  expect((await moduleStatus(api, "peer")).enabled).toBe(true);
});

test("a second module with a taken id is refused at registration", TAGS, async () => {
  const report = await api.call("harnix.diagnostics");
  const second = report.modules.find((m: any) => m.id === "probe" && m.version === "9.9.9");
  expect(second.enabled).toBe(false);
  expect(second.phase).toBe("registration");
  // The first one's method is still the first one's.
  expect((await api.call("app.probe.echo", { text: "x" })).text).toBe("x");
});

test("the diagnostics page says what is there and what is not", TAGS, async () => {
  const report = await api.call("harnix.diagnostics");
  expect(report.versions.module_api).toBe(1);
  expect(report.versions.app.name).toBe("harnix-testapp");
  expect(report.versions.agix.version).toBeTruthy();
  expect(report.unknown_config_sections).toEqual(["nobody"]);
  const settings = report.modules.find((m: any) => m.id === "settings");
  expect(settings.namespace).toBe("kernel");
  expect(settings.grants).toContain("config.setProvider");
  const probe = await moduleStatus(api, "probe");
  const echo = probe.methods.find((m: any) => m.name === "app.probe.echo");
  expect(echo.principals).toEqual(["device"]);
  expect(echo.schema.required).toEqual(["text"]);
});

test("a topic hands a subscriber the current value, then every new one", TAGS, async () => {
  const first = await api.call("harnix.subscribe", { topic: "app.probe.counter" });
  expect(first.value).toEqual({ n: 0 });
  await api.call("app.probe.bump");
  const pushed = await api.waitFor(
    (f) => f.method === "harnix.topic" && f.params.topic === "app.probe.counter" && f.params.value.n === 1,
    { describe: "the counter's new value" },
  );
  expect(pushed.params.value).toEqual({ n: 1 });
});

test("a parameterised topic is one value per parameter set", TAGS, async () => {
  const missing = await api.refused("harnix.subscribe", { topic: "app.probe.job" });
  expect(missing?.code).toBe(INVALID_PARAMS);
  await api.call("harnix.subscribe", { topic: "app.probe.job", params: { job_id: "a" } });
  await api.call("app.probe.bump", { job_id: "b" });
  await api.call("app.probe.bump", { job_id: "a" });
  const seen = await api.waitFor(
    (f) => f.method === "harnix.topic" && f.params.topic === "app.probe.job",
    { describe: "job a" },
  );
  expect(seen.params.params).toEqual({ job_id: "a" });
  expect(api.events.filter((f) => f.method === "harnix.topic" && f.params.params?.job_id === "b")).toEqual([]);
});

test("a topic declared for the kernel is not a device's to subscribe to", TAGS, async () => {
  const error = await api.refused("harnix.subscribe", { topic: "app.probe.secret" });
  expect(error?.code).toBe(REFUSED);
  const unknown = await api.refused("harnix.subscribe", { topic: "app.probe.nothing" });
  expect(unknown?.code).toBe(METHOD_NOT_FOUND);
  const { topics } = await api.call("harnix.reachable");
  expect(topics).toContain("app.probe.counter");
  expect(topics).not.toContain("app.probe.secret");
});
