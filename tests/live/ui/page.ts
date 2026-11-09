/**
 * Driving the interface: open it, start a session that replays a fixture, type into the composer.
 *
 * Sessions are created from inside the page through `window.harnix.app` — the same `session.create`
 * the new-session dialog sends, with the fixture's name, which only means something because the
 * backend was started in replay.
 */

import type { Page } from "@playwright/test";
import { expect } from "@harnix/testkit";

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
  await expect(page.locator("#conn.on")).toBeVisible({ timeout: 15_000 });
}

/** Create a session replaying `fixture`, open it, and return its id. */
/** Without a project root the session is in the engine's own working directory — the test project. */
export async function newSession(page: Page, fixture: string, projectRoot?: string): Promise<string> {
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
  await expect(page.locator(".settings-nav")).toBeVisible();
}

/* ── the flows, in the system space ──────────────────────────────────── */

/** The system space's Flows tab, from wherever the page is: the flows, then every run. */
export async function toFlows(page: Page): Promise<void> {
  await page.locator('#side button[data-space="system"]').click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "system");
  await page.locator('[data-testid="system-console"] [data-tab="flows"]').click();
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
  await page.locator('#details [data-tab="flow.reference"]').click();
  await expect(page.getByTestId("flow-reference-tab")).toBeVisible();
}
