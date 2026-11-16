/**
 * A project's overview and 待我处理 (harnix-app.md §3.2, §3.3), in a real browser against a real
 * backend: a project opened with no conversation open shows its overview in the right column —
 * what waits, its conversations, its tasks, its recent changes, its settings in force, its trust —
 * and a conversation opened there hands the column back to the conversation; a permission a turn
 * waits on (`asks-permission`) is in 待我处理 — the overview's block, the left column's entry, the
 * system space's "等你处理" — and a row opens the conversation, where answering it empties the list.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { newSession, open, say, spaceEntry, watchConsole } from "./page";

const TAGS = { tag: ["@feature:projects", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => {
  await backend?.dispose();
});

/** A project of this product, registered and opened in the projects space with no conversation open. */
async function project(page: Page, name: string) {
  const path = await page.evaluate(async (name) => {
    const { app } = (window as any).harnix;
    return (await app.call("harnix-app.projects.create", { name })).path as string;
  }, `${name}-${Date.now()}`);
  await page.evaluate(() => (window as any).harnix.app.refreshSessions());
  await spaceEntry(page, "projects").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
  await toEmptyPage(page);
  const row = page.getByTestId("projects").locator("[data-project]", { hasText: name });
  // Opened from closed, with nothing open: the overview comes up in the right column.
  if ((await row.getAttribute("data-open")) === "true") await row.getByRole("treeitem", { level: 1 }).click();
  await row.getByRole("treeitem", { level: 1 }).click();
  await expect(row).toHaveAttribute("data-open", "true");
  return { path, row };
}

/** The new conversation's page: no conversation open in the front space. */
async function toEmptyPage(page: Page) {
  await page.evaluate(() => (window as any).harnix.app.open(null));
  await expect.poll(() => page.evaluate(() => (window as any).harnix.app.snapshot.current ?? null)).toBeNull();
}

const overview = (page: Page) => page.locator('#details [data-testid="project-overview"]');
const blocks = (page: Page) => overview(page).locator('[data-testid="overview-block"]');

test("a project opened with nothing open shows its overview; a conversation opened takes the column back", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const { path } = await project(page, "overview");

  await expect(overview(page)).toBeVisible();
  await expect(page.locator('#details [role="tab"][data-tab="harnix-app.overview"]')).toHaveAttribute("aria-selected", "true");
  // The blocks by order: the flows' tasks and the schedules between the conversations and the recent changes.
  await expect.poll(async () => blocks(page).evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.block))).toEqual([
    "harnix-app.inbox",
    "harnix-app.sessions",
    "flow.tasks",
    "schedule.project",
    "harnix-app.changes",
    "harnix-app.settings",
    "harnix-app.trust",
  ]);
  await expect(overview(page).getByTestId("inbox-empty")).toBeVisible();
  await expect(overview(page).getByTestId("overview-tasks")).toBeVisible();
  // Settings in force, each with a way to where it is changed; a new project is not trusted.
  await expect(overview(page).locator('[data-testid="overview-setting"][data-setting="permission_mode"]')).toBeVisible();
  await expect(overview(page).locator('[data-testid="overview-setting"][data-setting="mcp"]')).toBeVisible();
  await expect(overview(page).getByTestId("overview-trust")).toHaveAttribute("data-trust", /^(untrusted|none)$/);

  // A conversation in the project: the column is its surface now.
  const sid = await newSession(page, "says-ready", path);
  await say(page, "Say exactly: fixture ready");
  await expect(page.getByTestId("conv-send")).toBeVisible({ timeout: 30_000 });
  await expect(overview(page)).toHaveCount(0);

  // Back on the new conversation's page: the overview lists it, and it opens from there.
  await toEmptyPage(page);
  await page.evaluate(() => (window as any).harnix.app.refreshSessions());
  const listed = overview(page).locator(`[data-testid="overview-session"][data-sid="${sid}"]`);
  await expect(listed).toBeVisible({ timeout: 10_000 });
  await listed.click();
  await expect.poll(() => page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId ?? null)).toBe(sid);
  expect(errors).toEqual([]);
});

test("a permission a turn waits on is in 待我处理 — the overview, the left column, the system space's count — and a row opens it to be answered", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const { path } = await project(page, "inbox");
  const sid = await newSession(page, "asks-permission", path);
  await say(page, "Use the Bash tool to run exactly: touch harnix-permission-fixture");
  await expect(page.getByTestId("conv-ask")).toBeVisible({ timeout: 30_000 });

  // The left column says something waits, and its list has the ask.
  const entry = page.getByTestId("inbox-open");
  await expect(entry).toHaveAttribute("data-count", "1", { timeout: 10_000 });

  // The project's overview lists it too.
  await toEmptyPage(page);
  const item = overview(page).locator('[data-testid="inbox-item"][data-kind="approval"]');
  await expect(item).toBeVisible();
  await expect(item).toHaveAttribute("data-source", "harnix-app.asks");
  await expect(item).toContainText("Bash");

  // The system space's "等你处理" opens the same list.
  await spaceEntry(page, "system").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "system");
  const waiting = page.locator('[data-testid="system-strip"] [data-kpi="waiting"]');
  await expect(waiting).toHaveAttribute("data-testid", "system-kpi-link");
  await expect(waiting.locator(".v")).toHaveText("1", { timeout: 10_000 });
  await waiting.click();
  const listed = page.locator('[data-testid="inbox-list"] [data-testid="inbox-item"]');
  await expect(listed).toHaveCount(1);
  // A row opens its conversation where it belongs, the dialog closing; the card is answered there.
  await listed.click();
  await expect(page.getByTestId("inbox-list")).toHaveCount(0);
  await expect(page.locator("#main")).not.toHaveAttribute("data-space", "system");
  await expect.poll(() => page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId ?? null)).toBe(sid);
  const ask = page.getByTestId("conv-ask");
  await ask.getByTestId("conv-ask-allow").click();
  await expect(ask).toHaveAttribute("data-state", "answered");
  // Nothing waits any more.
  await expect(page.getByTestId("inbox-open")).toHaveCount(0, { timeout: 15_000 });
  await toEmptyPage(page);
  await expect(overview(page).getByTestId("inbox-empty")).toBeVisible();
  expect(errors).toEqual([]);
});
