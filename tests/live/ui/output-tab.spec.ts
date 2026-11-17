/**
 * A background task's output in the session's right column (harnix-app.md §6.1): the output tab,
 * @harnix/observability's, opened from a Monitor's tool row, read-only and followed while it runs,
 * and stopped from its head (two presses, `observability.tasks.stop`); and the home page's figure
 * of the background tasks running (§10.1) counting it.
 *
 * The task is a Monitor printing a line every quarter second (`streams-output`). Monitor is a tool
 * of the `code` scene, so this backend activates it beside the default.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, say, toSessions, toSpace, watchConsole } from "./page";

const TAGS = { tag: ["@feature:session-space", "@layer:ui"] };
const HOME = { tag: ["@feature:home", "@layer:ui"] };

let backend: Backend;

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay", { profile: '\n[engine]\nscenes = ["code"]\n' }));
});
test.afterAll(async () => {
  await backend?.dispose();
});

/** A conversation of the `code` scene whose turn starts the counter, and has ended with it running. */
async function counting(page: Page): Promise<string> {
  await open(page, backend.url);
  await toSessions(page);
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
  return page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId as string);
}

test("output-tab: a Monitor's tool row opens its output in the right column, followed read-only, one tab a task", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  // The tab's box is as tall as the window allows: a short window, so the counter's lines overflow it.
  await page.setViewportSize({ width: 1280, height: 560 });
  await counting(page);

  // The task is listed after the command has run a second; its row's action finds it by the call's id.
  const row = page.locator('[data-testid="conv-tool"]').first();
  await row.getByTestId("conv-tool-toggle").click();
  const view = row.getByTestId("action-observability.output");
  await expect(view).toBeVisible();
  const outputs = page.locator('#details [data-testid="sidebar-tab"][data-kind="output"]');
  await expect.poll(async () => { await view.click(); return outputs.count(); }, { timeout: 15_000 }).toBeGreaterThan(0);
  // Asked again, the same task's tab is brought to the front, not opened twice.
  await view.click();
  await expect(outputs).toHaveCount(1);

  const tab = outputs.first();
  await expect(tab).toHaveAttribute("data-state", "current");
  const text = page.locator('#details [data-testid="output-tab"] [data-testid="task-output-text"]');
  await expect(text).toContainText("line-1", { timeout: 15_000 });
  await expect(page.locator('#details [data-testid="output-status"]')).toContainText(/Running|运行中/);
  // Read-only: there is no input, and what was printed stays in the box as more arrives.
  await expect(page.locator('#details [data-testid="output-tab"] textarea, #details [data-testid="output-tab"] input[type="text"]')).toHaveCount(0);
  const lines = async () => ((await text.textContent()) ?? "").split("\n").filter((l) => l.startsWith("line-")).length;
  const first = await lines();
  await expect.poll(lines, { timeout: 30_000 }).toBeGreaterThan(first);
  // Read piece by piece from where the last read ended: the lines in order, none twice.
  const seen = ((await text.textContent()) ?? "").split("\n").filter((l) => l.startsWith("line-"));
  const at = Number(seen[0]!.slice(5));
  expect(seen).toEqual(seen.map((_, i) => `line-${at + i}`));
  // Only a box that overflows can be read back in.
  await expect.poll(() => text.evaluate((el) => el.scrollHeight - el.clientHeight), { timeout: 30_000 }).toBeGreaterThan(40);
  // Following: the end is in view; reading back stops it, and the end again resumes it.
  await expect.poll(() => text.evaluate((el) => el.scrollHeight - el.clientHeight - el.scrollTop), { timeout: 5_000 }).toBeLessThanOrEqual(8);
  await text.evaluate((el) => { el.scrollTop = 0; });
  await expect(page.getByTestId("task-follow")).not.toBeChecked();
  await text.evaluate((el) => { el.scrollTop = el.scrollHeight; });
  await expect(page.getByTestId("task-follow")).toBeChecked();
  // Beside the pinned context tab, which stays last.
  const kinds = await page.locator('#details [data-testid="sidebar-tab"]').evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.kind));
  expect(kinds.at(-1)).toBe("context");
  expect(errors).toEqual([]);
});

/** The output tab of the counter's task, opened from its Monitor row. */
async function outputOf(page: Page) {
  const row = page.locator('[data-testid="conv-tool"]').first();
  await row.getByTestId("conv-tool-toggle").click();
  const view = row.getByTestId("action-observability.output");
  const outputs = page.locator('#details [data-testid="sidebar-tab"][data-kind="output"]');
  await expect.poll(async () => { await view.click(); return outputs.count(); }, { timeout: 15_000 }).toBeGreaterThan(0);
  return page.locator('#details [data-testid="output-tab"]');
}

test("output-tab: a running task is stopped from its tab, with a second press; stopped, it offers no stop", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await counting(page);
  const tab = await outputOf(page);
  await expect(tab.getByTestId("task-output-text")).toContainText("line-1", { timeout: 15_000 });
  await expect(tab).toHaveAttribute("data-status", "running");
  const stop = tab.getByTestId("output-stop");
  // One press only arms it: nothing is stopped by a stray click.
  await stop.click();
  await expect(stop).toHaveAttribute("data-armed", "true");
  await expect(stop).toHaveText(/Press again to stop|再按一次停止/);
  await page.waitForTimeout(500);
  await expect(tab).toHaveAttribute("data-status", "running");
  await stop.click();
  // Stopped long before its ten seconds: the list says so, and there is nothing left to stop.
  await expect(tab).toHaveAttribute("data-status", "stopped", { timeout: 10_000 });
  await expect(tab.getByTestId("output-status")).toHaveText(/Stopped|已停止/);
  await expect(tab.getByTestId("output-stop")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("the home page counts the background task while it runs", HOME, async ({ page }) => {
  const errors = watchConsole(page);
  await counting(page);
  await toSpace(page, "home");
  await expect(page.locator('[data-testid="home-stats"] [data-kpi="tasks"] .v')).not.toHaveText("0", { timeout: 15_000 });
  expect(errors).toEqual([]);
});
