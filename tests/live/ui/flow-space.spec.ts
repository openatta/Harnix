/**
 * The flow space's layout and the ways round it (harnix-app.md §11), against the Harnix app (which
 * has the flow module). It is the application's toolchain, not one of its surfaces: its entry is
 * at the bottom of the left column, beside the avatar, not among the spaces. In it the left column
 * is the general conversation and every flow, the one selected open on its runs; what is picked
 * there is the middle's conversation — the general one, a flow's own, a run's main session — and
 * the right column comes along to the list it is in (flows · runs · reference); what is selected
 * is shown beside the conversation.
 *
 * The flow-author conversations are in the backend space `flow`: a session of it opened from
 * elsewhere, or kept as the front one, opens here, in its flow's context.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { connected, open, spaceEntry, toSpace, watchConsole } from "./page";

const TAGS = { tag: ["@feature:flow", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => {
  await backend?.dispose();
});

/** A flow's own conversation (none: the general one), made by the module the first time it is asked for. */
async function conversationOf(page: Page, flow?: string): Promise<string> {
  return page.evaluate(async (f) => (await (window as any).harnix.app.call("flow.session", f ? { flow: f } : {})).session_id as string, flow);
}

async function current(page: Page): Promise<string | null> {
  return page.evaluate(() => ((window as any).harnix.app.snapshot.current?.sessionId as string | undefined) ?? null);
}

const flowEntry = (page: Page) => page.getByTestId("sidebar-foot").locator('[data-testid="space-entry"][data-space="flow"]');
const detailsTab = (page: Page, id: string) => page.locator(`#details [role="tab"][data-tab="${id}"]`);

test("its entry is at the bottom of the left column; in it, the general conversation and every flow on the left, the lists on the right", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  // Not among the spaces: at the bottom, with the avatar.
  await expect(page.getByTestId("space-list").locator('[data-space="flow"]')).toHaveCount(0);
  await expect(flowEntry(page)).toBeVisible();
  await expect(flowEntry(page)).toHaveAttribute("aria-pressed", "false");

  await flowEntry(page).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  await expect(flowEntry(page)).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("space-list").getByRole("button", { pressed: true })).toHaveCount(0);
  // The right column is open, its three lists in order, the flows in front.
  await expect(page.locator("#frame")).toHaveAttribute("data-details", "open");
  const tabs = await page.locator('#details [role="tab"][data-scope="space"]').evaluateAll((els) => els.map((el) => el.getAttribute("data-tab")));
  expect(tabs).toEqual(["flow.flows", "flow.runs", "flow.reference"]);
  await expect(detailsTab(page, "flow.flows")).toHaveAttribute("aria-selected", "true");
  await expect(page.locator('#details [data-testid="flow-def-example-review"]')).toBeVisible();
  // The left column: no way back (entered from nowhere else), the general conversation current, every flow.
  const left = page.getByTestId("flow-context");
  await expect(left).toHaveAttribute("data-flow", "");
  await expect(page.getByTestId("flow-back")).toHaveCount(0);
  await expect(page.getByTestId("flow-context-conversation")).toHaveAttribute("aria-current", "true");
  await expect(page.getByTestId("flow-context-flow-example-review")).toBeVisible();
  await expect(page.getByTestId("flow-context-runs")).toHaveCount(0);
  // The middle: the general conversation, nothing beside it.
  await expect.poll(() => current(page)).toBe(await conversationOf(page));
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  await expect(page.getByTestId("flow-focus-notice")).toHaveAttribute("data-flow", "");
  await expect(page.locator("#secondary")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("a flow picked on the left: its conversation in the middle, its details beside, its runs under it; a run picked: its main session, the runs list in front", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await toSpace(page, "flow");
  const own = await conversationOf(page, "example-review");
  const run = await page.evaluate(async () => (await (window as any).harnix.app.call("flow.start", { flow: "example-review", subject: "the left column" })).run as string);

  // The reference in front first: a pick brings the list it is in to the front.
  await detailsTab(page, "flow.reference").click();
  await page.getByTestId("flow-context-flow-example-review").click();
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "example-review");
  await expect(page.getByTestId("flow-context-flow-example-review")).toHaveAttribute("aria-current", "true");
  await expect(page.getByTestId("flow-context-conversation")).not.toHaveAttribute("aria-current", "true");
  await expect(detailsTab(page, "flow.flows")).toHaveAttribute("aria-selected", "true");
  await expect.poll(() => current(page)).toBe(own);
  await expect(page.getByTestId("flow-focus-notice")).toHaveAttribute("data-flow", "example-review");
  await expect(page.locator("#secondary")).toHaveAttribute("data-key", "flow:example-review");
  await expect(page.locator('#secondary [data-testid="flow-diagram"]')).toBeVisible();
  // The header, narrowed by the details, names the space alone, and the flow's buttons keep their words on one line.
  expect((await page.locator("#head").boundingBox())!.width).toBeLessThan(720);
  await expect(page.getByTestId("space-pill")).toBeVisible();
  await expect(page.locator('[data-testid="space-pill"] .sub')).toBeHidden();
  for (const button of ["flow-dry-start", "flow-start-open"]) {
    expect((await page.locator(`#secondary [data-testid="${button}"]`).boundingBox())!.height, `${button} on one line`).toBeLessThan(34);
  }

  // Its runs under it; one picked: its main session in the middle, the runs list in front with it picked too.
  const mine = page.getByTestId("flow-context-runs").getByTestId(`flow-run-${run}`);
  await expect(mine).toBeVisible();
  await mine.click();
  await expect(mine).toHaveAttribute("aria-current", "true");
  await expect(detailsTab(page, "flow.runs")).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(`#details [data-testid="flow-run-${run}"]`)).toHaveAttribute("aria-current", "true");
  await expect(page.locator('#secondary [data-testid="flow-run-detail"]')).toHaveAttribute("data-run", run);
  const main = async () =>
    page.evaluate(async (r) => {
      const v = await (window as any).harnix.app.call("flow.get", { run: r });
      return ((v.record.session_keys ?? {})["@main"]?.id as string | undefined)?.split("/")[1] ?? null;
    }, run);
  await expect.poll(main, { timeout: 15_000 }).not.toBeNull();
  await expect.poll(() => current(page), { timeout: 15_000 }).toBe(await main());

  // The general conversation picked: nothing selected, the flows list in front, nothing beside.
  await page.getByTestId("flow-context-conversation").click();
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "");
  await expect(detailsTab(page, "flow.flows")).toHaveAttribute("aria-selected", "true");
  await expect.poll(() => current(page)).toBe(await conversationOf(page));
  await expect(page.locator("#secondary")).toHaveCount(0);

  // Another space and back: the flow space as it was left.
  await toSpace(page, "sessions");
  await flowEntry(page).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  await expect.poll(() => current(page)).toBe(await conversationOf(page));
  await page.evaluate((r) => (window as any).harnix.app.call("flow.cancel", { run: r, reason: "done" }), run);
  expect(errors).toEqual([]);
});

test("a flow's session opened from elsewhere opens in the flow space, in its flow's context", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await toSpace(page, "sessions");
  const own = await conversationOf(page, "example-review");
  await page.evaluate((sessionId) => (window as any).harnix.host.session.open({ instance: "main", sessionId }, { switchSpace: true }), own);
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  await expect.poll(() => current(page)).toBe(own);
  await expect(flowEntry(page)).toHaveAttribute("aria-pressed", "true");
  // The flow is the session's own: the left column and the details follow.
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "example-review");
  await expect(page.locator("#secondary")).toHaveAttribute("data-key", "flow:example-review");
  expect(errors).toEqual([]);
});

test("a flow's session kept as the front one opens again on reload, in its flow's context", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await spaceEntry(page, "flow").click();
  const own = await conversationOf(page, "example-review");
  await page.evaluate((sessionId) => {
    localStorage.setItem("harnix.space", JSON.stringify("flow"));
    localStorage.setItem("harnix.space.current", JSON.stringify({ flow: { instance: "main", sessionId } }));
  }, own);
  await page.reload();
  await connected(page);
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  await expect.poll(() => current(page)).toBe(own);
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "example-review");
  await expect(flowEntry(page)).toHaveAttribute("aria-pressed", "true");
  expect(errors).toEqual([]);
});
