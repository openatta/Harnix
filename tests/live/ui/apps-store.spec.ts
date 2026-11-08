/**
 * Settings → Applications in the browser, against the mock store (`../api/apps-store.ts`): the
 * catalog offers the showcase; "Install" shows what it asks for and installs nothing until the
 * disclosure's own button is pressed; installed, it is listed as countersigned by the store with
 * its switches; switched off and on; updated, with only what the new version adds shown and asked
 * for; uninstalled, gone from the list and back on offer.
 */

import type { Page } from "@playwright/test";
import { rmSync } from "node:fs";

import { type Backend, connect, expect, test, topology } from "@harnix/testkit";

import { SHOWCASE_ID } from "../api/apps-host";
import { type MockStore, appsTestApp, mockStore, startStore } from "../api/apps-store";
import { open, openSettings, watchConsole } from "./page";

const TAGS = { tag: ["@feature:apps-store", "@layer:ui"] };

let store: MockStore;
let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  test.setTimeout(600_000);
  appsTestApp();
  store = mockStore();
  backend = await startStore(store, { topologies: process.env.HARNIX_TOPOLOGY ? [topology()] : undefined });
});
test.afterAll(async () => {
  await backend?.dispose();
  if (store) rmSync(store.dir, { recursive: true, force: true });
});

async function applications(page: Page): Promise<void> {
  await openSettings(page);
  await page.locator('.settings-nav [data-section="apps"]').click();
  await expect(page.locator(".settings-page")).toBeVisible();
}

const listed = async () => {
  const api = await connect(backend, { topology: topology() });
  try {
    return (await api.call("harnix.apps.list")).apps as any[];
  } finally {
    api.close();
  }
};

test("the store's application is installed through its disclosure, and only through it", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await applications(page);
  const offered = page.locator(`[data-testid="app-offered"][data-app="${SHOWCASE_ID}"]`);
  await expect(offered).toContainText("Showcase");
  await expect(offered).toContainText("0.2.0");
  await expect(page.getByTestId("app-installed")).toHaveCount(0);

  await offered.getByTestId("app-install").click();
  const disclosure = page.getByTestId("app-disclosure");
  await expect(disclosure).toBeVisible();
  await expect(disclosure).toContainText(/Showcase 0\.2\.0/);
  await expect(disclosure).toContainText(/Countersigned by the store|商店已副署/);
  for (const capability of ["net", "env", "storage_mb", "fs", "ui", "agent"]) {
    await expect(disclosure.locator(`[data-capability="${capability}"]`).first(), capability).toBeVisible();
  }
  await expect(disclosure).toContainText("SHOWCASE_EXTRA");
  await expect(disclosure).toContainText("api.example.com");
  // Shown, and nothing installed.
  expect(await listed()).toEqual([]);
  // Cancelled: still nothing.
  await disclosure.getByRole("button", { name: /Cancel|取消/ }).click();
  await expect(disclosure).toHaveCount(0);
  expect(await listed()).toEqual([]);

  await offered.getByTestId("app-install").click();
  await page.getByTestId("app-consent").click();
  const installed = page.locator(`[data-testid="app-installed"][data-app="${SHOWCASE_ID}"]`);
  await expect(installed).toBeVisible({ timeout: 30_000 });
  await expect(installed).toHaveAttribute("data-trust", "store");
  await expect(installed).toHaveAttribute("data-version", "0.2.0");
  await expect(installed).toContainText(/Countersigned by the store|商店已副署/);
  await expect(installed.locator('[data-capability="env"]', { hasText: "SHOWCASE_EXTRA" })).toBeVisible();
  await expect(offered).toContainText(/Installed \(0\.2\.0\)|已安装（0\.2\.0）/);
  expect((await listed())[0]).toMatchObject({ id: SHOWCASE_ID, version: "0.2.0", trust: "store" });
  expect(errors).toEqual([]);
});

test("switched off and on from its row", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await applications(page);
  const installed = page.locator(`[data-testid="app-installed"][data-app="${SHOWCASE_ID}"]`);
  await installed.getByTestId("app-toggle").click();
  await expect(installed.getByTestId("app-toggle")).toHaveText(/Enable|启用/);
  expect((await listed())[0].enabled).toBe(false);
  await installed.getByTestId("app-toggle").click();
  await expect(installed.getByTestId("app-toggle")).toHaveText(/Disable|停用/);
  expect((await listed())[0]).toMatchObject({ enabled: true, runtime: { state: "running" } });
});

test("an update shows only what it adds, and waits for its own consent", TAGS, async ({ page }) => {
  // From 0.1.0, installed (and consented to) by a client: the page only updates it.
  const api = await connect(backend, { topology: topology() });
  try {
    await api.call("harnix.apps.uninstall", { id: SHOWCASE_ID });
    const asked = await api.call("harnix.apps.install", { id: SHOWCASE_ID, version: "0.1.0" });
    await api.call("harnix.apps.install", { id: SHOWCASE_ID, version: "0.1.0", consent: asked.digest });
  } finally {
    api.close();
  }
  await open(page, backend.url);
  await applications(page);
  const installed = page.locator(`[data-testid="app-installed"][data-app="${SHOWCASE_ID}"]`);
  await expect(installed).toHaveAttribute("data-version", "0.1.0");
  await expect(installed.getByTestId("app-update")).toContainText("0.2.0");
  await installed.getByTestId("app-update").click();
  const disclosure = page.getByTestId("app-disclosure");
  await expect(disclosure).toContainText(/0\.1\.0.*0\.2\.0/);
  // What 0.1.0 already had is not asked for again.
  await expect(disclosure.locator("[data-capability]")).toHaveCount(3);
  await expect(disclosure.locator('[data-capability="env"]')).toContainText("SHOWCASE_EXTRA");
  await expect(disclosure.locator('[data-capability="net"]')).toContainText("api.example.com");
  await expect(disclosure.locator('[data-capability="storage_mb"]')).toContainText("2 MB");
  await expect(disclosure.locator('[data-capability="fs"]')).toHaveCount(0);
  expect((await listed())[0].version).toBe("0.1.0");
  await page.getByTestId("app-consent").click();
  await expect(installed).toHaveAttribute("data-version", "0.2.0", { timeout: 30_000 });
  expect((await listed())[0]).toMatchObject({ version: "0.2.0", trust: "store" });
});

test("uninstalled from its row: gone, and on offer again", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await applications(page);
  page.once("dialog", (dialog) => void dialog.accept());
  await page.locator(`[data-testid="app-installed"][data-app="${SHOWCASE_ID}"]`).getByTestId("app-uninstall").click();
  await expect(page.getByTestId("app-installed")).toHaveCount(0, { timeout: 30_000 });
  await expect(page.locator(`[data-testid="app-offered"][data-app="${SHOWCASE_ID}"]`).getByTestId("app-install")).toBeVisible();
  expect(await listed()).toEqual([]);
});
