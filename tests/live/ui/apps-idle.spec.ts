/**
 * An on-demand application's backend hibernating, in the browser: the system space's Apps tab
 * says so — not a fault, its backend asleep — and a call wakes it, which the tab shows too. The
 * showcase made on-demand in the test, on the apps-enabled test application, which idles such a
 * backend for four seconds here (`HARNIX_TESTAPP_APP_IDLE_MS`): long enough for the tab, which
 * looks once a second, to see it awake in between.
 */

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { type Backend, connect, expect, test, topology } from "@harnix/testkit";

import { SHOWCASE_ID, showcasePackage, startApps, variant } from "../api/apps-host";
import { appsTestApp } from "../api/apps-store";
import { open, watchConsole } from "./page";

const TAGS = { tag: ["@feature:apps-idle", "@layer:ui"] };

let backend: Backend;
let scratch: string;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  test.setTimeout(600_000);
  showcasePackage();
  scratch = mkdtempSync(join(tmpdir(), "hx-idle-ui-"));
  const path = variant(join(scratch, "idle.hxapp"), (e) => {
    const manifest = e.get("harnix-app.toml")!.toString().replace("watch_sessions = true", "watch_sessions = false");
    e.set("harnix-app.toml", Buffer.from(`${manifest}\n[backend]\nresident = "on-demand"\n`));
  });
  backend = await startApps({
    bin: appsTestApp(),
    env: { HARNIX_TESTAPP_APP_IDLE_MS: "4000" },
    topologies: process.env.HARNIX_TOPOLOGY ? [topology()] : undefined,
  });
  const api = await connect(backend, { topology: topology() });
  try {
    await api.call("harnix.apps.installFile", { path });
  } finally {
    api.close();
  }
});
test.afterAll(async () => {
  await backend?.dispose();
  if (scratch) rmSync(scratch, { recursive: true, force: true });
});

test("the Apps tab shows a hibernating backend, and a call waking it", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await page.locator('#side [data-space="system"]:not(#sessions)').click();
  await expect(page.getByTestId("system-console")).toBeVisible();
  await page.locator('[data-testid="system-console"] [data-tab="apps"]').click();
  const row = page.locator(`[data-testid="system-app"][data-app="${SHOWCASE_ID}"]`);
  await expect(row).toHaveAttribute("data-state", "hibernating", { timeout: 20_000 });
  await expect(row.locator('[data-field="state"]')).toHaveText(/Hibernating|休眠中/);
  await expect(row.locator('[data-field="backend"]')).toHaveText(/Asleep|休眠/);

  const api = await connect(backend, { topology: topology() });
  try {
    await api.call(`app.${SHOWCASE_ID}.state`);
  } finally {
    api.close();
  }
  await expect(row).toHaveAttribute("data-state", "running", { timeout: 10_000 });
  await expect(row.locator('[data-field="backend"]')).toHaveText(/Up|运行中/);
  expect(errors).toEqual([]);
});
