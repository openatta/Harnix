/**
 * The flows in the browser (xops-03 §6), against the Harnix app — the `harnix` binary,
 * which has the flow module, served the shell as built (`ui/dist`): they are the shell's own, in
 * the system space.
 *
 * The system space's Flows tab lists the flows and every run, and edits no definition: its first
 * rows add (in the general conversation), filter and sort. A flow clicked there makes the left
 * column that flow's — the way back, its conversation, its runs — and what is picked there is what
 * the middle talks to: the flow's own conversation (made by the module, `app.flow.session {flow}`)
 * or a run's main session; the general conversation is where a new flow is written. What is picked
 * opens beside the conversation, in the second side panel (xops-04 §7): a flow's diagram,
 * definition, parts (skills, rules, policies, gates, hooks) and YAML; a run's details. Closing that
 * panel leaves the conversation and the flow. The reference is the right column's one tab. An
 * empty conversation says what it is for.
 *
 * Recordings: `flow-ui-edit` (the Agent calls `flow_show`, then `flow_update {flow:
 * example-review}` with its title changed, then says so), `flow-author-unfocused` (`flow_update` with
 * no flow, and the Agent relays that a flow must be selected first). A replayed session names its
 * recording when it is made, and the module makes a flow's own conversation with none: these two
 * run in a flow-author conversation of the space the test makes, pointed at the flow with
 * `app.flow.focus` as a flow's own is. The real run's main session has no recording here: the test
 * passes its session station by force, as a person may.
 *
 * Without the flow module there are no flows: `examples/hello-app` (`spaces.spec.ts` lists its
 * spaces), and here a profile that drops them in `[shell] spaces`.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { flowReference, open, openFlow, reply, say, spaceEntry, toFlows, watchConsole } from "./page";

const TAGS = { tag: ["@feature:flow", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => { await backend?.dispose(); });

const select = openFlow;

/** The station panel, in the second side panel. */
const stationPanel = (page: Page) => page.locator('#secondary [data-testid="flow-station-panel"]');

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

test("the Flows tab: filter and sort; into a flow, its details beside the conversation — diagram, definition, parts, YAML; its own conversation in the middle", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await toFlows(page);
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
  // Nothing said in it yet: it says what it is for.
  await expect(page.getByTestId("flow-empty-conversation")).toHaveAttribute("data-kind", "flow");

  // The details are beside the conversation, not under the list.
  const side = page.locator("#secondary");
  await expect(side).toHaveAttribute("data-key", "flow:example-review");
  await expect(page.locator('#details [data-testid="flow-detail"]')).toHaveCount(0);
  await expect(page.getByTestId("flow-pick-hint")).toHaveCount(0);
  const graph = side.getByTestId("flow-diagram").getByTestId("flow-graph");
  await expect(graph).toHaveAttribute("data-mode", "definition");
  for (const id of ["review", "approve"]) await expect(side.getByTestId(`flow-node-${id}`)).toBeVisible();
  // The definition, station by station: the example's review keeps a rule and a read-only policy.
  const review = side.getByTestId("flow-def-station-review");
  await expect(review).toContainText("review-honest");
  await expect(side.getByTestId("flow-def-permissions-review-0")).toHaveAttribute("data-effect", "deny");
  // Its parts, each to open.
  await expect(side.getByTestId("flow-part-skill-review")).toContainText("Look at the subject");
  await expect(side.getByTestId("flow-part-rule-review-honest")).toContainText("Report what you saw");
  await expect(side.getByTestId("flow-part-policy-read-only")).toContainText("Look, do not touch");
  await expect(side.getByTestId("flow-parts-hook")).toBeVisible();
  await side.getByTestId("flow-yaml").locator("summary").click();
  await expect(side.getByTestId("flow-yaml-text")).toContainText("name: example-review");
  // Read-only: nothing takes text but the filters.
  await expect(page.locator("#details textarea, #secondary textarea")).toHaveCount(0);
  await side.getByTestId("flow-node-review").click();
  await expect(stationPanel(page)).toHaveAttribute("data-station", "review");
  await expect(stationPanel(page).getByTestId("flow-station-permissions")).toContainText("read-only");

  // Maximized, it takes the whole centre; restored, it is back beside the conversation.
  const max = page.getByTestId("secondary-max");
  await expect(max).toHaveAttribute("aria-pressed", "false");
  await max.click();
  await expect(page.locator("#frame")).toHaveAttribute("data-secondary-max", "");
  await expect(page.locator("#main")).toBeHidden();
  await expect(max).toHaveAttribute("aria-pressed", "true");
  const wide = (await side.boundingBox())!.width;
  await max.click();
  await expect(page.locator("#frame")).not.toHaveAttribute("data-secondary-max", "");
  await expect(page.locator("#main")).toBeVisible();
  expect((await side.boundingBox())!.width).toBeLessThan(wide);

  // Closed: the panel goes, the flow and its conversation stay; its conversation picked again, back.
  await page.getByTestId("secondary-close").click();
  await expect(side).toHaveCount(0);
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "example-review");
  await expect.poll(() => current(page)).toBe(own);
  await page.getByTestId("flow-context-conversation").click();
  await expect(side).toHaveAttribute("data-key", "flow:example-review");
  await expect.poll(() => current(page)).toBe(own);

  // Into it again from the Flows tab: the same conversation as before.
  await select(page, "example-review");
  await expect.poll(() => current(page)).toBe(own);
  expect(errors).toEqual([]);
});

test("add: the general conversation's context, its composer started", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await select(page, "example-review");
  await toFlows(page);
  await page.getByTestId("flow-add").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  const general = await conversationOf(page);
  expect(general).not.toBe(await conversationOf(page, "example-review"));
  await expect.poll(() => current(page)).toBe(general);
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "");
  await expect(page.getByTestId("flow-context-title")).toContainText(/General conversation|通用会话/);
  await expect(page.getByTestId("flow-context-runs")).toHaveCount(0);
  await expect(page.locator("#secondary")).toHaveCount(0);
  await expect(page.getByTestId("conv-input")).toHaveValue(/Create a new flow:|新建一个流程：/);
  await expect(page.getByTestId("flow-empty-conversation")).toHaveAttribute("data-kind", "general");
  await expect(page.getByTestId("flow-focus-notice")).toHaveAttribute("data-flow", "");
  await expect(page.getByTestId("flow-focus-notice")).toContainText(/general conversation|通用会话/);
  expect(errors).toEqual([]);
});

test("asked in a conversation on the flow, the Agent changes it, and the flow's context and the Flows tab follow", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await select(page, "example-review");
  const before = await page.locator('#secondary [data-testid="flow-detail"]').getAttribute("data-digest");
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

  await expect(page.getByTestId("flow-context-title")).toHaveText("Example: reviewed in conversation");
  await expect(page.locator("#secondary").getByTestId("flow-detail-title")).toHaveText("Example: reviewed in conversation");
  await expect(page.locator('#secondary [data-testid="flow-detail"]')).not.toHaveAttribute("data-digest", before ?? "");
  await expect(page.getByTestId("flow-focus-notice")).toContainText("Example: reviewed in conversation");
  await toFlows(page);
  const row = page.getByTestId("flow-def-example-review");
  await expect(row).toContainText("Example: reviewed in conversation");
  await expect(row).toHaveAttribute("data-origin", "override");
  expect(errors).toEqual([]);
});

test("in a conversation on no flow, the Agent is told to have one selected first, and says so", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await toFlows(page);
  await expect(page.getByTestId("flow-pick-hint")).toBeVisible();
  await page.getByTestId("flow-add").click();
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "");
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
  await select(page, "example-review");
  await page.getByTestId("flow-dry-start").click();
  const detail = page.locator('#secondary [data-testid="flow-run-detail"]');
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
  const panel = stationPanel(page);
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
  await select(page, "example-review");
  await page.getByTestId("flow-start-open").click();
  await expect(page.getByTestId("flow-start-flow")).toHaveValue("example-review");
  await page.getByTestId("flow-start-subject").fill("repo");
  await page.getByTestId("flow-start-submit").click();

  const detail = page.locator('#secondary [data-testid="flow-run-detail"]');
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
  const panel = stationPanel(page);
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

test("the Flows tab lists every run and dry run, filtered by state and kind; a run clicked opens its flow with it", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await toFlows(page);
  await expect(page.getByTestId("flow-runs-toolbar")).toBeVisible();
  await expect(page.getByTestId("flow-runs").getByTestId("flow-add")).toHaveCount(0);
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
  // Nothing picked yet: nothing beside the console.
  await expect(page.getByTestId("flow-run-pick-hint")).toBeVisible();
  await expect(page.locator("#secondary")).toHaveCount(0);

  // A run clicked: its flow's context, the run picked there and shown beside its main session.
  const first = rows.first();
  const run = (await first.getAttribute("data-testid"))!.replace(/^flow-run-/, "");
  await first.click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "example-review");
  await expect(page.getByTestId(`flow-run-${run}`)).toHaveAttribute("aria-current", "true");
  await expect(page.locator('#secondary [data-testid="flow-run-detail"]')).toHaveAttribute("data-run", run);
  await expect(page.getByTestId("flow-context-conversation")).not.toHaveAttribute("aria-current", "true");
  expect(errors).toEqual([]);
});

test("the reference, in the right column inside a flow: the guide, the catalog and the fields; an example starts a new flow in the general conversation", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await select(page, "example-review");
  const own = await conversationOf(page, "example-review");
  await flowReference(page);
  // The framework first: one picture, coloured by the guide's five groups; an element clicked goes
  // to its row in its group below.
  const fw = page.getByTestId("flow-framework");
  await expect(fw).toBeVisible();
  for (const g of [1, 2, 3, 4, 5]) await expect(page.getByTestId(`flow-guide-group-${g}`)).toBeVisible();
  await expect(page.getByTestId("flow-fw-gate")).toHaveAttribute("data-group", "3");
  await page.getByTestId("flow-fw-gate").click();
  const reached = page.locator('[data-guide-group="3"] [data-revealed]');
  await expect(reached).toHaveAttribute("data-guide-term", /^(Gate|门禁)$/);
  await expect(reached).toBeInViewport();
  await page.getByTestId("flow-fw-legend-4").click();
  await expect(page.locator('[data-guide-heading="4"][data-revealed]')).toBeInViewport();
  const guide = page.getByTestId("flow-guide");
  await expect(guide).toContainText(/Flow guide|流程指南/);
  await expect(guide.locator("table").first()).toBeVisible();
  await expect(page.getByTestId("flow-guide-example-2")).toBeVisible();
  await expect(page.getByTestId("flow-catalog-scenes")).toContainText("flow-harnix");
  await expect(page.getByTestId("flow-catalog-part-rule")).toContainText("review-honest");
  await expect(page.getByTestId("flow-catalog-part-policy")).toContainText("read-only");
  await expect(page.getByTestId("flow-ref-fields")).toContainText("stations");
  // Read beside the flow: its conversation and its details stay.
  await expect.poll(() => current(page)).toBe(own);
  await expect(page.locator("#secondary")).toHaveAttribute("data-key", "flow:example-review");

  // An example starts a new flow: the general conversation's context, its composer started.
  await page.getByTestId("flow-guide-like-0").click();
  const general = await conversationOf(page);
  await expect.poll(() => current(page)).toBe(general);
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "");
  await expect(page.getByTestId("conv-input")).toHaveValue(/name: draft-and-sign/);
  expect(errors).toEqual([]);
});

test("a profile that drops the flows in [shell] spaces does not get them", TAGS, async ({ page }) => {
  const without = await startHarnix(launchFor("replay", { profile: '\n[shell]\nspaces = ["sessions", "projects", "system"]\n' }));
  try {
    const errors = watchConsole(page);
    await open(page, without.url);
    await expect(spaceEntry(page, "sessions")).toBeVisible();
    await expect(page.locator('#side [data-space="flow"]')).toHaveCount(0);
    await page.locator('#side button[data-space="system"]').click();
    await expect(page.getByTestId("system-console")).toBeVisible();
    await expect(page.locator('[data-testid="system-console"] [role="tab"][data-tab="flows"]')).toHaveCount(0);
    // The module is still there; only the interface leaves it out.
    expect(await page.evaluate(() => (window as any).harnix.app.snapshot.reachable.includes("app.flow.defs"))).toBe(true);
    expect(errors).toEqual([]);
  } finally {
    await without.dispose();
  }
});

test("a profile that keeps the flows but not the system space gets them as a space of their own, as before", TAGS, async ({ page }) => {
  const alone = await startHarnix(launchFor("replay", { profile: '\n[shell]\nspaces = ["sessions", "flow"]\n' }));
  try {
    const errors = watchConsole(page);
    await open(page, alone.url);
    await expect(page.locator('#side button[data-space="system"]')).toHaveCount(0);
    await spaceEntry(page, "flow").click();
    await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
    await page.locator('#details [role="tab"][data-tab="flow.flows"]').click();
    await page.getByTestId("flow-def-example-review").click();
    await expect(page.locator('#secondary [data-testid="flow-detail"]')).toHaveAttribute("data-flow", "example-review");
    await expect.poll(() => current(page)).toBe(await conversationOf(page, "example-review"));
    await expect(page.getByTestId("flow-context")).toHaveCount(0);
    expect(errors).toEqual([]);
  } finally {
    await alone.dispose();
  }
});
