/**
 * A task in a project (harnix-app.md §3.2, system-space.md §5), in a real browser against a real
 * backend: a run started with `project_root` is a row of the project in the projects space, with
 * its flow, state and station; the project's "+" offers a new task, whose dialog needs a subject
 * and starts the run shown in the projects space (`show_in_space`); a task opened is its main
 * session in the middle of the projects space with the 流程 tab in the right column; the row's
 * menu enters the flow's space instead, with the way back; and the system console's flows page is
 * as it was. No turn is run to a model: the run's main session is replayed from nothing and its
 * station is forced through by hand where it matters.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, openRun, spaceEntry, toFlows, watchConsole } from "./page";

const TAGS = { tag: ["@feature:projects", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => { await backend?.dispose(); });

/** A project of this product, registered and opened in the projects space; its directory and its row. */
async function project(page: Page, name: string) {
  const path = await page.evaluate(async (name) => {
    const { app } = (window as any).harnix;
    return (await app.call("harnix-app.projects.create", { name })).path as string;
  }, `${name}-${Date.now()}`);
  await page.evaluate(() => (window as any).harnix.app.refreshSessions());
  await spaceEntry(page, "projects").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
  const row = page.getByTestId("projects").locator("[data-project]", { hasText: name });
  if ((await row.getAttribute("data-open")) !== "true") await row.getByRole("treeitem", { level: 1 }).click();
  await expect(row).toHaveAttribute("data-open", "true");
  return { path, row };
}

const current = (page: Page) => page.evaluate(() => ((window as any).harnix.app.snapshot.current?.sessionId as string | undefined) ?? null);
const mainOf = (page: Page, run: string) =>
  page.evaluate(async (r) => {
    const v = await (window as any).harnix.app.call("flow.get", { run: r });
    return ((v.record.session_keys ?? {})["@main"] as string | undefined)?.split("/")[1] ?? null;
  }, run);
const chip = (page: Page, kind: string) => page.locator(`#details [data-testid="sidebar-tab"][data-kind="${kind}"]`);

test("a run started in a project is a task row of the project, with its flow, state and station", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const { path, row } = await project(page, "tasks-list");
  const run = await page.evaluate(async (root) => (await (window as any).harnix.app.call("flow.start", { flow: "example-review", subject: "the repo", title: "Review it", project_root: root })).run as string, path);

  const item = row.locator(`[data-testid="project-item"][data-item="${run}"]`);
  await expect(item).toBeVisible();
  await expect(item).toHaveAttribute("data-kind", "flow.task");
  await expect(item).toContainText("Review it");
  await expect(item).toContainText("Example: review and approve");
  const meta = item.getByTestId("task-meta");
  await expect(meta).toHaveAttribute("data-phase", /^(running|waiting)$/, { timeout: 30_000 });
  await expect(meta).toHaveAttribute("data-step", /^\d\/3$/);
  // The row is counted on the project, and the task is not in the session space's list.
  await expect(row.locator(".prj .c")).toHaveText("1");
  await spaceEntry(page, "sessions").click();
  await expect(page.locator(`[data-testid="session-row"][data-sid="${await mainOf(page, run)}"]`)).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("the project's + offers a new task: the dialog needs a subject, says the project is changed, and starts the run; the task opens on its main session with the 流程 tab", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const { path, row } = await project(page, "tasks-new");
  await row.getByTestId("project-new").click();
  await page.locator('[data-testid="host-menu"] [data-item="item:flow.task"]').click();

  // The dialog: the built-in flow picked, the broken one (none here) would be disabled; the subject is required.
  const flow = page.getByTestId("task-flow-example-review");
  await expect(flow).toHaveAttribute("aria-pressed", "true");
  await expect(flow).toHaveAttribute("data-origin", "builtin");
  await expect(page.getByTestId("task-modifies")).toBeVisible();
  await expect(page.getByTestId("task-worktree")).toBeDisabled();
  await expect(page.getByTestId("task-output-dir")).toHaveAttribute("placeholder", /^flows\/example-review-\d{10}$/);
  await expect(page.getByTestId("task-start")).toBeDisabled();
  await page.getByTestId("task-subject").fill("the repo, from the dialog");
  await page.getByTestId("task-title").fill("From the dialog");
  await expect(page.getByTestId("task-start")).toBeEnabled();
  await page.getByTestId("task-start").click();
  await expect(page.locator("#modal")).toHaveCount(0);

  // Opened where it was started: the projects space, its main session in the middle (the module
  // shows it in the session space), the tab in the right column.
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
  const tab = page.getByTestId("flow-task-tab");
  await expect(tab).toHaveAttribute("data-run", /^example-review-/, { timeout: 15_000 });
  const run = (await tab.getAttribute("data-run"))!;
  await expect(chip(page, "flow")).toHaveAttribute("data-state", "current");
  await expect.poll(() => current(page), { timeout: 15_000 }).toBe(await mainOf(page, run));
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
  // Nothing beside the conversation: the run is in the right column; no way back to draw.
  await expect(page.locator("#secondary")).toHaveCount(0);
  await expect(page.getByTestId("flow-back")).toHaveCount(0);
  await expect(page.getByTestId("flow-run-panel")).toHaveAttribute("data-run", run);
  await expect(page.getByTestId("flow-run-phase")).toHaveAttribute("data-phase", /^(running|waiting)$/);
  await expect(page.getByTestId("flow-run-stations").locator("tr[data-testid^=flow-run-station-]")).toHaveCount(3);
  await expect(page.getByTestId("flow-output-dir")).toHaveAttribute("data-path", /example-review-\d{10}/);
  // The run's record says the project, the title and where the session is shown; the row is current.
  const record = await page.evaluate(async (r) => (await (window as any).harnix.app.call("flow.get", { run: r })).record, run);
  expect(record.project_root).toBe(path);
  expect(record.title).toBe("From the dialog");
  expect(record.show_in_space).toBe("sessions");
  await expect(row.locator(`[data-testid="project-item"][data-item="${run}"]`)).toHaveAttribute("data-state", "current");
  // The session is lent to the session space, and the session space's list does not show it as a conversation.
  const lent = await page.evaluate(async (sid) => (await (window as any).harnix.app.call("harnix.sessions.list", { space: "sessions" })).sessions.find((r: any) => r.session_id === sid), await mainOf(page, run));
  expect(lent).toMatchObject({ space: "flow", exposed_from: "flow" });
  await spaceEntry(page, "sessions").click();
  await expect(page.locator(`[data-testid="session-row"][data-sid="${await mainOf(page, run)}"]`)).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("a task's row opens it in the projects space too, where a person can talk to it; the 流程 tab keeps the run across a reload", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const { path, row } = await project(page, "tasks-open");
  const run = await page.evaluate(
    async (root) => (await (window as any).harnix.app.call("flow.start", { flow: "example-review", subject: "again", project_root: root, show_in_space: "sessions" })).run as string,
    path,
  );
  const item = row.locator(`[data-testid="project-item"][data-item="${run}"]`);
  await item.click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
  await expect(page.getByTestId("flow-task-tab")).toHaveAttribute("data-run", run, { timeout: 15_000 });
  await expect.poll(() => current(page), { timeout: 15_000 }).toBe(await mainOf(page, run));
  // The composer is open to the session: a person can talk to the task from here.
  await expect(page.getByTestId("conv-input")).toBeVisible();

  // The station panel is on the station at work: its hands are there (force, for a session station).
  await expect(page.getByTestId("flow-run-panel")).toHaveAttribute("data-station", "review", { timeout: 30_000 });
  await expect(page.getByTestId("flow-station-panel").getByTestId("flow-force")).toBeVisible();

  // Reloaded: the tab is still there, on the same run (the module says whose main session this is; no params after a reload).
  await page.reload();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects", { timeout: 15_000 });
  await expect(page.getByTestId("flow-task-tab")).toHaveAttribute("data-run", run, { timeout: 15_000 });
  expect(errors).toEqual([]);
});

test("the row's menu opens a task in the flows: the flow's space in front on the run, with the way back; a run shown nowhere else opens so from its row", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const { path, row } = await project(page, "tasks-enter");
  const run = await page.evaluate(
    async (root) => (await (window as any).harnix.app.call("flow.start", { flow: "example-review", subject: "shown", project_root: root, show_in_space: "sessions" })).run as string,
    path,
  );
  const item = row.locator(`[data-testid="project-item"][data-item="${run}"]`);
  await item.hover();
  await item.locator("..").getByTestId("project-item-more").click();
  await page.locator('[data-testid="host-menu"] [data-item="flow.task.enter"]').click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow", { timeout: 15_000 });
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "example-review");
  await expect(page.getByTestId("flow-task-tab")).toHaveAttribute("data-run", run, { timeout: 15_000 });
  await expect.poll(() => current(page), { timeout: 15_000 }).toBe(await mainOf(page, run));
  await expect(page.locator("#secondary")).toHaveCount(0);
  // Back goes to the projects space, where the task is current.
  await expect(page.getByTestId("flow-back")).toHaveAttribute("data-to", "projects");
  await page.getByTestId("flow-back").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
  await expect(item).toHaveAttribute("data-state", "current");

  // A run started without a space to show in (the console's way, a schedule's): its row enters the flows too.
  const elsewhere = await page.evaluate(async (root) => (await (window as any).harnix.app.call("flow.start", { flow: "example-review", subject: "unshown", project_root: root })).run as string, path);
  await row.locator(`[data-testid="project-item"][data-item="${elsewhere}"]`).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow", { timeout: 15_000 });
  await expect(page.getByTestId("flow-task-tab")).toHaveAttribute("data-run", elsewhere, { timeout: 15_000 });
  await expect(page.getByTestId("flow-back")).toHaveAttribute("data-to", "projects");
  expect(errors).toEqual([]);
});

test("the system console's flows page is as it was: a run opened there has its details beside the conversation", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const { path } = await project(page, "tasks-console");
  const run = await page.evaluate(async (root) => (await (window as any).harnix.app.call("flow.start", { flow: "example-review", subject: "console", project_root: root })).run as string, path);
  await toFlows(page);
  await expect(page.getByTestId("flow-def-example-review")).toBeVisible();
  await openRun(page, run);
  await expect(page.locator('#secondary [data-testid="flow-run-detail"]')).toHaveAttribute("data-run", run);
  await expect(page.locator("#secondary").getByTestId("flow-run-phase")).toBeVisible();
  await expect(page.getByTestId("flow-back")).toHaveAttribute("data-to", "system");
  expect(errors).toEqual([]);
});
