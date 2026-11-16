/**
 * The centre's views and the right column's context and usage tabs (harnix-app.md §2, §6;
 * harnix-ext.md §22.10), in a real browser against a real backend: the switch over the conversation,
 * the trajectory of a session with a recording (its turn, its calls and the tool call, the calls
 * beside the turn, the time overview, the inspector, the way back to the conversation), the choice
 * remembered for the session across a reload and not for another, and the two tabs from the guide.
 *
 * Replayed sessions record nothing, so the recording is put in place as the system space's tests do
 * (`recorded`): `calls-a-tool`, whose one turn calls Bash once.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { connected, open, recorded, sessionRow, watchConsole } from "./page";

const TAGS = { tag: ["@feature:session-space", "@layer:ui"] };
const TRAJECTORY = "observability.trajectory";

let backend: Backend;
test.describe.configure({ mode: "serial" });
test.beforeAll(async () => {
  backend = await startHarnix(launchFor("none"));
});
test.afterAll(async () => {
  await backend?.dispose();
});

const main = (page: Page) => page.locator("#main");
const frame = (page: Page) => page.locator("#frame");
const viewTab = (page: Page, view: string) => page.locator(`[data-testid="view-switch"] [role="tab"][data-view="${view}"]`);
const chip = (page: Page, kind: string) => page.locator(`#details [data-testid="sidebar-tab"][data-kind="${kind}"]`);

/** A session with the `calls-a-tool` recording, opened in the session space. */
async function openRecorded(page: Page): Promise<string> {
  const sid = await recorded(page, backend, { fixture: "calls-a-tool" });
  await sessionRow(page, sid).click();
  await expect(main(page)).toHaveAttribute("data-state", "open");
  return sid;
}

async function expand(page: Page): Promise<void> {
  if ((await frame(page).getAttribute("data-details")) !== "open") await page.getByTestId("details-toggle").click();
  await expect(frame(page)).toHaveAttribute("data-details", "open");
}

test("the switch over the conversation: the conversation by default, the trajectory on a click, remembered for this session across a reload and not for another", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  const sid = await openRecorded(page);
  await expect(main(page)).toHaveAttribute("data-view", "conversation");
  await expect(viewTab(page, "conversation")).toHaveAttribute("aria-selected", "true");
  await expect(viewTab(page, TRAJECTORY)).toContainText(/轨迹|Trajectory/);
  // The header: the status dot and the context ring, beside the usage.
  await expect(page.getByTestId("head-status")).toHaveAttribute("data-state", /idle|running|waiting/);

  await viewTab(page, TRAJECTORY).click();
  await expect(main(page)).toHaveAttribute("data-view", TRAJECTORY);
  await expect(viewTab(page, TRAJECTORY)).toHaveAttribute("aria-selected", "true");
  const trajectory = page.locator('#main [data-testid="conversation-view"] [data-testid="trajectory"]');
  await expect(trajectory).toHaveAttribute("data-session", sid);
  // The conversation is still on the page, under the view.
  await expect(page.locator("#main > .hx-chat.under-view")).toHaveCount(1);

  await page.reload();
  await connected(page);
  await expect(main(page)).toHaveAttribute("data-view", TRAJECTORY, { timeout: 15_000 });

  // Another session: its own choice, the conversation; back, the first keeps its.
  const other = await recorded(page, backend, { fixture: "calls-a-tool" });
  await sessionRow(page, other).click();
  await expect(main(page)).toHaveAttribute("data-view", "conversation");
  await sessionRow(page, sid).click();
  await expect(main(page)).toHaveAttribute("data-view", TRAJECTORY);
  await viewTab(page, "conversation").click();
  await expect(main(page)).toHaveAttribute("data-view", "conversation");
  await expect(page.locator('[data-testid="conversation-view"]')).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("the trajectory: the turn with its calls and the Bash call, the calls beside it, the overview, the inspector beside the rows, and the way back to the conversation", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await openRecorded(page);
  await viewTab(page, TRAJECTORY).click();
  const trajectory = page.locator('#main [data-testid="trajectory"]');
  await expect(trajectory).toBeVisible();
  // Every read of the session claims the session space's.
  await expect(trajectory.getByTestId("turn-row")).toHaveCount(1);
  const calls = trajectory.getByTestId("system-call");
  await expect(calls).toHaveCount(2);
  await expect(calls.first().getByTestId("call-ttft")).toBeVisible();
  const tool = trajectory.getByTestId("system-call-tool");
  await expect(tool).toHaveCount(1);
  await expect(tool).toContainText("Bash");
  await expect(tool).toContainText("echo harnix-fixture");
  // The title and the memory calls, beside the turn.
  await expect(trajectory.getByTestId("traj-aux")).toHaveCount(2);
  // The overview: a bar per timed call, the real start, the whole stretch.
  const overview = trajectory.getByTestId("traj-overview");
  await expect(overview.getByTestId("timeline-call")).toHaveCount(4);
  await expect(overview.getByTestId("traj-start")).toHaveText(/\d/);
  await expect(trajectory.getByTestId("traj-count")).toContainText("1");

  // Folding: the call's tool call folds away; a search finds it again.
  await calls.first().getByTestId("call-fold").click();
  await expect(tool).toHaveCount(0);
  await trajectory.getByTestId("calls-search").fill("echo");
  await expect(tool).toHaveCount(1);
  await trajectory.getByTestId("calls-search").fill("");
  await calls.first().getByTestId("call-fold").click();

  // The inspector beside the rows, the rows staying: input, output with the tool's result, the difference from the call before.
  await calls.nth(1).click();
  const detail = trajectory.getByTestId("call-detail");
  await expect(detail).toHaveAttribute("data-call", "1");
  await expect(calls).toHaveCount(2);
  await expect(detail.getByTestId("call-input")).toContainText(/scene\./);
  await detail.locator('[data-side="output"]').click();
  await expect(detail.getByTestId("call-output")).toContainText("harnix-fixture");
  await detail.getByTestId("call-compare").click();
  await expect(trajectory.locator('[data-testid="system-diff"], .diff.none').first()).toBeVisible();
  // Picking a bar on the overview opens its call.
  await overview.locator('[data-testid="timeline-call"][data-call="0"]').click();
  await expect(detail).toHaveAttribute("data-call", "0");
  await expect(detail.getByTestId("call-output")).toContainText(/Bash|tool_use/);

  // Back to the conversation: the tool call's row revealed.
  await detail.getByTestId("call-locate").click();
  await expect(main(page)).toHaveAttribute("data-view", "conversation");
  expect(errors).toEqual([]);
});

test("the context and usage tabs from the guide: the report and its parts, the figures, and the way into the trajectory", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await openRecorded(page);
  await expand(page);
  await expect(page.locator('[data-testid="sidebar-guide-entry"][data-kind="context"]')).toContainText(/上下文|Context/);
  await expect(page.locator('[data-testid="sidebar-guide-entry"][data-kind="usage"]')).toContainText(/用量|Usage/);

  await page.locator('[data-testid="sidebar-guide-entry"][data-kind="context"]').click();
  await expect(chip(page, "context")).toHaveAttribute("data-state", "current");
  const context = page.getByTestId("context-tab");
  // The product has the system space: the report, with its blocks; and the parts the kernel and the settings give.
  await expect(context.getByTestId("context-report")).toBeVisible();
  await expect(context.getByTestId("system-block").first()).toBeVisible();
  await expect(context.getByTestId("context-skills")).toBeVisible();
  await expect(context.getByTestId("context-hooks")).toBeVisible();
  await expect(context.getByTestId("context-mcp")).toBeVisible();
  // No project: nothing held back, no trust button.
  await expect(context.getByTestId("context-trust")).toHaveCount(0);

  // The usage tab, beside it: this run's figures from the recording, and the door into the trajectory.
  await page.getByTestId("sidebar-add").click();
  await page.locator('[data-testid="sidebar-guide-entry"][data-kind="usage"]').click();
  await expect(chip(page, "usage")).toHaveAttribute("data-state", "current");
  const usage = page.getByTestId("usage-tab");
  await expect(usage.locator('[data-cell="turns"] .v')).toHaveText("1");
  await expect(usage.locator('[data-cell="calls"] .v')).toContainText("2");
  await expect(usage.locator('[data-cell="hit"] .v')).toHaveText(/^(\d+%|—)$/);
  await usage.getByTestId("usage-open-trajectory").click();
  await expect(main(page)).toHaveAttribute("data-view", TRAJECTORY);
  await expect(page.locator('#main [data-testid="trajectory"]')).toBeVisible();
  expect(errors).toEqual([]);
});
