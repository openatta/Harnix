/**
 * A proposal in the flow space: a station's session proposed to send the work back (`flow_propose`),
 * and the person sees it as a card in that session — accepts it, which reworks the run once and
 * leaves a line saying so, or dismisses the one the next round makes.
 *
 * The test application's `demo-propose` (crates/testapp/src/flow_demo.rs): its `propose` station
 * replays `flow-propose` — a guarded write refused, `flow_propose {kind: rework, to: propose}`,
 * `flow_submit` — and `approve` waits for a confirmation behind it.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, test } from "@harnix/testkit";

import { startTestApp } from "../api/testapp";
import { open, watchConsole } from "./page";

const TAGS = { tag: ["@feature:flow", "@layer:ui"] };

let backend: Backend;
test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(async () => {
  await backend?.dispose();
});

/** The run's record once `pass` holds of it. */
async function until(page: Page, run: string, pass: (record: any) => boolean): Promise<any> {
  let record: any = null;
  await expect
    .poll(
      async () => {
        record = (await page.evaluate(async (r) => (window as any).harnix.app.call("app.flow.get", { run: r }), run)).record;
        return pass(record);
      },
      { timeout: 60_000 },
    )
    .toBe(true);
  return record;
}

const panel = (page: Page) => page.locator('#secondary [data-testid="flow-station-panel"]');

/** Open the `propose` station's session of the round showing, from the station's panel. */
async function openProposeSession(page: Page, run: string): Promise<void> {
  await page.locator('#side .spaces [data-space="flow"]').click();
  await page.locator('#details [data-tab="flow.runs"]').click();
  const row = page.getByTestId(`flow-run-${run}`);
  if ((await row.getAttribute("aria-current")) !== "true") await row.click();
  const showing = (await panel(page).count()) ? await panel(page).getAttribute("data-station") : null;
  if (showing !== "propose") await page.getByTestId("flow-node-propose").click();
  await expect(panel(page)).toHaveAttribute("data-station", "propose");
  const runTab = panel(page).getByTestId("flow-tab-run");
  if ((await runTab.getAttribute("aria-selected")) !== "true") await runTab.click();
  await panel(page).getByTestId("flow-open-session-propose").click();
}

test("a proposal is a card in its session: accepted once, it reworks the run and says so; the next is dismissed", TAGS, async ({ page }) => {
  test.setTimeout(150_000);
  const errors = watchConsole(page);
  await open(page, backend.url);
  const run = await page.evaluate(async () => (await (window as any).harnix.app.call("app.flow.start", { flow: "demo-propose", subject: "the page" })).run as string);
  const first = await until(page, run, (r) => r.stations.approve?.status === "working");
  const firstSession = first.stations.propose.tasks[0].session as string;

  await openProposeSession(page, run);
  await expect.poll(() => page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId)).toBe(firstSession.split("/")[1]);
  const card = page.getByTestId("flow-proposal");
  await expect(card).toHaveAttribute("data-kind", "rework", { timeout: 15_000 });
  await card.getByTestId("flow-proposal-accept").click();
  await expect(card.getByTestId("flow-proposal-accepted")).toBeVisible();
  // Once: the run went back to `propose` a single time.
  const second = await until(page, run, (r) => r.stations.propose.round === 2 && r.stations.approve?.status === "working");
  const history = await page.evaluate(async (r) => (window as any).harnix.app.call("app.flow.history", { run: r }), run);
  expect(history.filter((h: any) => h.op === "rework")).toHaveLength(1);

  // Opened again, the session says the proposal was taken up, and by whom.
  await page.reload();
  await expect(page.locator("#conn.on")).toBeVisible({ timeout: 15_000 });
  const [instance, sessionId] = firstSession.split("/");
  await page.evaluate(({ instance, sessionId }) => (window as any).harnix.app.open({ instance, sessionId }, { switchSpace: true }), { instance, sessionId });
  await expect(page.getByTestId("flow-proposal-handled")).toBeVisible({ timeout: 15_000 });

  // The second round proposed again; this one is dismissed, and nothing happens to the run.
  await openProposeSession(page, run);
  await expect.poll(() => page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId)).toBe(second.stations.propose.tasks[0].session.split("/")[1]);
  const again = page.getByTestId("flow-proposal");
  await expect(again).toBeVisible({ timeout: 15_000 });
  await again.getByTestId("flow-proposal-dismiss").click();
  await expect(page.getByTestId("flow-proposal")).toHaveCount(0);
  const after = await page.evaluate(async (r) => (window as any).harnix.app.call("app.flow.history", { run: r }), run);
  expect(after.filter((h: any) => h.op === "rework")).toHaveLength(1);
  await page.evaluate((r) => (window as any).harnix.app.call("app.flow.cancel", { run: r, reason: "done" }), run);
  expect(errors).toEqual([]);
});

test("a task held on a person's answer says what it asked and why, until it is answered", TAGS, async ({ page }) => {
  test.setTimeout(120_000);
  const errors = watchConsole(page);
  await open(page, backend.url);
  const run = await page.evaluate(async () => (await (window as any).harnix.app.call("app.flow.start", { flow: "demo-asked", subject: "repo" })).run as string);
  const held = await until(page, run, (r) => !!r.stations.edit?.tasks?.[0]?.asking);
  const task = held.stations.edit.tasks[0];

  await page.locator('#side .spaces [data-space="flow"]').click();
  await page.locator('#details [data-tab="flow.runs"]').click();
  await page.getByTestId(`flow-run-${run}`).click();
  await page.getByTestId("flow-node-edit").click();
  await expect(panel(page)).toHaveAttribute("data-station", "edit");
  const asking = panel(page).getByTestId(`flow-asking-${task.id}`);
  await expect(asking).toHaveAttribute("data-tool", "TodoWrite");
  await expect(asking).toContainText("the todo list is a person's call here");

  // Answered in the run's main session: the panel lets go of it.
  const [, session_id] = String(held.session_keys["@main"]).split("/");
  await page.evaluate(async (sid) => {
    const { app } = (window as any).harnix;
    const attached = await app.call("harnix.attach", { session_id: sid });
    const ask = attached.pending_prompts.find((p: any) => p.tool_name === "TodoWrite");
    await app.call("session.respondToPrompt", { session_id: sid, prompt_id: ask.prompt_id, decision: { type: "permit" } });
  }, session_id);
  await expect(asking).toHaveCount(0, { timeout: 30_000 });
  await page.evaluate((r) => (window as any).harnix.app.call("app.flow.cancel", { run: r, reason: "done" }), run).catch(() => undefined);
  expect(errors).toEqual([]);
});

test("the reference tab says whether command gates run here", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await page.locator('#side .spaces [data-space="flow"]').click();
  await page.locator('#details [data-tab="flow.reference"]').click();
  await expect(page.getByTestId("flow-reference-tab")).toBeVisible();
  // The test application turns them on.
  await expect(page.getByTestId("flow-catalog-command-gates")).toBeVisible();
  const on = await page.evaluate(async () => (await (window as any).harnix.app.call("app.flow.catalog", {})).command_gates);
  expect(on).toBe(true);
});

test("a station's session is Harnix's own: in the system space's all tab it is marked so", TAGS, async ({ page }) => {
  test.setTimeout(120_000);
  await open(page, backend.url);
  const run = await page.evaluate(async () => (await (window as any).harnix.app.call("app.flow.start", { flow: "demo-propose", subject: "marked" })).run as string);
  const record = await until(page, run, (r) => Boolean(r.stations.propose?.tasks?.[0]?.session));
  const [instance, sid] = (record.stations.propose.tasks[0].session as string).split("/");
  await page.locator('#side [data-space="system"]').click();
  await page.locator('[data-testid="system-console"] [data-tab="all"]').click();
  const row = page.locator(`[data-testid="system-agents"] .tgroup[data-instance="${instance}"][data-sid="${sid}"]`);
  await expect(row.getByTestId("system-agent-harnix")).toBeVisible({ timeout: 15_000 });
});
