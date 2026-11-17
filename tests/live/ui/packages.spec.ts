/**
 * An extension package's interface in the browser: the showcase (`examples/packages/showcase`),
 * installed unsigned from a file on the apps-enabled binary, puts its `home.card` in front of a
 * new conversation — in an isolated frame, its in-page module listed as not running (only a
 * trusted publisher's runs in the page) — and the page reports nothing. Its space draws the
 * backend's board, in a frame too: it calls the backend's methods and draws what its topics push,
 * through the shell's bridge (`packages-frames.spec.ts` has the rest of the frames). The settings'
 * diagnostics show its VM (harnix-app.md §12.1). Disabled, the card is gone after a reload.
 */

import type { Page } from "@playwright/test";
import { type Backend, connect, expect, test } from "@harnix/testkit";

import { SHOWCASE_ID, showcasePackage, startApps } from "../api/packages-host";
import { diagnosticsBlock, open, toSessions, watchConsole } from "./page";

const TAGS = { tag: ["@feature:packages", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  test.setTimeout(600_000);
  const { path } = showcasePackage();
  backend = await startApps();
  const api = await connect(backend);
  try {
    await api.call("packages.installFile", { path });
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
  // The cards are the new conversation's page, the session space's.
  await toSessions(page);
  await expect
    .poll(() => page.evaluate(() => (window as any).harnix.app.registry.at("home.card").map((c: any) => c.owner)), { timeout: 15_000 })
    .toContain(SHOWCASE_ID);
  const frame = page.frameLocator(`[data-testid="surface-frame"][data-package="${SHOWCASE_ID}"][data-point="home.card"]`);
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
  await page.locator(`#side [data-space="${space}"]:not(#space-column)`).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", space);
}

test("its space: the board calls the backend, and draws what the backend's topics push", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await toSpace(page, SHOWCASE_ID);
  const frame = page.frameLocator(`[data-testid="surface-frame"][data-package="${SHOWCASE_ID}"][data-point="space"]`);
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
    await api.call(`${SHOWCASE_ID}.notes.add`, { text: "from another client" });
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

test("the diagnostics' packages block shows the application's VM, its calls and its refusals", TAGS, async ({ page }) => {
  const api = await connect(backend);
  try {
    // Something refused to show.
    expect((await api.call(`${SHOWCASE_ID}.overreach`)).refused).toBe(true);
  } finally {
    api.close();
  }
  await open(page, backend.url);
  const block = await diagnosticsBlock(page, "packages.runtime");
  const row = block.locator(`[data-testid="packages-runtime-row"][data-package="${SHOWCASE_ID}"]`);
  await expect(row).toHaveAttribute("data-state", "running", { timeout: 15_000 });
  await expect(row.locator('[data-field="backend"]')).toHaveText(/Up|运行中/);
  await expect(row.locator('[data-field="refused"]')).toHaveText("1", { timeout: 10_000 });
  await expect(row.locator('[data-field="calls"]')).not.toHaveText("—");
  await row.click();
  const detail = block.locator(`[data-testid="packages-runtime-detail"][data-package="${SHOWCASE_ID}"]`);
  await expect(detail.getByTestId("packages-runtime-refusal")).toContainText("packages.uninstall");
  await expect(detail).toContainText(`${SHOWCASE_ID}.notes.add`);
});

test("the packages block in Chinese: a header never breaks a word, the table fits its box, the VM's figures in the row opened", TAGS, async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("harnix.locale", "zh-CN"));
  await open(page, backend.url);
  const block = await diagnosticsBlock(page, "packages.runtime");
  const table = block.locator('[data-testid="packages-runtime"] table');
  const row = block.locator(`[data-testid="packages-runtime-row"][data-package="${SHOWCASE_ID}"]`);
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
  const vm = block.locator(`[data-testid="packages-runtime-detail"][data-package="${SHOWCASE_ID}"]`).getByTestId("packages-runtime-vm");
  await expect(vm.locator('[data-field="memory"]')).toContainText("MB");
  await expect(vm.locator('[data-field="storage"]')).toContainText("MB");
  await expect(row).toHaveAttribute("aria-expanded", "true");
});

test("disabled, its card is gone from the page", TAGS, async ({ page }) => {
  const api = await connect(backend);
  try {
    await api.call("packages.disable", { id: SHOWCASE_ID });
  } finally {
    api.close();
  }
  await open(page, backend.url);
  await toSessions(page);
  await expect(page.locator("#hero")).toBeVisible();
  const owners = await page.evaluate(() => (window as any).harnix.app.registry.at("home.card").map((c: any) => c.owner));
  expect(owners).not.toContain(SHOWCASE_ID);
  await expect(page.locator(`[data-testid="surface-frame"][data-package="${SHOWCASE_ID}"]`)).toHaveCount(0);
  // And the diagnostics say it is off.
  const block = await diagnosticsBlock(page, "packages.runtime");
  await expect(block.locator(`[data-testid="packages-runtime-row"][data-package="${SHOWCASE_ID}"]`)).toHaveAttribute("data-state", "stopped", { timeout: 15_000 });
});
