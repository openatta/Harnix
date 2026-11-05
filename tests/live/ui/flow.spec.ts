/**
 * The flow space in the browser (xops-02 §7), against the empty application — the `harnix` binary,
 * which has the flow module, served the shell as built (`ui/dist`): the space is the shell's own.
 *
 * The centre is a conversation whose sessions run `flow-author`; the right column shows the flows,
 * their runs and dry runs, and the reference, and edits no definition. The flow selected there is
 * the conversation's current flow (`app.flow.focus`).
 *
 * Recordings: `flow-ui-edit` (the Agent calls `flow_show`, then `flow_update {flow:
 * example-review}` with its title changed, then says so), `flow-author-unfocused` (`flow_update` with
 * nothing selected, and the Agent relays that a flow must be selected first). The real run's
 * session station has no recording here: the test passes it by force, as a person may.
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

/** A conversation of the flow space replaying `fixture`; its scene is the space's, `flow-author`. */
async function converse(page: Page, fixture: string): Promise<string> {
  const id = await page.evaluate(async (name) => {
    const { app } = (window as any).harnix;
    await app.create({ options: { recorder: { name } } });
    return app.snapshot.current.sessionId as string;
  }, fixture);
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  const scene = await page.evaluate(async (session_id) => (await (window as any).harnix.app.call("session.get", { session_id, space: "flow" })).scene, id);
  expect(scene).toBe("flow-author");
  return id;
}

/** Permit the ask the turn raises (a tool that changes something). */
async function allow(page: Page): Promise<void> {
  const ask = page.getByTestId("conv-ask").last();
  await expect(ask).toBeVisible({ timeout: 30_000 });
  await ask.getByTestId("conv-ask-allow").click();
  await expect(ask).toHaveAttribute("data-state", "answered");
}

test("the flow space lists the built-in example; selecting it draws it and makes it the conversation's flow", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  // The conversation in the centre, its sessions on the left, the flows on the right.
  await expect(page.getByTestId("flow-flows")).toBeVisible();
  const example = page.getByTestId("flow-def-example-review");
  await expect(example).toHaveAttribute("data-origin", "builtin");
  await expect(example).toHaveAttribute("data-valid", "true");

  await select(page, "example-review");
  const graph = page.getByTestId("flow-graph");
  await expect(graph).toHaveAttribute("data-mode", "definition");
  for (const id of ["review", "approve"]) await expect(page.getByTestId(`flow-node-${id}`)).toBeVisible();
  await expect(page.getByTestId("flow-skill-body-review")).toContainText("Look at the subject");
  // A station's definition, read-only.
  await page.getByTestId("flow-node-review").click();
  await expect(page.locator('#details [data-testid="flow-station-panel"]')).toHaveAttribute("data-station", "review");

  // A conversation started now works on it: the line over the composer says which.
  const session = await converse(page, "says-ready");
  const notice = page.getByTestId("flow-focus-notice");
  await expect(notice).toHaveAttribute("data-flow", "example-review");
  await expect(notice).toContainText("Example: review and approve");
  const focus = await page.evaluate(
    async (session_id) => (await (window as any).harnix.app.call("harnix.subscribe", { topic: "app.flow.focus", params: { instance: "system", session_id } })).value,
    session,
  );
  expect(focus).toEqual({ flow: "example-review" });

  // The reference: the fields, this application's catalog, a box that checks and saves nothing.
  await tab(page, "reference");
  const reference = page.getByTestId("flow-reference");
  await expect(reference.getByTestId("flow-ref-fields")).toContainText("stations");
  await expect(reference.getByTestId("flow-catalog-scenes")).toContainText("flow-chat");
  await reference.getByTestId("flow-validate").click();
  await expect(reference.getByTestId("flow-valid")).toBeVisible();
  expect(errors).toEqual([]);
});

test("asked in the conversation, the Agent changes the selected flow, and the right column follows", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await select(page, "example-review");
  const before = await page.getByTestId("flow-detail").getAttribute("data-digest");
  await converse(page, "flow-ui-edit");
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

test("with nothing selected, the Agent is told to have one selected first, and says so", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await expect(page.getByTestId("flow-pick-hint")).toBeVisible();
  await converse(page, "flow-author-unfocused");
  const notice = page.getByTestId("flow-focus-notice");
  await expect(notice).toHaveAttribute("data-flow", "");
  await expect(notice).toContainText(/select one on the right|先在右栏选择/);

  await say(page, "Change the flow's title.");
  await expect(page.getByTestId("flow-tool-target").last()).toHaveAttribute("data-flow", "authored", { timeout: 30_000 });
  await allow(page);
  await expect(reply(page)).toContainText("No flow is selected", { timeout: 30_000 });
  await expect(page.getByTestId("conv-tool-name").filter({ hasText: /flow_update/ })).toHaveCount(1);
  expect(errors).toEqual([]);
});

test("a dry run from the runs tab steps, runs to the approval, shows what its session would be sent, and finishes once confirmed", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await select(page, "example-review");
  await tab(page, "runs");
  await page.getByTestId("flow-runs-dry-start").click();
  const detail = page.getByTestId("flow-run-detail");
  await expect(detail).toHaveAttribute("data-run", /^dry-example-review-/);
  const run = (await detail.getAttribute("data-run"))!;
  await expect(page.getByTestId(`flow-run-${run}`)).toHaveAttribute("data-dry", "true");
  await expect(page.getByTestId("flow-graph")).toHaveAttribute("data-mode", "run");

  await page.getByTestId("flow-dry-step").click();
  await expect(page.getByTestId("flow-dry-last")).toHaveAttribute("data-steps", "1");
  await page.getByTestId("flow-dry-run").click();
  await expect(page.getByTestId("flow-dry-last")).toHaveAttribute("data-pending", "0");
  const approve = page.getByTestId("flow-node-approve");
  await expect(approve).toHaveAttribute("data-status", "working");
  await expect(page.getByTestId("flow-node-review")).toHaveAttribute("data-status", "passed");

  // The mocked session task: the whole message its session would have been sent.
  await page.getByTestId("flow-node-review").click();
  const panel = page.locator('#details [data-testid="flow-station-panel"]');
  await expect(panel).toHaveAttribute("data-station", "review");
  await panel.getByTestId("flow-preview-review").locator("summary").click();
  await expect(panel.getByTestId("flow-preview-message-review")).toContainText("Skill `review`");

  await approve.click();
  await expect(panel).toHaveAttribute("data-station", "approve");
  await panel.getByTestId("flow-confirm").click();
  await expect(page.getByTestId("flow-run-status")).toHaveAttribute("data-status", "finished", { timeout: 15_000 });
  await expect(approve).toHaveAttribute("data-status", "passed");

  // It is among the flow's dry runs on the flows tab too.
  await tab(page, "flows");
  await expect(page.getByTestId(`flow-dry-${run}`)).toHaveAttribute("data-status", "finished");
  expect(errors).toEqual([]);
});

test("a run started from the runs tab is listed and drawn; forced past its session and confirmed, it finishes", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await select(page, "example-review");
  await tab(page, "runs");
  await page.getByTestId("flow-start-open").click();
  await expect(page.getByTestId("flow-start-flow")).toHaveValue("example-review");
  await page.getByTestId("flow-start-subject").fill("repo");
  await page.getByTestId("flow-start-submit").click();

  const detail = page.getByTestId("flow-run-detail");
  await expect(detail).toHaveAttribute("data-run", /^example-review-/);
  const run = (await detail.getAttribute("data-run"))!;
  await expect(page.getByTestId(`flow-run-${run}`)).toBeVisible();
  await expect(page.getByTestId(`flow-run-${run}`)).not.toHaveAttribute("data-dry", "true");
  await expect(page.getByTestId("flow-dry-controls")).toHaveCount(0);
  await expect(page.getByTestId("flow-graph")).toHaveAttribute("data-mode", "run");
  // Entered: its session is working on it, or — with no recording to replay — has already failed.
  const review = page.getByTestId("flow-node-review");
  await expect(review).toHaveAttribute("data-status", /^(working|unmet)$/, { timeout: 30_000 });

  await review.click();
  const panel = page.locator('#details [data-testid="flow-station-panel"]');
  await panel.getByTestId("flow-force").click();
  await panel.getByTestId("flow-reason").fill("reviewed by hand");
  await panel.getByTestId("flow-action-submit").click();
  await expect(review).toHaveAttribute("data-status", "passed", { timeout: 15_000 });

  const approve = page.getByTestId("flow-node-approve");
  await expect(approve).toHaveAttribute("data-status", "working");
  await approve.click();
  await panel.getByTestId("flow-confirm").click();
  await expect(page.getByTestId("flow-run-status")).toHaveAttribute("data-status", "finished", { timeout: 15_000 });
  await expect(page.getByTestId(`flow-run-${run}`)).toHaveAttribute("data-status", "finished");
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
