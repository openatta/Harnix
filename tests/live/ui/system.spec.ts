/**
 * The system space in the browser (design §9.2, M8-0): the instances on the left, the console in
 * the centre — the process at a glance, what is running, the diagnostics — and whatever is picked
 * there inspected on the right: an instance, or a session's facts and operations, every model call
 * with its input and output, its context, its tasks, its frames. The operations of §9.3 take effect
 * where the session belongs and are seen by whoever else is watching it.
 *
 * Three instances (the instance suites' profile: `system`, `side` in process, `ci` a spawned
 * daemon), so the tree has more than one branch to show.
 */

import { cpSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { type Backend, RECORDINGS, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { INSTANCES_PROFILE } from "../api/instances";
import { open, reply, watchConsole } from "./page";

const TREE = { tag: ["@feature:system", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay", { profile: INSTANCES_PROFILE }));
});
test.afterAll(() => backend?.stop());

async function toSystem(page: Page): Promise<void> {
  await page.locator('#side [data-space="system"]').click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "system");
  await expect(page.getByTestId("system-console")).toBeVisible();
}

/** A session made the way the session space makes one, on `instance`, with one replayed turn. */
async function sessionWithTurn(page: Page, instance: string, fixture = "says-ready", message = "Say exactly: fixture ready"): Promise<string> {
  return page.evaluate(
    async ({ instance, fixture, message }) => {
      const { app } = (window as any).harnix;
      const created = await app
        .call("session.create", { instance, space: "sessions", scene: instance === "side" ? "chat" : "coding", options: { recorder: { name: fixture } } })
        .catch((e: any) => {
          throw new Error(JSON.stringify(e));
        });
      const key = { instance, session_id: created.session_id, space: "sessions" };
      await app.call("harnix.attach", key);
      const settled = new Promise<void>((resolve) => {
        const off = app.client.onNotify((method: string, params: any) => {
          if (method === "harnix.turn_settled" && params.session_id === created.session_id) {
            off();
            resolve();
          }
        });
      });
      await app.call("harnix.send", { ...key, message });
      await settled;
      return created.session_id as string;
    },
    { instance, fixture, message },
  );
}

/** Pick an instance on the left, then one of its sessions in the right column. */
async function pick(page: Page, instance: string, sessionId: string): Promise<void> {
  await page.locator(`#side .system-tree .inst[data-instance="${instance}"]`).click();
  await page.locator(`[data-testid="system-instance"] .inst-sessions[data-instance="${instance}"] [data-sid="${sessionId}"]`).click();
  await expect(page.getByTestId("system-session")).toContainText(sessionId);
}

async function tab(page: Page, id: string): Promise<void> {
  await page.locator(`#details .tabs [data-tab="system.${id}"]`).click();
}

test("system-tree: every instance, its sessions of every space, and their tasks; no conversation here", TREE, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const onSide = await sessionWithTurn(page, "side");
  const withAgent = await sessionWithTurn(
    page,
    "system",
    "spawns-agent",
    "Use the Agent tool exactly once to launch a general-purpose sub-agent with the description 'list files' and the prompt 'Run ls in the current directory and reply with the file names only.' When it returns, reply with one line: 'sub-agent found:' followed by the names.",
  );
  await toSystem(page);
  await expect(page.getByTestId("conv-input")).toHaveCount(0);

  const tree = page.getByTestId("system-tree");
  for (const id of ["system", "side", "ci"]) {
    await expect(tree.locator(`.inst[data-instance="${id}"]`)).toBeVisible();
    await expect(page.locator(`[data-testid="system-running"] .tgroup[data-instance="${id}"]`)).toBeVisible();
  }
  await expect(tree.locator('.inst[data-instance="side"]')).toHaveAttribute("data-state", "ready");
  await expect(tree.locator('.inst[data-instance="ci"]')).toHaveAttribute("data-state", "ready", { timeout: 30_000 });
  // Finished work is not listed: both turns have settled.
  await expect(page.locator('[data-testid="system-running"] .tr.l1')).toHaveCount(0, { timeout: 5_000 });

  await pick(page, "side", onSide);
  await expect(page.getByTestId("system-session")).toContainText("side");

  await pick(page, "system", withAgent);
  await tab(page, "tasks");
  const tasks = page.getByTestId("system-tasks");
  await expect(tasks.locator('.task[data-kind="agent"]')).toHaveCount(1);
  await expect(tasks).toContainText("list files");

  await page.locator('#side .system-tree .inst[data-instance="ci"]').click();
  await expect(page.getByTestId("system-instance")).toContainText(/daemon/);
  expect(errors).toEqual([]);
});

test("system-strip: versions, uptime, memory and the connected users", TREE, async ({ page, context }) => {
  await open(page, backend.url);
  const other = await context.newPage();
  await open(other, backend.url);
  await toSystem(page);
  const strip = page.getByTestId("system-strip");
  await expect(strip.locator('[data-kpi="harnix"] .v')).toHaveText(/^0\.4\.\d+$/);
  await expect(strip.locator('[data-kpi="memory"] .v')).toHaveText(/MB$/);
  await expect.poll(async () => Number(await strip.locator('[data-kpi="users"] .v').textContent()), { timeout: 10_000 }).toBeGreaterThanOrEqual(2);
  await strip.locator('[data-kpi="users"]').hover();
  await expect(strip.getByTestId("connections").locator(".conn-row").first()).toBeVisible();
  await other.close();
});

test("system-calls: every model call with what went in and what came out, and its tool calls", TREE, async ({ page }) => {
  await open(page, backend.url);
  const sid = await sessionWithTurn(page, "system");
  // A replayed session writes no recording of its own: give it the one its fixture was made from.
  cpSync(join(RECORDINGS, "spawns-agent"), join(backend.scratch, "agix", "recordings", sid), { recursive: true });
  await toSystem(page);
  await pick(page, "system", sid);
  await tab(page, "calls");
  const calls = page.getByTestId("system-call");
  await expect(calls.first()).toBeVisible();
  await expect(calls.first().locator(".l2 .in")).toContainText(/system \d+/);
  await expect(page.locator('#details .step.k-tool[data-tool]').first()).toContainText("Agent");
  await page.locator('#details [data-filter="tools"]').click();
  await expect(calls).toHaveCount(1);
  await page.locator('#details [data-filter="all"]').click();

  await calls.nth(1).click();
  const detail = page.getByTestId("call-detail");
  await expect(detail.getByTestId("call-input")).toContainText(/scene\./);
  await detail.locator('[data-side="output"]').click();
  await expect(detail.getByTestId("call-output")).toContainText(/tool_result|sub-agent/);
  await detail.getByTestId("call-compare").click();
  await expect(page.locator('#details [data-testid="system-diff"], #details .diff.none').first()).toBeVisible();
  await detail.getByTestId("call-back").click();
  await expect(calls.first()).toBeVisible();
});

test("system-ops: operations take effect, and another viewer sees them", TREE, async ({ page, context }) => {
  await open(page, backend.url);
  const sid = await sessionWithTurn(page, "system");

  // Someone else is watching it in the session space.
  const other = await context.newPage();
  await open(other, backend.url);
  await other.locator('#side .spaces [data-space="sessions"]').click();
  await other.evaluate((sid) => (window as any).harnix.app.open({ instance: "system", sessionId: sid }), sid);
  await expect(reply(other)).toContainText(/fixture ready/i);

  await toSystem(page);
  await pick(page, "system", sid);

  // Rename: the facts and the other viewer's list follow.
  await page.getByTestId("op-rename").click();
  await page.locator("#modal input").fill("watched from system");
  await page.locator("#modal .btn.primary").click();
  await expect(page.getByTestId("system-session")).toContainText("watched from system");
  await other.evaluate(() => (window as any).harnix.app.refreshSessions());
  await expect(other.locator(`#side .srow[data-sid="${sid}"]`)).toContainText("watched from system");

  // Model: the context the next call will carry says so.
  await page.getByTestId("op-model").click();
  await page.locator("#modal input").fill("replayed-model-x");
  await page.locator("#modal .btn.primary").click();
  await tab(page, "context");
  await expect(page.getByTestId("system-context")).toContainText("replayed-model-x");
  await expect(page.locator('[data-testid="system-block"]').first()).toBeVisible();
  await expect(page.getByTestId("context-stack")).toBeVisible();

  // Compact and clear run as turns: the other viewer's conversation shows them.
  await tab(page, "overview");
  const before = await other.locator('[data-testid="conv-message"]').count();
  await page.getByTestId("op-compact").click();
  await expect.poll(() => other.locator('[data-testid="conv-message"]').count(), { timeout: 30_000 }).toBeGreaterThan(before);
  await page.getByTestId("op-clear").click();
  await page.getByTestId("confirm").click();
  await tab(page, "context");
  await expect(page.getByTestId("system-messages").locator(".message")).toHaveCount(0, { timeout: 30_000 });

  // Interrupt with nothing running is not an error.
  await tab(page, "overview");
  await page.getByTestId("op-interrupt").click();
  await expect(page.locator("#banner.on")).toHaveCount(0);

  // Fork selects the copy here.
  await page.getByTestId("op-fork").click();
  await expect(page.getByTestId("system-session")).not.toContainText(sid);

  // Back to the original, and open it where it belongs: the session space shows the new name.
  await pick(page, "system", sid);
  await page.getByTestId("open-in-space").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "sessions");
  await expect(page.locator(`#side .srow[data-sid="${sid}"]`)).toHaveClass(/on/);
  await expect(page.locator("#head .title")).toContainText("watched from system");

  // Delete, from the system space.
  await toSystem(page);
  await pick(page, "system", sid);
  await page.getByTestId("op-delete").click();
  await page.getByTestId("confirm").click();
  await page.locator('#side .system-tree .inst[data-instance="system"]').click();
  await expect(page.locator(`[data-testid="system-instance"] [data-sid="${sid}"]`)).toHaveCount(0);
  await other.close();
});

test("system-ops: an instance restarts, and its sessions are there after", TREE, async ({ page }) => {
  await open(page, backend.url);
  const sid = await sessionWithTurn(page, "side");
  await toSystem(page);
  await page.locator('#side .system-tree .inst[data-instance="side"]').click();
  await page.getByTestId("inst-restart").click();
  await expect(page.locator('#side .system-tree .inst[data-instance="side"]')).toHaveAttribute("data-state", "ready", { timeout: 30_000 });
  await expect(page.locator(`[data-testid="system-instance"] .inst-sessions[data-instance="side"] [data-sid="${sid}"]`)).toBeVisible();
  const { entries } = await page.evaluate(() => (window as any).harnix.app.call("harnix.audit", { limit: 100 }));
  expect(entries.some((e: any) => e.method === "harnix.instances.restart")).toBe(true);
});

test("system-frames: a session's frames arrive as it runs, with their seq", TREE, async ({ page }) => {
  await open(page, backend.url);
  const sid = await sessionWithTurn(page, "system");
  await toSystem(page);
  await pick(page, "system", sid);
  await tab(page, "frames");
  await expect(page.getByTestId("system-frames")).toBeVisible();
  // Another turn in the watched session: its frames are listed here as they come.
  await page.evaluate(
    (sid) => (window as any).harnix.app.call("harnix.send", { instance: "system", session_id: sid, space: "system", message: "Say exactly: fixture ready" }),
    sid,
  );
  await expect(page.locator("#details .frames tr").first()).toBeVisible({ timeout: 30_000 });
});
