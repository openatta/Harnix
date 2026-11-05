/**
 * The flow space in the browser (xops-03 §6), against the empty application — the `harnix` binary,
 * which has the flow module, served the shell as built (`ui/dist`): the space is the shell's own.
 *
 * What is selected on the right is what the middle talks to: a flow's own conversation (made by the
 * module, `app.flow.session {flow}`), a run's main session, or — nothing selected, the reference —
 * the general conversation. The right column shows and edits no definition: its first rows add (in
 * the general conversation), filter and sort.
 *
 * Recordings: `flow-ui-edit` (the Agent calls `flow_show`, then `flow_update {flow:
 * example-review}` with its title changed, then says so), `flow-author-unfocused` (`flow_update` with
 * no flow, and the Agent relays that a flow must be selected first). A replayed session names its
 * recording when it is made, and the module makes a flow's own conversation with none: these two
 * run in a flow-author conversation of the space the test makes, pointed at the flow with
 * `app.flow.focus` as a flow's own is. The real run's main session has no recording here: the test
 * passes its session station by force, as a person may.
 *
 * Without the flow module there is no flow space: `examples/hello-app` (`spaces.spec.ts` lists its
 * spaces), and here a profile that drops it in `[shell] spaces`.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, reply, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:flow", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(() => backend?.stop());

async function inFlow(page: Page): Promise<void> {
  await page.locator('#side .spaces [data-space="flow"]').click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
}

async function tab(page: Page, id: "flows" | "runs" | "reference"): Promise<void> {
  await page.locator(`#details [data-tab="flow.${id}"]`).click();
}

async function select(page: Page, flow: string): Promise<void> {
  await tab(page, "flows");
  await page.getByTestId(`flow-def-${flow}`).click();
  await expect(page.getByTestId("flow-detail")).toHaveAttribute("data-flow", flow);
}

/** The session open in the middle. */
async function current(page: Page): Promise<string | null> {
  return page.evaluate(() => ((window as any).harnix.app.snapshot.current?.sessionId as string | undefined) ?? null);
}

/** What the module answers for a flow's conversation (no flow: the general one). */
async function conversationOf(page: Page, flow?: string): Promise<string> {
  return page.evaluate(async (f) => (await (window as any).harnix.app.call("app.flow.session", f ? { flow: f } : {})).session_id as string, flow);
}

/** A flow-author conversation of the space replaying `fixture`, pointed at `flow` (none: no flow). */
async function converse(page: Page, fixture: string, flow: string | null): Promise<string> {
  const id = await page.evaluate(async (name) => {
    const { app } = (window as any).harnix;
    await app.create({ options: { recorder: { name } } });
    return app.snapshot.current.sessionId as string;
  }, fixture);
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  const scene = await page.evaluate(async (session_id) => (await (window as any).harnix.app.call("session.get", { session_id, space: "flow" })).scene, id);
  expect(scene).toBe("flow-author");
  if (flow) {
    await page.evaluate(async ({ session_id, flow }) => (window as any).harnix.app.call("app.flow.focus", { instance: "system", session_id, flow }), { session_id: id, flow });
  }
  return id;
}

/** Permit the ask the turn raises (a tool that changes something). */
async function allow(page: Page): Promise<void> {
  const ask = page.getByTestId("conv-ask").last();
  await expect(ask).toBeVisible({ timeout: 30_000 });
  await ask.getByTestId("conv-ask-allow").click();
  await expect(ask).toHaveAttribute("data-state", "answered");
}

test("the flows tab: filter and sort; a flow drawn with its YAML; selecting it opens its own conversation, nothing selected the general one", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await tab(page, "flows");
  await expect(page.getByTestId("flow-flows-toolbar")).toBeVisible();
  const example = page.getByTestId("flow-def-example-review");
  await expect(example).toHaveAttribute("data-origin", "builtin");
  await expect(example).toHaveAttribute("data-valid", "true");

  // Filter and sort: the first row's right side.
  await page.getByTestId("flow-filter-name").fill("no such flow");
  await expect(example).toHaveCount(0);
  await page.getByTestId("flow-filter-name").fill("review");
  await expect(example).toBeVisible();
  await page.getByTestId("flow-filter-origin").selectOption("user");
  await expect(example).toHaveCount(0);
  await page.getByTestId("flow-filter-origin").selectOption("builtin");
  await page.getByTestId("flow-filter-valid").selectOption("valid");
  await page.getByTestId("flow-sort-flows").selectOption("title");
  await expect(example).toBeVisible();

  await select(page, "example-review");
  const own = await conversationOf(page, "example-review");
  await expect.poll(() => current(page)).toBe(own);
  const notice = page.getByTestId("flow-focus-notice");
  await expect(notice).toHaveAttribute("data-flow", "example-review");
  await expect(notice).toContainText("Example: review and approve");

  const graph = page.getByTestId("flow-graph");
  await expect(graph).toHaveAttribute("data-mode", "definition");
  for (const id of ["review", "approve"]) await expect(page.getByTestId(`flow-node-${id}`)).toBeVisible();
  await expect(page.getByTestId("flow-skill-body-review")).toContainText("Look at the subject");
  await page.getByTestId("flow-yaml").locator("summary").click();
  await expect(page.getByTestId("flow-yaml-text")).toContainText("name: example-review");
  // Read-only: nothing in the right column takes text but the filters.
  await expect(page.locator("#details textarea")).toHaveCount(0);
  await page.getByTestId("flow-node-review").click();
  await expect(page.locator('#details [data-testid="flow-station-panel"]')).toHaveAttribute("data-station", "review");

  // Selected again: nothing is, and the middle is the general conversation.
  await page.getByTestId("flow-def-example-review").click();
  await expect(page.getByTestId("flow-pick-hint")).toBeVisible();
  const general = await conversationOf(page);
  expect(general).not.toBe(own);
  await expect.poll(() => current(page)).toBe(general);
  await expect(page.getByTestId("flow-focus-notice")).toHaveAttribute("data-flow", "");
  await expect(page.getByTestId("flow-focus-notice")).toContainText(/general conversation|通用会话/);

  // Back to it: the same conversation as before.
  await select(page, "example-review");
  await expect.poll(() => current(page)).toBe(own);
  expect(errors).toEqual([]);
});

test("add: the general conversation, its composer started", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await select(page, "example-review");
  await page.getByTestId("flow-add").click();
  await expect.poll(() => current(page)).toBe(await conversationOf(page));
  await expect(page.getByTestId("conv-input")).toHaveValue(/Create a new flow:|新建一个流程：/);
  expect(errors).toEqual([]);
});

test("asked in a conversation on the flow, the Agent changes it, and the right column follows", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await select(page, "example-review");
  const before = await page.getByTestId("flow-detail").getAttribute("data-digest");
  await converse(page, "flow-ui-edit", "example-review");
  await expect(page.getByTestId("flow-focus-notice")).toHaveAttribute("data-flow", "example-review");

  await say(page, "Show the current flow and change its title to “Example: reviewed in conversation”.");
  // `flow_show` reads; `flow_update` changes something, so it asks. The card itself only says
  // "Allow flow_update?": the call's row above it names the flow and the new title.
  await expect(page.getByTestId("conv-ask").last()).toBeVisible({ timeout: 30_000 });
  const target = page.getByTestId("flow-tool-target").last();
  await expect(target).toHaveAttribute("data-tool", "update");
  await expect(target).toHaveAttribute("data-flow", "example-review");
  await expect(target).toContainText("Example: reviewed in conversation");
  await allow(page);
  await expect(reply(page)).toContainText("reviewed in conversation", { timeout: 30_000 });

  const row = page.getByTestId("flow-def-example-review");
  await expect(row).toContainText("Example: reviewed in conversation");
  await expect(row).toHaveAttribute("data-origin", "override");
  await expect(page.getByTestId("flow-detail-title")).toHaveText("Example: reviewed in conversation");
  await expect(page.getByTestId("flow-detail")).not.toHaveAttribute("data-digest", before ?? "");
  await expect(page.getByTestId("flow-focus-notice")).toContainText("Example: reviewed in conversation");
  expect(errors).toEqual([]);
});

test("in a conversation on no flow, the Agent is told to have one selected first, and says so", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await tab(page, "flows");
  await expect(page.getByTestId("flow-pick-hint")).toBeVisible();
  await converse(page, "flow-author-unfocused", null);
  const notice = page.getByTestId("flow-focus-notice");
  await expect(notice).toHaveAttribute("data-flow", "");
  await expect(notice).toContainText(/general conversation|通用会话/);

  await say(page, "Change the flow's title.");
  await expect(page.getByTestId("flow-tool-target").last()).toHaveAttribute("data-flow", "authored", { timeout: 30_000 });
  await allow(page);
  await expect(reply(page)).toContainText("No flow is selected", { timeout: 30_000 });
  await expect(page.getByTestId("conv-tool-name").filter({ hasText: /flow_update/ })).toHaveCount(1);
  expect(errors).toEqual([]);
});

test("a dry run started on the flow is in the runs tab: it steps, runs to the approval, shows its session's message, and finishes once confirmed", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await select(page, "example-review");
  await page.getByTestId("flow-dry-start").click();
  const detail = page.getByTestId("flow-run-detail");
  await expect(detail).toHaveAttribute("data-run", /^dry-example-review-/);
  const run = (await detail.getAttribute("data-run"))!;
  await expect(page.getByTestId(`flow-run-${run}`)).toHaveAttribute("data-dry", "true");
  await expect(page.getByTestId("flow-graph")).toHaveAttribute("data-mode", "run");
  await expect(page.getByTestId("flow-pause-run")).toHaveCount(0);

  await page.getByTestId("flow-dry-step").click();
  await expect(page.getByTestId("flow-dry-last")).toHaveAttribute("data-steps", "1");
  await page.getByTestId("flow-dry-run").click();
  await expect(page.getByTestId("flow-dry-last")).toHaveAttribute("data-pending", "0");
  const approve = page.getByTestId("flow-node-approve");
  await expect(approve).toHaveAttribute("data-status", "working");
  await expect(page.getByTestId("flow-node-review")).toHaveAttribute("data-status", "passed");
  await expect(page.getByTestId("flow-run-phase")).toHaveAttribute("data-phase", "waiting");

  // The mocked session task: the whole message the main session would have been sent.
  await page.getByTestId("flow-node-review").click();
  const panel = page.locator('#details [data-testid="flow-station-panel"]');
  await expect(panel).toHaveAttribute("data-station", "review");
  await panel.getByTestId("flow-preview-review").locator("summary").click();
  await expect(panel.getByTestId("flow-preview-message-review")).toContainText("Skill `review`");

  await approve.click();
  await expect(panel).toHaveAttribute("data-station", "approve");
  await panel.getByTestId("flow-confirm").click();
  await expect(page.getByTestId("flow-run-phase")).toHaveAttribute("data-phase", "finished", { timeout: 15_000 });
  await expect(approve).toHaveAttribute("data-status", "passed");
  await expect(page.getByTestId(`flow-run-${run}`)).toHaveAttribute("data-phase", "finished");
  expect(errors).toEqual([]);
});

test("a run started on the flow: its main session in the middle, its state, pause and resume, each station's time; forced and confirmed, it finishes", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await select(page, "example-review");
  await page.getByTestId("flow-start-open").click();
  await expect(page.getByTestId("flow-start-flow")).toHaveValue("example-review");
  await page.getByTestId("flow-start-subject").fill("repo");
  await page.getByTestId("flow-start-submit").click();

  const detail = page.getByTestId("flow-run-detail");
  await expect(detail).toHaveAttribute("data-run", /^example-review-/);
  const run = (await detail.getAttribute("data-run"))!;
  const row = page.getByTestId(`flow-run-${run}`);
  await expect(row).toBeVisible();
  await expect(row).not.toHaveAttribute("data-dry", "true");
  await expect(page.getByTestId("flow-dry-controls")).toHaveCount(0);
  await expect(page.getByTestId("flow-graph")).toHaveAttribute("data-mode", "run");

  // The middle is the run's main session.
  const main = async () =>
    page.evaluate(async (r) => {
      const v = await (window as any).harnix.app.call("app.flow.get", { run: r });
      return ((v.record.session_keys ?? {})["@main"] as string | undefined)?.split("/")[1] ?? null;
    }, run);
  await expect.poll(main, { timeout: 15_000 }).not.toBeNull();
  await expect.poll(() => current(page), { timeout: 15_000 }).toBe(await main());
  await expect(page.getByTestId("flow-open-main")).toBeVisible();

  // Entered: its main session works on it, or — with no recording to replay — it has failed.
  const review = page.getByTestId("flow-node-review");
  await expect(review).toHaveAttribute("data-status", /^(working|unmet)$/, { timeout: 30_000 });
  await expect(review).toHaveAttribute("data-elapsed", /^\d+$/);
  const phase = page.getByTestId("flow-run-phase");
  await expect(phase).toHaveAttribute("data-phase", /^(running|waiting)$/);

  // Pause: what is in hand finishes, then it is paused; resumed, it goes on.
  await page.getByTestId("flow-pause-run").click();
  await expect(phase).toHaveAttribute("data-phase", /^(pausing|paused)$/);
  await expect(phase).toHaveAttribute("data-phase", "paused", { timeout: 30_000 });
  await expect(row).toHaveAttribute("data-phase", "paused");
  await page.getByTestId("flow-resume-run").click();
  await expect(phase).toHaveAttribute("data-phase", /^(running|waiting)$/);

  await review.click();
  const panel = page.locator('#details [data-testid="flow-station-panel"]');
  await expect(panel.getByTestId("flow-station-spent")).toBeVisible();
  await panel.getByTestId("flow-force").click();
  await panel.getByTestId("flow-reason").fill("reviewed by hand");
  await panel.getByTestId("flow-action-submit").click();
  await expect(review).toHaveAttribute("data-status", "passed", { timeout: 15_000 });

  const approve = page.getByTestId("flow-node-approve");
  await expect(approve).toHaveAttribute("data-status", "working");
  await expect(phase).toHaveAttribute("data-reason", "confirm");
  await approve.click();
  await panel.getByTestId("flow-confirm").click();
  await expect(phase).toHaveAttribute("data-phase", "finished", { timeout: 15_000 });
  await expect(row).toHaveAttribute("data-phase", "finished");
  expect(errors).toEqual([]);
});

test("the runs tab lists every run and dry run, filtered by state and kind", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await tab(page, "runs");
  await expect(page.getByTestId("flow-runs-toolbar")).toBeVisible();
  await expect(page.getByTestId("flow-add")).toHaveCount(0);
  const list = page.getByTestId("flow-run-list");
  const rows = list.locator('[data-testid^="flow-run-"]');
  await expect(rows.first()).toBeVisible();
  // The tests before left a finished run and a finished dry run of the example.
  await page.getByTestId("flow-filter-kind").selectOption("dry");
  await expect(list.locator('[data-dry="true"]').first()).toBeVisible();
  await expect(list.locator('[data-testid^="flow-run-"]:not([data-dry="true"])')).toHaveCount(0);
  await page.getByTestId("flow-filter-kind").selectOption("real");
  await expect(list.locator('[data-dry="true"]')).toHaveCount(0);
  await page.getByTestId("flow-filter-state").selectOption("finished");
  await expect(list.locator('[data-phase="finished"]').first()).toBeVisible();
  await expect(list.locator('[data-testid^="flow-run-"]:not([data-phase="finished"])')).toHaveCount(0);
  await page.getByTestId("flow-filter-state").selectOption("running");
  await expect(rows).toHaveCount(0);
  await page.getByTestId("flow-filter-state").selectOption("");
  await page.getByTestId("flow-sort-runs").selectOption("duration");
  await expect(rows.first()).toBeVisible();
  // Nothing selected in the runs tab: the general conversation.
  await expect.poll(() => current(page)).toBe(await conversationOf(page));
  expect(errors).toEqual([]);
});

test("the reference tab: the guide, the catalog and the fields; an example starts a new flow in the general conversation", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await select(page, "example-review");
  await tab(page, "reference");
  const guide = page.getByTestId("flow-guide");
  await expect(guide).toContainText(/Flow guide|流程指南/);
  await expect(guide.locator("table").first()).toBeVisible();
  await expect(page.getByTestId("flow-guide-example-2")).toBeVisible();
  await expect(page.getByTestId("flow-catalog-scenes")).toContainText("flow-chat");
  await expect(page.getByTestId("flow-ref-fields")).toContainText("stations");
  await expect(page.getByTestId("flow-validate-box")).toHaveCount(0);
  // The reference talks in the general conversation.
  const general = await conversationOf(page);
  await expect.poll(() => current(page)).toBe(general);

  await page.getByTestId("flow-guide-like-0").click();
  await expect.poll(() => current(page)).toBe(general);
  await expect(page.getByTestId("conv-input")).toHaveValue(/name: draft-and-sign/);
  expect(errors).toEqual([]);
});

test("a profile that drops the flow space in [shell] spaces does not get it", TAGS, async ({ page }) => {
  const without = await startHarnix(launchFor("replay", { profile: '\n[shell]\nspaces = ["sessions", "projects", "system"]\n' }));
  try {
    const errors = watchConsole(page);
    await open(page, without.url);
    await expect(page.locator('#side .spaces [data-space="sessions"]')).toBeVisible();
    await expect(page.locator('#side [data-space="flow"]')).toHaveCount(0);
    // The module is still there; only the interface leaves it out.
    expect(await page.evaluate(() => (window as any).harnix.app.snapshot.reachable.includes("app.flow.defs"))).toBe(true);
    expect(errors).toEqual([]);
  } finally {
    without.stop();
  }
});
