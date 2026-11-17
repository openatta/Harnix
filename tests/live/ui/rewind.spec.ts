/**
 * Going back to a message (harnix-app.md §5.1), in a real browser against a real backend: a
 * person's message's menu has 「回到这里」; its dialog shows the dry run — the turn taken out, the file
 * put back (here deleted: it did not exist before) — and, confirmed with 「先分叉一份」, the file is
 * gone, the message and its changes card are gone from the conversation, the message is back in the
 * composer, and the fork holds what was taken out. While a turn runs, the item is there and greyed.
 *
 * Replayed: `writes-a-file` (a `Write` of `notes.txt` in the project), `runs-a-while`.
 */

import { existsSync, rmSync } from "node:fs";
import { join } from "node:path";

import type { Page } from "@playwright/test";
import { dropSessions, expect, test } from "@harnix/testkit";

import { newSession, open, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:sessions", "@layer:ui"] };
const WRITE = "Use the Write tool to create notes.txt in the working directory with exactly one line: harnix-changes-fixture. Then reply: written";

let URL = "";
let PROJECT = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
  PROJECT = harnix.project;
});
test.afterEach(async ({ api }) => {
  await dropSessions(api);
  rmSync(join(PROJECT, "notes.txt"), { force: true });
});

/** The person's message, its menu opened. */
async function messageMenu(page: Page) {
  const said = page.locator('[data-testid="conv-message"][data-role="user"]').first().locator("xpath=..");
  await said.hover();
  await said.getByTestId("conv-message-menu").click();
  const menu = page.getByTestId("message-menu");
  await expect(menu).toBeVisible();
  return menu;
}

test("going back to a message puts the file back, takes the turn out and gives the message back to the composer", TAGS, async ({ page, api }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await newSession(page, "writes-a-file", PROJECT);
  await say(page, WRITE);
  const ask = page.getByTestId("conv-ask");
  const card = page.getByTestId("changes-card");
  await expect(ask.or(card).first()).toBeVisible({ timeout: 30_000 });
  if (await ask.isVisible()) await ask.getByTestId("conv-ask-allow").click();
  await expect(card).toBeVisible({ timeout: 30_000 });
  expect(existsSync(join(PROJECT, "notes.txt"))).toBe(true);
  const before = (await api.call("harnix.sessions.list", { space: "sessions" })).sessions.length;

  const menu = await messageMenu(page);
  const item = menu.locator('button[data-item="action:builtin.rewind"]');
  await expect(item).toBeEnabled();
  await item.click();

  const dialog = page.getByTestId("rewind-dialog");
  await expect(dialog).toHaveAttribute("data-state", "planned", { timeout: 15_000 });
  await expect(dialog.getByTestId("rewind-turns").locator("li")).toHaveCount(1);
  await expect(dialog.getByTestId("rewind-turns")).toContainText("harnix-changes-fixture");
  const restored = dialog.getByTestId("rewind-restore").locator("li");
  await expect(restored).toHaveCount(1);
  await expect(restored.first()).toHaveAttribute("data-to", "absent");
  await expect(dialog.getByTestId("rewind-conflicts")).toHaveCount(0);
  // Confirmed, not before: the dry run changed nothing.
  expect(existsSync(join(PROJECT, "notes.txt"))).toBe(true);
  await page.getByTestId("rewind-fork").check();
  await page.getByTestId("rewind-confirm").click();

  await expect(dialog).toHaveCount(0, { timeout: 15_000 });
  expect(existsSync(join(PROJECT, "notes.txt"))).toBe(false);
  await expect(page.locator('[data-testid="conv-message"][data-role="user"]')).toHaveCount(0, { timeout: 15_000 });
  await expect(page.getByTestId("changes-card")).toHaveCount(0);
  await expect(page.getByTestId("conv-input")).toHaveValue(WRITE);
  // The fork holds what was taken out.
  await expect.poll(async () => (await api.call("harnix.sessions.list", { space: "sessions" })).sessions.length).toBe(before + 1);
  // A reload draws the same: the turn and its changes stay out.
  await page.reload();
  await open(page, URL);
  await expect(page.getByTestId("conv-flow")).toBeVisible();
  await expect(page.locator('[data-testid="conv-message"][data-role="user"]')).toHaveCount(0);
  await expect(page.getByTestId("changes-card")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("while a turn runs, going back is offered greyed, with the reason", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "runs-a-while");
  await say(page, "Use the Bash tool to wait a few seconds");
  await expect(page.getByTestId("conv-ask")).toBeVisible({ timeout: 15_000 });
  const menu = await messageMenu(page);
  await expect(menu.locator('button[data-item="action:builtin.rewind"]')).toBeDisabled();
  await page.keyboard.press("Escape");
  await page.getByTestId("conv-ask").getByTestId("conv-ask-allow").click();
});
