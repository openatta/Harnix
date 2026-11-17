/**
 * Driving the interface: open it, start a session that replays a fixture, type into the composer.
 *
 * Sessions are created from inside the page through `window.harnix.app` — the same `harnix.session.create`
 * the new-session dialog sends, with the fixture's name, which only means something because the
 * backend was started in replay.
 *
 * Elements are found by what they are, not by how they are drawn: a `data-testid`, a role, a data
 * attribute. A state is read from `data-state` (or ARIA), never from a style class such as `.on`:
 * restyling the shell must not break a test whose behaviour did not change.
 */

import { cpSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import type { Locator, Page } from "@playwright/test";
import { type Backend, RECORDINGS, connect, expect } from "@harnix/testkit";

/** Console errors are failures: a page that throws while still looking right is the failure a
 *  screenshot cannot show. */
export function watchConsole(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() !== "error") return;
    // A backend serving split streams only refuses the duplex socket (409), and the browser logs the
    // refusal itself on the way to the fallback. That is the designed path, not a fault.
    if (/WebSocket connection to .* failed: .*409/.test(m.text())) return;
    errors.push(m.text());
  });
  return errors;
}

export async function open(page: Page, url: string): Promise<void> {
  await page.goto(url);
  await connected(page);
}

/** The page is connected: the avatar says so (`data-state` is the client's status). */
export async function connected(page: Page): Promise<void> {
  await expect(page.locator('[data-testid="avatar"][data-state="open"]')).toBeVisible({ timeout: 15_000 });
}

/* ── the shell's parts, found by name ────────────────────────────────── */

/** A space's entry in the left column (the top list or the bottom), by the space's id. */
export const spaceEntry = (page: Page, space: string): Locator => page.locator(`[data-testid="space-entry"][data-space="${space}"]`);

/** A session's row in the left column; the open one has `data-state="current"`. */
export const sessionRow = (page: Page, sessionId: string): Locator => page.locator(`[data-testid="session-row"][data-sid="${sessionId}"]`);

/** A section's entry in the settings' navigation; the shown one has `data-state="current"`. */
export const settingsSection = (page: Page, section: string): Locator => page.getByTestId("settings-nav").locator(`[data-section="${section}"]`);

/** A tab of a tab strip (`role="tab"`), by its `data-tab`. */
export const tab = (scope: Page | Locator, id: string): Locator => scope.locator(`[role="tab"][data-tab="${id}"]`);

/**
 * The shell has started: it has landed on a space whose contribution is in. Connected is not
 * started — until then a session would be made for the space it was in before (none, on a fresh
 * page). Where a space says it is the landing (the home page), the landing has been looked at too:
 * the tab keeps that it landed (`harnix.landed`), so a switch made before it is not undone by it.
 */
export async function started(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const { app } = (window as unknown as { harnix: { app: any } }).harnix;
    const space = app.snapshot.space as string;
    const spaces = app.registry.at("space") as { id: string; landing?: boolean }[];
    if (space === "" || !spaces.some((c) => c.id === space)) return false;
    return !spaces.some((c) => c.landing) || sessionStorage.getItem("harnix.landed") !== null;
  }, undefined, { timeout: 15_000 });
}

/** The space in front, by its entry in the left column. */
export async function toSpace(page: Page, space: string): Promise<void> {
  await started(page);
  if ((await page.locator("#main").getAttribute("data-space")) !== space) await spaceEntry(page, space).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", space);
}

/** The session space in front: where a page opened afresh is not (it lands on the home page). */
export const toSessions = (page: Page): Promise<void> => toSpace(page, "sessions");

/**
 * Create a session replaying `fixture`, open it, and return its id. In the space in front — unless
 * that space holds no conversation (the home page, where a page lands): then in the session space.
 * Without a project root the session is in the engine's own working directory — the test project.
 */
export async function newSession(page: Page, fixture: string, projectRoot?: string): Promise<string> {
  await started(page);
  const talks = await page.evaluate(() => {
    const { app } = (window as unknown as { harnix: { app: any } }).harnix;
    const space = app.registry.at("space").find((c: { id: string }) => c.id === app.snapshot.space);
    return space?.center?.mode !== "hidden";
  });
  if (!talks) await toSessions(page);
  const id = await page.evaluate(
    async ({ name, root }) => {
      const { app } = (window as unknown as { harnix: { app: any } }).harnix;
      const params: Record<string, unknown> = { scene: "harnix", options: { recorder: { name } } };
      if (root) params.project_root = root;
      try {
        await app.create(params);
      } catch (e) {
        throw new Error(JSON.stringify(e));
      }
      return app.snapshot.current.sessionId as string;
    },
    { name: fixture, root: projectRoot },
  );
  expect(id, "no session was created").toBeTruthy();
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  return id;
}

/** Type into the composer and press Enter, as a person would. */
export async function say(page: Page, text: string): Promise<void> {
  const input = page.getByTestId("conv-input");
  await input.click();
  await input.fill(text);
  await input.press("Enter");
}

/** The assistant's reply in the flow, once it has some text. */
export function reply(page: Page) {
  return page.locator('[data-testid="conv-group"] [data-testid="conv-message"]').first();
}

/** Settings: the avatar's menu, then its first item. */
export async function openSettings(page: Page): Promise<void> {
  await page.getByTestId("avatar").click();
  await page.getByTestId("open-settings").click();
  await expect(page.getByTestId("settings-nav")).toBeVisible();
}

/**
 * Settings → 诊断 (harnix-app.md §12), from wherever the page is, and a block another package put
 * after the shell's own content (`settings.section` `within: "diagnostics"`), by its id.
 */
export async function diagnosticsBlock(page: Page, section: string): Promise<Locator> {
  if (!(await page.getByTestId("settings-nav").isVisible())) await openSettings(page);
  await settingsSection(page, "diagnostics").click();
  await expect(settingsSection(page, "diagnostics")).toHaveAttribute("data-state", "current");
  const block = page.locator(`[data-testid="settings-within"][data-section="${section}"]`);
  await expect(block).toBeVisible();
  return block;
}

/* ── the flow space ───────────────────────────────────────────────────── */

/** A tab of the right column in front, the column opened first if it is closed. */
export async function detailsTab(page: Page, id: string): Promise<void> {
  if ((await page.locator("#frame").getAttribute("data-details")) !== "open") await page.getByTestId("details-toggle").click();
  await tab(page.locator("#details"), id).click();
  await expect(tab(page.locator("#details"), id)).toHaveAttribute("aria-selected", "true");
}

/** The flow space (harnix-app.md §11), from wherever the page is, its right column on the flows. */
export async function toFlows(page: Page): Promise<void> {
  await toSpace(page, "flow");
  await detailsTab(page, "flow.flows");
  await expect(page.locator('#details [data-testid="flow-flows"]')).toBeVisible();
}

/** The flow space, its right column on every run and dry run. */
export async function toRuns(page: Page): Promise<void> {
  await toSpace(page, "flow");
  await detailsTab(page, "flow.runs");
  await expect(page.locator('#details [data-testid="flow-runs"]')).toBeVisible();
}

/** Into a flow from the flows list (or still in it): the left column on it, its details beside its conversation. */
export async function openFlow(page: Page, flow: string): Promise<void> {
  await toFlows(page);
  // A click on the row already selected lets go of it.
  const row = page.locator(`#details [data-testid="flow-def-${flow}"]`);
  if ((await row.getAttribute("aria-current")) !== "true") await row.click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", flow);
  await expect(page.locator('#secondary [data-testid="flow-detail"]')).toHaveAttribute("data-flow", flow);
}

/** Into a run from the runs list, the run shown beside its main session. */
export async function openRun(page: Page, run: string): Promise<void> {
  await toRuns(page);
  const row = page.locator(`#details [data-testid="flow-run-${run}"]`);
  if ((await row.getAttribute("aria-current")) !== "true") await row.click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  await expect(page.locator('#secondary [data-testid="flow-run-detail"]')).toHaveAttribute("data-run", run);
}

/** The reference, the right column's third tab inside the flow space. */
export async function flowReference(page: Page): Promise<void> {
  await detailsTab(page, "flow.reference");
  await expect(page.getByTestId("flow-reference-tab")).toBeVisible();
}

/**
 * A session with a recording, for what reads one. A replayed session records nothing, and the
 * readers read a session's recording through its engine (testing.md §6): so `backend` is one
 * whose sessions record (`launchFor("none")`: nothing to replay and no model, so nothing on it runs
 * a turn). The session, on `main` in the session space, has run no turn; its recording is `lines`,
 * or the fixture `fixture` with the id it was recorded under replaced by the session's, put where
 * `main`'s engine records it. Nothing is written over it: a recording is rewritten by the session's
 * first call, and this session makes none. Opens the page on `backend`, the session space in front.
 *
 * The session is made, and its recording written, with the page away: the engine's reader keeps
 * what it read until the session shows activity (`crates/engine/src/activity.rs`), and writing a
 * recording behind the engine's back is none — so a page that read the session while the file was
 * half written (the session's row reads its usage as soon as it is listed) would go on seeing that.
 */
export async function recorded(
  page: Page,
  backend: Backend,
  source: { fixture: string; edit?: (lines: string[]) => string[] } | { lines: (sid: string) => object[] },
): Promise<string> {
  await page.goto("about:blank");
  const api = await connect(backend);
  let sid: string;
  try {
    const created = await api.call("harnix.session.create", { instance: "main", space: "sessions", scene: "harnix" });
    sid = created.session_id as string;
    await api.call("harnix.session.attach", { instance: "main", session_id: sid, space: "sessions" });
  } finally {
    api.close();
  }
  const dir = join(backend.scratch, "agix", "recordings", sid);
  if ("fixture" in source) {
    cpSync(join(RECORDINGS, source.fixture), dir, { recursive: true });
    const text = readFileSync(join(dir, "calls.jsonl"), "utf8");
    const was = (JSON.parse(text.slice(0, text.indexOf("\n"))) as { session_id: string }).session_id;
    const lines = text.split(was).join(sid).trimEnd().split("\n");
    writeFileSync(join(dir, "calls.jsonl"), (source.edit ? source.edit(lines) : lines).join("\n") + "\n");
  } else {
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, "calls.jsonl"), source.lines(sid).map((l) => JSON.stringify(l)).join("\n") + "\n");
  }
  await open(page, backend.url);
  await toSessions(page);
  return sid;
}

/**
 * The page's Mod key: its shortcuts follow the platform the page reports (`shortcuts.platform`),
 * which the emulated device sets, not the machine Playwright runs on — Desktop Chrome reports
 * Windows on a Mac host, where `ControlOrMeta` would press ⌘.
 */
export async function modKey(page: Page): Promise<"Meta" | "Control"> {
  return (await page.evaluate(() => (window as any).harnix.app.shortcuts.platform)) === "mac" ? "Meta" : "Control";
}
