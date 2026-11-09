/**
 * A runtime application's interface in the browser: the showcase (`examples/apps/showcase`),
 * installed unsigned from a file on the apps-enabled binary, puts its `home.card` in front of a
 * new conversation — in an isolated frame, its in-page module listed as not running (only a
 * trusted publisher's runs in the page) — and the page reports nothing. Its space draws the
 * backend's board, in a frame too: it calls the backend's methods and draws what its topics push,
 * through the shell's bridge (`apps-frames.spec.ts` has the rest of the frames). The system
 * space's Apps tab shows its VM. Disabled, the card is gone after a reload.
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

test("an application's card is drawn in its frame, signed with its id; its module does not run", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  const violations: string[] = [];
  page.on("console", (m) => {
    if (/Content Security Policy|Refused to/i.test(m.text())) violations.push(m.text());
  });
  await open(page, backend.url);
  await expect
    .poll(() => page.evaluate(() => (window as any).harnix.app.registry.at("home.card").map((c: any) => c.owner)), { timeout: 15_000 })
    .toContain(SHOWCASE_ID);
  const frame = page.frameLocator(`[data-testid="surface-frame"][data-app="${SHOWCASE_ID}"][data-point="home.card"]`);
  const card = frame.getByTestId("showcase-card");
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card).toContainText(/Seven tools|七个工具/);
  // Unsigned: the module that would run in the page is said not to, in the diagnostics.
  const refused = await page.evaluate(() => (window as any).harnix.app.registry.refusals().filter((r: any) => r.id === "showcase.js"));
  expect(refused.length).toBeGreaterThan(0);
  expect(violations).toEqual([]);
  expect(errors).toEqual([]);
});

async function toSpace(page: Page, space: string): Promise<void> {
  await page.locator(`#side [data-space="${space}"]:not(#sessions)`).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", space);
}

test("its space: the board calls the backend, and draws what the backend's topics push", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await toSpace(page, SHOWCASE_ID);
  const frame = page.frameLocator(`[data-testid="surface-frame"][data-app="${SHOWCASE_ID}"][data-point="space"]`);
  const board = frame.getByTestId("showcase-board");
  await expect(board).toBeVisible({ timeout: 15_000 });
  await expect(frame.getByTestId("showcase-note")).toHaveCount(0);

  // A method, the backend, its topic, the frame.
  await frame.getByTestId("showcase-note-input").fill("from the browser");
  await frame.getByTestId("showcase-note-add").click();
  await expect(frame.getByTestId("showcase-note")).toHaveText(["from the browser"]);
  await expect(frame.getByTestId("showcase-note-input")).toHaveValue("");
  // Another client's note arrives the same way, without the page asking.
  const api = await connect(backend);
  try {
    await api.call(`app.${SHOWCASE_ID}.notes.add`, { text: "from another client" });
  } finally {
    api.close();
  }
  await expect(frame.getByTestId("showcase-note")).toHaveText(["from the browser", "from another client"]);

  // The backend's memory, and its timer.
  await frame.getByTestId("showcase-bump").click();
  await expect(frame.getByTestId("showcase-counter")).toHaveText(/1/);
  await frame.getByTestId("showcase-heartbeat-start").click();
  await expect(frame.getByTestId("showcase-ticks")).toHaveText(/3/, { timeout: 10_000 });
  await expect(frame.getByTestId("showcase-error")).toHaveCount(0);
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
  await page.locator('#side [data-space="system"]:not(#sessions)').click();
  await expect(page.getByTestId("system-console")).toBeVisible();
  await page.locator('[data-testid="system-console"] [role="tab"][data-tab="apps"]').click();
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

test("the Apps tab in Chinese: a header never breaks a word, the table fits its column, the VM's figures in the row opened", TAGS, async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("harnix.locale", "zh-CN"));
  await open(page, backend.url);
  await page.locator('#side [data-space="system"]:not(#sessions)').click();
  await page.locator('[data-testid="system-console"] [role="tab"][data-tab="apps"]').click();
  const table = page.locator('[data-testid="system-apps"] table');
  const row = page.locator(`[data-testid="system-app"][data-app="${SHOWCASE_ID}"]`);
  await expect(row).toBeVisible({ timeout: 15_000 });
  // One line each: "状态" is not drawn as 状 over 态. Counted as the lines its text is laid out on.
  const lines = (el: Element) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    return new Set([...range.getClientRects()].filter((r) => r.width > 0).map((r) => Math.round(r.top))).size;
  };
  for (const th of await table.locator("th").all()) expect(await th.evaluate(lines), `${await th.textContent()} on one line`).toBe(1);
  for (const field of ["state", "backend"]) expect(await row.locator(`[data-field="${field}"]`).evaluate(lines), `${field} on one line`).toBe(1);
  // The page does not scroll sideways for it.
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  // Opened by the keyboard: its recycles, faults, memory and storage.
  await row.focus();
  await page.keyboard.press("Enter");
  const vm = page.locator(`[data-testid="system-app-detail"][data-app="${SHOWCASE_ID}"]`).getByTestId("system-app-vm");
  await expect(vm.locator('[data-field="memory"]')).toContainText("MB");
  await expect(vm.locator('[data-field="storage"]')).toContainText("MB");
  await expect(row).toHaveAttribute("aria-expanded", "true");
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
  await expect(page.locator(`[data-testid="surface-frame"][data-app="${SHOWCASE_ID}"]`)).toHaveCount(0);
  // And the Apps tab says it is off.
  await page.locator('#side [data-space="system"]:not(#sessions)').click();
  await page.locator('[data-testid="system-console"] [role="tab"][data-tab="apps"]').click();
  await expect(page.locator(`[data-testid="system-app"][data-app="${SHOWCASE_ID}"]`)).toHaveAttribute("data-state", "stopped", { timeout: 15_000 });
});
