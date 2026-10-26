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
      const params: Record<string, unknown> = { scene: "coding", options: { recorder: { name } } };
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
