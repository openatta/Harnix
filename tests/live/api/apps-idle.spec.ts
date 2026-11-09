/**
 * Runtime applications that idle: `[backend] resident = "on-demand"`. Its backend is stopped once
 * nothing holds it — no subscription, no timer, no call in flight — for `idle_secs`; the next
 * method call starts it again, `init` and all, with what it kept in its key-value store and none
 * of what it held in memory. Hibernating is listed (`harnix.apps.list`, the `harnix.apps` topic),
 * is not a fault and is not a recycle; two calls at once on a hibernating backend start one
 * instance; an application with a subscription or a timer stays up.
 *
 * The showcase, made on-demand in the test (its `watch_sessions` off, so `init` subscribes to
 * nothing), on the apps-enabled test application: a profile cannot make `idle_secs` shorter than a
 * minute, and only a test application can (`HARNIX_TESTAPP_APP_IDLE_MS`, through a hidden
 * `AppBuilder` knob).
 */

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { REFUSED } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, test, topology } from "@harnix/testkit";

import { SHOWCASE_ID, showcasePackage, startApps, variant } from "./apps-host";
import { appsTestApp } from "./apps-store";

const TAGS = { tag: ["@feature:apps-idle", "@layer:api"] };

const M = (name: string) => `app.${SHOWCASE_ID}.${name}`;
/** What the test application idles an on-demand backend for. */
const IDLE_MS = 1500;

const listed = async (api: TestClient) => (await api.call("harnix.apps.list")).apps.find((a: any) => a.id === SHOWCASE_ID);
const stateOf = async (api: TestClient) => (await listed(api))?.runtime?.state;

/** The showcase as an on-demand backend; `watch` keeps its subscription in `init`. */
function onDemand(scratch: string, name: string, watch = false, extra = ""): string {
  return variant(join(scratch, `${name}.hxapp`), (e) => {
    let manifest = e.get("harnix-app.toml")!.toString();
    if (!watch) manifest = manifest.replace("watch_sessions = true", "watch_sessions = false");
    manifest += `\n[backend]\nresident = "on-demand"\n${extra}`;
    e.set("harnix-app.toml", Buffer.from(manifest));
  });
}

async function until(what: string, f: () => Promise<boolean>, ms = 20_000): Promise<void> {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    if (await f()) return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`never: ${what}`);
}

test.describe("an application whose backend idles", () => {
  test.describe.configure({ mode: "serial" });

  let backend: Backend;
  let api: TestClient;
  let scratch: string;

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    showcasePackage();
    scratch = mkdtempSync(join(tmpdir(), "hx-idle-"));
    backend = await startApps({ bin: appsTestApp(), env: { HARNIX_TESTAPP_APP_IDLE_MS: String(IDLE_MS) } });
  });
  test.afterAll(async () => {
    await backend?.dispose();
    if (scratch) rmSync(scratch, { recursive: true, force: true });
  });
  test.beforeEach(async () => {
    api = await connect(backend, { topology: topology() });
  });
  test.afterEach(async () => {
    if ((await api.call("harnix.apps.list")).apps.some((a: any) => a.id === SHOWCASE_ID)) {
      await api.call("harnix.apps.uninstall", { id: SHOWCASE_ID });
    }
    api?.close();
  });

  test("[backend] that does not hold together is refused by name", TAGS, async () => {
    const cases: [string, string, string][] = [
      ["too-short", "`[backend] idle_secs` is 60", "idle_secs = 5\n"],
      ["unknown-mode", "`[backend] resident`", 'resident = "sometimes"\n'],
    ];
    for (const [name, says, extra] of cases) {
      const path = variant(join(scratch, `${name}.hxapp`), (e) => {
        const manifest = e.get("harnix-app.toml")!.toString();
        const backendSection = extra.startsWith("resident") ? `\n[backend]\n${extra}` : `\n[backend]\nresident = "on-demand"\n${extra}`;
        e.set("harnix-app.toml", Buffer.from(manifest + backendSection));
      });
      const error = await api.refused("harnix.apps.installFile", { path });
      expect(error?.code, name).toBe(REFUSED);
      expect(error?.message, name).toContain(says);
    }
    expect((await api.call("harnix.apps.list")).apps).toEqual([]);
  });

  test("idle, it hibernates; a call starts it again, its store kept and its memory not", TAGS, async () => {
    await api.call("harnix.apps.installFile", { path: onDemand(scratch, "idle") });
    await api.call("harnix.subscribe", { topic: "harnix.apps" });
    expect(await stateOf(api)).toBe("running");
    await api.call(M("notes.add"), { text: "kept in the store" });
    expect((await api.call(M("counter.bump"))).counter).toBe(1);
    const before = await api.call(M("state"));

    await until("hibernating", async () => (await stateOf(api)) === "hibernating");
    const app = await listed(api);
    expect(app.runtime).toMatchObject({ state: "hibernating", resident: "on-demand" });
    expect(app.runtime.health).toMatchObject({ hibernating: true, recycles: 0, faults: 0 });
    // Told on the topic too.
    await api.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === "harnix.apps" && f.params.value?.apps?.some((a: any) => a.id === SHOWCASE_ID && a.runtime?.state === "hibernating"),
      { describe: "harnix.apps saying it hibernates" },
    );

    // A call wakes it: `init` again, the notes from the store, the counter from nothing.
    const after = await api.call(M("state"));
    expect(after.started_ms).toBeGreaterThan(before.started_ms);
    expect(after.notes).toBe(1);
    expect(after.counter).toBe(0);
    const woken = (await listed(api)).runtime;
    expect(woken.state).toBe("running");
    expect(woken.health).toMatchObject({ hibernating: false, wakes: 1, recycles: 0, faults: 0 });
  });

  test("two calls at once on a hibernating backend start one instance", TAGS, async () => {
    await api.call("harnix.apps.installFile", { path: onDemand(scratch, "twice") });
    await until("hibernating", async () => (await stateOf(api)) === "hibernating");
    const [a, b] = await Promise.all([api.call(M("state")), api.call(M("state"))]);
    expect(a.started_ms).toBe(b.started_ms);
    expect((await listed(api)).runtime.health.wakes).toBe(1);
  });

  test("a subscription keeps it up; a timer keeps it up while it is armed", TAGS, async () => {
    // Subscribed in `init`: never hibernates.
    await api.call("harnix.apps.installFile", { path: onDemand(scratch, "watching", true) });
    expect((await listed(api)).runtime.health.subscriptions).toBe(1);
    await new Promise((r) => setTimeout(r, IDLE_MS * 3));
    expect(await stateOf(api)).toBe("running");
    await api.call("harnix.apps.uninstall", { id: SHOWCASE_ID });

    // A timer firing four times, a second apart: up while it is armed, hibernating after.
    await api.call("harnix.apps.installFile", { path: onDemand(scratch, "timed") });
    await api.call(M("timer.start"), { ms: 1000, times: 4 });
    const started = Date.now();
    await until("the last heartbeat", async () => (await api.call(M("state"))).ticks >= 4, 15_000);
    expect(Date.now() - started).toBeGreaterThan(3000);
    expect((await listed(api)).runtime.health.wakes).toBe(0);
    await until("hibernating once the timer is done", async () => (await stateOf(api)) === "hibernating");
  });

  test("disabled while it hibernates, it is off; enabled, it runs with its store", TAGS, async () => {
    await api.call("harnix.apps.installFile", { path: onDemand(scratch, "switched") });
    await api.call(M("notes.add"), { text: "before it slept" });
    await until("hibernating", async () => (await stateOf(api)) === "hibernating");
    await api.call("harnix.apps.disable", { id: SHOWCASE_ID });
    expect(await stateOf(api)).toBe("stopped");
    expect((await api.refused(M("state")))?.message).toBeTruthy();
    await api.call("harnix.apps.enable", { id: SHOWCASE_ID });
    expect(await stateOf(api)).toBe("running");
    expect((await api.call(M("notes.list"))).notes.map((n: any) => n.text)).toEqual(["before it slept"]);
  });

  test("resident by default: the showcase as it ships never hibernates", TAGS, async () => {
    await api.call("harnix.apps.installFile", { path: showcasePackage().path });
    await new Promise((r) => setTimeout(r, IDLE_MS * 3));
    const app = await listed(api);
    expect(app.runtime).toMatchObject({ state: "running", resident: "always" });
    expect(app.runtime.health.hibernating).toBe(false);
  });
});
