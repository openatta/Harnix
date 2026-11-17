/**
 * The home page in the browser (harnix-app.md §10): a page opened afresh lands on it, first in the
 * left column; a reload stays where the tab was. It holds no conversation — no composer, no "new
 * session" — only the blocks of `harnix-app.home.panels`, by order, each its package's: the first
 * run while unfinished (`first-run.spec.ts`), the process at a glance (§10.1), what waits, the
 * recent sessions, the recent tasks, every schedule (§10.2, `schedule.spec.ts`), the usage of the
 * last 30 days (§10.3, `usage-summary.spec.ts`).
 * That the backend refuses a session there is the API's (`no_sessions`).
 *
 * Replayed: `says-ready`, `runs-a-while` (a turn whose Bash call waits on its permission).
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { connected, newSession, open, reply, say, sessionRow, spaceEntry, toSpace, watchConsole } from "./page";

const TAGS = { tag: ["@feature:home", "@layer:ui"] };
const BLOCKS = ["harnix-app.setup", "observability.stats", "harnix-app.inbox", "harnix-app.sessions", "flow.tasks", "schedule.all", "observability.summary"];

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => {
  await backend?.dispose();
});

const main = (page: Page) => page.locator("#main");
const block = (page: Page, id: string) => page.locator(`[data-testid="home-block"][data-block="${id}"]`);

test("a page opened afresh lands on the home page, first in the left column: its blocks by order, no conversation and no new session of its own", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await expect(main(page)).toHaveAttribute("data-space", "home");
  const ids = await page.getByTestId("space-list").getByTestId("space-entry").evaluateAll((nodes) => nodes.map((n) => (n as HTMLElement).dataset.space));
  expect(ids[0]).toBe("home");
  await expect(spaceEntry(page, "home")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("home")).toBeVisible();
  await expect.poll(() => page.getByTestId("home-block").evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.block))).toEqual(BLOCKS);
  // The wide ones take the page's width.
  for (const id of ["observability.stats", "schedule.all", "observability.summary"]) await expect(block(page, id)).toHaveClass(/\bwide\b/);
  // A page of its own: nothing to type into, no way to start a session here, no right column.
  await expect(page.getByTestId("conv-input")).toHaveCount(0);
  await expect(page.locator("#new")).toHaveCount(0);
  await expect(page.locator("#hero")).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).harnix.app.snapshot.current ?? null)).toBeNull();
  expect(errors).toEqual([]);
});

test("a reload stays where the tab was; another tab lands on the home page again", TAGS, async ({ page, context }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await expect(main(page)).toHaveAttribute("data-space", "home");
  await spaceEntry(page, "sessions").click();
  await expect(main(page)).toHaveAttribute("data-space", "sessions");
  await page.reload();
  await connected(page);
  await expect(main(page)).toHaveAttribute("data-space", "sessions");
  // The front space is kept for the next tab too, but a new tab lands first.
  const other = await context.newPage();
  await open(other, backend.url);
  await expect(main(other)).toHaveAttribute("data-space", "home");
  await other.close();
  expect(errors).toEqual([]);
});

test("the statistics: the versions, the uptime counting on, memory and threads, the instances, and who is connected", TAGS, async ({ page, context }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const other = await context.newPage();
  await open(other, backend.url);
  const stats = block(page, "observability.stats").getByTestId("home-stats");
  await expect(stats.locator('[data-kpi="harnix"] .v')).toHaveText(/^\d+\.\d+\.\d+$/, { timeout: 15_000 });
  await expect(stats.locator('[data-kpi="agix"] .v')).toHaveText(/^\d+\.\d+\.\d+$/);
  await expect(stats.locator('[data-kpi="memory"] .v')).toHaveText(/MB$/);
  await expect(stats.locator('[data-kpi="threads"] .v')).toHaveText(/^\d+$/);
  await expect(stats.locator('[data-kpi="instances"] .v')).toHaveText(/^[1-9]\d*$/);
  // The uptime counts on by the second, between the pushes.
  const uptime = stats.locator('[data-kpi="uptime"] .v');
  const before = await uptime.textContent();
  await expect(uptime).not.toHaveText(before ?? "", { timeout: 5_000 });
  // Both pages connected; the list of them on hovering.
  await expect.poll(async () => Number(await stats.locator('[data-kpi="users"] .v').textContent()), { timeout: 15_000 }).toBeGreaterThanOrEqual(2);
  await stats.locator('[data-kpi="users"]').hover();
  await expect(stats.getByTestId("home-connections").locator(".row").first()).toBeVisible();
  await other.close();
  expect(errors).toEqual([]);
});

test("a turn waiting on its permission: the running sessions and what waits are counted, the count opens the list, a row the conversation", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const sid = await newSession(page, "runs-a-while");
  await say(page, "Use the Bash tool to wait a few seconds");
  await expect(page.getByTestId("conv-ask")).toBeVisible({ timeout: 30_000 });

  await toSpace(page, "home");
  const stats = page.getByTestId("home-stats");
  await expect(stats.locator('[data-kpi="sessions"] .v')).toHaveText("1", { timeout: 15_000 });
  const waiting = page.getByTestId("home-stats-waiting");
  await expect(waiting.locator(".v")).toHaveText("1", { timeout: 15_000 });
  await expect(block(page, "harnix-app.inbox").locator('[data-testid="inbox-item"][data-kind="approval"]')).toBeVisible();
  // Its running row among the recent sessions.
  await expect(block(page, "harnix-app.sessions").locator(`[data-testid="home-session"][data-sid="${sid}"] .live`)).toBeVisible();

  await waiting.click();
  const listed = page.getByTestId("modal").locator('[data-testid="inbox-item"]');
  await expect(listed).toHaveCount(1);
  await listed.click();
  await expect(page.getByTestId("modal")).toHaveCount(0);
  await expect(main(page)).toHaveAttribute("data-space", "sessions");
  await page.getByTestId("conv-ask").getByTestId("conv-ask-allow").click();
  await expect(page.getByTestId("conv-send")).toBeVisible({ timeout: 30_000 });

  await toSpace(page, "home");
  await expect(stats.locator('[data-kpi="sessions"] .v')).toHaveText("0", { timeout: 15_000 });
  await expect(waiting.locator(".v")).toHaveText("0");
  await expect(block(page, "harnix-app.inbox").getByTestId("inbox-empty")).toBeVisible();
  expect(errors).toEqual([]);
});

test("the recent sessions: each opening where it belongs; 全部会话 is the session space", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const sid = await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  await page.evaluate(() => (window as any).harnix.app.open(null));

  await toSpace(page, "home");
  const sessions = block(page, "harnix-app.sessions").getByTestId("home-sessions");
  const row = sessions.locator(`[data-testid="home-session"][data-sid="${sid}"]`);
  // Among the latest: the backend is the suite's, and the session the test before left running ends
  // its turn about now too, so which of the two is first is not this test's to say.
  await expect(row).toBeVisible({ timeout: 15_000 });
  await row.click();
  await expect(main(page)).toHaveAttribute("data-space", "sessions");
  await expect(sessionRow(page, sid)).toHaveAttribute("data-state", "current");
  await expect(reply(page)).toContainText(/fixture ready/i);

  await toSpace(page, "home");
  await sessions.getByTestId("home-sessions-all").click();
  await expect(main(page)).toHaveAttribute("data-space", "sessions");
  expect(errors).toEqual([]);
});

test("the recent tasks: every project's runs counted, the latest listed; one opens in the flow space, the way back to the home page", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const run = await page.evaluate(async () => (await (window as any).harnix.app.call("flow.start", { flow: "example-review", subject: "from home" })).run as string);
  const tasks = block(page, "flow.tasks").getByTestId("home-tasks");
  const row = tasks.locator(`[data-testid="home-task"][data-run="${run}"]`);
  await expect(row).toBeVisible({ timeout: 15_000 });
  await expect(tasks.getByTestId("home-tasks-counts")).toHaveAttribute("data-running", /^[1-9]\d*$/);
  await row.click();
  await expect(main(page)).toHaveAttribute("data-space", "flow", { timeout: 15_000 });
  await expect(page.locator('[data-testid="flow-task-tab"]:visible')).toHaveAttribute("data-run", run, { timeout: 15_000 });
  await expect(page.getByTestId("flow-back")).toHaveAttribute("data-to", "home");
  await page.getByTestId("flow-back").click();
  await expect(main(page)).toHaveAttribute("data-space", "home");
  await page.evaluate((r) => (window as any).harnix.app.call("flow.cancel", { run: r, reason: "done" }), run);
  await expect(tasks.locator(`[data-testid="home-task"][data-run="${run}"]`)).toBeVisible();
  expect(errors).toEqual([]);
});
