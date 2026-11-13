/**
 * An application's interface in isolated frames, the backend's half: who signed a package decides
 * what the page is told of its interface at hello — a trusted publisher's `[[ui]]` modules, which
 * the page imports, or everyone else's `[[surface]]`s, which the shell draws in sandboxed frames
 * (with the module that would have run in the page listed as not running). The frames' pages are
 * served under a policy of their own; the page's own allows frames of its origin and nothing
 * else; under `[ext.packages] dev` a change to an unsigned application's files moves `runtime.ui_rev`,
 * which the page reloads its frames on. A manifest that asks for a frame what is the shell's own,
 * or names a page it does not carry, is refused at install; anything else it lists, the router
 * narrows call by call. (The browser's half is
 * `ui/packages-frames.spec.ts`.)
 */

import { execFileSync } from "node:child_process";
import { appendFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { SHOWCASE_CHATS, SHOWCASE_ID, appsTools, showcasePackage, startApps, variant } from "./packages-host";
import { MOCK_PUBLISHER, STORE_FIXTURE } from "./packages-store";

const TAGS = { tag: ["@feature:packages", "@layer:api"] };
const base = (b: Backend) => b.url.replace(/\/$/, "");

const surfaces = (hello: any) => (hello.contributions ?? []).filter((c: any) => c.plugin === SHOWCASE_ID && c.frame !== undefined);
const modules = (hello: any) => (hello.contributions ?? []).filter((c: any) => c.plugin === SHOWCASE_ID && typeof c.module === "string");

let scratch: string;
test.beforeAll(() => {
  scratch = mkdtempSync(join(tmpdir(), "hx-frames-api-"));
});
test.afterAll(() => {
  if (scratch) rmSync(scratch, { recursive: true, force: true });
});

/** The showcase with its manifest edited, `signed` by the mock publisher or not. */
function edited(name: string, edit: (manifest: string) => string, signed = false): string {
  const plain = variant(join(scratch, `${name}.hxext`), (entries) => {
    entries.set("harnix-ext.toml", Buffer.from(edit(entries.get("harnix-ext.toml")!.toString())));
  });
  if (!signed) return plain;
  const out = join(scratch, `${name}.signed.hxext`);
  execFileSync(appsTools().pack, ["sign", plain, "--key", join(STORE_FIXTURE, "mock-publisher.key"), "-o", out], { encoding: "utf8" });
  return out;
}

test.describe("unsigned, under [ext.packages] dev", () => {
  test.describe.configure({ mode: "serial" });
  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    showcasePackage();
    backend = await startApps();
  });
  test.beforeEach(async () => {
    api = await connect(backend);
  });
  test.afterEach(() => api?.close());
  test.afterAll(async () => {
    await backend?.dispose();
  });

  test("its interface is its surfaces: the page is told frames, and the module that will not run", TAGS, async () => {
    await api.call("packages.installFile", { path: showcasePackage().path });
    const hello = await api.call("harnix.ui");
    expect(modules(hello)).toEqual([]);
    const refused = (hello.contributions ?? []).filter((c: any) => c.plugin === SHOWCASE_ID && c.refused);
    expect(refused.map((c: any) => c.refused)).toEqual(["showcase.js", "showcase.js"]);
    expect(refused[0].reason).toContain("trusted publisher");
    const listed = surfaces(hello);
    expect(listed.map((c: any) => c.point)).toEqual(["space", "space.sidebar", "space.tab", "space.secondary", "details.panel", "home.card", "settings.section", "modal", "space"]);
    const centre = listed[0];
    expect(centre).toMatchObject({
      frame: `/ext/${SHOWCASE_ID}/ui/frame/index.html`,
      surface: { id: "space", space: SHOWCASE_ID, sessions: SHOWCASE_ID, icon: "file", center: "hidden", title: { en: "Showcase", "zh-CN": "示例应用" } },
      interface: { calls: ["harnix.sessions.list", "engine.session.get"], links: true, trust: "unsigned", spaces: [SHOWCASE_ID, SHOWCASE_CHATS] },
    });
    // The second space has no page: its centre is the conversation, its left column the shell's list.
    expect(listed.at(-1)).toMatchObject({ frame: null, surface: { space: SHOWCASE_CHATS, sessions: SHOWCASE_CHATS, center: "chat" } });
    const runtime = (await api.call("packages.list")).packages[0].runtime;
    expect(runtime.frames).toBe(true);
    expect(runtime.surfaces).toContain("space.tab:heartbeat");
    expect(runtime.ui_rev).toMatch(/^[0-9a-f]{16}$/);
  });

  test("its pages are served under the frames' policy, and the page's own allows its frames", TAGS, async () => {
    const page = await fetch(`${base(backend)}/ext/${SHOWCASE_ID}/ui/frame/index.html`);
    expect(page.status).toBe(200);
    expect(page.headers.get("content-type")).toContain("text/html");
    const csp = page.headers.get("content-security-policy") ?? "";
    for (const directive of [
      "default-src 'none'",
      "script-src 'self'",
      "style-src 'self' 'unsafe-inline'",
      "img-src 'self' data: blob:",
      "connect-src 'none'",
      "frame-src 'none'",
      "form-action 'none'",
      "frame-ancestors 'self'",
    ]) {
      expect(csp, directive).toContain(directive);
    }
    expect(page.headers.get("referrer-policy")).toBe("no-referrer");
    const script = await fetch(`${base(backend)}/ext/${SHOWCASE_ID}/ui/frame/vendor/harnix-ext-sdk.js`);
    expect(script.headers.get("content-type")).toContain("javascript");
    expect(await script.text()).toContain("harnix-ui-bridge@2");
    expect((await fetch(`${base(backend)}/ext/${SHOWCASE_ID}/ui/../harnix-ext.toml`)).status).toBe(404);
    const shell = await fetch(`${base(backend)}/`);
    const pageCsp = shell.headers.get("content-security-policy") ?? "";
    expect(pageCsp).toContain("frame-src 'self'");
    expect(pageCsp).toContain("frame-ancestors 'none'");
  });

  test("a change to its files moves its ui_rev, which the page reloads its frames on", TAGS, async () => {
    await api.call("harnix.topics.subscribe", { topic: "packages.list" });
    const before = (await api.call("packages.list")).packages[0].runtime.ui_rev;
    const dir = (await api.call("packages.list")).dir;
    appendFileSync(join(dir, SHOWCASE_ID, "0.1.0", "ui", "frame", "showcase.css"), "\n/* a change */\n");
    await expect.poll(async () => (await api.call("packages.list")).packages[0].runtime.ui_rev, { timeout: 10_000 }).not.toBe(before);
    // Pushed on the topic too: the page needs no polling of its own.
    const pushed = () =>
      api.events
        .filter((e) => e.method === "harnix.topic" && e.params.topic === "packages.list")
        .map((e) => e.params.value?.packages?.[0]?.runtime?.ui_rev)
        .at(-1);
    await expect.poll(pushed, { timeout: 10_000 }).not.toBe(before);
    await api.call("packages.uninstall", { id: SHOWCASE_ID });
  });

  test("a frame is never given what is the shell's own, nor a page its package does not carry", TAGS, async () => {
    const shells = edited("calls", (m) => m.replace('calls = ["harnix.sessions.list", "engine.session.get"]', 'calls = ["harnix.sessions.list", "packages.uninstall"]'));
    expect((await api.refused("packages.installFile", { path: shells }))?.message).toContain("the shell's own");
    // Beyond the shell's own, a name is not refused at install: the router narrows the frame's
    // call by its row, to the package's own spaces and their sessions.
    const unlisted = edited("unlisted", (m) => m.replace('calls = ["harnix.sessions.list", "engine.session.get"]', 'calls = ["harnix.sessions.list", "harnix.session.delete"]'));
    await api.call("packages.installFile", { path: unlisted });
    const elsewhere = await api.call("harnix.session.create", {});
    const narrowed = await api.refused("harnix.session.delete", { session_id: elsewhere.session_id, $ext: SHOWCASE_ID });
    expect(narrowed?.message, "a frame deleted a session outside its package's spaces").toContain("own space");
    await api.call("harnix.session.delete", { session_id: elsewhere.session_id });
    await api.call("packages.uninstall", { id: SHOWCASE_ID });
    const missing = edited("page", (m) => m.replace('point = "home.card"\npage = "frame/index.html"', 'point = "home.card"\npage = "frame/missing.html"'));
    expect((await api.refused("packages.installFile", { path: missing }))?.message).toContain("frame/missing.html");
    const unknown = edited("point", (m) => m.replace('point = "modal"', 'point = "flow.block"'));
    expect((await api.refused("packages.installFile", { path: unknown }))?.message).toContain("point");
    expect((await api.call("packages.list")).packages).toEqual([]);
  });
});

test.describe("signed by a publisher this Harnix trusts", () => {
  test.describe.configure({ mode: "serial" });
  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    showcasePackage();
    backend = await startApps({ dev: false, profile: `\n[ext.packages.trusted_publishers]\nexample = ["${MOCK_PUBLISHER}"]\n` });
    api = await connect(backend);
  });
  test.afterAll(async () => {
    api?.close();
    await backend?.dispose();
  });

  test("its interface runs in the page: the page is told its modules, and no frames", TAGS, async () => {
    expect((await api.call("packages.installFile", { path: edited("trusted", (m) => m, true) })).trust).toBe("trusted");
    const hello = await api.call("harnix.ui");
    expect(modules(hello).map((c: any) => c.module)).toEqual([`/ext/${SHOWCASE_ID}/ui/showcase.js`, `/ext/${SHOWCASE_ID}/ui/showcase.js`]);
    expect(surfaces(hello)).toEqual([]);
    const runtime = (await api.call("packages.list")).packages[0].runtime;
    expect(runtime.frames).toBe(false);
    // Signed, its files are not watched: nothing changes under a verified package.
    expect(runtime.ui_rev ?? null).toBeNull();
    await api.call("packages.uninstall", { id: SHOWCASE_ID });
  });

  test("asking for isolation, the same application runs in frames", TAGS, async () => {
    const isolated = edited("isolated", (m) => m.replace("[interface]\n", "[interface]\nisolate = true\n"), true);
    expect((await api.call("packages.installFile", { path: isolated })).trust).toBe("trusted");
    const hello = await api.call("harnix.ui");
    expect(modules(hello)).toEqual([]);
    expect(surfaces(hello).length).toBe(9);
    expect(surfaces(hello)[0].interface.trust).toBe("trusted");
    await api.call("packages.uninstall", { id: SHOWCASE_ID });
  });
});
