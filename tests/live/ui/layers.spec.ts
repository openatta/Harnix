/**
 * The page's layers — dialogs, menus, the banner — driven the way a person drives them, against a
 * real backend with turns replayed.
 *
 * The default answer of an ask can be seen.
 */

import type { Page } from "@playwright/test";
import { expect, test } from "@harnix/testkit";

import { newSession, open, say } from "./page";

const SESSIONS = { tag: ["@feature:sessions", "@layer:ui"] };

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});

/** A session's turn waiting on its ask: the ask on the page, the turn running. */
async function asking(page: Page): Promise<void> {
  await newSession(page, "asks-permission");
  await say(page, "Use the Bash tool to run exactly: touch harnix-permission-fixture");
  await expect(page.getByTestId("conv-ask")).toBeVisible({ timeout: 30_000 });
}

test("an ask's default answer is drawn in the accent, not white on white", SESSIONS, async ({ page }) => {
  await open(page, URL);
  await asking(page);
  const ask = page.getByTestId("conv-ask");
  const colours = await ask.evaluate((card) => {
    const primary = card.querySelector<HTMLElement>('[data-testid="conv-ask-allow"]')!;
    // `--hx-accent` as the browser resolves it, here: an element filled with it.
    const probe = document.createElement("span");
    probe.style.background = "var(--hx-accent)";
    card.append(probe);
    const accent = getComputedStyle(probe).backgroundColor;
    probe.remove();
    const other = card.querySelector<HTMLElement>('[data-testid="conv-ask-allow-session"]')!;
    return { primary: getComputedStyle(primary).backgroundColor, text: getComputedStyle(primary).color, accent, other: getComputedStyle(other).backgroundColor };
  });
  expect(colours.primary).toBe(colours.accent);
  expect(colours.primary).not.toBe(colours.text);
  expect(colours.primary).not.toBe(colours.other);
  // Answered, so the turn does not stay running on the backend the next tests share.
  await ask.getByTestId("conv-ask-deny").click();
  await expect(ask).toHaveAttribute("data-state", "answered");
});
