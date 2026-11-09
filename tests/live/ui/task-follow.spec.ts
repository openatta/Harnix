/**
 * A running task's output followed in the system space: the tasks tab reads it while it runs and,
 * following, keeps what just arrived in view.
 *
 * The task is a Monitor printing a line every quarter second (`streams-output`). Monitor is a tool
 * of the `code` scene, so this backend activates it beside the default.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";

import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:system", "@layer:ui"] };

let backend: Backend;

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay", { profile: '\n[engine]\nscenes = ["code"]\n' }));
  // Teams are the operator's switch too: on for this backend's project, before any session reads
  // its settings.
  mkdirSync(join(backend.project, ".agix"), { recursive: true });
  writeFileSync(join(backend.project, ".agix", "settings.json"), JSON.stringify({ execution: { team_enabled: true } }));
});
test.afterAll(async () => {
  await backend?.dispose();
});

/**
 * The running task of session `sid` in the Agents list, found by its own id. Another session's
 * task (an earlier test's monitor still counting) can be listed too, and the list orders its rows
 * by how they stand, which changes as turns end: a locator by position can point at one row when
 * it is resolved and another when it is clicked.
 */
async function runningTask(page: Page, sid: string) {
  const any = page.locator(`[data-testid="system-agents"] [data-testid="system-agent-group"][data-sid="${sid}"] [data-task][data-status="running"]`).first();
  await expect(any).toBeVisible({ timeout: 15_000 });
  const id = (await any.getAttribute("data-task"))!;
  return page.locator(`[data-testid="system-agents"] [data-task="${id}"]`);
}

test("system-tasks: a running task's output grows in the tasks tab and, followed, stays scrolled to its end", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await page.evaluate(() =>
    (window as any).harnix.app.create({ scene: "code", options: { recorder: { name: "streams-output" }, permission_mode: "default" } }),
  );
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  await say(page, "Start watching the counter");
  const ask = page.getByTestId("conv-ask");
  await expect(ask).toBeVisible({ timeout: 30_000 });
  await ask.getByTestId("conv-ask-allow").click();

  const sid = await page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId as string);

  await page.locator('#side [data-space="system"]').click();
  await expect(page.getByTestId("system-console")).toBeVisible();
  const row = await runningTask(page, sid);
  await row.click();

  const text = page.getByTestId("task-output-text");
  await expect(text).toContainText("line-1");
  await expect(page.getByTestId("task-follow")).toBeChecked();
  // The output is under the task's header, as wide as the task — not squeezed in beside it.
  // Measured once the right column has finished opening: while it slides open the two are measured
  // at different moments of it.
  const width = async (selector: string) => (await page.locator(selector).first().boundingBox())!.width;
  await expect
    .poll(async () => (await width('[data-testid="task-output-text"]')) / (await width('[data-testid="system-task"][data-state="current"]')), { timeout: 5_000 })
    .toBeGreaterThan(0.8);
  // The task's own detail: what it is, how it stands.
  const detail = page.getByTestId("system-task");
  await expect(detail.locator(".facts")).toContainText(/Monitor|监视器/);
  await expect(detail.locator(".facts")).toContainText(/Running|运行中/);
  const lines = async () => ((await text.textContent()) ?? "").split("\n").filter((l) => l.startsWith("line-")).length;
  const first = await lines();
  // Long enough to overflow the box (it holds about seventeen lines), so following has to scroll. The
  // counter prints four lines a second, and slower on a loaded machine: it gets the time it needs.
  await expect.poll(lines, { timeout: 30_000 }).toBeGreaterThanOrEqual(Math.max(first + 3, 25));
  expect(await text.evaluate((el) => el.scrollHeight > el.clientHeight), "the output overflows its box").toBe(true);
  // Read piece by piece from where the last read ended: the lines in order, none twice.
  const seen = ((await text.textContent()) ?? "").split("\n").filter((l) => l.startsWith("line-"));
  const at = Number(seen[0]!.slice(5));
  expect(seen).toEqual(seen.map((_, i) => `line-${at + i}`));
  // Following: the newest line is in view, the box scrolled to its end.
  await expect
    .poll(() => text.evaluate((el) => el.scrollHeight - el.clientHeight - el.scrollTop), { timeout: 5_000 })
    .toBeLessThanOrEqual(2);
  expect(errors).toEqual([]);
});

test("system-tasks: a task that outlives its turn is listed as running, and is stopped from the tasks tab", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await page.evaluate(() =>
    (window as any).harnix.app.create({ scene: "code", options: { recorder: { name: "streams-output" }, permission_mode: "default" } }),
  );
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  await say(page, "Start watching the counter");
  const ask = page.getByTestId("conv-ask");
  await expect(ask).toBeVisible({ timeout: 30_000 });
  await ask.getByTestId("conv-ask-allow").click();
  // The turn ends long before the monitor does: what is running is the task alone.
  await expect(page.getByTestId("conv-send")).toBeVisible({ timeout: 30_000 });

  const sid = await page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId as string);

  await page.locator('#side [data-space="system"]').click();
  // The first test's monitor may still be counting: this session's task is the one looked for.
  await expect(page.locator('[data-testid="system-strip"] [data-kpi="tasks"] .v')).not.toHaveText("0", { timeout: 15_000 });
  const row = await runningTask(page, sid);
  const id = (await row.getAttribute("data-task"))!;
  await row.click();
  const task = page.locator('[data-testid="system-task"][data-state="current"]');
  await expect(task).toHaveAttribute("data-status", "running");
  // As DSH has it: the first press arms the button, the second stops.
  await task.getByTestId("task-stop").click();
  await expect(task.getByTestId("task-stop")).toHaveAttribute("data-armed", "");
  await expect(task).toHaveAttribute("data-status", "running");
  await task.getByTestId("task-stop").click();
  // Long before its ten seconds are up: no longer running, and nothing left to stop.
  await expect(task).not.toHaveAttribute("data-status", /running|waiting|pending/, { timeout: 5_000 });
  await expect(task.getByTestId("task-stop")).toHaveCount(0);
  await expect(page.locator(`[data-testid="system-agents"] [data-task="${id}"][data-status="running"]`)).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("system-teams: a team's stage and members are drawn under its Agent and on the tasks tab", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await page.evaluate(() => (window as any).harnix.app.create({ scene: "code", options: { recorder: { name: "runs-a-team" } } }));
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  const id = await page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId as string);
  await say(page, "Run a team of one: alpha says it is here");
  await expect(page.locator('[data-testid="conv-message"]', { hasText: "Team done." })).toBeVisible({ timeout: 30_000 });

  await page.locator('#side [data-space="system"]').click();
  const row = page.locator(`[data-testid="system-agents"] [data-testid="system-agent-group"][data-sid="${id}"]`);
  await row.getByTestId("system-agent-toggle").click();
  const team = row.locator('[data-testid="team-board"] .team[data-team="ui-team"]');
  await expect(team).toHaveAttribute("data-status", "completed", { timeout: 15_000 });
  await expect(team).toContainText(/1 of 1|1 \/ 1/);
  await expect(team.locator('.member[data-member="alpha"]')).toHaveAttribute("data-status", "done");
  // And on the Agent's tasks tab.
  await row.getByTestId("system-agent").click();
  await page.locator('#details [role="tab"][data-tab="system.tasks"]').click();
  await expect(page.locator('[data-testid="system-tasks"] [data-testid="team-board"] .team[data-team="ui-team"]')).toBeVisible();
});

test("system-tasks: a running sub-agent is told something from its own detail, and it is delivered", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await page.evaluate(() => (window as any).harnix.app.create({ scene: "harnix", options: { recorder: { name: "messages-a-subagent" } } }));
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  const id = await page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId as string);
  await say(page, "Start a background sub-agent that waits, then answers");
  // The sub-agent's Bash call asks in its parent's conversation; let it through.
  const ask = page.getByTestId("conv-ask");
  await expect(ask).toBeVisible({ timeout: 30_000 });
  await ask.getByTestId("conv-ask-allow").click();

  await page.locator('#side [data-space="system"]').click();
  const row = page.locator(`[data-testid="system-agents"] [data-testid="system-agent-group"][data-sid="${id}"]`);
  const sub = row.getByTestId("system-subagent").first();
  // A row with a task running opens by itself; open it only if it has not.
  const toggle = row.getByTestId("system-agent-toggle");
  if ((await toggle.getAttribute("aria-expanded")) !== "true") await toggle.click();
  await expect(sub).toHaveAttribute("data-status", "running", { timeout: 15_000 });
  await sub.click();
  const pane = page.getByTestId("system-subagent-pane");
  await pane.getByTestId("task-message-text").fill("When you are done, say so.");
  await pane.getByTestId("task-message-send").click();
  await expect(pane.getByTestId("task-message-said")).toHaveText(/Delivered|已送达/);
  await expect(pane.getByTestId("task-message-text")).toHaveValue("");
});
