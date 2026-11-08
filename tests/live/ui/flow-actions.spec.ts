/**
 * What a person does to a run from the flow space's station panel and run header: let a held station
 * in, hand an external task in, reject a confirmation, send work back (rework), confirm, and cancel a
 * run with a reason.
 *
 * The flow is saved by the test (the empty application's packs are writable) and has no session
 * station, so nothing here calls a model:
 *
 *   draft    held on entry; an external task whose output hands it in
 *   approve  a confirmation, which may send the work back to `draft`
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, watchConsole } from "./page";

const TAGS = { tag: ["@feature:flow", "@layer:ui"] };

const FLOW = [
  "name: ui-actions",
  "title: Driven from the station panel",
  "stations:",
  "  - id: draft",
  "    title: Draft",
  "    hold: true",
  "    executor: {kind: external, who: author}",
  "    output: {type: object, required: [text], properties: {text: {type: string}}}",
  "    exit: [{kind: output}]",
  "  - id: approve",
  "    title: Approve",
  "    needs: [draft]",
  "    exit: [{kind: confirm}]",
  "    rework_to: [draft]",
].join("\n");

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => { await backend?.dispose(); });

/** The flow space, a run of `ui-actions` started, and its details beside the conversation. */
async function startRun(page: Page): Promise<string> {
  await open(page, backend.url);
  const run = await page.evaluate(async (yaml) => {
    const { app } = (window as any).harnix;
    await app.call("app.flow.save", { yaml });
    return (await app.call("app.flow.start", { flow: "ui-actions", subject: "the page" })).run as string;
  }, FLOW);
  await page.locator('#side .spaces [data-space="flow"]').click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  await page.locator('#details [data-tab="flow.runs"]').click();
  await page.getByTestId(`flow-run-${run}`).click();
  await expect(page.locator('#secondary [data-testid="flow-run-detail"]')).toHaveAttribute("data-run", run);
  return run;
}

const node = (page: Page, station: string) => page.getByTestId(`flow-node-${station}`);
const panel = (page: Page) => page.locator('#secondary [data-testid="flow-station-panel"]');

/** Open a station's panel on its run tab. */
async function station(page: Page, id: string): Promise<void> {
  // Clicking the node that is already open lets go of it.
  const showing = (await panel(page).count()) ? await panel(page).getAttribute("data-station") : null;
  if (showing !== id) await node(page, id).click();
  await expect(panel(page)).toHaveAttribute("data-station", id);
  const runTab = panel(page).getByTestId("flow-tab-run");
  if ((await runTab.getAttribute("aria-selected")) !== "true") await runTab.click();
}

/** Hand the external `draft` task in with `text`, from its form (already open after a refusal). */
async function handIn(page: Page, text: string): Promise<void> {
  await station(page, "draft");
  const opener = panel(page).getByTestId("flow-hand-in-draft");
  if (await opener.count()) await opener.click();
  const form = panel(page).getByTestId("flow-hand-in-form-draft");
  await form.getByTestId("flow-hand-in-output-draft").fill(JSON.stringify({ text }));
  await form.getByTestId("flow-hand-in-submit-draft").click();
}

test("held, released, handed in, rejected, sent back, handed in again and confirmed: the run finishes", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await startRun(page);

  // Held on entry: nothing to hand in until it is let in.
  await expect(node(page, "draft")).toHaveAttribute("data-status", "held", { timeout: 15_000 });
  await station(page, "draft");
  await expect(panel(page).getByTestId("flow-hand-in-draft")).toHaveCount(0);
  await panel(page).getByTestId("flow-release").click();
  await expect(node(page, "draft")).toHaveAttribute("data-status", "working");

  // An output that breaks the station's contract is refused, and says so.
  await panel(page).getByTestId("flow-hand-in-draft").click();
  const form = panel(page).getByTestId("flow-hand-in-form-draft");
  await form.getByTestId("flow-hand-in-output-draft").fill('{"nope": 1}');
  await form.getByTestId("flow-hand-in-submit-draft").click();
  await expect(form.getByTestId("flow-hand-in-error-draft")).toBeVisible();

  await handIn(page, "v1");
  await expect(node(page, "draft")).toHaveAttribute("data-status", "passed", { timeout: 15_000 });
  await expect(node(page, "approve")).toHaveAttribute("data-status", "working");

  // Rejected (a reason is optional here): it waits again.
  await station(page, "approve");
  await panel(page).getByTestId("flow-reject").click();
  await expect(panel(page).getByTestId("flow-action-form")).toHaveAttribute("data-kind", "reject");
  await panel(page).getByTestId("flow-reason").fill("not yet");
  await panel(page).getByTestId("flow-action-submit").click();
  await expect(node(page, "approve")).not.toHaveAttribute("data-status", "passed");

  // Sent back to the draft: it is working again, in a new round.
  await panel(page).getByTestId("flow-rework").click();
  await panel(page).getByTestId("flow-rework-target").selectOption("draft");
  await panel(page).getByTestId("flow-reason").fill("needs a second pass");
  await panel(page).getByTestId("flow-action-submit").click();
  await expect(node(page, "draft")).toHaveAttribute("data-status", /^(held|working)$/, { timeout: 15_000 });
  if ((await node(page, "draft").getAttribute("data-status")) === "held") {
    await station(page, "draft");
    await panel(page).getByTestId("flow-release").click();
  }
  await handIn(page, "v2");
  await expect(node(page, "approve")).toHaveAttribute("data-status", "working", { timeout: 15_000 });

  await station(page, "approve");
  await panel(page).getByTestId("flow-confirm").click();
  await expect(page.getByTestId("flow-run-phase")).toHaveAttribute("data-phase", "finished", { timeout: 15_000 });
  expect(errors).toEqual([]);
});

test("a run is cancelled from its header, and only with a reason", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  const run = await startRun(page);
  await page.getByTestId("flow-cancel-run").click();
  const form = page.getByTestId("flow-cancel-form");
  await expect(form).toBeVisible();
  await expect(page.getByTestId("flow-cancel-submit")).toBeDisabled();
  await page.getByTestId("flow-cancel-reason").fill("started by mistake");
  await page.getByTestId("flow-cancel-submit").click();
  await expect(page.getByTestId("flow-run-phase")).toHaveAttribute("data-phase", "cancelled", { timeout: 15_000 });
  await expect(page.getByTestId(`flow-run-${run}`)).toHaveAttribute("data-phase", "cancelled");
  await expect(page.getByTestId("flow-cancel-run")).toBeDisabled();
  // The reason is on the record of what happened to the run.
  const history = await page.evaluate(async (r) => (window as any).harnix.app.call("app.flow.history", { run: r }), run);
  expect(JSON.stringify(history)).toContain("started by mistake");
  expect(errors).toEqual([]);
});
