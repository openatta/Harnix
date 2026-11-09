/**
 * The page's layers — dialogs, menus, the banner — driven the way a person drives them, keyboard
 * first, against a real backend with turns replayed.
 *
 * What is under test is what a layer owes the person in front of it: the focus goes into a dialog,
 * stays there, and comes back to where it was; Escape closes only the top layer; the shortcuts
 * leave a dialog alone; a menu opens where it was asked for, not as a dialog in the middle of the
 * page, and can be worked without a mouse; what the banner says, and an ask arriving, is said aloud.
 * And the default answer of an ask can be seen.
 */

import type { Page } from "@playwright/test";
import { expect, test } from "@harnix/testkit";

import { newSession, open, openSettings, say, sessionRow, spaceEntry } from "./page";

const SHELL = { tag: ["@feature:shell", "@layer:ui"] };
const SESSIONS = { tag: ["@feature:sessions", "@layer:ui"] };
const SPACE = { tag: ["@feature:session-space", "@layer:ui"] };

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});

const row = sessionRow;
/** What has the focus, by its `data-testid` (or its tag when it has none). */
const focused = (page: Page) => page.evaluate(() => (document.activeElement as HTMLElement | null)?.dataset.testid ?? document.activeElement?.tagName ?? null);
const focusIn = (page: Page, selector: string) => page.evaluate((s) => !!document.activeElement?.closest(s), selector);

/** A session's turn waiting on its ask: the ask on the page, the turn running. */
async function asking(page: Page): Promise<void> {
  await newSession(page, "asks-permission");
  await say(page, "Use the Bash tool to run exactly: touch harnix-permission-fixture");
  await expect(page.getByTestId("conv-ask")).toBeVisible({ timeout: 30_000 });
}

test("an ask's default answer is drawn in the accent, not white on white", SESSIONS, async ({ page }) => {
  await open(page, URL);
  await asking(page);
  const ask = page.getByTestId("conv-ask");
  const colours = await ask.evaluate((card) => {
    const primary = card.querySelector<HTMLElement>('[data-testid="conv-ask-allow"]')!;
    // `--hx-accent` as the browser resolves it, here: an element filled with it.
    const probe = document.createElement("span");
    probe.style.background = "var(--hx-accent)";
    card.append(probe);
    const accent = getComputedStyle(probe).backgroundColor;
    probe.remove();
    const other = card.querySelector<HTMLElement>('[data-testid="conv-ask-allow-session"]')!;
    return { primary: getComputedStyle(primary).backgroundColor, text: getComputedStyle(primary).color, accent, other: getComputedStyle(other).backgroundColor };
  });
  expect(colours.primary).toBe(colours.accent);
  expect(colours.primary).not.toBe(colours.text);
  expect(colours.primary).not.toBe(colours.other);
  // Answered, so the turn does not stay running on the backend the next tests share.
  await ask.getByTestId("conv-ask-deny").click();
  await expect(ask).toHaveAttribute("data-state", "answered");
});

test("the focus goes into a dialog, goes round inside it, and comes back to what opened it", SHELL, async ({ page }) => {
  await open(page, URL);
  await spaceEntry(page, "projects").click();
  const add = page.getByTestId("add-project");
  await add.focus();
  await page.keyboard.press("Enter");
  const dialog = page.locator("#modal");
  await expect(dialog).toBeVisible();
  await expect(dialog).toHaveAttribute("role", "dialog");
  await expect(dialog).toHaveAttribute("aria-labelledby", /.+/);
  // Into its first field, not its ×; the page under it is inert.
  await expect.poll(() => focusIn(page, "#modal .body")).toBe(true);
  await expect(page.locator("#frame")).toHaveAttribute("inert", "");
  // Back past its first stop goes round to its last, and forward from there to its first.
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Shift+Tab");
  expect(await page.evaluate(() => document.activeElement === document.querySelector('#modal [data-testid="project-add"]'))).toBe(true);
  await page.keyboard.press("Tab");
  expect(await focusIn(page, "#modal")).toBe(true);
  for (let i = 0; i < 12; i += 1) {
    await page.keyboard.press("Tab");
    expect(await focusIn(page, "#modal"), `Tab ${i + 1} left the dialog`).toBe(true);
  }
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(page.locator("#frame")).not.toHaveAttribute("inert", "");
  expect(await focused(page)).toBe("add-project");
});

test("Escape closes the top layer only: a dialog over the settings, then the settings", SHELL, async ({ page }) => {
  await open(page, URL);
  await openSettings(page);
  await page.evaluate(() => (window as any).harnix.app.openModal({ title: "Over the settings", content: null }));
  const dialogs = page.getByTestId("modal");
  await expect(dialogs).toHaveCount(2);
  // The settings are under it, inert; the one over them has the focus.
  await expect(page.locator("#modal")).toHaveAttribute("inert", "");
  expect(await focusIn(page, '[data-testid="modal"]:not(#modal)')).toBe(true);
  await page.keyboard.press("Escape");
  await expect(dialogs).toHaveCount(1);
  await expect(page.getByTestId("settings-nav")).toBeVisible();
  await expect(page.locator("#modal")).not.toHaveAttribute("inert", "");
  expect(await focusIn(page, "#modal")).toBe(true);
  await page.keyboard.press("Escape");
  await expect(dialogs).toHaveCount(0);
  await expect(page.locator("#veil")).toHaveAttribute("data-state", "closed");
});

test("the shortcuts leave a dialog alone: ⌘K and ⌘/ do nothing while one is open", SHELL, async ({ page }) => {
  await open(page, URL);
  const id = await newSession(page, "says-ready");
  const current = () => page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId ?? null);
  await openSettings(page);
  await page.keyboard.press("ControlOrMeta+k");
  await page.keyboard.press("ControlOrMeta+/");
  expect(await current()).toBe(id);
  expect(await focusIn(page, "#modal")).toBe(true);
  await expect(page.getByTestId("settings-nav")).toBeVisible();

  // Closed, they are the shell's again.
  await page.keyboard.press("Escape");
  await expect(page.locator("#modal")).toHaveCount(0);
  await page.keyboard.press("ControlOrMeta+/");
  expect(await focused(page)).toBe("conv-input");
  await page.keyboard.press("ControlOrMeta+k");
  await expect.poll(current).toBeNull();
});

test("the closed right column is out of the way: inert, so Tab does not walk through it", SHELL, async ({ page }) => {
  await open(page, URL);
  const details = page.locator("#details");
  await page.evaluate(() => (window as any).harnix.app.toggleDetails(false));
  await expect(details).toHaveAttribute("inert", "");
  await page.evaluate(() => (window as any).harnix.app.toggleDetails(true));
  await expect(details).not.toHaveAttribute("inert", "");
  await page.evaluate(() => (window as any).harnix.app.toggleDetails(false));
  await page.getByTestId("details-toggle").focus();
  for (let i = 0; i < 8; i += 1) {
    await page.keyboard.press("Tab");
    expect(await focusIn(page, "#details"), `Tab ${i + 1} went into the closed right column`).toBe(false);
  }
});

test("a session's menu is worked by keyboard alone, and the focus comes back to its ⋯", SPACE, async ({ page }) => {
  await open(page, URL);
  const id = await newSession(page, "says-ready");
  await expect(row(page, id)).toBeVisible();
  await row(page, id).focus();
  // `⋯` is the row's sibling: the next stop.
  await page.keyboard.press("Tab");
  expect(await focused(page)).toBe("session-more");
  await page.keyboard.press("Enter");
  const menu = page.getByTestId("session-menu");
  await expect(menu).toBeVisible();
  await expect(menu).toHaveAttribute("role", "menu");
  await expect(menu.locator('[role="menuitem"]').first()).toBeFocused();
  // Escape closes it and gives the focus back.
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  expect(await focused(page)).toBe("session-more");

  await page.keyboard.press("Enter");
  await expect(menu.locator('[data-item="open"]')).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menu.locator('[data-item="rename"]')).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(menu).toHaveCount(0);
  const input = page.locator("#modal input");
  await expect(input).toBeFocused();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("renamed by keyboard");
  await page.keyboard.press("Enter");
  await expect(row(page, id).locator(".name")).toHaveText("renamed by keyboard");
  await expect(page.locator("#modal")).toHaveCount(0);
  expect(await focused(page)).toBe("session-more");
});

test("the composer's + and model menus are worked by keyboard alone, like the shell's", SESSIONS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "says-ready");
  const input = page.getByTestId("conv-input");

  // `+`: a menu, the focus on its first item; Escape closes it and gives the focus back.
  const plus = page.getByTestId("conv-attach");
  await expect(plus).toHaveAttribute("aria-haspopup", "menu");
  await plus.focus();
  await page.keyboard.press("Enter");
  const attach = page.getByTestId("conv-attach-menu");
  await expect(attach).toHaveAttribute("role", "menu");
  const items = attach.locator('[role="menuitem"]:not(:disabled)');
  await expect(items.first()).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(attach).toHaveCount(0);
  expect(await focused(page)).toBe("conv-attach");

  // ↑/↓ go round, Home and End go to the ends, and Enter runs the item: `@` is inserted.
  await page.keyboard.press("Enter");
  await expect(items.first()).toBeFocused();
  const count = await items.count();
  expect(count, "the + menu has items to move between").toBeGreaterThan(1);
  await page.keyboard.press("ArrowDown");
  await expect(items.nth(1)).toBeFocused();
  await page.keyboard.press("End");
  await expect(items.last()).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(items.first()).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await expect(items.last()).toBeFocused();
  await page.keyboard.press("Home");
  await expect(items.first()).toBeFocused();
  for (let i = 0; i < count && (await focused(page)) !== "conv-attach-mention"; i += 1) await page.keyboard.press("ArrowDown");
  expect(await focused(page)).toBe("conv-attach-mention");
  await page.keyboard.press("Enter");
  await expect(attach).toHaveCount(0);
  await expect(input).toBeFocused();
  await expect(input).toHaveValue(/@$/);
  await input.fill("");

  // Tab closes a menu too, the focus back on what opened it.
  await plus.focus();
  await page.keyboard.press("Enter");
  await expect(items.first()).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(attach).toHaveCount(0);
  expect(await focused(page)).toBe("conv-attach");

  // The model: the options, then the field for a name of one's own — reached with ↓ like an item.
  const model = page.getByTestId("conv-model");
  await expect(model).toHaveAttribute("aria-haspopup", "menu");
  await model.focus();
  await page.keyboard.press("Enter");
  const models = page.getByTestId("conv-model-menu");
  await expect(models).toHaveAttribute("role", "menu");
  await expect(models.getByTestId("conv-model-option").first()).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(models).toHaveCount(0);
  expect(await focused(page)).toBe("conv-model");
  await page.keyboard.press("Enter");
  await expect(models.getByTestId("conv-model-option").first()).toBeFocused();
  await page.keyboard.press("End");
  await expect(models.getByTestId("conv-model-custom")).toBeFocused();
  await page.keyboard.type("claude-keyboard-model");
  await page.keyboard.press("Enter");
  await expect(model).toContainText("claude-keyboard-model");
  await expect(models).toHaveCount(0);
});

test("a right click opens the session's menu where it was clicked, not a dialog in the middle", SPACE, async ({ page }) => {
  await open(page, URL);
  const id = await newSession(page, "says-ready");
  await expect(row(page, id)).toBeVisible();
  const box = (await row(page, id).boundingBox())!;
  const point = { x: box.x + 30, y: box.y + box.height / 2 };
  await page.mouse.click(point.x, point.y, { button: "right" });
  const menu = page.getByTestId("session-menu");
  await expect(menu).toBeVisible();
  await expect(page.locator("#veil")).toHaveAttribute("data-state", "closed");
  await expect(page.locator("#modal")).toHaveCount(0);
  const at = (await menu.boundingBox())!;
  expect(Math.abs(at.x - point.x), "opened at the pointer").toBeLessThan(2);
  expect(Math.abs(at.y - point.y), "opened at the pointer").toBeLessThan(2);
  for (const item of ["open", "rename", "archive", "fork", "export", "close", "delete"]) {
    await expect(menu.locator(`[role="menuitem"][data-item="${item}"]`)).toBeVisible();
  }
  // A click elsewhere closes it.
  await page.locator("#main").click({ position: { x: 10, y: 200 } });
  await expect(menu).toHaveCount(0);
});

test("a turn is not stopped by one Escape too many: the first says what the second does", SESSIONS, async ({ page }) => {
  await open(page, URL);
  await asking(page);
  const input = page.getByTestId("conv-input");
  await input.focus();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("conv-esc-hint")).toBeVisible();
  await expect(page.getByTestId("conv-esc-hint")).toHaveAttribute("role", "status");
  await expect(page.getByTestId("conv-ask")).toHaveAttribute("data-state", "pending");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("conv-note").last()).toContainText(/已中断|interrupted/i, { timeout: 20_000 });
  await expect(page.getByTestId("conv-esc-hint")).toHaveCount(0);
});

test("the banner is a live region, and its notices queue", SHELL, async ({ page }) => {
  await open(page, URL);
  const banner = page.locator("#banner");
  await page.evaluate(() => {
    const { app } = (window as any).harnix;
    app.notice("first of two");
    app.notice("second of two", "error");
  });
  await expect(banner).toHaveAttribute("role", "status");
  await expect(banner).toContainText("first of two");
  await expect(page.getByTestId("banner-waiting")).toBeVisible();
  await banner.getByRole("button").click();
  await expect(banner).toHaveAttribute("role", "alert");
  await expect(banner).toContainText("second of two");
  await banner.getByRole("button").click();
  await expect(banner).toHaveAttribute("data-state", "closed");
});

test("an ask's arrival is said aloud on its own, once", SESSIONS, async ({ page }) => {
  await open(page, URL);
  await asking(page);
  const said = page.getByTestId("ask-announce");
  await expect(said).toHaveAttribute("role", "alert");
  await expect(said).toContainText("Bash");
  await page.getByTestId("conv-ask").getByTestId("conv-ask-allow").click();
  await expect(page.getByTestId("conv-ask")).toHaveAttribute("data-state", "answered");
  await expect(said).toBeEmpty();
});
