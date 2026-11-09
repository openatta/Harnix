/**
 * Where the flows are: in the system space, against the Harnix app (which has the flow
 * module). They are the application's toolchain, not one of its surfaces, so no Flows space is
 * listed among the spaces: the system console has a Flows tab, and a flow clicked there makes the
 * left column that flow's — its name, the way back, its conversation, its runs — with its
 * conversation in the middle and its details beside it. Back is the system tree and the console
 * again, on the tab it was left on.
 *
 * The flow-author conversations are still in the backend space `flow`, and the interface space that
 * shows them keeps that id (shown inside the system space, not listed): a session of it stored as
 * the front one, or opened from elsewhere, still opens there.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, openFlow, toFlows, watchConsole } from "./page";

const TAGS = { tag: ["@feature:flow", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => { await backend?.dispose(); });

/** A flow's own conversation, made by the module the first time it is asked for. */
async function ownConversation(page: Page, flow: string): Promise<string> {
  return page.evaluate(async (f) => (await (window as any).harnix.app.call("app.flow.session", { flow: f })).session_id as string, flow);
}

async function current(page: Page): Promise<string | null> {
  return page.evaluate(() => ((window as any).harnix.app.snapshot.current?.sessionId as string | undefined) ?? null);
}

const systemEntry = (page: Page) => page.locator('#side .l-bottom [data-space="system"]');

test("no Flows space among the spaces: the system console has a Flows tab, after all and before diagnostics; active stays the first", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await expect(page.locator('#side .spaces [data-space="sessions"]')).toBeVisible();
  await expect(page.locator('#side [data-space="flow"]')).toHaveCount(0);

  await systemEntry(page).click();
  const tabs = page.locator('[data-testid="system-console"] .stabs [data-tab]');
  await expect(tabs.first()).toHaveAttribute("data-tab", "active");
  await expect(tabs.first()).toHaveAttribute("aria-selected", "true");
  const order = await tabs.evaluateAll((els) => els.map((el) => el.getAttribute("data-tab")));
  expect(order.filter((id) => id !== "apps")).toEqual(["active", "all", "flows", "diagnostics"]);
  expect(order.indexOf("flows")).toBeLessThan(order.indexOf("diagnostics"));
  if (order.includes("apps")) expect(order.indexOf("flows")).toBeLessThan(order.indexOf("apps"));

  // The strip stays at the top; the flows and the runs under the tabs.
  await page.locator('[data-testid="system-console"] [data-tab="flows"]').click();
  await expect(page.getByTestId("system-strip")).toBeVisible();
  await expect(page.getByTestId("flow-def-example-review")).toBeVisible();
  await expect(page.getByTestId("flow-runs-toolbar")).toBeVisible();
  // The left column is still the system tree, and nothing is beside the console.
  await expect(page.getByTestId("system-tree")).toBeVisible();
  await expect(page.locator("#secondary")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("into a flow: the left column is its context, its conversation in the middle and its details beside; back is the system tree and the Flows tab", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await openFlow(page, "example-review");
  const own = await ownConversation(page, "example-review");

  // The left column: the way back, the flow's name, its conversation (open), its runs.
  await expect(page.getByTestId("system-tree")).toHaveCount(0);
  await expect(page.getByTestId("flow-back")).toBeVisible();
  await expect(page.getByTestId("flow-context-title")).toHaveText("Example: review and approve");
  await expect(page.getByTestId("flow-context-conversation")).toHaveAttribute("aria-current", "true");
  await expect(page.getByTestId("flow-context-runs")).toBeAttached();
  // Still the system space as far as the left column's entries go.
  await expect(systemEntry(page)).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator('#side .spaces [aria-pressed="true"]')).toHaveCount(0);

  // The middle: its own conversation, the line over the composer naming it.
  await expect.poll(() => current(page)).toBe(own);
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  await expect(page.getByTestId("flow-focus-notice")).toHaveAttribute("data-flow", "example-review");
  // Beside it: the flow's details.
  await expect(page.locator("#secondary")).toHaveAttribute("data-key", "flow:example-review");
  await expect(page.locator('#secondary [data-testid="flow-diagram"]')).toBeVisible();

  // Back: the system tree, the console on the Flows tab, nothing beside it.
  await page.getByTestId("flow-back").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "system");
  await expect(page.getByTestId("system-tree")).toBeVisible();
  await expect(page.getByTestId("flow-context")).toHaveCount(0);
  await expect(page.locator('[data-testid="system-console"] [data-tab="flows"]')).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("system-flows")).toBeVisible();
  await expect(page.locator("#secondary")).toHaveCount(0);

  // And in again, by the system entry's way round: the same conversation.
  await toFlows(page);
  await page.getByTestId("flow-def-example-review").click();
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "example-review");
  await expect.poll(() => current(page)).toBe(own);
  expect(errors).toEqual([]);
});

test("a flow's session opened from elsewhere opens inside the system space, in its flow's context", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const own = await ownConversation(page, "example-review");
  await expect(page.locator("#main")).toHaveAttribute("data-space", "sessions");
  await page.evaluate((sessionId) => (window as any).harnix.host.session.open({ instance: "system", sessionId }, { switchSpace: true }), own);
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  await expect.poll(() => current(page)).toBe(own);
  await expect(systemEntry(page)).toHaveAttribute("aria-pressed", "true");
  // The flow is the session's own: its context follows.
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "example-review");
  await expect(page.locator("#secondary")).toHaveAttribute("data-key", "flow:example-review");
  expect(errors).toEqual([]);
});

test("an old flow session kept as the front one still opens on load", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const own = await ownConversation(page, "example-review");
  // What a page of the separate Flows space left in this browser.
  await page.evaluate((sessionId) => {
    localStorage.setItem("harnix.space", JSON.stringify("flow"));
    localStorage.setItem("harnix.space.current", JSON.stringify({ flow: { instance: "system", sessionId } }));
  }, own);
  await page.reload();
  await expect(page.locator("#conn.on")).toBeVisible({ timeout: 15_000 });
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  await expect.poll(() => current(page)).toBe(own);
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "example-review");
  await expect(systemEntry(page)).toHaveAttribute("aria-pressed", "true");
  // Back from it, as from any flow.
  await page.getByTestId("flow-back").click();
  await expect(page.getByTestId("system-tree")).toBeVisible();
  expect(errors).toEqual([]);
});
