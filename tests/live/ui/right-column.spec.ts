/**
 * The right column's pages about the conversation's project (harnix-app.md §6), in a real browser
 * against a real backend: the files tab from the guide, a file opened in the preview and a line
 * landed on, a file tool's path in its row opening the preview, and the changes card a turn that
 * wrote a file leaves in the conversation — still there after a reload — opening the changes tab
 * at its turn, with the file's diff.
 *
 * The turn that writes is replayed from `writes-a-file`: a `Write` of `notes.txt` in the project.
 */

import { rmSync } from "node:fs";
import { join } from "node:path";

import type { Page } from "@playwright/test";
import { dropSessions, expect, test } from "@harnix/testkit";

import { newSession, open, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:session-space", "@layer:ui"] };
const WRITE = "Use the Write tool to create notes.txt in the working directory with exactly one line: harnix-changes-fixture. Then reply: written";

let URL = "";
let PROJECT = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
  PROJECT = harnix.project;
});
// The worker's backend is shared by every file that uses it: a test leaves no session, and no file, behind.
test.afterEach(async ({ api }) => {
  await dropSessions(api);
  rmSync(join(PROJECT, "notes.txt"), { force: true });
});

const frame = (page: Page) => page.locator("#frame");
const chip = (page: Page, kind: string) => page.locator(`#details [data-testid="sidebar-tab"][data-kind="${kind}"]`);

/** The column open on the session's surface. */
async function expand(page: Page): Promise<void> {
  if ((await frame(page).getAttribute("data-details")) !== "open") await page.getByTestId("details-toggle").click();
  await expect(frame(page)).toHaveAttribute("data-details", "open");
}

/** The `writes-a-file` turn, its permission allowed if it is asked for, until its changes card is in. */
async function writeTurn(page: Page): Promise<void> {
  await say(page, WRITE);
  const ask = page.getByTestId("conv-ask");
  const card = page.getByTestId("changes-card");
  await expect(ask.or(card).first()).toBeVisible({ timeout: 30_000 });
  if (await ask.isVisible()) await ask.getByTestId("conv-ask-allow").click();
  await expect(card).toBeVisible({ timeout: 30_000 });
}

test("the files tab lists the session's project, and a file opens in the preview at a line", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await newSession(page, "says-ready", PROJECT);
  await expand(page);
  // From the guide: its files card.
  await page.locator('[data-testid="sidebar-guide-entry"][data-kind="files"]').click();
  await expect(chip(page, "files")).toHaveAttribute("data-state", "current");
  const files = page.getByTestId("files-tab");
  await expect(files).toHaveAttribute("data-state", "ready");
  await expect(files.locator('[data-testid="files-row"][data-path="Cargo.toml"]')).toBeVisible();
  await expect(files.locator('[data-testid="files-row"][data-path="README.md"]')).toBeVisible();
  // A file opens in the preview, a tab of its own beside the tree.
  await files.locator('[data-testid="files-row"][data-path="Cargo.toml"]').click();
  await expect(chip(page, "preview")).toHaveAttribute("data-state", "current");
  await expect(chip(page, "preview").locator('[role="tab"]')).toHaveText("Cargo.toml");
  const preview = page.getByTestId("preview-tab");
  await expect(preview).toHaveAttribute("data-path", "Cargo.toml");
  await expect(preview.getByTestId("preview-code")).toContainText("[package]");
  // Sent to a line, as a file link with a line sends it: the same tab, that line marked.
  await page.evaluate(() => (window as any).harnix.host.sidebar.openResource("harnix-resource://file/Cargo.toml", { params: { line: 2 } }));
  await expect(chip(page, "preview")).toHaveCount(1);
  await expect(preview.getByTestId("preview-hit")).toHaveAttribute("data-line", "2");
  await expect(preview.getByTestId("preview-hit")).toContainText('name = "fixture"');
  // Out of the project is not a file of it: the preview says it cannot read it.
  await page.evaluate(() => (window as any).harnix.host.sidebar.openResource("harnix-resource://file/../outside.txt"));
  await expect(page.getByTestId("preview-message")).toContainText(/outside the session's project/);
  expect(errors).toEqual([]);
});

test("a file tool's path in its row opens the file in the preview", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "writes-a-file", PROJECT);
  await writeTurn(page);
  const row = page.getByTestId("conv-tool").first();
  const arg = row.getByTestId("conv-tool-args");
  await expect(arg).toHaveAttribute("data-action", "harnix-app.open-file");
  await arg.click();
  // The row did not open for it; the column did, on the file.
  await expect(row.getByTestId("conv-tool-toggle")).toHaveAttribute("aria-expanded", "false");
  await expect(frame(page)).toHaveAttribute("data-details", "open");
  const preview = page.getByTestId("preview-tab");
  await expect(preview).toHaveAttribute("data-path", "notes.txt");
  await expect(preview.getByTestId("preview-code")).toContainText("harnix-changes-fixture");
  // Under the call, the same door as a button.
  await row.getByTestId("conv-tool-toggle").click();
  await expect(row.getByTestId("action-harnix-app.open-file")).toBeVisible();
});

test("a turn that wrote a file leaves a changes card, after a reload too, that opens the changes tab with its diff", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await newSession(page, "writes-a-file", PROJECT);
  await writeTurn(page);
  const card = page.getByTestId("changes-card");
  await expect(card).toContainText(/改了 1 个文件|Files changed: 1/);
  await expect(card).toContainText("notes.txt");
  const turn = await card.getAttribute("data-turn");
  expect(turn).toBeTruthy();
  // Kept in the session's log, not only seen go by: a reload draws it again.
  await page.reload();
  await open(page, URL);
  await expect(page.getByTestId("changes-card")).toHaveAttribute("data-turn", turn!, { timeout: 20_000 });
  await page.getByTestId("changes-card-open").click();
  await expect(frame(page)).toHaveAttribute("data-details", "open");
  await expect(chip(page, "changes")).toHaveAttribute("data-state", "current");
  const changes = page.getByTestId("changes-tab");
  await expect(changes).toHaveAttribute("data-turn", turn!);
  const file = changes.locator('[data-testid="changes-file"][data-path="notes.txt"]');
  await expect(file).toHaveAttribute("data-state", "open");
  await expect(file.locator('[data-testid="conv-diff-line"][data-op="add"]')).toContainText("harnix-changes-fixture");
  // From a changed file to the file itself.
  await file.getByTestId("changes-open-file").click();
  await expect(page.getByTestId("preview-tab")).toHaveAttribute("data-path", "notes.txt");
  expect(errors).toEqual([]);
});
