/**
 * The extension log in the settings' diagnostics (harnix-app.md §12.2), with the showcase
 * installed: its backend's line is there, under its id; filtered to it, or to errors only, the
 * block asks again and shows what it was asked for; live, the line its backend logs on stopping
 * comes in without a reload.
 */

import { type Backend, connect, expect, test, topology } from "@harnix/testkit";

import { SHOWCASE_ID, showcasePackage, startApps } from "../api/packages-host";
import { diagnosticsBlock, open, watchConsole } from "./page";

const TAGS = { tag: ["@feature:packages", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  test.setTimeout(600_000);
  showcasePackage();
  backend = await startApps({ topologies: process.env.HARNIX_TOPOLOGY ? [topology()] : undefined });
  const api = await connect(backend, { topology: topology() });
  try {
    await api.call("packages.installFile", { path: showcasePackage().path });
    await api.call(`${SHOWCASE_ID}.state`);
  } finally {
    api.close();
  }
});
test.afterAll(async () => {
  await backend?.dispose();
});

test("the extension log: its line under its id, filtered by extension and level, followed live", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const tab = (await diagnosticsBlock(page, "observability.extlog")).getByTestId("extlog");
  await expect(tab).toBeVisible();
  const ours = tab.locator(`[data-testid="extlog-line"][data-extension="${SHOWCASE_ID}"]`);
  await expect(ours.filter({ hasText: "the showcase's backend started" })).toHaveCount(1, { timeout: 20_000 });

  await page.getByTestId("extlog-extension").selectOption(SHOWCASE_ID);
  await expect(tab.locator('[data-testid="extlog-line"]').first()).toBeVisible();
  const others = tab.locator(`[data-testid="extlog-line"]:not([data-extension="${SHOWCASE_ID}"])`);
  await expect(others).toHaveCount(0);

  await page.getByTestId("extlog-level").selectOption("error");
  await expect(tab.locator('[data-testid="extlog-line"]:not([data-level="error"])')).toHaveCount(0);
  await page.getByTestId("extlog-level").selectOption("");

  // Live: switched off, its backend says it stopped, and the line comes in without a reload.
  const api = await connect(backend, { topology: topology() });
  try {
    await api.call("packages.disable", { id: SHOWCASE_ID });
    await api.call("packages.enable", { id: SHOWCASE_ID });
  } finally {
    api.close();
  }
  await expect(ours.filter({ hasText: "the showcase's backend stopped" }).first()).toBeVisible({ timeout: 20_000 });

  // Not live: nothing more comes in.
  await page.getByTestId("extlog-live").click();
  await expect(page.getByTestId("extlog-live")).toHaveAttribute("aria-pressed", "false");
  expect(errors).toEqual([]);
});
