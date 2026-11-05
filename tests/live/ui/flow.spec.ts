/**
 * The flow space (xops-01 §7) in the browser, against `harnix-testapp` with `@harnix/flow`'s
 * `registerFlowSpace()` compiled into its interface (`target/testapp-ui`, built by
 * `scripts/check.sh` or by hand — see the skip message).
 *
 * The space shows a definition as a graph with its reference beside it, starts a run from its
 * list, and confirms a station from the station panel; the run's session station replays
 * `flow-analyze`.
 */

import { existsSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { type Backend, ROOT, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, watchConsole } from "./page";

const TAGS = { tag: ["@feature:flow", "@layer:ui"] };

const UI = join(ROOT, "target", "testapp-ui");

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  test.skip(
    !existsSync(join(UI, "index.html")),
    "the testapp interface is not built: HARNIX_UI_MODULES=$PWD/ui/packages/flow/demo/register.tsx HARNIX_UI_OUT=$PWD/target/testapp-ui pnpm --filter @harnix/shell build",
  );
  backend = await startHarnix(launchFor("replay", { bin: "harnix-testapp", uiDir: UI }));
});
test.afterAll(() => backend?.stop());

async function inFlow(page: Page): Promise<void> {
  await page.locator('#side .spaces [data-space="flow"]').click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
}

test("a flow's definition is drawn as a graph, with its stations and the reference beside it", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await page.getByTestId("flow-def-demo-review").click();
  const graph = page.getByTestId("flow-graph");
  await expect(graph).toHaveAttribute("data-mode", "definition");
  for (const id of ["collect", "analyze", "lint", "self-check", "review", "report"]) {
    await expect(page.getByTestId(`flow-node-${id}`)).toBeVisible();
  }

  // A station's definition: its skill's body, and the protocol its sessions are given.
  await page.getByTestId("flow-node-analyze").click();
  const panel = page.locator('#details [data-testid="flow-station-panel"]');
  await expect(panel).toHaveAttribute("data-station", "analyze");
  await expect(panel.getByTestId("flow-skill-analyze")).toContainText("Read the module named in your item");
  await expect(panel.getByTestId("flow-protocol")).toContainText("flow_submit");

  // The reference: the definition's fields, the application's catalog, a validation box.
  await page.locator('#details [data-tab="flow.reference"]').click();
  const reference = page.getByTestId("flow-reference");
  await expect(reference.getByTestId("flow-ref-fields")).toContainText("stations");
  await expect(reference.getByTestId("flow-catalog-scenes")).toContainText("flow-chat");
  await reference.getByTestId("flow-validate").click();
  await expect(reference.getByTestId("flow-valid")).toBeVisible();
  expect(errors).toEqual([]);
});

test("a run started from the list goes to its confirmation, and confirming it in the station panel finishes it", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await inFlow(page);
  await page.getByTestId("flow-start-open").click();
  await page.getByTestId("flow-start-flow").selectOption("demo-linear");
  await page.getByTestId("flow-start-subject").fill("repo");
  await page.getByTestId("flow-start-submit").click();

  const overview = page.locator('[data-testid="flow-overview"][data-run]');
  await expect(overview).toBeVisible();
  const run = await overview.getAttribute("data-run");
  expect(run).toMatch(/^demo-linear-/);
  await expect(page.getByTestId(`flow-run-${run}`)).toBeVisible();
  await expect(page.getByTestId("flow-graph")).toHaveAttribute("data-mode", "run");
  // The program and the session station pass by themselves; the last one waits for a person.
  await expect(page.getByTestId("flow-node-analyze")).toHaveAttribute("data-status", "passed", { timeout: 30_000 });
  const approve = page.getByTestId("flow-node-approve");
  await expect(approve).toHaveAttribute("data-status", "working", { timeout: 30_000 });

  await approve.click();
  const panel = page.locator('#details [data-testid="flow-station-panel"]');
  await expect(panel).toHaveAttribute("data-station", "approve");
  await expect(panel.getByTestId("flow-station-status")).toHaveAttribute("data-status", "working");
  await panel.getByTestId("flow-confirm").click();

  await expect(page.getByTestId("flow-run-status")).toHaveAttribute("data-status", "finished", { timeout: 15_000 });
  await expect(approve).toHaveAttribute("data-status", "passed");
  // The analyze task's session opens from its row.
  await page.getByTestId("flow-node-analyze").click();
  await expect(panel.getByTestId("flow-task-analyze")).toHaveAttribute("data-status", "submitted");
  await expect(panel.getByTestId("flow-open-session-analyze")).toBeVisible();
  expect(errors).toEqual([]);
});

test("a run that finished before a restart opens from the list, read from the module", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const run = await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    return (await app.call("app.flow.start", { flow: "demo-linear", subject: "before-restart" })).run as string;
  });
  await expect
    .poll(() => page.evaluate(async (r) => (await (window as any).harnix.app.call("app.flow.get", { run: r })).record.stations.approve.status, run), { timeout: 30_000 })
    .toBe("working");
  await page.evaluate(async (r) => (window as any).harnix.app.call("app.flow.confirm", { run: r, station: "approve", pass: true }), run);

  backend = await backend.restart();
  await open(page, backend.url);
  await inFlow(page);
  await page.getByTestId(`flow-run-${run}`).click();
  await expect(page.getByTestId("flow-run-status")).toHaveAttribute("data-status", "finished");
  await expect(page.getByTestId("flow-node-approve")).toHaveAttribute("data-status", "passed");
  expect(errors).toEqual([]);
});
