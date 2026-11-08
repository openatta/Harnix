/**
 * A runtime application's interface module in the browser: the showcase (`examples/apps/showcase`),
 * installed from a file on the apps-enabled binary, puts its `home.card` in front of a new
 * conversation — imported from `/apps/<id>/ui/`, on the shell's own React, with words from its own
 * language pack — and the page reports nothing. Its space draws the backend's board: the module
 * calls the backend's methods and draws what its topics push, over the page's own connection. The
 * system space's Apps tab shows its VM. Disabled, the card is gone after a reload.
 */

import type { Page } from "@playwright/test";
import { type Backend, connect, expect, test } from "@harnix/testkit";

import { SHOWCASE_ID, showcasePackage, startApps } from "../api/apps-host";
import { open, watchConsole } from "./page";

const TAGS = { tag: ["@feature:apps", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  test.setTimeout(600_000);
  const { path } = showcasePackage();
  backend = await startApps();
  const api = await connect(backend);
  try {
    await api.call("harnix.apps.installFile", { path });
  } finally {
    api.close();
  }
});
test.afterAll(async () => { await backend?.dispose(); });

test("an application's interface module draws its card, signed with its id", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  const violations: string[] = [];
  page.on("console", (m) => {
    if (/Content Security Policy|Refused to/i.test(m.text())) violations.push(m.text());
  });
  await open(page, backend.url);
  await expect
    .poll(() => page.evaluate(() => (window as any).harnix.app.registry.at("home.card").map((c: any) => c.owner)), { timeout: 15_000 })
    .toContain(SHOWCASE_ID);
  const card = page.getByTestId("showcase-card");
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card).toContainText(/Showcase|示例应用/);
  await expect(card).toContainText(/Seven tools|七个工具/);
  expect(violations).toEqual([]);
  expect(errors).toEqual([]);
});

async function toSpace(page: Page, space: string): Promise<void> {
  await page.locator(`#side [data-space="${space}"]`).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", space);
}

test("its space: the board calls the backend, and draws what the backend's topics push", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await toSpace(page, SHOWCASE_ID);
  const board = page.getByTestId("showcase-board");
  await expect(board).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("showcase-note")).toHaveCount(0);

  // A method, the backend, its topic, the page.
  await page.getByTestId("showcase-note-input").fill("from the browser");
  await page.getByTestId("showcase-note-add").click();
  await expect(page.getByTestId("showcase-note")).toHaveText(["from the browser"]);
  await expect(page.getByTestId("showcase-note-input")).toHaveValue("");
  // Another client's note arrives the same way, without the page asking.
  const api = await connect(backend);
  try {
    await api.call(`app.${SHOWCASE_ID}.notes.add`, { text: "from another client" });
  } finally {
    api.close();
  }
  await expect(page.getByTestId("showcase-note")).toHaveText(["from the browser", "from another client"]);

  // The backend's memory, and its timer.
  await page.getByTestId("showcase-bump").click();
  await expect(page.getByTestId("showcase-counter")).toHaveText(/1/);
  await page.getByTestId("showcase-heartbeat-start").click();
  await expect(page.getByTestId("showcase-ticks")).toHaveText(/3/, { timeout: 10_000 });
  await expect(page.getByTestId("showcase-error")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("the system space's Apps tab shows the application's VM, its calls and its refusals", TAGS, async ({ page }) => {
  const api = await connect(backend);
  try {
    // Something refused to show.
    expect((await api.call(`app.${SHOWCASE_ID}.overreach`)).refused).toBe(true);
  } finally {
    api.close();
  }
  await open(page, backend.url);
  await page.locator('#side [data-space="system"]').click();
  await expect(page.getByTestId("system-console")).toBeVisible();
  await page.locator('[data-testid="system-console"] [data-tab="apps"]').click();
  const row = page.locator(`[data-testid="system-app"][data-app="${SHOWCASE_ID}"]`);
  await expect(row).toHaveAttribute("data-state", "running", { timeout: 15_000 });
  await expect(row.locator('[data-field="backend"]')).toHaveText(/Up|运行中/);
  await expect(row.locator('[data-field="refused"]')).toHaveText("1", { timeout: 10_000 });
  await expect(row.locator('[data-field="calls"]')).not.toHaveText("—");
  await row.click();
  const detail = page.locator(`[data-testid="system-app-detail"][data-app="${SHOWCASE_ID}"]`);
  await expect(detail.getByTestId("system-app-refusal")).toContainText("harnix.apps.uninstall");
  await expect(detail).toContainText(`app.${SHOWCASE_ID}.notes.add`);
});

test("disabled, its card is gone from the page", TAGS, async ({ page }) => {
  const api = await connect(backend);
  try {
    await api.call("harnix.apps.disable", { id: SHOWCASE_ID });
  } finally {
    api.close();
  }
  await open(page, backend.url);
  await expect(page.locator("#hero")).toBeVisible();
  const owners = await page.evaluate(() => (window as any).harnix.app.registry.at("home.card").map((c: any) => c.owner));
  expect(owners).not.toContain(SHOWCASE_ID);
  await expect(page.getByTestId("showcase-card")).toHaveCount(0);
  // And the Apps tab says it is off.
  await page.locator('#side [data-space="system"]').click();
  await page.locator('[data-testid="system-console"] [data-tab="apps"]').click();
  await expect(page.locator(`[data-testid="system-app"][data-app="${SHOWCASE_ID}"]`)).toHaveAttribute("data-state", "stopped", { timeout: 15_000 });
});
