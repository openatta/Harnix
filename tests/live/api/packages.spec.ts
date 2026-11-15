/**
 * Extension packages, underneath the carriers: what is installed (the records under
 * `<state>/ext/<id>/install.json`), and names that enter the route table after the process
 * started and leave it again — no WebAssembly. The test application's `fakehost` registers a fake
 * package through the host's registrar (`HostServices`, which only an extension granted
 * `extensions.manage` is handed; crates/testapp/src/fake_app.rs): a device calls its methods
 * through the same admission point as everything else, subscribes to its topic, and when it is
 * withdrawn the method is gone like one that never existed and the subscriber is told the topic
 * ended. A build without the `packages` extension (the test application's default) has no
 * `packages.*`, leaves the records as they are, and still has the registrar for whoever holds it.
 */

import { INVALID_PARAMS, METHOD_NOT_FOUND, REFUSED } from "@harnix/core/protocol";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { type Backend, type TestClient, carrier, connect, expect, test } from "@harnix/testkit";

import { startApps } from "./packages-host";
import { appsTestApp } from "./packages-store";
import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:packages", "@layer:api"] };

/** Where the packages are kept, under the backend's `HOME`: `~/.harnix/ext/`. */
const extDir = (scratch: string) => join(scratch, "home", ".harnix", "ext");

/** Install records as an installation leaves them, one that does not read, and a native
 *  extension's directory (no package id), which is not a package's. */
function seedRecords(scratch: string): void {
  const record = (id: string, body: string) => {
    mkdirSync(join(extDir(scratch), id), { recursive: true });
    writeFileSync(join(extDir(scratch), id, "install.json"), body);
  };
  record("acme.notes", JSON.stringify({ id: "acme.notes", version: "1.2.0", title: "Notes" }));
  record("acme.broken", "not json");
  mkdirSync(join(extDir(scratch), "probe"), { recursive: true });
}

const audit = async (api: TestClient) => (await api.call("harnix.audit", { limit: 500 })).entries as any[];

test.describe("with the packages extension", () => {
  test.describe.configure({ mode: "serial" });

  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    backend = await startApps({ bin: appsTestApp(), seed: seedRecords });
  });
  test.afterAll(async () => { await backend?.dispose(); });
  test.beforeEach(async () => {
    api = await connect(backend);
  });
  test.afterEach(() => api?.close());

  test("the install records are listed, and one that does not read is listed apart", TAGS, async () => {
    const listed = await api.call("packages.list");
    expect(listed.packages).toEqual([
      expect.objectContaining({ id: "acme.notes", version: "1.2.0", title: "Notes", enabled: false, registered: false, trust: "unsigned" }),
    ]);
    expect(listed.unreadable.map((u: any) => u.dir)).toEqual(["acme.broken"]);
    expect(listed.registered).toEqual([]);
    expect(listed.runtime).toMatchObject({ carrier: expect.any(String) });

    expect((await api.refused("packages.enable", { id: "nobody.here" }))?.code).toBe(INVALID_PARAMS);
    expect((await api.refused("packages.disable", { id: "../etc" }))?.code).toBe(INVALID_PARAMS);
  });

  test("one whose files are gone does not start, and stays off", TAGS, async () => {
    // A record without its package: switched on, it says what is missing, and nothing is
    // registered.
    const e = await api.refused("packages.enable", { id: "acme.notes" });
    expect(e?.code).toBe(REFUSED);
    expect(e?.message).toContain("its files are not there");
    const notes = (await api.call("packages.list")).packages.find((p: any) => p.id === "acme.notes");
    expect(notes).toMatchObject({ enabled: false, registered: false });
    expect(await api.call("packages.disable", { id: "acme.notes" })).toMatchObject({ enabled: false });
    // Switching one is on the record, refused or not.
    const switched = (await audit(api)).filter((e) => e.method === "packages.enable" || e.method === "packages.disable");
    expect(switched.length).toBeGreaterThan(0);
  });

  test("the host's registrar is handed only to an extension granted extensions.manage", TAGS, async () => {
    const { extensions } = await api.call("harnix.extensions");
    const managers = extensions.filter((x: any) => (x.granted ?? []).includes("extensions.manage")).map((x: any) => x.id).sort();
    expect(managers).toEqual(["fakehost", "packages"]);
  });

  test("a registered package's method answers a device, through the same door, on the record", TAGS, async () => {
    const spaces = await api.call("harnix.topics.subscribe", { topic: "harnix.spaces" });
    expect(spaces.value.spaces.map((s: any) => s.id)).not.toContain("acme.fake");

    const summary = await api.call("fakehost.register", { id: "acme.fake" });
    expect(summary).toMatchObject({
      id: "acme.fake",
      by: "fakehost",
      methods: ["acme.fake.echo", "acme.fake.slow", "acme.fake.internal"],
      topics: ["acme.fake.ticks"],
      spaces: ["acme.fake"],
    });

    const answer = await api.call("acme.fake.echo", { text: "hello" });
    expect(answer.params).toEqual({ text: "hello" });
    expect(answer.caller.kind).toBe("device");
    // Only who it was declared for.
    expect((await api.refused("acme.fake.internal", {}))?.code).toBe(REFUSED);
    const { methods, topics } = await api.call("harnix.reachable");
    expect(methods).toContain("acme.fake.echo");
    expect(methods).not.toContain("acme.fake.internal");
    expect(topics).toContain("acme.fake.ticks");

    // Its space is listed, and whoever watches the list is told.
    expect((await api.call("harnix.spaces.list")).spaces.map((s: any) => s.id)).toContain("acme.fake");
    await api.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === "harnix.spaces" && f.params.value.spaces.some((s: any) => s.id === "acme.fake"),
      { describe: "the spaces list with the package's space" },
    );
    expect((await api.call("packages.list")).registered.map((r: any) => r.id)).toEqual(["acme.fake"]);

    // Who declared what, and when; and who called it.
    const entries = await audit(api);
    const declared = entries.filter((e) => e.decision === "declare" && e.subject === "ext:fakehost");
    expect(declared.map((e) => e.method).sort()).toEqual(
      ["acme.fake.echo", "acme.fake.internal", "acme.fake.slow", "space:acme.fake", "topic:acme.fake.ticks"],
    );
    expect(declared.every((e) => typeof e.at_ms === "number")).toBe(true);
    expect(entries.some((e) => e.method === "acme.fake.echo" && e.decision === "allow" && e.subject.startsWith("device:"))).toBe(true);
    expect(entries.some((e) => e.method === "acme.fake.internal" && e.decision === "refuse")).toBe(true);
  });

  test("its topic is subscribed to and published on", TAGS, async () => {
    const first = await api.call("harnix.topics.subscribe", { topic: "acme.fake.ticks" });
    expect(first.topic).toBe("acme.fake.ticks");
    await api.call("acme.fake.echo", { n: 1 });
    const pushed = await api.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === "acme.fake.ticks" && f.params.value?.params?.n === 1,
      { describe: "the package's publish" },
    );
    expect(pushed.params.value).toEqual({ params: { n: 1 } });
  });

  test("withdrawn: a call in flight finishes, the next is not found, the subscriber is told", TAGS, async () => {
    await api.call("harnix.topics.subscribe", { topic: "acme.fake.ticks" });
    await api.call("harnix.topics.subscribe", { topic: "harnix.spaces" });
    const watcher = await connect(backend);
    try {
      await watcher.call("harnix.topics.subscribe", { topic: "acme.fake.ticks" });
      const slow = api.call("acme.fake.slow", { ms: 1500 });
      await new Promise((r) => setTimeout(r, 300));
      const withdrawn = await api.call("fakehost.unregister", { id: "acme.fake" });
      expect(withdrawn).toMatchObject({ spaces: ["acme.fake"], topics: ["acme.fake.ticks"] });
      expect(withdrawn.subscriptions_ended).toBeGreaterThanOrEqual(2);
      expect(await slow).toEqual({ slept: 1500 });

      // Like a name nobody ever declared.
      const gone = await api.refused("acme.fake.echo", {});
      expect(gone?.code).toBe(METHOD_NOT_FOUND);
      const { methods, topics } = await api.call("harnix.reachable");
      expect(methods).not.toContain("acme.fake.echo");
      expect(topics).not.toContain("acme.fake.ticks");
      for (const client of [api, watcher]) {
        const ended = await client.waitFor(
          (f) => f.method === "harnix.topic_ended" && f.params.topic === "acme.fake.ticks",
          { describe: "the topic's end" },
        );
        expect(ended.params.reason).toContain("withdrawn");
      }
      expect((await api.refused("harnix.topics.subscribe", { topic: "acme.fake.ticks" }))?.code).toBe(METHOD_NOT_FOUND);
      expect((await api.call("harnix.spaces.list")).spaces.map((s: any) => s.id)).not.toContain("acme.fake");
      await api.waitFor(
        (f) => f.method === "harnix.topic" && f.params.topic === "harnix.spaces" && !f.params.value.spaces.some((s: any) => s.id === "acme.fake"),
        { describe: "the spaces list without the package's space" },
      );
      const withdrew = (await audit(api)).filter((e) => e.decision === "withdraw" && e.subject === "ext:fakehost");
      expect(withdrew.map((e) => e.method).sort()).toEqual(
        ["acme.fake.echo", "acme.fake.internal", "acme.fake.slow", "space:acme.fake", "topic:acme.fake.ticks"],
      );
      expect((await api.refused("fakehost.unregister", { id: "acme.fake" }))?.code).toBe(INVALID_PARAMS);
    } finally {
      watcher.close();
    }
  });

  test("an id that collides with what is there, or is no package's, is refused, and nothing changes", TAGS, async () => {
    // A package id has two parts (one part, or not an id at all, is no package's), and its first
    // is no native extension's: their names keep precedence.
    for (const id of ["probe", "fleet", "Not An Id"]) {
      expect((await api.refused("fakehost.register", { id }))?.code, id).toBe(INVALID_PARAMS);
    }
    expect((await api.refused("fakehost.register", { id: "probe.sub" }))?.code).toBe(REFUSED);
    expect((await api.call("probe.echo", { text: "still mine" })).text).toBe("still mine");
    // Twice.
    await api.call("fakehost.register", { id: "acme.twice" });
    expect((await api.refused("fakehost.register", { id: "acme.twice" }))?.code).toBe(REFUSED);
    expect((await api.call("acme.twice.echo", { x: 1 })).params).toEqual({ x: 1 });
    await api.call("fakehost.unregister", { id: "acme.twice" });
  });
});

test.describe("without the packages extension", () => {
  // No package runs here: another carrier would run the same thing again.
  test.skip(carrier() === "process", "no package runs without the packages extension");
  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    // The test application's default build: no carrier, so no `packages`.
    backend = await startTestApp({ seed: seedRecords });
  });
  test.afterAll(async () => { await backend?.dispose(); });
  test.beforeEach(async () => {
    api = await connect(backend);
  });
  test.afterEach(() => api?.close());

  test("there is no packages.*, and the records stay as they are", TAGS, async () => {
    for (const method of ["packages.list", "packages.enable", "packages.disable", "packages.installFile", "packages.uninstall"]) {
      expect((await api.refused(method, { id: "acme.notes" }))?.code, method).toBe(METHOD_NOT_FOUND);
    }
    const { methods } = await api.call("harnix.reachable");
    expect(methods.filter((m: string) => m.startsWith("packages."))).toEqual([]);
    expect((await api.call("harnix.extensions")).extensions.map((x: any) => x.id)).not.toContain("packages");
    const record = JSON.parse(readFileSync(join(backend.state, "ext", "acme.notes", "install.json"), "utf8"));
    expect(record).toEqual({ id: "acme.notes", version: "1.2.0", title: "Notes" });
  });

  test("the host's registrar is still there for whoever was granted it", TAGS, async () => {
    expect(await api.call("fakehost.register", { id: "acme.fake" })).toMatchObject({ id: "acme.fake", by: "fakehost" });
    expect((await api.call("acme.fake.echo", { text: "hi" })).params).toEqual({ text: "hi" });
    await api.call("fakehost.unregister", { id: "acme.fake" });
    expect((await api.refused("acme.fake.echo", {}))?.code).toBe(METHOD_NOT_FOUND);
  });
});
