/**
 * What a person does to a run from a flow's station panel and run header, inside the system space: let a held station
 * in, hand an external task in, reject a confirmation, send work back (rework), confirm, and cancel a
 * run with a reason.
 *
 * The flow is saved by the test (the Harnix app's packs are writable) and has no session
 * station, so nothing here calls a model:
 *
 *   draft    held on entry; an external task whose output hands it in
 *   approve  a confirmation, which may send the work back to `draft`
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test, topology } from "@harnix/testkit";

import { open, openFlow, openRun, toFlows, watchConsole } from "./page";

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

/** A run of `ui-actions` started, entered from the system space's Flows tab: its details beside the conversation. */
async function startRun(page: Page): Promise<string> {
  await open(page, backend.url);
  const run = await page.evaluate(async (yaml) => {
    const { app } = (window as any).harnix;
    await app.call("app.flow.save", { yaml });
    return (await app.call("app.flow.start", { flow: "ui-actions", subject: "the page" })).run as string;
  }, FLOW);
  await openRun(page, run);
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
  // Found when the split-streams pass started offering split streams only. The hand-in form is drawn
  // while its task is ready, or claimed and held by this panel (flow/src/StationPanel.tsx); over split
  // streams the push that marks the task claimed can come before the claim's answer, so the form is
  // dropped for that moment and drawn again empty, and the refusal it was about to show is lost. Over
  // one connection the answer usually comes first, so there it fails only now and then.
  test.fixme(topology() === "split_streams", "the hand-in form loses its refusal when the claim's push beats its answer");
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

const PAIR = [
  "name: ui-pair",
  "title: Two hands",
  "stations:",
  "  - id: pair",
  "    tasks: [{id: x, executor: {kind: external}}, {id: y, executor: {kind: external}}]",
  "    output: {type: object}",
  "    exit: [{kind: output}]",
  "  - id: check",
  "    needs: [pair]",
  "    exit: [{kind: confirm}]",
  "    rework_to: [pair]",
].join("\n");


test("a station's panel says its state and, on its definition tab, its work, contract and exits; the run says what and whose", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await startRun(page);
  await expect(node(page, "draft")).toHaveAttribute("data-status", "held", { timeout: 15_000 });
  await station(page, "draft");
  await expect(panel(page).getByTestId("flow-station-status")).toHaveAttribute("data-status", "held");
  await expect(panel(page).getByTestId("flow-actions")).toBeVisible();

  await panel(page).getByTestId("flow-tab-definition").click();
  const def = panel(page).getByTestId("flow-definition");
  await expect(def).toBeVisible();
  await expect(def.getByTestId("flow-output-contract")).toContainText("text");
  await expect(def.getByTestId("flow-exit").locator("li")).toHaveCount(1);

  const meta = page.getByTestId("flow-run-meta");
  await expect(meta).toContainText("ui-actions");
  await expect(meta).toContainText("the page");
  // No session station: the run has its sessions section, with no main session in it.
  await expect(page.getByTestId("flow-run-sessions")).toBeVisible();
  expect(errors).toEqual([]);
});

test("an action the module refuses says why, in the panel", TAGS, async ({ page }) => {
  await startRun(page);
  await expect(node(page, "draft")).toHaveAttribute("data-status", "held", { timeout: 15_000 });
  await station(page, "draft");
  // The module refuses this one release as it would a stale one: the panel shows the refusal.
  await page.evaluate(() => {
    const client = (window as any).harnix.app.client;
    const call = client.call.bind(client);
    client.call = (method: string, params: unknown) =>
      method === "app.flow.release" ? Promise.reject(new Error("the station is not held any more")) : call(method, params);
  });
  await panel(page).getByTestId("flow-release").click();
  await expect(panel(page).getByTestId("flow-error")).toContainText("not held any more");
  await expect(node(page, "draft")).toHaveAttribute("data-status", "held");
});

test("work sent back to a station of two tasks names which of them go again", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const run = await page.evaluate(async (yaml) => {
    const { app } = (window as any).harnix;
    try {
      await app.call("app.flow.save", { yaml });
      return (await app.call("app.flow.start", { flow: "ui-pair", subject: "both" })).run as string;
    } catch (e) {
      throw new Error(JSON.stringify(e));
    }
  }, PAIR);
  await openRun(page, run);
  for (const task of ["x", "y"]) {
    await station(page, "pair");
    await panel(page).getByTestId(`flow-hand-in-${task}`).click();
    const form = panel(page).getByTestId(`flow-hand-in-form-${task}`);
    await form.getByTestId(`flow-hand-in-output-${task}`).fill(JSON.stringify({ by: task }));
    await form.getByTestId(`flow-hand-in-submit-${task}`).click();
    await expect(form).toHaveCount(0);
  }
  await expect(node(page, "check")).toHaveAttribute("data-status", "working", { timeout: 15_000 });

  await station(page, "check");
  await panel(page).getByTestId("flow-rework").click();
  await panel(page).getByTestId("flow-rework-target").selectOption("pair");
  const pick = panel(page).getByTestId("flow-rework-tasks");
  await expect(pick.locator("input[type=checkbox]")).toHaveCount(2);
  await pick.getByTestId("flow-rework-task-x").check();
  await panel(page).getByTestId("flow-reason").fill("x again");
  await panel(page).getByTestId("flow-action-submit").click();
  await expect(node(page, "pair")).toHaveAttribute("data-status", "working", { timeout: 15_000 });
  const tasks = await page.evaluate(async (r) => (await (window as any).harnix.app.call("app.flow.get", { run: r })).record.stations.pair.tasks, run);
  const status = Object.fromEntries(tasks.map((t: any) => [t.id, t.status]));
  expect(status.x, JSON.stringify(tasks)).not.toBe(status.y);
  expect(errors).toEqual([]);
});

test("a run started from a flow's details: what it is about, a title and inputs, refused until they are right", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await page.evaluate((yaml) => (window as any).harnix.app.call("app.flow.save", { yaml }), FLOW);
  await openFlow(page, "ui-actions");
  await page.getByTestId("flow-start-open").click();
  const form = page.getByTestId("flow-start-form");
  await expect(form).toBeVisible();
  await page.getByTestId("flow-start-subject").fill("");
  await expect(page.getByTestId("flow-start-submit"), "nothing to start on").toBeDisabled();
  await page.getByTestId("flow-start-subject").fill("from the form");
  await page.getByTestId("flow-start-title").fill("Started by hand");
  await page.getByTestId("flow-start-inputs").fill("{not json");
  await page.getByTestId("flow-start-submit").click();
  await expect(page.getByTestId("flow-start-error")).toBeVisible();

  // JSON, but an input the flow does not declare: the module's refusal, in the same place.
  await page.getByTestId("flow-start-inputs").fill('{"note": "hi"}');
  await page.getByTestId("flow-start-submit").click();
  await expect(page.getByTestId("flow-start-error")).toContainText("note");
  await page.getByTestId("flow-start-inputs").fill("");
  await page.getByTestId("flow-start-submit").click();
  await expect(form).toHaveCount(0);
  // The runs tab, the new run picked: it is the one started here.
  const meta = page.getByTestId("flow-run-meta");
  await expect(meta).toContainText("from the form", { timeout: 15_000 });
  await expect(page.locator('#secondary [data-testid="flow-run-detail"]')).toContainText("Started by hand");
  expect(errors).toEqual([]);
});

test("the runs narrowed to one flow's: by the filter, and inside the flow", TAGS, async ({ page }) => {
  await open(page, backend.url);
  const [a, b] = await page.evaluate(
    async ([one, two]) => {
      const { app } = (window as any).harnix;
      await app.call("app.flow.save", { yaml: one });
      await app.call("app.flow.save", { yaml: two });
      return [
        (await app.call("app.flow.start", { flow: "ui-actions", subject: "a" })).run as string,
        (await app.call("app.flow.start", { flow: "ui-pair", subject: "b" })).run as string,
      ];
    },
    [FLOW, PAIR],
  );
  await toFlows(page);
  await expect(page.getByTestId(`flow-run-${a}`)).toBeVisible();
  await expect(page.getByTestId(`flow-run-${b}`)).toBeVisible();
  await page.getByTestId("flow-filter-flow").selectOption("ui-pair");
  await expect(page.getByTestId(`flow-run-${a}`)).toHaveCount(0);
  await expect(page.getByTestId(`flow-run-${b}`)).toBeVisible();
  await page.getByTestId("flow-filter-flow").selectOption("");
  await expect(page.getByTestId(`flow-run-${a}`)).toBeVisible();
  // Inside a flow, the left column lists that flow's runs alone.
  await openFlow(page, "ui-pair");
  const mine = page.getByTestId("flow-context-runs");
  await expect(mine.getByTestId(`flow-run-${b}`)).toBeVisible();
  await expect(mine.getByTestId(`flow-run-${a}`)).toHaveCount(0);
});

test("a flow broken by a part taken from it says what is wrong, and cannot be started", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    await app.call("app.flow.part.save", { kind: "rule", name: "ui-mine", text: "# Mine\n\nKeep it short." });
    await app.call("app.flow.save", { yaml: "name: ui-ruled\nrules: [ui-mine]\nstations: [{id: a, exit: [{kind: confirm}]}]" });
    await app.call("app.flow.part.delete", { kind: "rule", name: "ui-mine" });
  });
  await toFlows(page);
  await expect(page.getByTestId("flow-def-ui-ruled")).toHaveAttribute("data-valid", "false");
  await openFlow(page, "ui-ruled");
  await expect(page.getByTestId("flow-def-problems")).toContainText("ui-mine");
  await expect(page.getByTestId("flow-start-open")).toBeDisabled();
  // Its definition is still read in full.
  await expect(page.getByTestId("flow-def-flow-level")).toContainText("ui-mine");
  await page.evaluate(() => (window as any).harnix.app.call("app.flow.delete", { name: "ui-ruled" }));
  expect(errors).toEqual([]);
});

const RETRY = [
  "name: ui-retry",
  "title: Tried again",
  "stations:",
  "  - id: draft",
  "    executor: {kind: external, who: author}",
  "    output: {type: object, required: [text], properties: {text: {type: string}}}",
  "    exit: [{kind: output}]",
  "  - id: approve",
  "    needs: [draft]",
  "    permissions: [{effect: deny, tools: [Bash]}]",
  "    exit: [{kind: confirm}]",
].join("\n");

async function startRetry(page: Page): Promise<string> {
  await open(page, backend.url);
  const run = await page.evaluate(async (yaml) => {
    const { app } = (window as any).harnix;
    await app.call("app.flow.save", { yaml });
    return (await app.call("app.flow.start", { flow: "ui-retry", subject: "again" })).run as string;
  }, RETRY);
  await openRun(page, run);
  return run;
}

test("a hand-in held while it is refused shows its lease; given up, the task fails and is tried again", TAGS, async ({ page }) => {
  // The same race as the first test's: deterministic over split streams, now and then over one connection.
  test.fixme(topology() === "split_streams", "the hand-in form loses its refusal when the claim's push beats its answer");
  const errors = watchConsole(page);
  await startRetry(page);
  await station(page, "draft");
  await panel(page).getByTestId("flow-hand-in-draft").click();
  const form = panel(page).getByTestId("flow-hand-in-form-draft");
  await form.getByTestId("flow-hand-in-output-draft").fill('{"nope": 1}');
  await form.getByTestId("flow-hand-in-submit-draft").click();
  await expect(form.getByTestId("flow-hand-in-error-draft")).toBeVisible();
  // Claimed for the hand-in: the task is leased to this person until it is handed in or given up.
  await expect(form).toHaveAttribute("data-held", "true");
  await expect(panel(page).getByTestId("flow-lease-draft")).toBeVisible();

  await form.getByTestId("flow-hand-in-give-up-draft").click();
  await expect(panel(page).getByTestId("flow-task-draft")).toHaveAttribute("data-status", "failed", { timeout: 15_000 });
  await panel(page).getByTestId("flow-retry-draft").click();
  await expect(panel(page).getByTestId("flow-task-draft")).not.toHaveAttribute("data-status", "failed", { timeout: 15_000 });
  await handIn(page, "second try");
  await expect(node(page, "draft")).toHaveAttribute("data-status", "passed", { timeout: 15_000 });
  expect(errors).toEqual([]);
});

test("marks on the diagrams: a guarded station on the flow's; a forced one on the run's, with who and why", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await page.evaluate((yaml) => (window as any).harnix.app.call("app.flow.save", { yaml }), RETRY);
  await openFlow(page, "ui-retry");
  await expect(page.locator('#secondary [data-testid="flow-diagram"]').getByTestId("flow-guarded-approve")).toBeAttached();
  await expect(page.locator('#secondary [data-testid="flow-diagram"]').getByTestId("flow-guarded-draft")).toHaveCount(0);

  await startRetry(page);
  await handIn(page, "done");
  await expect(node(page, "approve")).toHaveAttribute("data-status", "working", { timeout: 15_000 });
  await station(page, "approve");
  await panel(page).getByTestId("flow-force").click();
  await expect(panel(page).getByTestId("flow-action-submit"), "forcing needs a reason").toBeDisabled();
  await panel(page).getByTestId("flow-reason").fill("nobody to confirm today");
  await panel(page).getByTestId("flow-action-submit").click();
  await expect(page.getByTestId("flow-forced-approve")).toBeAttached({ timeout: 15_000 });
  await expect(panel(page).getByTestId("flow-forced")).toContainText("nobody to confirm today");
  await expect(page.getByTestId("flow-run-phase")).toHaveAttribute("data-phase", "finished", { timeout: 15_000 });
  expect(errors).toEqual([]);
});
