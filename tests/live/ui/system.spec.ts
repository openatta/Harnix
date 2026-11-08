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

import { cpSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { type Backend, RECORDINGS, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { INSTANCES_PROFILE } from "../api/instances";
import { newSession, open, reply, say, watchConsole } from "./page";

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
        .call("session.create", { instance, space: "sessions", scene: "harnix", options: { recorder: { name: fixture } } })
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
  for (const id of ["system", "side", "ci"]) await expect(tree.locator(`.inst[data-instance="${id}"]`)).toBeVisible();
  await expect(tree.locator('.inst[data-instance="side"]')).toHaveAttribute("data-state", "ready");
  await expect(tree.locator('.inst[data-instance="ci"]')).toHaveAttribute("data-state", "ready", { timeout: 30_000 });
  // Both turns have settled; both Agents are loaded, so active, and idle.
  const agents = page.getByTestId("system-agents");
  await expect(agents).toHaveAttribute("data-scope", "active");
  await expect(agents.locator(`.tgroup[data-instance="side"][data-sid="${onSide}"]`)).toHaveAttribute("data-state", "idle", { timeout: 10_000 });
  const parent = agents.locator(`.tgroup[data-instance="system"][data-sid="${withAgent}"]`);
  await expect(parent).toHaveAttribute("data-state", "idle");
  // Its sub-agent is counted on the row, and under it once the row is open.
  await expect(parent.locator('[data-stat="subagents"]')).toHaveText("1", { timeout: 10_000 });
  await parent.getByTestId("system-agent-toggle").click();
  await expect(parent.getByTestId("system-subagent")).toHaveCount(1);
  await expect(parent.getByTestId("system-subagent")).toContainText("list files");

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

test("system-agents: the active tab lists the loaded Agents, the all tab every one; a row carries its sub-agents, tasks, tokens and model time", TREE, async ({ page }) => {
  await open(page, backend.url);
  const id = await sessionWithTurn(page, "system");
  // A replayed session records nothing; give it a recording: two calls of its own, one of a sub-agent's.
  const dir = join(backend.scratch, "agix", "recordings", id);
  mkdirSync(dir, { recursive: true });
  const lines = [
    { type: "recording", version: 1, session_id: id },
    { type: "call", seq: 1, ts: 1000, session_id: id, turn: 0, step: 0, params: { model: "big" } },
    { type: "end", seq: 2, ts: 1900, call: 1, outcome: { status: "ok" }, usage: { input_tokens: 600, cache_read_input_tokens: 400, output_tokens: 34 }, duration_ms: 900 },
    { type: "call", seq: 3, ts: 2000, session_id: "SUB-1", parent_session_id: id, turn: 0, step: 0, params: { model: "small" } },
    { type: "end", seq: 4, ts: 2600, call: 3, outcome: { status: "ok" }, usage: { input_tokens: 150, output_tokens: 50 }, duration_ms: 600 },
  ];
  writeFileSync(join(dir, "calls.jsonl"), lines.map((l) => JSON.stringify(l)).join("\n") + "\n");
  await toSystem(page);

  const agents = page.getByTestId("system-agents");
  const row = agents.locator(`.tgroup[data-instance="system"][data-sid="${id}"]`);
  await expect(row).toHaveAttribute("data-state", "idle", { timeout: 10_000 });
  await expect(row.locator('[data-stat="tokens"]')).toHaveText("1.2k", { timeout: 10_000 }); // 1034 + 200
  await expect(row.locator('[data-stat="time"]')).toHaveText("1.5s");
  await expect(row.locator('[data-stat="subagents"]')).toHaveText("1");
  await expect(row.locator('[data-stat="tasks"]')).toHaveText("0");
  await expect(row.locator('[data-stat="tokens"]')).toHaveAttribute("title", /cache read 400/);
  await expect(row.getByTestId("system-agent-harnix")).toHaveCount(0);
  // Opened: the sub-agent under it, with its own figures; clicked, the Agent is on the right.
  await row.getByTestId("system-agent-toggle").click();
  const sub = row.locator('[data-testid="system-subagent"][data-sub="SUB-1"]');
  await expect(sub).toContainText("small");
  await expect(sub).toContainText("200");
  await row.getByTestId("system-agent").click();
  const pane = page.getByTestId("system-session");
  await expect(pane).toContainText(id);
  // On the right: the Agent's own figures this run, as DSH counts them, and its sub-agents' sum.
  const stats = pane.getByTestId("agent-stats");
  await expect(stats.locator('[data-cell="turns"] .v')).toHaveText("1");
  await expect(stats.locator('[data-cell="calls"] .v')).toHaveText("1");
  await expect(stats.locator('[data-cell="total"] .v')).toHaveText("1.0k"); // 600 + 400 + 34
  await expect(stats.locator('[data-cell="hit"] .v')).toHaveText("40%");
  await expect(stats.locator('[data-cell="model"] .v')).toHaveText("900ms");
  await expect(pane.getByTestId("agent-tokens")).toContainText(/cache read 400/);
  await expect(pane.getByTestId("agent-subs")).toContainText("1");
  await expect(pane.getByTestId("agent-goal")).toHaveCount(0);
  // The model the next call goes to, and how full that call's context is.
  await expect(pane.locator(".facts")).toContainText(/claude-|big/);
  await expect(pane.getByTestId("agent-context")).toBeVisible();
  // The sub-agent, picked: its own figures out of the parent's recording, and the way back.
  await sub.click();
  const subPane = page.getByTestId("system-subagent-pane");
  await expect(subPane.getByTestId("agent-stats").locator('[data-cell="total"] .v')).toHaveText("200");
  await expect(subPane.getByTestId("agent-stats").locator('[data-cell="model"] .v')).toHaveText("600ms");
  await expect(subPane.locator(".facts")).toContainText("small");
  await subPane.getByTestId("agent-open-parent").click();
  await expect(page.getByTestId("system-session")).toContainText(id);

  // The strip counts the Agents and what waits on a person.
  const strip = page.getByTestId("system-strip");
  await expect(strip.locator('[data-kpi="agents"] .v')).toHaveText(/^\d+\/\d+$/);
  await expect(strip.locator('[data-kpi="waiting"] .v')).toHaveText("0");
  await expect.poll(async () => Number(await strip.locator('[data-kpi="threads"] .v').textContent())).toBeGreaterThan(1);

  // The all tab: every main Agent, this one among them.
  await page.locator('[data-testid="system-console"] [data-tab="all"]').click();
  await expect(agents).toHaveAttribute("data-scope", "all");
  await expect(agents.locator(`.tgroup[data-sid="${id}"]`)).toBeVisible();
});

test("system-agents: a running turn's clock counts on; one that finished unseen says so until picked; the list is searched and filtered", TREE, async ({ page }) => {
  await open(page, backend.url);
  await toSystem(page);
  // Started from elsewhere while the list is in view: four seconds of Bash, its ask let through.
  const id = await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    const created = await app.call("session.create", {
      instance: "system", space: "sessions", scene: "harnix", options: { recorder: { name: "runs-a-while" } },
    });
    const key = { instance: "system", session_id: created.session_id, space: "sessions" };
    app.client.onNotify((method: string, params: any) => {
      if (method === "harnix.event" && params.session_id === created.session_id && params.event?.kind === "prompt") {
        void app.call("session.respondToPrompt", { ...key, prompt_id: params.event.prompt_id, decision: { type: "permit" } });
      }
    });
    await app.call("harnix.attach", key);
    await app.call("harnix.send", { ...key, message: "Use the Bash tool to wait a few seconds" });
    return created.session_id as string;
  });
  const agents = page.getByTestId("system-agents");
  const row = agents.locator(`.tgroup[data-sid="${id}"]`);
  const clock = row.getByTestId("system-agent-elapsed");
  await expect(clock).toBeVisible({ timeout: 10_000 });
  const first = await clock.textContent();
  await expect.poll(() => clock.textContent(), { timeout: 5_000 }).not.toBe(first);
  // Done while nobody had it open or picked: DSH's "completed", until someone looks.
  await expect(row).toHaveAttribute("data-state", "done", { timeout: 30_000 });
  await expect(row.locator(".act")).toHaveText(/not yet seen|未看/);
  await expect(clock).toHaveCount(0);
  // Kept in this browser: a reload does not forget it.
  await page.reload();
  await expect(page.locator("#conn.on")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("system-console")).toBeVisible();
  await expect(row).toHaveAttribute("data-state", "done", { timeout: 10_000 });
  await row.getByTestId("system-agent").click();
  await expect(row).toHaveAttribute("data-state", "idle");
  await expect(row.getByTestId("system-agent")).toHaveAttribute("title", new RegExp(id));

  // Searched: by its id, one row; by nothing anyone has, none, and it says so.
  const search = page.getByTestId("system-agents-search");
  await search.fill(id);
  await expect(agents.locator(".tgroup")).toHaveCount(1);
  await expect(page.getByTestId("system-agents-count")).toHaveText(/^1 (of|\/) \d+$/);
  await search.fill("nothing-is-called-this");
  await expect(agents.locator(".tgroup")).toHaveCount(0);
  await expect(agents.locator(".empty")).toHaveText(/No Agent matches|没有符合/);
  // Filtered by who started it: Harnix's own are all marked.
  await search.fill("");
  await page.getByTestId("system-agents-who").selectOption("harnix");
  await expect(agents.locator(".tgroup:not(:has([data-testid='system-agent-harnix']))")).toHaveCount(0);
  await page.getByTestId("system-agents-who").selectOption("user");
  await expect(agents.locator(`.tgroup[data-sid="${id}"]`)).toBeVisible();
});

test("system-agents: an Agent with a goal shows it on its row and on the right", TREE, async ({ page }) => {
  await open(page, backend.url);
  await newSession(page, "sets-a-goal");
  await say(page, "Set the goal: keep the fixture tests green");
  await expect(page.locator('[data-testid="conv-message"]', { hasText: "Goal set." })).toBeVisible({ timeout: 30_000 });
  const id = await page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId as string);
  await toSystem(page);
  const row = page.locator(`[data-testid="system-agents"] .tgroup[data-sid="${id}"]`);
  const chip = row.getByTestId("system-agent-goal");
  await expect(chip).toHaveAttribute("data-phase", "active", { timeout: 15_000 });
  await expect(chip).toHaveAttribute("title", "Keep the fixture tests green");
  await row.getByTestId("system-agent").click();
  const goal = page.getByTestId("system-session").getByTestId("agent-goal");
  await expect(goal).toHaveAttribute("data-phase", "active");
  await expect(goal).toContainText("Keep the fixture tests green");
  await expect(goal).toContainText(/Round|轮/);

  // Edited from here: a new objective and fewer rounds.
  await goal.getByTestId("agent-goal-edit").click();
  await page.getByTestId("goal-objective").fill("Keep them green, and fast");
  await page.getByTestId("goal-rounds").fill("5");
  await page.getByTestId("goal-save").click();
  await expect(goal).toContainText("Keep them green, and fast");
  await expect(goal).toContainText(/of 5|共 5 轮/);
  // Paused, it shows the way back.
  if (await goal.getByTestId("agent-goal-pause").count()) await goal.getByTestId("agent-goal-pause").click();
  await expect(goal.getByTestId("agent-goal-resume")).toBeVisible();
  // Cleared (after a confirm): no goal, and a way to set one.
  await goal.getByTestId("agent-goal-clear").click();
  await page.getByTestId("confirm").click();
  const pane = page.getByTestId("system-session");
  await expect(pane.getByTestId("agent-goal")).toHaveCount(0);
  await expect(pane.getByTestId("agent-goal-set")).toBeVisible();
  // A bad form says so and is not sent.
  await pane.getByTestId("agent-goal-set").click();
  await page.getByTestId("goal-save").click();
  await expect(page.getByTestId("goal-error")).toBeVisible();
});

test("system-agents: an Agent waiting on its permission ask says so, first in the list, and counts on the strip", TREE, async ({ page }) => {
  await open(page, backend.url);
  await newSession(page, "asks-permission");
  await say(page, "Use the Bash tool to run exactly: touch harnix-permission-fixture");
  await expect(page.getByTestId("conv-ask")).toBeVisible({ timeout: 30_000 });
  const id = await page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId as string);
  await toSystem(page);
  const first = page.locator('[data-testid="system-agents"] .tgroup').first();
  await expect(first).toHaveAttribute("data-sid", id, { timeout: 10_000 });
  await expect(first).toHaveAttribute("data-state", "waiting");
  await expect(first.locator(".act")).toHaveText(/Approval|等批准/);
  await expect(page.locator('[data-testid="system-strip"] [data-kpi="waiting"] .v')).toHaveText("1");
  // Answered from its own conversation, it waits on nobody.
  await page.locator('#side [data-space="sessions"]').click();
  await page.evaluate((sid) => (window as any).harnix.app.open({ instance: "system", sessionId: sid }), id);
  await page.getByTestId("conv-ask").getByTestId("conv-ask-allow").click();
  await toSystem(page);
  await expect(page.locator(`[data-testid="system-agents"] .tgroup[data-sid="${id}"]`)).not.toHaveAttribute("data-state", "waiting", { timeout: 15_000 });
  await expect(page.locator('[data-testid="system-strip"] [data-kpi="waiting"] .v')).toHaveText("0");
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

test("system-context: a message of the context is edited and removed from the page, and the injected one is neither", TREE, async ({ page }) => {
  await open(page, backend.url);
  const sid = await sessionWithTurn(page, "system");
  await toSystem(page);
  await pick(page, "system", sid);
  await tab(page, "context");
  const messages = page.getByTestId("system-messages");
  const user = messages.locator('.message[data-origin="user"]').first();
  await expect(user).toContainText("Say exactly: fixture ready");
  // What the engine put at the start is shown, and is not the host's to change.
  const injected = messages.locator('.message[data-origin="injected"]');
  await expect(injected.first()).toBeVisible();
  await expect(injected.getByTestId("message-edit")).toHaveCount(0);
  await expect(injected.getByTestId("message-drop")).toHaveCount(0);
  // The model's own message is neither.
  await expect(messages.locator('.message[data-origin="assistant"]').getByTestId("message-edit")).toHaveCount(0);

  await user.getByTestId("message-edit").click();
  const input = page.locator("#modal input");
  await input.fill("Say exactly: edited from the page");
  await input.press("Enter");
  await expect(messages.locator('.message[data-origin="user"]').first()).toContainText("edited from the page");

  await messages.locator('.message[data-origin="user"]').first().getByTestId("message-drop").click();
  await expect(messages).toContainText(/removed by the host/i);
  const history = await page.evaluate(
    async (sid) => (await (window as any).harnix.app.call("session.history", { instance: "system", session_id: sid, space: "system" })).messages,
    sid,
  );
  expect(JSON.stringify(history)).not.toContain("edited from the page");
});

/** A conversation in the session space that called a tool, with the recording its fixture was made
 *  from where `system`'s engine keeps them (a replayed session writes none of its own). */
async function conversationWithTool(page: Page): Promise<string> {
  await open(page, backend.url);
  const sid = await newSession(page, "calls-a-tool");
  await say(page, "Use the Bash tool to run exactly: echo harnix-fixture");
  await expect(page.locator('[data-testid="conv-tool"]').first()).toHaveAttribute("data-state", "ok", { timeout: 30_000 });
  cpSync(join(RECORDINGS, "calls-a-tool"), join(backend.scratch, "agix", "recordings", sid), { recursive: true });
  return sid;
}

test("system-jump: the header's button opens the space's sessions in the system space, this one first", TREE, async ({ page }) => {
  const errors = watchConsole(page);
  const sid = await conversationWithTool(page);
  await page.getByTestId("head-in-system").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "system");
  const list = page.getByTestId("system-space");
  await expect(list).toHaveAttribute("data-space", "sessions");
  const first = list.locator(".srow").first();
  await expect(first).toHaveAttribute("data-sid", sid);
  await expect(first).toHaveAttribute("data-current", "true");
  // Every row is a session of that space, and one click inspects it.
  await first.click();
  await expect(page.getByTestId("system-session")).toContainText(sid);
  // And back: the session's overview lists its space's sessions again.
  await page.getByTestId("system-space-sessions").click();
  await expect(page.getByTestId("system-space")).toHaveAttribute("data-space", "sessions");
  expect(errors).toEqual([]);
});

test("system-jump: a tool row opens the call that asked for it, and the call finds its way back to the row", TREE, async ({ page }) => {
  const errors = watchConsole(page);
  const sid = await conversationWithTool(page);
  const row = page.locator('[data-testid="conv-tool"]').first();
  const tool = (await row.getAttribute("data-id"))!;
  await row.getByTestId("conv-tool-toggle").click();
  await row.getByTestId("tool-in-system").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "system");
  await expect(page.getByTestId("system-session")).toHaveCount(0);
  const picked = page.locator(`#details [data-tool="${tool}"]`);
  await expect(picked).toHaveClass(/\bon\b/);
  await expect(page.locator('#details [data-testid="system-call"].on')).toHaveCount(1);

  // From the call's detail, back to the conversation, the row found and marked.
  await page.locator('#details [data-testid="system-call"].on').click();
  await page.getByTestId("call-locate").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "sessions");
  await expect.poll(() => page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId)).toBe(sid);
  await expect(page.locator(`[data-testid="conv-tool"][data-id="${tool}"]`)).toHaveAttribute("data-located", "true", { timeout: 10_000 });
  expect(errors).toEqual([]);
});

test("system-context: measured apart from estimated, and a fork from any message", TREE, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const sid = await sessionWithTurn(page, "system");
  await toSystem(page);
  await pick(page, "system", sid);
  await tab(page, "context");
  const pane = page.getByTestId("system-context");
  // The bar is as long as what the provider counted; the estimates that divide it say they are.
  await expect(pane.getByTestId("context-stack")).toHaveAttribute("data-measured", /^[1-9]\d*$/);
  await expect(pane.locator(".legend2")).toContainText("~");

  // A fork that keeps up to the user's message: a new session, picked, its context that much of it.
  const user = pane.locator('.message[data-origin="user"]').first();
  const seq = Number(await user.getAttribute("data-seq"));
  const before = await pane.locator(".message").count();
  expect(before, "the original goes on past the user's message").toBeGreaterThan(seq + 1);
  const sessions = () => page.evaluate(async () => (await (window as any).harnix.app.call("harnix.sessions", { space: "system" })).sessions.length);
  const listed = await sessions();
  await user.getByTestId("message-fork").click();
  await expect(page.getByTestId("system-context").locator(".message")).toHaveCount(seq + 1);
  await expect.poll(sessions).toBe(listed + 1);
  expect(errors).toEqual([]);
});

test("system-timeline: calls on a time line, searched and narrowed by a stretch of time; a row per turn", TREE, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const sid = await sessionWithTurn(page, "system");
  cpSync(join(RECORDINGS, "spawns-agent"), join(backend.scratch, "agix", "recordings", sid), { recursive: true });
  await toSystem(page);
  await pick(page, "system", sid);
  await tab(page, "calls");
  const calls = page.getByTestId("system-call");
  await expect(calls.first()).toBeVisible();
  const all = await calls.count();
  const timeline = page.getByTestId("calls-timeline");
  await expect(timeline.getByTestId("timeline-call")).toHaveCount(all);
  await expect(calls.first().getByTestId("call-ttft")).toBeVisible();

  // Search: only the calls that say or call the Agent tool, fewer than all of them.
  await page.getByTestId("calls-search").fill("agent");
  await expect.poll(() => calls.count()).toBeLessThan(all);
  expect(await calls.count()).toBeGreaterThan(0);
  await page.getByTestId("calls-search").fill("no call says this");
  await expect(page.getByTestId("calls-none")).toBeVisible();
  await page.getByTestId("calls-search").fill("");
  await expect(calls).toHaveCount(all);

  // Drag across the first call's bar alone: the list is that call.
  const first = await timeline.locator('[data-call="0"]').boundingBox();
  const box = (await timeline.boundingBox())!;
  await page.mouse.move(first!.x + 1, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(first!.x + first!.width - 1, box.y + box.height / 2, { steps: 5 });
  await page.mouse.up();
  await expect(page.getByTestId("timeline-range")).toBeVisible();
  await expect(calls).toHaveCount(1);
  await expect(calls.first()).toHaveAttribute("data-call", "0");
  await page.getByTestId("timeline-clear").click();
  await expect(calls).toHaveCount(all);

  // Per turn: one row for the turn, opened into its calls; the context as a change or a total.
  await tab(page, "turns");
  const rows = page.getByTestId("turn-row");
  await expect(rows).toHaveCount(1);
  await rows.first().click();
  await expect(page.getByTestId("turn-step")).toHaveCount(2);
  await page.getByTestId("turns-total").click();
  await expect(page.getByTestId("turn-context").first()).not.toHaveText("—");
  expect(errors).toEqual([]);
});

test("system-calls: a failed call is one click away, and listed alone on asking", TREE, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const sid = await sessionWithTurn(page, "system");
  // `calls-a-tool`, its last call failed as an overloaded provider would have failed it.
  const dir = join(backend.scratch, "agix", "recordings", sid);
  cpSync(join(RECORDINGS, "calls-a-tool"), dir, { recursive: true });
  const lines = readFileSync(join(dir, "calls.jsonl"), "utf8").trimEnd().split("\n");
  const last = lines.map((line) => JSON.parse(line).type).lastIndexOf("end");
  lines[last] = JSON.stringify({ ...JSON.parse(lines[last]!), outcome: { status: "error", error: "overloaded" } });
  writeFileSync(join(dir, "calls.jsonl"), lines.join("\n") + "\n");

  await toSystem(page);
  await pick(page, "system", sid);
  await tab(page, "calls");
  const calls = page.getByTestId("system-call");
  await expect(calls).toHaveCount(2);
  await expect(calls.nth(1)).toHaveClass(/\bbad\b/);
  await expect(calls.nth(0)).not.toHaveClass(/\bbad\b/);

  await page.locator('#details [data-filter="failed"]').click();
  await expect(calls).toHaveCount(1);
  await expect(calls.first()).toHaveAttribute("data-call", "1");
  await page.locator('#details [data-filter="tools"]').click();
  await expect(calls).toHaveCount(1);
  await expect(calls.first()).toHaveAttribute("data-call", "0");
  await page.locator('#details [data-filter="all"]').click();
  await expect(calls).toHaveCount(2);

  await page.getByTestId("jump-to-failure").click();
  await expect(page.locator('#details [data-testid="system-call"][data-call="1"]')).toBeInViewport();
  expect(errors).toEqual([]);
});

test("system-turns: a turn's cache hits are said as a share, or not at all", TREE, async ({ page }) => {
  await open(page, backend.url);
  const sid = await sessionWithTurn(page, "system");
  cpSync(join(RECORDINGS, "calls-a-tool"), join(backend.scratch, "agix", "recordings", sid), { recursive: true });
  await toSystem(page);
  await pick(page, "system", sid);
  await tab(page, "turns");
  await expect(page.getByTestId("system-turns")).toBeVisible();
  await expect(page.getByTestId("turn-cache").first()).toHaveText(/^(\d+%|—)$/);
});

test("system-frames: narrowed to one kind of frame", TREE, async ({ page }) => {
  await open(page, backend.url);
  const sid = await sessionWithTurn(page, "system");
  await toSystem(page);
  await pick(page, "system", sid);
  await tab(page, "frames");
  await page.evaluate(
    (sid) => (window as any).harnix.app.call("harnix.send", { instance: "system", session_id: sid, space: "system", message: "Say exactly: fixture ready" }),
    sid,
  );
  const rows = page.locator("#details .frames tbody tr");
  await expect.poll(() => rows.count(), { timeout: 30_000 }).toBeGreaterThan(2);
  const kinds = page.getByTestId("frames-kind");
  const offered = await kinds.locator("option").evaluateAll((os) => os.map((o) => (o as HTMLOptionElement).value).filter(Boolean));
  expect(offered.length, "a turn's frames are of more than one kind").toBeGreaterThan(1);
  const all = await rows.count();
  await kinds.selectOption(offered[0]!);
  await expect.poll(() => rows.count()).toBeLessThan(all);
  for (const text of await rows.allTextContents()) expect(text).toContain(offered[0]!);
});

test("system-instances: an instance made from a template is listed and picked, and stopped again", TREE, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await toSystem(page);
  await page.locator('#side .system-tree .inst[data-instance="system"]').click();
  const form = page.getByTestId("new-instance");
  await expect(form).toBeVisible();
  await expect(page.getByTestId("new-instance-create"), "no name, nothing to make").toBeDisabled();
  await page.getByTestId("new-instance-name").fill("w-ui");
  // The template's own parameter (`team`, which its data root is made from).
  await form.locator(".field", { hasText: "team" }).locator("input").fill("ui");
  await page.getByTestId("new-instance-create").click();
  const made = page.locator('#side .system-tree .inst[data-instance="w-ui"]');
  await expect(made).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("system-instance")).toContainText("w-ui");
  await expect(made).toHaveAttribute("data-state", "ready", { timeout: 60_000 });

  // Stopping asks first; confirmed, the instance is gone and the system instance is picked.
  await page.getByTestId("inst-stop").click();
  await page.getByTestId("confirm").click();
  await expect(made).toHaveCount(0, { timeout: 30_000 });
  await expect(page.getByTestId("system-instance")).toContainText("system");
  expect(errors).toEqual([]);
});
