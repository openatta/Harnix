/**
 * Keyboard shortcuts, from the settings' "Keyboard shortcuts" section: the new-session key recorded
 * anew, and the page answering to the new key and not the old; a key another command has refused
 * with a warning; one key restored, then all of them.
 *
 * The keys are kept in this browser (`harnix.shortcuts.<command>`): each test starts from a page
 * whose storage it clears, so the order of the tests does not matter.
 */

import type { Locator, Page } from "@playwright/test";
import { expect, test } from "@harnix/testkit";

import { newSession, open, openSettings, settingsSection } from "./page";

const SETTINGS = { tag: ["@feature:settings", "@layer:ui"] };
const SHELL = { tag: ["@feature:shell", "@layer:ui"] };

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});

/** The page, with no key of the person's own left from another test. */
async function fresh(page: Page): Promise<void> {
  await open(page, URL);
  await page.evaluate(() => {
    for (const key of Object.keys(localStorage)) if (key.startsWith("harnix.shortcuts.")) localStorage.removeItem(key);
    (window as any).harnix.app.shortcuts.reload();
  });
}

async function shortcutsSection(page: Page): Promise<void> {
  await openSettings(page);
  await settingsSection(page, "shortcuts").click();
  await expect(page.getByTestId("shortcuts")).toBeVisible();
}

const row = (page: Page, command: string): Locator => page.locator(`[data-testid="shortcut-row"][data-command="${command}"]`);

/** Record `keys` as `command`'s key: click its key, then press them. ⌥⇧ with a letter: a key no
 *  browser keeps for itself, on any platform. */
async function record(page: Page, command: string, keys: string): Promise<void> {
  const button = row(page, command).getByTestId("shortcut-keys");
  await button.click();
  await expect(button).toHaveAttribute("data-state", "recording");
  await page.keyboard.press(keys);
  await expect(button).not.toHaveAttribute("data-state", "recording");
}

/** What the person keeps for the keys, in this browser. */
const stored = (page: Page) =>
  page.evaluate(() => Object.fromEntries(Object.keys(localStorage).filter((k) => k.startsWith("harnix.shortcuts.")).map((k) => [k, JSON.parse(localStorage.getItem(k)!)])));

const current = (page: Page) => page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId ?? null);

test("the new-session key, recorded anew: the new key starts a conversation, ⌘K no longer does, and the left column shows it", SHELL, async ({ page }) => {
  await fresh(page);
  const mac = await page.evaluate(() => (window as any).harnix.app.shortcuts.platform === "mac");
  const id = await newSession(page, "says-ready");
  await shortcutsSection(page);
  const keys = row(page, "shell.session.new").getByTestId("shortcut-keys");
  await expect(keys).toHaveText(mac ? "⌘K" : "Ctrl+K");

  await record(page, "shell.session.new", "Alt+Shift+j");
  await expect(keys).toHaveText(mac ? "⌥⇧J" : "Alt+Shift+J");
  await expect(row(page, "shell.session.new")).toHaveAttribute("data-state", "custom");
  expect(await stored(page)).toEqual({ "harnix.shortcuts.shell.session.new": "Alt+Shift+J" });

  // The dialog closed, the page answers to the new key and not the old one.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("settings-nav")).toHaveCount(0);
  await expect(page.locator("#new kbd")).toHaveText(mac ? "⌥⇧J" : "Alt+Shift+J");
  await page.keyboard.press("ControlOrMeta+k");
  expect(await current(page)).toBe(id);
  await page.keyboard.press("Alt+Shift+j");
  await expect.poll(() => current(page)).toBeNull();
});

test("a key another command has is refused with a warning, and the old key stays", SETTINGS, async ({ page }) => {
  await fresh(page);
  await shortcutsSection(page);
  const new_ = row(page, "shell.session.new");
  const before = await new_.getByTestId("shortcut-keys").textContent();

  // ⌘/ is the composer's.
  await record(page, "shell.session.new", "ControlOrMeta+/");
  await expect(new_.getByTestId("shortcut-warning")).toContainText(/聚焦输入框|Focus the composer/);
  await expect(new_.getByTestId("shortcut-keys")).toHaveText(before!);
  // ⌘C is copying text: never a command's.
  await record(page, "shell.session.new", "ControlOrMeta+c");
  await expect(new_.getByTestId("shortcut-warning")).toBeVisible();
  // A bare letter would be taken from whoever is typing.
  await record(page, "shell.session.new", "j");
  await expect(new_.getByTestId("shortcut-warning")).toBeVisible();
  await expect(new_.getByTestId("shortcut-keys")).toHaveText(before!);
  expect(await stored(page)).toEqual({});

  // Escape while recording gives up, and leaves the dialog open.
  await new_.getByTestId("shortcut-keys").click();
  await page.keyboard.press("Escape");
  await expect(new_.getByTestId("shortcut-keys")).not.toHaveAttribute("data-state", "recording");
  await expect(page.getByTestId("settings-nav")).toBeVisible();
});

test("one key restored to its default, then every key after asking", SETTINGS, async ({ page }) => {
  await fresh(page);
  await shortcutsSection(page);
  await record(page, "shell.session.new", "Alt+Shift+j");
  await record(page, "shell.details.toggle", "Alt+Shift+d");
  await record(page, "shell.sidebar.toggle", "Alt+Shift+b");
  expect(Object.keys(await stored(page))).toHaveLength(3);
  await expect(page.getByTestId("shortcuts-custom")).toContainText("3");

  await row(page, "shell.session.new").getByTestId("shortcut-reset").click();
  await expect(row(page, "shell.session.new")).not.toHaveAttribute("data-state", "custom");
  expect(Object.keys(await stored(page))).toHaveLength(2);

  await page.getByTestId("shortcuts-reset-all").click();
  // Asked first: called off, nothing changes.
  await page.getByTestId("shortcuts-reset-all-cancel").click();
  expect(Object.keys(await stored(page))).toHaveLength(2);
  await page.getByTestId("shortcuts-reset-all").click();
  await page.getByTestId("shortcuts-reset-all-confirm").click();
  await expect(row(page, "shell.details.toggle").getByTestId("shortcut-keys")).toHaveAttribute("data-state", "none");
  expect(await stored(page)).toEqual({});
  await expect(page.getByTestId("shortcuts-reset-all")).toBeDisabled();
});
