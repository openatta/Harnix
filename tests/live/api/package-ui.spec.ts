/**
 * Installed packages' interface modules (design §10.5, M8-1–M8-3), against the real installer.
 *
 *   demo-rows     its `[[ui]]` is contribution API 1: the entry does not take, with the reason,
 *                 and nothing of it is offered to the page
 *   demo-rows-v2  API 2: offered at hello with its version, served same-origin, nothing else of
 *                 the package reachable
 *
 * With `[ui] runtime_modules = false` neither is offered or served — the packages are the
 * engine's alone. And the page itself: its one inline script, the import map, is allowed by its
 * hash and nothing is loosened.
 */

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { type Backend, ROOT, type TestClient, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:packages", "@layer:api"] };
const PACKAGES = join(ROOT, "tests", "fixtures", "packages");

test.describe.configure({ mode: "serial" });

/** Upload a fixture package and have the engine install it. */
export async function install(api: TestClient, backend: Backend, name: string): Promise<void> {
  const zip = readFileSync(join(PACKAGES, `${name}.zip`));
  const grant = await api.call("harnix.plugins.upload", { name: `${name}.zip` });
  const uploaded = await fetch(`${backend.url.replace(/\/$/, "")}${grant.url}`, { method: "POST", body: zip });
  expect(uploaded.ok, `upload of ${name}`).toBe(true);
  await api.call("harnix.plugins.install", { path: grant.path, name, version: "1.0.0", checksum: createHash("sha256").update(zip).digest("hex") });
}

test.describe("runtime modules on", () => {
  let backend: Backend;
  let api: TestClient;
  test.beforeAll(async () => {
    backend = await startHarnix(launchFor("replay"));
    api = await connect(backend);
    await install(api, backend, "demo-rows");
    await install(api, backend, "demo-rows-v2");
  });
  test.afterAll(() => {
    api?.close();
    backend?.stop();
  });

  test("a v2 module is offered and served; a v1 one does not take, and says why", TAGS, async () => {
    const listed = await api.call("harnix.plugins.list");
    const v1 = listed.contributes.find((c: any) => c.plugin === "demo-rows");
    const v2 = listed.contributes.find((c: any) => c.plugin === "demo-rows-v2");
    expect(v1.ui).toEqual([]);
    expect(v1.inert.join(" ")).toContain("这一条不生效");
    expect(v2.ui.map((e: any) => [e.point, e.module, e.api_version])).toEqual([["tool.row", "rows.js", 2]]);
    expect(v2.inert).toEqual([]);

    // A fresh client is told what to load: the v2 entry, with its version, and nothing of v1.
    const fresh = await connect(backend);
    try {
      const hello = await fresh.call("harnix.hello");
      expect(hello.contrib_api_version).toBe(2);
      expect(hello.shell.runtime_modules).toBe(true);
      expect(hello.contributions).toEqual([{ plugin: "demo-rows-v2", point: "tool.row", api_version: 2, module: "/plugins/demo-rows-v2/ui/rows.js" }]);
    } finally {
      fresh.close();
    }

    const module = await fetch(`${backend.url}plugins/demo-rows-v2/ui/rows.js`);
    expect(module.status).toBe(200);
    expect(await module.text()).toContain('from "@harnix/react"');
    const manifest = await fetch(`${backend.url}plugins/demo-rows-v2/ui/plugin.toml`);
    expect(manifest.status).not.toBe(200);
  });

  test("the page's import map is allowed by its hash, and nothing else inline", TAGS, async () => {
    const response = await fetch(backend.url);
    const page = await response.text();
    const map = /<script type="importmap">([\s\S]*?)<\/script>/.exec(page)?.[1];
    expect(map, "the page has no import map").toBeTruthy();
    const imports = JSON.parse(map!).imports as Record<string, string>;
    expect(Object.keys(imports).sort()).toEqual(["@harnix/react", "react", "react-dom", "react-dom/client", "react/jsx-runtime"]);
    for (const path of Object.values(imports)) expect((await fetch(new URL(path, backend.url))).status).toBe(200);
    const csp = response.headers.get("content-security-policy") ?? "";
    const hash = createHash("sha256").update(map!).digest("base64");
    expect(csp).toContain(`script-src 'self' 'sha256-${hash}'`);
    expect(csp).not.toContain("unsafe");
  });
});

test.describe("runtime modules off", () => {
  let backend: Backend;
  let api: TestClient;
  test.beforeAll(async () => {
    backend = await startHarnix(launchFor("replay", { profile: "\n[ui]\nruntime_modules = false\n" }));
    api = await connect(backend);
    await install(api, backend, "demo-rows-v2");
  });
  test.afterAll(() => {
    api?.close();
    backend?.stop();
  });

  test("with the switch off, no package's interface module is offered or served", TAGS, async () => {
    const fresh = await connect(backend);
    try {
      const hello = await fresh.call("harnix.hello");
      expect(hello.shell.runtime_modules).toBe(false);
      expect(hello.contributions).toEqual([]);
    } finally {
      fresh.close();
    }
    const module = await fetch(`${backend.url}plugins/demo-rows-v2/ui/rows.js`);
    expect(module.status).not.toBe(200);
    // The package itself is installed and the engine's: only its interface is off.
    const listed = await api.call("harnix.plugins.list");
    expect(listed.plugins.map((p: any) => p.name)).toContain("demo-rows-v2");
  });
});
