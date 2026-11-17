/**
 * The system console's extension log in the browser, with the showcase installed: its backend's
 * line is there, under its id; filtered to it, or to errors only, the tab asks again and shows what
 * it was asked for; live, the line its backend logs on stopping comes in without a reload.
 */

import { type Backend, connect, expect, test, topology } from "@harnix/testkit";

import { SHOWCASE_ID, showcasePackage, startApps } from "../api/packages-host";
import { open, watchConsole } from "./page";

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
  await page.locator('#side [data-space="system"]:not(#space-column)').click();
  await expect(page.getByTestId("system-console")).toBeVisible();
  await page.locator('[data-testid="system-console"] [role="tab"][data-tab="system.extlog"]').click();
  const tab = page.getByTestId("system-extlog");
  await expect(tab).toBeVisible();
  const ours = tab.locator(`[data-testid="extlog-line"][data-extension="${SHOWCASE_ID}"]`);
  await expect(ours.filter({ hasText: "the showcase's backend started" })).toHaveCount(1, { timeout: 20_000 });
  await expect(page.getByTestId("system-about")).toHaveAttribute("data-tab", "system.extlog");

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
