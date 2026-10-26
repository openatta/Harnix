/**
 * The system space in the browser (design §9.2): the instance tree on the left, the chosen
 * session's conversation in the centre, the inspector on the right — and the operations of §9.3,
 * which take effect where the session belongs and are seen by whoever else is watching it.
 *
 * Three instances (the instance suites' profile: `system`, `side` in process, `ci` a spawned
 * daemon), so the tree has more than one branch to show.
 */

import { cpSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { type Backend, RECORDINGS, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { INSTANCES_PROFILE } from "../api/instances";
import { open, reply, say, watchConsole } from "./page";

const TREE = { tag: ["@feature:system", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay", { profile: INSTANCES_PROFILE }));
});
test.afterAll(() => backend?.stop());

async function toSystem(page: Page): Promise<void> {
  await page.locator('#side .spaces [data-space="system"]').click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "system");
  await expect(page.locator("#details")).toBeVisible();
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

/** Open an instance in the tree, then one of its sessions. */
async function pick(page: Page, instance: string, sessionId: string): Promise<void> {
  await page.locator(`#side .system-tree .inst[data-instance="${instance}"]`).click();
  await page.locator(`#side .inst-sessions[data-instance="${instance}"] [data-sid="${sessionId}"]`).click();
  await expect(page.getByTestId("conv-input")).toBeEnabled();
}

async function tab(page: Page, id: string): Promise<void> {
  await page.locator(`#details .tabs [data-tab="system.${id}"]`).click();
}

test("system-tree: every instance, its sessions of every space, and their tasks", TREE, async ({ page }) => {
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

  const tree = page.getByTestId("system-tree");
  for (const id of ["system", "side", "ci"]) {
    await expect(tree.locator(`.inst[data-instance="${id}"]`)).toBeVisible();
  }
  await expect(tree.locator('.inst[data-instance="side"]')).toHaveAttribute("data-state", "ready");
  await expect(tree.locator('.inst[data-instance="ci"]')).toHaveAttribute("data-state", "ready", { timeout: 30_000 });

  await pick(page, "side", onSide);
  await expect(page.locator(`#side .inst-sessions [data-sid="${onSide}"] .space-tag`)).toHaveText("sessions");
  await expect(reply(page)).toContainText(/fixture ready/i);
  await tab(page, "session");
  await expect(page.getByTestId("system-session")).toContainText(onSide);
  await expect(page.getByTestId("system-session")).toContainText("side");

  await pick(page, "system", withAgent);
  const tasks = page.getByTestId("system-tasks");
  await expect(tasks.locator('.task[data-kind="agent"]')).toHaveCount(1);
  await expect(tasks).toContainText("list files");

  await tab(page, "instance");
  await page.locator('#side .system-tree .inst[data-instance="ci"]').click();
  await expect(page.getByTestId("system-instance")).toContainText(/daemon/);
  expect(errors).toEqual([]);
});

test("system-calls: each call's envelope, and what changed from the one before", TREE, async ({ page }) => {
  await open(page, backend.url);
  const sid = await sessionWithTurn(page, "system");
  // A replayed session writes no recording of its own: give it the one its fixture was made from.
  cpSync(join(RECORDINGS, "spawns-agent"), join(backend.scratch, "agix", "recordings", sid), { recursive: true });
  await toSystem(page);
  await pick(page, "system", sid);
  await tab(page, "calls");
  const calls = page.getByTestId("system-call");
  await expect(calls.first()).toBeVisible();
  const count = await calls.count();
  await calls.first().getByTestId("system-call-toggle").click();
  await expect(page.getByTestId("system-calls")).toContainText(/第一次调用|The first call/);
  if (count > 1) {
    await calls.nth(1).getByTestId("system-call-toggle").click();
    await expect(page.getByTestId("system-diff").first()).toBeVisible();
  }
  await page.getByTestId("system-calls").getByTestId("open-in-details").first().click();
  await expect(page.getByTestId("details-request")).toBeVisible();
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
  await tab(page, "session");

  // Rename: the tree and the other viewer's list follow.
  await page.getByTestId("op-rename").click();
  await page.locator("#modal input").fill("watched from system");
  await page.locator("#modal .btn.primary").click();
  await expect(page.locator(`#side .inst-sessions [data-sid="${sid}"]`)).toContainText("watched from system");
  await other.evaluate(() => (window as any).harnix.app.refreshSessions());
  await expect(other.locator(`#side .srow[data-sid="${sid}"]`)).toContainText("watched from system");

  // Model: the context the next call will carry says so.
  await page.getByTestId("op-model").click();
  await page.locator("#modal input").fill("replayed-model-x");
  await page.locator("#modal .btn.primary").click();
  await tab(page, "context");
  await expect(page.getByTestId("system-context")).toContainText("replayed-model-x");
  await expect(page.locator('[data-testid="system-block"]').first()).toBeVisible();

  // Compact and clear run as turns: the other viewer's conversation shows them.
  await tab(page, "session");
  const before = await other.locator('[data-testid="conv-message"]').count();
  await page.getByTestId("op-compact").click();
  await expect.poll(() => other.locator('[data-testid="conv-message"]').count(), { timeout: 30_000 }).toBeGreaterThan(before);
  await page.getByTestId("op-clear").click();
  await page.getByTestId("confirm").click();
  await tab(page, "context");
  await expect(page.getByTestId("system-messages").locator(".message")).toHaveCount(0, { timeout: 30_000 });

  // Interrupt with nothing running is not an error.
  await tab(page, "session");
  await page.getByTestId("op-interrupt").click();
  await expect(page.locator("#banner.on")).toHaveCount(0);

  // Fork opens the copy here.
  await page.getByTestId("op-fork").click();
  await expect.poll(() => page.evaluate(() => (window as any).harnix.app.snapshot.currentBySpace.system?.sessionId)).not.toBe(sid);

  // Back to the original, and open it where it belongs: the session space shows the new name.
  await pick(page, "system", sid);
  await page.getByTestId("open-in-space").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "sessions");
  await expect(page.locator(`#side .srow[data-sid="${sid}"]`)).toHaveClass(/on/);
  await expect(page.locator("#head .title")).toContainText("watched from system");

  // Delete, from the system space.
  await toSystem(page);
  await pick(page, "system", sid);
  await tab(page, "session");
  await page.getByTestId("op-delete").click();
  await page.getByTestId("confirm").click();
  await expect(page.locator(`#side .inst-sessions [data-sid="${sid}"]`)).toHaveCount(0);
  await other.close();
});

test("system-ops: an instance restarts, and its sessions are there after", TREE, async ({ page }) => {
  await open(page, backend.url);
  const sid = await sessionWithTurn(page, "side");
  await toSystem(page);
  await page.locator('#side .system-tree .inst[data-instance="side"]').click();
  await tab(page, "instance");
  await page.getByTestId("inst-restart").click();
  await expect(page.locator('#side .system-tree .inst[data-instance="side"]')).toHaveAttribute("data-state", "ready", { timeout: 30_000 });
  await expect(page.locator(`#side .inst-sessions[data-instance="side"] [data-sid="${sid}"]`)).toBeVisible();
  const { entries } = await page.evaluate(() => (window as any).harnix.app.call("harnix.audit", { limit: 100 }));
  expect(entries.some((e: any) => e.method === "harnix.instances.restart")).toBe(true);
});

test("the system space speaks in its own session: a new one is its own", TREE, async ({ page }) => {
  await open(page, backend.url);
  await toSystem(page);
  const key = await page.evaluate(() => (window as any).harnix.app.create({ options: { recorder: { name: "says-ready" } } }));
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  const spaces = await page.evaluate(async (key) => {
    const { app } = (window as any).harnix;
    const ids = async (space: string) => (await app.call("harnix.sessions", { space })).sessions.map((s: any) => s.session_id);
    return { sessions: await ids("sessions"), system: await ids("system") };
  }, key);
  expect(spaces.sessions).not.toContain(key.sessionId);
  expect(spaces.system).toContain(key.sessionId);
});
