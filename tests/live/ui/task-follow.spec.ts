/**
 * A running task's output followed in the system space: the tasks tab reads it while it runs and,
 * following, keeps what just arrived in view.
 *
 * The task is a Monitor printing a line every quarter second (`streams-output`). Monitor is a tool
 * of the `code` scene, so this backend activates it beside the default.
 */

import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:system", "@layer:ui"] };

let backend: Backend;

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay", { profile: '\n[engine]\nscenes = ["code"]\n' }));
});
test.afterAll(async () => {
  await backend?.dispose();
});

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

  await page.locator('#side [data-space="system"]').click();
  await expect(page.getByTestId("system-console")).toBeVisible();
  const row = page.locator('[data-testid="system-running"] [data-task][data-status="running"]').first();
  await expect(row).toBeVisible({ timeout: 15_000 });
  await row.click();

  const text = page.getByTestId("task-output-text");
  await expect(text).toContainText("line-1");
  await expect(page.getByTestId("task-follow")).toBeChecked();
  // The output is under the task's header, as wide as the task — not squeezed in beside it.
  const width = async (selector: string) => (await page.locator(selector).first().boundingBox())!.width;
  expect(await width('[data-testid="task-output-text"]')).toBeGreaterThan(0.8 * (await width('[data-testid="system-tasks"] .task.on')));
  const lines = async () => ((await text.textContent()) ?? "").split("\n").filter((l) => l.startsWith("line-")).length;
  const first = await lines();
  // Long enough to overflow the box (it holds about seventeen lines), so following has to scroll.
  await expect.poll(lines, { timeout: 15_000 }).toBeGreaterThanOrEqual(Math.max(first + 3, 25));
  expect(await text.evaluate((el) => el.scrollHeight > el.clientHeight), "the output overflows its box").toBe(true);
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

  await page.locator('#side [data-space="system"]').click();
  await expect(page.locator('[data-testid="system-strip"] [data-kpi="tasks"] .v')).toHaveText("1", { timeout: 15_000 });
  const row = page.locator('[data-testid="system-running"] [data-task][data-status="running"]').first();
  await expect(row).toBeVisible({ timeout: 15_000 });
  const id = (await row.getAttribute("data-task"))!;
  await row.click();
  const task = page.locator('[data-testid="system-tasks"] .task.on');
  await expect(task).toHaveAttribute("data-status", "running");
  await task.getByTestId("task-stop").click();
  // Long before its ten seconds are up: no longer running, and nothing left to stop.
  await expect(task).not.toHaveAttribute("data-status", /running|waiting|pending/, { timeout: 5_000 });
  await expect(task.getByTestId("task-stop")).toHaveCount(0);
  await expect(page.locator(`[data-testid="system-running"] [data-task="${id}"]`)).toHaveCount(0);
  expect(errors).toEqual([]);
});
