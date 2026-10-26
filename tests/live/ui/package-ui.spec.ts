/**
 * An installed package's interface module in the browser (design §10.5, M8-1, M8-2): the v2 module
 * is imported through the page's import map, registers on the shell's own React, draws its row in
 * the conversation with words from its own language pack — and the page's CSP, which admits the
 * import map by its hash alone, reports nothing. The v1 package beside it changes nothing on the
 * page: its entry is listed as not taking.
 */

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { type Backend, ROOT, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { newSession, open, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:packages", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
  const api = await connect(backend);
  try {
    for (const name of ["demo-rows", "demo-rows-v2"]) {
      const zip = readFileSync(join(ROOT, "tests", "fixtures", "packages", `${name}.zip`));
      const grant = await api.call("harnix.plugins.upload", { name: `${name}.zip` });
      await fetch(`${backend.url.replace(/\/$/, "")}${grant.url}`, { method: "POST", body: zip });
      await api.call("harnix.plugins.install", { path: grant.path, name, version: "1.0.0", checksum: createHash("sha256").update(zip).digest("hex") });
    }
  } finally {
    api.close();
  }
});
test.afterAll(() => backend?.stop());

test("package-ui: a v2 module draws its row on the shell's React, and the CSP has nothing to say", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  const violations: string[] = [];
  page.on("console", (m) => {
    if (/Content Security Policy|Refused to/i.test(m.text())) violations.push(m.text());
  });
  await open(page, backend.url);
  await expect
    .poll(() => page.evaluate(() => (window as any).harnix.app.registry.at("tool.row").map((c: any) => c.owner)), { timeout: 15_000 })
    .toContain("demo-rows-v2");
  await newSession(page, "calls-a-tool");
  await say(page, "Use the Bash tool to run exactly: echo harnix-fixture");
  const row = page.getByTestId("demo-row");
  await expect(row).toBeVisible({ timeout: 30_000 });
  await expect(row).toContainText(/demo-rows (v2|第二版)/);
  // Nothing of the v1 package registered.
  const owners = await page.evaluate(() => (window as any).harnix.app.registry.at("tool.row").map((c: any) => c.owner));
  expect(owners).not.toContain("demo-rows");
  expect(violations).toEqual([]);
  expect(errors).toEqual([]);
});

test("package-ui: the diagnostics list each entry, the import map and the CSP that admits it", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await page.getByTestId("avatar").click();
  await page.getByTestId("open-settings").click();
  await page.locator(".settings-nav .nav-item", { hasText: /Diagnostics|诊断/ }).click();
  await expect(page.getByTestId("import-map")).toContainText("@harnix/react");
  await expect(page.getByTestId("csp")).toContainText("'sha256-");
  const page_ = page.locator(".settings-page");
  await expect(page_).toContainText("demo-rows-v2");
  await expect(page_).toContainText("这一条不生效");
});
