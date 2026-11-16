/**
 * Schedules in a real browser against a real backend (harnix-app.md §9, system-space.md §6.2): a
 * daily schedule made for a session from its menu, its clock on the session's row; «run now» sends
 * the turn, and the delivery is in its history; a task schedule made from a project's overview starts
 * a task that is a row of the project, opening in the projects space; the system console's tab lists
 * every schedule with its source and state — the models' jobs, if any, read-only — and pauses,
 * resumes and deletes one. The editor's guided frequency, the zone picker and the conflict review are
 * the package's unit tests (`ui/packages/schedule/test`).
 *
 * Replayed: `says-ready` (the turn «run now» sends). Nothing here waits for a time to come.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { newSession, open, reply, sessionRow, spaceEntry, watchConsole } from "./page";

const TAGS = { tag: ["@feature:schedule", "@layer:ui"] };
const PROMPT = "Say exactly: fixture ready";

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => {
  await backend?.dispose();
});

const editor = (page: Page) => page.getByTestId("schedule-editor");
const call = (page: Page, method: string, params: Record<string, unknown>) =>
  page.evaluate(async ({ method, params }) => (window as any).harnix.app.call(method, params), { method, params });

/** A project of this product, registered and opened in the projects space with no conversation open: its overview in the right column. */
async function project(page: Page, name: string) {
  const path = await page.evaluate(async (name) => (await (window as any).harnix.app.call("harnix-app.projects.create", { name })).path as string, `${name}-${Date.now()}`);
  await page.evaluate(() => (window as any).harnix.app.refreshSessions());
  await spaceEntry(page, "projects").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
  await page.evaluate(() => (window as any).harnix.app.open(null));
  const row = page.getByTestId("projects").locator("[data-project]", { hasText: name });
  if ((await row.getAttribute("data-open")) === "true") await row.getByRole("treeitem", { level: 1 }).click();
  await row.getByRole("treeitem", { level: 1 }).click();
  await expect(row).toHaveAttribute("data-open", "true");
  return { path, row };
}

test("a daily schedule for a session from its menu: its clock on the row; run now sends the turn, the history has the delivery", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const id = await newSession(page, "says-ready");
  await expect(sessionRow(page, id)).toBeVisible();
  await expect(sessionRow(page, id).getByTestId("session-schedule-badge")).toHaveCount(0);

  // The session's menu: "定时…", a list of its schedules (none yet) and a new one.
  await sessionRow(page, id).click({ button: "right" });
  await page.locator('[data-testid="session-menu"] [data-item="action:schedule.session"]').click();
  const pane = page.getByTestId("schedule-session");
  await expect(pane).toHaveAttribute("data-sid", id);
  await expect(pane.getByTestId("schedule-row")).toHaveCount(0);
  await page.getByTestId("schedule-new").click();

  // The editor: a turn in this session; nothing goes while the title and the prompt are missing.
  await expect(editor(page)).toHaveAttribute("data-kind", "session");
  await page.getByTestId("schedule-save").click();
  await expect(page.getByTestId("schedule-problem-title")).toBeVisible();
  await expect(page.getByTestId("schedule-problem-prompt")).toBeVisible();
  await page.getByTestId("schedule-title").fill("Morning check");
  await page.getByTestId("schedule-prompt").fill(PROMPT);
  await page.getByTestId("schedule-freq-daily").click();
  await page.getByTestId("schedule-time").fill("09:00");
  await expect(page.getByTestId("schedule-freq-summary")).toContainText("09:00");
  // The zone is this machine's unless picked; a search narrows the list.
  await expect(page.getByTestId("schedule-zone-note")).toBeVisible();
  await page.getByTestId("schedule-save").click();
  await expect(editor(page)).toHaveCount(0);

  // Listed, on, with its next time; the session's row has the clock.
  const row = pane.getByTestId("schedule-row");
  await expect(row).toHaveCount(1, { timeout: 10_000 });
  await expect(row).toHaveAttribute("data-state", "on");
  await expect(row.getByTestId("schedule-next")).toHaveAttribute("data-next", /^\d+$/);
  await expect(sessionRow(page, id).getByTestId("session-schedule-badge")).toHaveAttribute("data-count", "1");
  const made = (await call(page, "schedule.list", { space: "sessions" })).schedules.find((s: any) => s.title === "Morning check");
  expect(made).toMatchObject({ space: "sessions", timing: { kind: "daily", time: "09:00" }, target: { kind: "session", session_id: id, prompt: PROMPT } });

  // Run now: delivered, and the delivery is the history's first.
  await row.getByTestId("schedule-run-now").click();
  await expect(row.getByTestId("schedule-last")).toHaveAttribute("data-outcome", "delivered", { timeout: 15_000 });
  await row.getByTestId("schedule-history-toggle").click();
  const receipt = row.getByTestId("schedule-receipt");
  await expect(receipt).toHaveCount(1);
  await expect(receipt).toHaveAttribute("data-outcome", "delivered");
  // Again within the minute: skipped, said why.
  await row.getByTestId("schedule-run-now").click();
  await expect(row.getByTestId("schedule-receipt")).toHaveCount(2, { timeout: 10_000 });
  await expect(row.getByTestId("schedule-receipt").first()).toHaveAttribute("data-outcome", "skipped");

  // The turn arrived in the session.
  await page.keyboard.press("Escape");
  await expect(pane).toHaveCount(0);
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  expect(errors).toEqual([]);
});

test("a task schedule from a project's overview starts a task: a row of the project, opening in the projects space", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const { path, row } = await project(page, "scheduled");
  const overview = page.locator('#details [data-testid="project-overview"]');
  await expect(overview).toBeVisible();
  // The block at 40: between the tasks and the recent changes.
  await expect
    .poll(async () => overview.locator('[data-testid="overview-block"]').evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.block)))
    .toEqual(["harnix-app.inbox", "harnix-app.sessions", "flow.tasks", "schedule.project", "harnix-app.changes", "harnix-app.settings", "harnix-app.trust"]);
  const block = overview.getByTestId("overview-schedules");
  await block.getByTestId("schedule-new").click();

  await page.getByTestId("schedule-target-task").click();
  await expect(page.getByTestId("schedule-project-fixed")).toHaveText(path);
  await page.getByTestId("schedule-flow").selectOption("example-review");
  await page.getByTestId("schedule-title").fill("Workday review");
  await page.getByTestId("schedule-subject").fill("the repo, on a schedule");
  await page.getByTestId("schedule-freq-workdays").click();
  await page.getByTestId("schedule-time").fill("09:00");
  await page.getByTestId("schedule-save").click();
  await expect(editor(page)).toHaveCount(0);

  const scheduled = block.locator('[data-testid="schedule-row"][data-kind="task"]');
  await expect(scheduled).toHaveCount(1, { timeout: 10_000 });
  const made = (await call(page, "schedule.list", { project: path })).schedules[0];
  expect(made).toMatchObject({ project: path, timing: { kind: "weekly", weekdays: [1, 2, 3, 4, 5], time: "09:00" }, target: { kind: "task", flow: "example-review", show_in_space: "sessions" } });

  // Run now: a run of the project, said to be the schedule's, shown in the session space.
  await scheduled.getByTestId("schedule-run-now").click();
  await expect(scheduled.getByTestId("schedule-last")).toHaveAttribute("data-outcome", "delivered", { timeout: 15_000 });
  const run = ((await call(page, "schedule.history", { id: made.id })).receipts[0].target.run) as string;
  expect(run).toMatch(/^example-review-/);
  const record = (await call(page, "flow.get", { run })).record;
  expect(record).toMatchObject({ project_root: path, show_in_space: "sessions", trigger: { kind: "schedule", schedule: made.id } });

  // The task is a row of the project, with the schedule's clock and its next time.
  const item = row.locator(`[data-testid="project-item"][data-item="${run}"]`);
  await expect(item).toBeVisible({ timeout: 15_000 });
  await expect(item.getByTestId("task-badges").locator(".hx-flow-task-clock")).toBeVisible();
  await expect(item.getByTestId("task-schedule-next")).toHaveAttribute("data-schedule", made.id);
  // Opened: its main session in the middle of the projects space, the 流程 tab on the run.
  await item.click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
  await expect(page.getByTestId("flow-task-tab")).toHaveAttribute("data-run", run, { timeout: 15_000 });
  // The delivery opens the same task: its main session, in the projects space.
  const main = ((record.session_keys ?? {})["@main"] as string).split("/")[1];
  await page.evaluate(() => (window as any).harnix.app.open(null));
  await scheduled.getByTestId("schedule-history-toggle").click();
  await scheduled.getByTestId("schedule-receipt-open").first().click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
  await expect.poll(() => page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId ?? null), { timeout: 15_000 }).toBe(main);
  await call(page, "flow.cancel", { run, reason: "done" });
  expect(errors).toEqual([]);
});

test("the console's tab lists every schedule with its source and state; the models' jobs, if any, only deletable; pause, resume, delete", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await page.locator('#side [data-space="system"]').click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "system");
  await page.locator('[data-testid="system-console"] [role="tab"][data-tab="schedule"]').click();
  const tab = page.getByTestId("system-schedules");
  await expect(tab).toBeVisible();

  // Both made above, by a person, on.
  const morning = tab.locator('[data-testid="schedule-row"]', { hasText: "Morning check" });
  const review = tab.locator('[data-testid="schedule-row"]', { hasText: "Workday review" });
  await expect(morning).toHaveAttribute("data-source", "user");
  await expect(morning).toHaveAttribute("data-state", "on");
  await expect(review).toHaveAttribute("data-kind", "task");
  await expect(morning.getByTestId("schedule-next")).toContainText(/·/);

  // A model's job is listed read-only: deletable, nothing else.
  const models = tab.getByTestId("schedule-model-row");
  for (const model of await models.all()) {
    await expect(model).toHaveAttribute("data-source", "model");
    await expect(model.getByTestId("schedule-model-delete")).toBeVisible();
    await expect(model.getByTestId("schedule-edit")).toHaveCount(0);
    await expect(model.getByTestId("schedule-run-now")).toHaveCount(0);
  }

  // Filtered by state; paused and resumed.
  await morning.getByTestId("schedule-toggle").click();
  await expect(morning).toHaveAttribute("data-state", "paused");
  await tab.getByTestId("schedule-filter-state").selectOption("paused");
  await expect(tab.getByTestId("schedule-row")).toHaveCount(1);
  await tab.getByTestId("schedule-filter-state").selectOption("");
  await morning.getByTestId("schedule-toggle").click();
  await expect(morning).toHaveAttribute("data-state", "on");

  // Deleted after asking: gone from the tab, the session it went to still there.
  await review.getByTestId("schedule-delete").click();
  await page.getByTestId("confirm").click();
  await expect(review).toHaveCount(0);
  expect(errors).toEqual([]);
});
