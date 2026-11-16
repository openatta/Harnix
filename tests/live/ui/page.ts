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
import { type Backend, RECORDINGS, expect } from "@harnix/testkit";

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

/** Create a session replaying `fixture`, open it, and return its id. */
/** Without a project root the session is in the engine's own working directory — the test project. */
export async function newSession(page: Page, fixture: string, projectRoot?: string): Promise<string> {
  // Connected is not started: until the shell has landed on a space whose contribution is in, a
  // session would be made for the space it was in before (none, on a fresh page) and opened
  // there, and the space it lands on has no current one.
  await page.waitForFunction(() => {
    const { app } = (window as unknown as { harnix: { app: any } }).harnix;
    const space = app.snapshot.space as string;
    return space !== "" && app.registry.at("space").some((c: { id: string }) => c.id === space);
  }, undefined, { timeout: 15_000 });
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

/* ── the flows, in the system space ──────────────────────────────────── */

/** The system space's Flows tab, from wherever the page is: the flows, then every run. */
export async function toFlows(page: Page): Promise<void> {
  await spaceEntry(page, "system").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "system");
  await tab(page.getByTestId("system-console"), "flow.flows").click();
  await expect(page.getByTestId("system-flows")).toBeVisible();
}

/** Into a flow from the Flows tab: the left column its context, its details beside its conversation. */
export async function openFlow(page: Page, flow: string): Promise<void> {
  await toFlows(page);
  await page.getByTestId(`flow-def-${flow}`).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", flow);
  await expect(page.locator('#secondary [data-testid="flow-detail"]')).toHaveAttribute("data-flow", flow);
}

/** Into a run's flow from the Flows tab, the run shown beside its main session. */
export async function openRun(page: Page, run: string): Promise<void> {
  await toFlows(page);
  await page.getByTestId(`flow-run-${run}`).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "flow");
  await expect(page.locator('#secondary [data-testid="flow-run-detail"]')).toHaveAttribute("data-run", run);
}

/** The reference, the right column's one tab inside a flow. */
export async function flowReference(page: Page): Promise<void> {
  if ((await page.locator("#frame").getAttribute("data-details")) !== "open") await page.getByTestId("details-toggle").click();
  await tab(page.locator("#details"), "flow.reference").click();
  await expect(page.getByTestId("flow-reference-tab")).toBeVisible();
}

/**
 * A session with a recording, for what reads one. A replayed session records nothing, and the
 * readers read a session's recording through its engine (system-space.md §8): so `backend` is one
 * whose sessions record (`launchFor("none")`: nothing to replay and no model, so nothing on it runs
 * a turn). The session, on `main` in the session space, has run no turn; its recording is `lines`,
 * or the fixture `fixture` with the id it was recorded under replaced by the session's, put where
 * `main`'s engine records it. Nothing is written over it: a recording is rewritten by the session's
 * first call, and this session makes none. Opens the page on `backend`.
 */
export async function recorded(
  page: Page,
  backend: Backend,
  source: { fixture: string; edit?: (lines: string[]) => string[] } | { lines: (sid: string) => object[] },
): Promise<string> {
  await open(page, backend.url);
  const sid = await page.evaluate(async () => {
    const { app } = (window as unknown as { harnix: { app: any } }).harnix;
    const created = await app.call("harnix.session.create", { instance: "main", space: "sessions", scene: "harnix" });
    await app.call("harnix.session.attach", { instance: "main", session_id: created.session_id, space: "sessions" });
    return created.session_id as string;
  });
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
