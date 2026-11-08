/**
 * Runtime applications, stage one: names that enter the authorization table after the process
 * started, and leave it again — no WASM yet. The testapp's `fakehost` module registers a fake
 * application through the kernel's app host (crates/testapp/src/fake_app.rs): a device calls its
 * methods through the same admission point as everything else, subscribes to its topic, and when
 * it is withdrawn the method is gone like one that never existed and the subscriber is told the
 * topic ended. With `[apps] enabled = false` (the default) none of it exists.
 */

import { INVALID_PARAMS, METHOD_NOT_FOUND, REFUSED } from "@harnix/core/protocol";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:apps", "@layer:api"] };

/** Install records as an installation leaves them, under `~/.harnix/apps/`. */
function seedRecords(scratch: string): void {
  const dir = join(scratch, "home", ".harnix", "apps");
  const record = (id: string, body: string) => {
    mkdirSync(join(dir, id), { recursive: true });
    writeFileSync(join(dir, id, "install.json"), body);
  };
  record("acme.notes", JSON.stringify({ id: "acme.notes", version: "1.2.0", title: "Notes" }));
  record("broken", "not json");
}

const audit = async (api: TestClient) => (await api.call("harnix.audit", { limit: 500 })).entries as any[];

test.describe("with [apps] enabled", () => {
  test.describe.configure({ mode: "serial" });

  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    backend = await startTestApp({ profile: "\n[apps]\nenabled = true\n", seed: seedRecords });
  });
  test.afterAll(async () => { await backend?.dispose(); });
  test.beforeEach(async () => {
    api = await connect(backend);
  });
  test.afterEach(() => api?.close());

  test("the install records are listed and switched on and off", TAGS, async () => {
    const listed = await api.call("harnix.apps.list");
    expect(listed.apps).toEqual([
      // A record written before signatures says nothing of them: unsigned, from a file.
      { id: "acme.notes", version: "1.2.0", title: "Notes", enabled: false, registered: false, trust: "unsigned", source: "file", data_bytes: 0 },
    ]);
    expect(listed.unreadable.map((u: any) => u.dir)).toEqual(["broken"]);
    expect(listed.registered).toEqual([]);

    expect(await api.call("harnix.apps.enable", { id: "acme.notes" })).toMatchObject({ id: "acme.notes", enabled: true });
    expect((await api.call("harnix.apps.list")).apps[0].enabled).toBe(true);
    expect(await api.call("harnix.apps.disable", { id: "acme.notes" })).toMatchObject({ enabled: false, withdrawn: false });
    expect((await api.call("harnix.apps.list")).apps[0].enabled).toBe(false);

    expect((await api.refused("harnix.apps.enable", { id: "nobody" }))?.code).toBe(INVALID_PARAMS);
    expect((await api.refused("harnix.apps.disable", { id: "../etc" }))?.code).toBe(INVALID_PARAMS);
    // Switching one is on the record.
    const switched = (await audit(api)).filter((e) => e.method === "harnix.apps.enable" && e.decision === "allow");
    expect(switched.length).toBeGreaterThan(0);
  });

  test("a build without the application runtime keeps the records and installs nothing", TAGS, async () => {
    // `harnix-testapp` is built without the `apps` feature.
    const error = await api.refused("harnix.apps.installFile", { path: "/nowhere/app.hxapp" });
    expect(error?.code).toBe(REFUSED);
    expect(error?.message).toContain("`apps` feature");
    expect((await api.call("harnix.apps.list")).runtime).toBeNull();
  });

  test("the host is handed to the apps built-in alone: another compiled-in module gets none", TAGS, async () => {
    // `probe` is an ordinary module of the test application; `fakehost` is let in by name
    // (`AppBuilder::app_host`), the way only a test application would.
    expect(await api.call("app.probe.has_app_host")).toEqual({ has: false });
  });

  test("a registered application's method answers a device, through the same door, on the record", TAGS, async () => {
    const spaces = await api.call("harnix.subscribe", { topic: "harnix.spaces" });
    expect(spaces.value.spaces.map((s: any) => s.id)).not.toContain("fake");

    const summary = await api.call("app.fakehost.register", { id: "fake" });
    expect(summary).toMatchObject({
      id: "fake",
      by: "fakehost",
      methods: ["app.fake.echo", "app.fake.slow", "app.fake.internal"],
      topics: ["app.fake.ticks"],
      spaces: ["fake"],
    });

    const answer = await api.call("app.fake.echo", { text: "hello" });
    expect(answer.params).toEqual({ text: "hello" });
    expect(answer.caller.kind).toBe("device");
    // Only who it was declared for.
    expect((await api.refused("app.fake.internal", {}))?.code).toBe(REFUSED);
    const { methods, topics } = await api.call("harnix.reachable");
    expect(methods).toContain("app.fake.echo");
    expect(methods).not.toContain("app.fake.internal");
    expect(topics).toContain("app.fake.ticks");

    // Its space is listed, and whoever watches the list is told.
    expect((await api.call("harnix.spaces.list")).spaces.map((s: any) => s.id)).toContain("fake");
    await api.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === "harnix.spaces" && f.params.value.spaces.some((s: any) => s.id === "fake"),
      { describe: "the spaces list with the application's space" },
    );
    expect((await api.call("harnix.apps.list")).registered.map((r: any) => r.id)).toEqual(["fake"]);

    // Who declared what, and when; and who called it.
    const entries = await audit(api);
    const declared = entries.filter((e) => e.decision === "declare" && e.subject === "module:fakehost");
    expect(declared.map((e) => e.method).sort()).toEqual(
      ["app.fake.echo", "app.fake.internal", "app.fake.slow", "space:fake", "topic:app.fake.ticks"],
    );
    expect(declared.every((e) => typeof e.at_ms === "number")).toBe(true);
    expect(entries.some((e) => e.method === "app.fake.echo" && e.decision === "allow" && e.subject.startsWith("device:"))).toBe(true);
    expect(entries.some((e) => e.method === "app.fake.internal" && e.decision === "refuse")).toBe(true);
  });

  test("its topic is subscribed to and published on", TAGS, async () => {
    const first = await api.call("harnix.subscribe", { topic: "app.fake.ticks" });
    expect(first.topic).toBe("app.fake.ticks");
    await api.call("app.fake.echo", { n: 1 });
    const pushed = await api.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === "app.fake.ticks" && f.params.value?.params?.n === 1,
      { describe: "the application's publish" },
    );
    expect(pushed.params.value).toEqual({ params: { n: 1 } });
  });

  test("withdrawn: a call in flight finishes, the next is not found, the subscriber is told", TAGS, async () => {
    await api.call("harnix.subscribe", { topic: "app.fake.ticks" });
    await api.call("harnix.subscribe", { topic: "harnix.spaces" });
    const watcher = await connect(backend);
    try {
      await watcher.call("harnix.subscribe", { topic: "app.fake.ticks" });
      const slow = api.call("app.fake.slow", { ms: 1500 });
      await new Promise((r) => setTimeout(r, 300));
      const withdrawn = await api.call("app.fakehost.unregister", { id: "fake" });
      expect(withdrawn).toMatchObject({ id: "fake", spaces: ["fake"], topics: ["app.fake.ticks"] });
      expect(withdrawn.subscriptions_ended).toBeGreaterThanOrEqual(2);
      expect(await slow).toEqual({ slept: 1500 });

      // Like a name nobody ever declared.
      const gone = await api.refused("app.fake.echo", {});
      expect(gone?.code).toBe(METHOD_NOT_FOUND);
      const { methods, topics } = await api.call("harnix.reachable");
      expect(methods).not.toContain("app.fake.echo");
      expect(topics).not.toContain("app.fake.ticks");
      for (const client of [api, watcher]) {
        const ended = await client.waitFor(
          (f) => f.method === "harnix.topic_ended" && f.params.topic === "app.fake.ticks",
          { describe: "the topic's end" },
        );
        expect(ended.params.reason).toContain("withdrawn");
      }
      expect((await api.refused("harnix.subscribe", { topic: "app.fake.ticks" }))?.code).toBe(METHOD_NOT_FOUND);
      expect((await api.call("harnix.spaces.list")).spaces.map((s: any) => s.id)).not.toContain("fake");
      await api.waitFor(
        (f) => f.method === "harnix.topic" && f.params.topic === "harnix.spaces" && !f.params.value.spaces.some((s: any) => s.id === "fake"),
        { describe: "the spaces list without the application's space" },
      );
      const withdrew = (await audit(api)).filter((e) => e.decision === "withdraw" && e.subject === "module:fakehost");
      expect(withdrew.map((e) => e.method).sort()).toEqual(
        ["app.fake.echo", "app.fake.internal", "app.fake.slow", "space:fake", "topic:app.fake.ticks"],
      );
      expect((await api.refused("app.fakehost.unregister", { id: "fake" }))?.code).toBe(INVALID_PARAMS);
    } finally {
      watcher.close();
    }
  });

  test("a name or id that collides with what is there is refused, and nothing changes", TAGS, async () => {
    // A compiled-in module's id, or one nested under it: its names keep precedence.
    for (const id of ["probe", "probe.sub", "fleet"]) {
      const error = await api.refused("app.fakehost.register", { id });
      expect(error?.code, id).toBe(REFUSED);
      expect(error?.message, id).toContain("compiled-in");
    }
    expect((await api.call("app.probe.echo", { text: "still mine" })).text).toBe("still mine");
    // Not an id at all.
    expect((await api.refused("app.fakehost.register", { id: "Not An Id" }))?.code).toBe(REFUSED);
    // Twice.
    await api.call("app.fakehost.register", { id: "twice" });
    expect((await api.refused("app.fakehost.register", { id: "twice" }))?.code).toBe(REFUSED);
    expect((await api.refused("app.fakehost.register", { id: "twice.inner" }))?.code).toBe(REFUSED);
    expect((await api.call("app.twice.echo", { x: 1 })).params).toEqual({ x: 1 });
    await api.call("app.fakehost.unregister", { id: "twice" });

    const refusals = (await audit(api)).filter((e) => e.decision === "refuse" && e.subject === "module:fakehost");
    expect(refusals.length).toBeGreaterThanOrEqual(6);
  });
});

test.describe("with [apps] off", () => {
  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    backend = await startTestApp({ seed: seedRecords });
  });
  test.afterAll(async () => { await backend?.dispose(); });
  test.beforeEach(async () => {
    api = await connect(backend);
  });
  test.afterEach(() => api?.close());

  test("nothing of it exists: no harnix.apps.*, no host, no spaces topic", TAGS, async () => {
    for (const method of ["harnix.apps.list", "harnix.apps.enable", "harnix.apps.disable", "harnix.apps.installFile", "harnix.apps.uninstall"]) {
      expect((await api.refused(method, { id: "acme.notes" }))?.code, method).toBe(METHOD_NOT_FOUND);
    }
    const { methods, topics } = await api.call("harnix.reachable");
    expect(methods.filter((m: string) => m.startsWith("harnix.apps."))).toEqual([]);
    expect(topics).not.toContain("harnix.spaces");
    expect((await api.refused("harnix.subscribe", { topic: "harnix.spaces" }))?.code).toBe(METHOD_NOT_FOUND);
    const error = await api.refused("app.fakehost.register", { id: "fake" });
    expect(error?.code).toBe(REFUSED);
    expect(error?.message).toContain("[apps] enabled = false");
    expect((await api.refused("app.fake.echo", {}))?.code).toBe(METHOD_NOT_FOUND);
  });
});
