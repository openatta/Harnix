/**
 * Runtime applications from a store: the catalog, installing by name with a disclosure and a
 * consent, the signed showcase running without dev mode, an update that asks for more and so asks
 * again, what is not what was signed refused — a signature, a file, a download, an engine — and
 * uninstalling taking everything with it. Against a mock store (`apps-store.ts`) that only
 * `harnix-testapp` built with `apps` trusts; then the same store against the apps binary, whose
 * built-in store key is not the mock's, so nothing from it installs — and a trusted publisher's
 * package that does.
 */

import { existsSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { INVALID_PARAMS, REFUSED } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, sleep, test, topology } from "@harnix/testkit";

import { SHOWCASE_ID, appsTools, showcasePackage } from "./apps-host";
import { MOCK_PUBLISHER, type MockStore, appsTestApp, mockStore, sha256, startStore, withArtifacts } from "./apps-store";

const TAGS = { tag: ["@feature:apps-store", "@layer:api"] };
const TOOL = (name: string) => `app__example_showcase__${name}`;
const OTHER_ENGINE = "wasmtime-0.0.0+elsewhere+000000000000";

type Session = { instance: string; session_id: string };

async function session(api: TestClient, recording: string): Promise<Session> {
  const created = await api.call("session.create", {
    instance: "system",
    project_root: api.project,
    options: { recorder: { name: recording } },
  });
  const s = { instance: "system", session_id: created.session_id };
  await api.call("harnix.attach", s);
  return s;
}

/** Send, permit every ask, and wait for the turn to settle. */
async function turn(api: TestClient, s: Session, message: string): Promise<any> {
  const sent = await api.call("harnix.send", { ...s, message });
  const answered = new Set<string>();
  const deadline = Date.now() + 90_000;
  for (;;) {
    const settled = api.events.find(
      (f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id && (!sent.turn_id || f.params.turn_id === sent.turn_id),
    );
    if (settled) return settled;
    for (const f of api.events) {
      if (f.method !== "harnix.event" || f.params.event?.kind !== "prompt" || f.params.session_id !== s.session_id) continue;
      const id = f.params.event.prompt_id;
      if (answered.has(id)) continue;
      answered.add(id);
      await api.call("session.respondToPrompt", { ...s, prompt_id: id, decision: { type: "permit" } });
    }
    if (Date.now() > deadline) throw new Error(`the turn "${message}" did not settle`);
    await sleep(50);
  }
}

const results = (api: TestClient, s: Session) => JSON.stringify(api.eventsOfKind("tool_result", s.session_id));

/** Install by name as a person would: the disclosure, then the consent. */
async function installByName(api: TestClient, version?: string): Promise<{ disclosed: any; installed: any }> {
  const disclosed = await api.call("harnix.apps.install", { id: SHOWCASE_ID, version });
  expect(disclosed.consent).toBe("required");
  const installed = await api.call("harnix.apps.install", { id: SHOWCASE_ID, version, consent: disclosed.digest });
  return { disclosed, installed };
}

test.describe("the store, trusted", () => {
  test.describe.configure({ mode: "serial" });

  let store: MockStore;
  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    appsTestApp();
    store = mockStore();
    backend = await startStore(store);
  });
  test.afterAll(async () => {
    await backend?.dispose();
    if (store) rmSync(store.dir, { recursive: true, force: true });
  });
  test.beforeEach(async () => {
    api = await connect(backend, { topology: topology() });
  });
  test.afterEach(() => {
    store?.write(store.index);
    api?.close();
  });

  test("the catalog lists what the store has, for this engine", TAGS, async () => {
    const { fingerprint } = showcasePackage();
    const catalog = await api.call("harnix.apps.catalog");
    expect(catalog).toMatchObject({ name: "Mock store", store: store.url, fingerprint });
    expect(catalog.apps).toHaveLength(1);
    const app = catalog.apps[0];
    expect(app).toMatchObject({ id: SHOWCASE_ID, title: "Showcase", publisher: "example", latest: "0.2.0", installable: true });
    expect(app.installed).toBeUndefined();
    expect(app.versions.map((v: any) => v.version)).toEqual(["0.2.0", "0.1.0"]);
    expect(app.versions[0].engines).toEqual([fingerprint]);
    expect(app.capabilities.env).toEqual(["SHOWCASE_TOKEN", "SHOWCASE_EXTRA"]);
    // Nothing is installed by looking.
    expect((await api.call("harnix.apps.list")).apps).toEqual([]);
  });

  test("what is not what was signed is refused, and nothing of it stays", TAGS, async () => {
    const original = store.packages["0.1.0"];
    const cases: [string, () => any, string][] = [
      [
        "a bad countersignature",
        () => {
          const path = store.derive("0.1.0", "bad-signature", (e) => {
            const s = JSON.parse(e.get("SIGNATURES")!.toString());
            s.store.signature = (s.store.signature.startsWith("00") ? "11" : "00") + s.store.signature.slice(2);
            e.set("SIGNATURES", Buffer.from(JSON.stringify(s)));
          });
          return withArtifacts(store.index, "0.1.0", { [showcasePackage().fingerprint]: { url: "packages/bad-signature.hxapp", sha256: sha256(path) } });
        },
        "the store's countersignature does not verify",
      ],
      [
        "a bad publisher signature",
        () => {
          const path = store.derive("0.1.0", "bad-publisher", (e) => {
            const s = JSON.parse(e.get("SIGNATURES")!.toString());
            s.publisher.signature = "00".repeat(64);
            e.set("SIGNATURES", Buffer.from(JSON.stringify(s)));
          });
          return withArtifacts(store.index, "0.1.0", { [showcasePackage().fingerprint]: { url: "packages/bad-publisher.hxapp", sha256: sha256(path) } });
        },
        "the publisher's signature does not verify",
      ],
      [
        "a file that is not the one signed",
        () => {
          const path = store.derive("0.1.0", "tampered", (e) => {
            e.set("ui/showcase.js", Buffer.from(`${e.get("ui/showcase.js")!.toString()}\n// and something else\n`));
          });
          return withArtifacts(store.index, "0.1.0", { [showcasePackage().fingerprint]: { url: "packages/tampered.hxapp", sha256: sha256(path) } });
        },
        "`ui/showcase.js` is not the file that was signed",
      ],
      [
        "a download that is not the one listed",
        () => withArtifacts(store.index, "0.1.0", { [showcasePackage().fingerprint]: { url: "packages/example.showcase-0.2.0.hxapp", sha256: sha256(original) } }),
        "is not the one the store lists",
      ],
      [
        "no build for this engine",
        () => withArtifacts(store.index, "0.1.0", { [OTHER_ENGINE]: { url: "packages/example.showcase-0.1.0.hxapp", sha256: sha256(original) } }),
        "has no build for this Harnix's engine",
      ],
    ];
    for (const [name, make, says] of cases) {
      store.write(make());
      const error = await api.refused("harnix.apps.install", { id: SHOWCASE_ID, version: "0.1.0" });
      expect(error?.code, name).toBe(REFUSED);
      expect(error?.message, name).toContain(says);
      if (name === "no build for this engine") {
        expect(error?.message).toContain(showcasePackage().fingerprint);
        expect(error?.message).toContain(OTHER_ENGINE);
        expect(error?.message).toContain("upgrade Harnix");
      }
      expect((await api.call("harnix.apps.list")).apps, name).toEqual([]);
      expect(existsSync(join(backend.state, "apps", SHOWCASE_ID)), name).toBe(false);
    }
    expect((await api.refused("harnix.apps.install", { id: "example.nobody" }))?.message).toContain("no application `example.nobody`");
    expect((await api.refused("harnix.apps.install", { id: SHOWCASE_ID, version: "9.9.9" }))?.message).toContain("no version 9.9.9");
    expect((await api.refused("harnix.apps.install", {}))?.code).toBe(INVALID_PARAMS);
    expect((await api.refused("harnix.apps.update", { id: SHOWCASE_ID }))?.message).toContain("no application");
    const plugins = (await api.call("plugin.list")).plugins.map((p: any) => p.name);
    expect(plugins).not.toContain("example-showcase");
  });

  test("installed by name: disclosed first, and installed only on that consent", TAGS, async () => {
    const disclosed = await api.call("harnix.apps.install", { id: SHOWCASE_ID, version: "0.1.0" });
    expect(disclosed).toMatchObject({ id: SHOWCASE_ID, version: "0.1.0", consent: "required", changed: false });
    expect(disclosed.digest).toMatch(/^[0-9a-f]{64}$/);
    expect(disclosed.disclosure).toMatchObject({
      id: SHOWCASE_ID,
      publisher: "example",
      trust: "store",
      publisher_key: MOCK_PUBLISHER,
      capabilities: {
        storage_mb: 1,
        fs: ["${project}"],
        net: ["127.0.0.1"],
        env: ["SHOWCASE_TOKEN"],
        hub: ["session.create", "harnix.send", "topic:harnix.sessions.status"],
      },
      component: { world: "harnix:app/backend-app@0.1.0" },
      ui: [
        { point: "home.card", module: "showcase.js" },
        { point: "space", module: "showcase.js" },
      ],
      agent: { package: "example-showcase", install_into: ["system"] },
    });
    expect(disclosed.disclosure.lines.map((l: any) => l.capability)).toEqual([
      "net",
      "env",
      "storage_mb",
      "hub",
      "hub",
      "hub",
      "fs",
      "ui",
      "ui",
      "agent",
    ]);
    // Disclosed, and nothing done.
    expect((await api.call("harnix.apps.list")).apps).toEqual([]);

    // A consent to something else is no consent: disclosed again.
    const again = await api.call("harnix.apps.install", { id: SHOWCASE_ID, version: "0.1.0", consent: "0".repeat(64) });
    expect(again).toMatchObject({ consent: "required", changed: true, digest: disclosed.digest });
    expect((await api.call("harnix.apps.list")).apps).toEqual([]);

    const installed = await api.call("harnix.apps.install", { id: SHOWCASE_ID, version: "0.1.0", consent: disclosed.digest });
    expect(installed).toMatchObject({ id: SHOWCASE_ID, version: "0.1.0", enabled: true, trust: "store", source: "store" });
    const listed = (await api.call("harnix.apps.list")).apps;
    expect(listed).toHaveLength(1);
    expect(listed[0]).toMatchObject({
      id: SHOWCASE_ID,
      version: "0.1.0",
      trust: "store",
      source: "store",
      runtime: { state: "running" },
      app: { publisher: "example", capabilities: { storage_mb: 1 } },
    });
    expect(listed[0].data_bytes).toBeGreaterThanOrEqual(0);
    expect((await api.call("harnix.apps.list")).runtime).toMatchObject({ dev: false });
    expect((await api.call("harnix.apps.catalog")).apps[0].installed).toBe("0.1.0");
    expect((await api.refused("harnix.apps.install", { id: SHOWCASE_ID }))?.message).toContain("installed already");
    // The download it was installed from is gone.
    const staging = join(backend.state, "apps", ".staging");
    expect(existsSync(staging) ? readdirSync(staging) : []).toEqual([]);
  });

  test("the signed showcase runs without dev mode", TAGS, async () => {
    const echo = await session(api, "app-echo");
    const settled = await turn(api, echo, "Use the showcase's echo tool");
    expect(settled.params.error, JSON.stringify(settled.params.error)).toBeUndefined();
    expect(results(api, echo)).toContain("showcase-echo-42");
    const count = await session(api, "app-count");
    await turn(api, count, "count");
    expect(results(api, count)).toContain("count = 1");
    expect((await api.call("harnix.apps.list")).apps[0].data_bytes, "the count is kept").toBeGreaterThan(0);
  });

  test("an update that asks for more lists only that, and waits for consent", TAGS, async () => {
    const asked = await api.call("harnix.apps.update", { id: SHOWCASE_ID });
    expect(asked).toMatchObject({ id: SHOWCASE_ID, version: "0.2.0", consent: "required" });
    expect(asked.disclosure.update).toMatchObject({ from: "0.1.0", needs_consent: true });
    expect(asked.disclosure.update.added).toEqual([
      { capability: "net", value: "api.example.com" },
      { capability: "env", value: "SHOWCASE_EXTRA" },
      { capability: "storage_mb", value: 2, from: 1 },
    ]);
    // Not yet.
    expect((await api.call("harnix.apps.list")).apps[0].version).toBe("0.1.0");
    // Consent to installing it is not consent to this update.
    const fresh = await api.refused("harnix.apps.install", { id: SHOWCASE_ID, consent: asked.digest });
    expect(fresh?.message).toContain("installed already");

    const updated = await api.call("harnix.apps.update", { id: SHOWCASE_ID, consent: asked.digest });
    expect(updated).toMatchObject({ id: SHOWCASE_ID, version: "0.2.0", from: "0.1.0", trust: "store", enabled: true });
    const listed = (await api.call("harnix.apps.list")).apps[0];
    expect(listed).toMatchObject({ version: "0.2.0", trust: "store", runtime: { state: "running" } });
    expect(listed.app.capabilities.env).toEqual(["SHOWCASE_TOKEN", "SHOWCASE_EXTRA"]);
    expect(existsSync(join(backend.state, "apps", SHOWCASE_ID, "0.1.0")), "the old version's files").toBe(false);
    // Its data came along: the count goes on.
    const count = await session(api, "app-count");
    await turn(api, count, "count");
    expect(results(api, count)).toContain("count = 2");
    // Nothing newer: nothing to do.
    expect(await api.call("harnix.apps.update", { id: SHOWCASE_ID })).toMatchObject({ up_to_date: true, version: "0.2.0" });
  });

  test("uninstalled by name: everything it brought goes, its data with it", TAGS, async () => {
    const removed = await api.call("harnix.apps.uninstall", { id: SHOWCASE_ID });
    expect(removed).toMatchObject({ id: SHOWCASE_ID, uninstalled: true, removed: { agent: { package: "example-showcase" }, skills: { system: ["showcase-tour"] } } });
    expect((await api.call("harnix.apps.list")).apps).toEqual([]);
    expect((await api.call("plugin.list")).plugins.map((p: any) => p.name)).not.toContain("example-showcase");
    expect(JSON.stringify(await api.call("commands.list", { instance: "system" }))).not.toContain("showcase-tour");
    expect(existsSync(join(backend.state, "apps", SHOWCASE_ID)), "its directory, kv and data").toBe(false);
    expect((await fetch(`${backend.url.replace(/\/$/, "")}/apps/${SHOWCASE_ID}/ui/showcase.js`)).status).toBe(404);
    const s = await session(api, "says-ready");
    const context = JSON.stringify(await api.call("harnix.system.context", s));
    expect(context).not.toContain(TOOL("echo"));
    // Installed again, it starts from nothing: the count with it.
    await installByName(api, "0.1.0");
    const count = await session(api, "app-count");
    await turn(api, count, "count");
    expect(results(api, count)).toContain("count = 1");
    await api.call("harnix.apps.uninstall", { id: SHOWCASE_ID });
  });
});

test.describe("the store, on a Harnix with only the store's own key", () => {
  test.describe.configure({ mode: "serial" });

  let store: MockStore;
  let backend: Backend;
  let trusting: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    store = mockStore();
    const { bin } = appsTools();
    backend = await startStore(store, { bin });
    trusting = await startStore(store, {
      bin,
      profile: `\n[apps.trusted_publishers]\nexample = ["${MOCK_PUBLISHER}"]\n`,
    });
  });
  test.afterAll(async () => {
    await backend?.dispose();
    await trusting?.dispose();
    if (store) rmSync(store.dir, { recursive: true, force: true });
  });
  test.afterEach(() => api?.close());

  test("the mock store's countersignature authorizes nothing here", TAGS, async () => {
    api = await connect(backend, { topology: topology() });
    // It may be read; nothing from it installs.
    expect((await api.call("harnix.apps.catalog")).apps[0].id).toBe(SHOWCASE_ID);
    const error = await api.refused("harnix.apps.install", { id: SHOWCASE_ID, version: "0.1.0" });
    expect(error?.code).toBe(REFUSED);
    expect(error?.message).toContain("a store this Harnix does not trust");
    expect((await api.refused("harnix.apps.installFile", { path: store.packages["0.1.0"] }))?.message).toContain(
      "a store this Harnix does not trust",
    );
    // Signed by a publisher this profile does not trust, either.
    expect((await api.refused("harnix.apps.installFile", { path: store.publisherOnly }))?.message).toContain("not one the profile trusts");
    // And unsigned, without dev mode, as before.
    expect((await api.refused("harnix.apps.installFile", { path: showcasePackage().path }))?.message).toContain("[apps] dev = true");
    expect((await api.call("harnix.apps.list")).apps).toEqual([]);
  });

  test("a publisher the profile trusts installs and runs without the store or dev mode", TAGS, async () => {
    api = await connect(trusting, { topology: topology() });
    const installed = await api.call("harnix.apps.installFile", { path: store.publisherOnly });
    expect(installed).toMatchObject({ id: SHOWCASE_ID, trust: "trusted", source: "file" });
    expect((await api.call("harnix.apps.list")).apps[0]).toMatchObject({ trust: "trusted", runtime: { state: "running" } });
    const echo = await session(api, "app-echo");
    await turn(api, echo, "Use the showcase's echo tool");
    expect(results(api, echo)).toContain("showcase-echo-42");
    // A record that says it is trusted is checked again at every start: a changed file stops it.
    await api.call("harnix.apps.disable", { id: SHOWCASE_ID });
    const root = join(trusting.state, "apps", SHOWCASE_ID, "0.1.0");
    const module = join(root, "ui", "showcase.js");
    writeFileSync(module, `${readFileSync(module, "utf8")}\n// changed on disk\n`);
    const error = await api.refused("harnix.apps.enable", { id: SHOWCASE_ID });
    expect(error?.message).toContain("`ui/showcase.js` is not the file that was signed");
    expect((await api.call("harnix.apps.list")).apps[0]).toMatchObject({ enabled: false, runtime: { state: "failed" } });
    await api.call("harnix.apps.uninstall", { id: SHOWCASE_ID });
  });
});
