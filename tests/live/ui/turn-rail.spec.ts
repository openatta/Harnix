/**
 * The turn rail on the conversation's right edge: a tick per turn once there are two, a preview on pointing
 * (the prompt on one line, the reply on three), a click that lands on the turn, ⌥↑ from anywhere but a field
 * with text in it, and no rail where the conversation is narrow.
 *
 * Turns are replayed from `many-turns` (`says-ready`'s one call sixty times over): every reply is "fixture
 * ready".
 */

import type { Page } from "@playwright/test";
import { dropSessions, expect, test } from "@harnix/testkit";

import { newSession, open, say } from "./page";

const TAGS = { tag: ["@feature:sessions", "@layer:ui"] };

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});
test.afterEach(async ({ api }) => {
  await dropSessions(api);
});

/** A conversation of `turns` settled turns, the n-th asking "Turn n, please." */
async function turns(page: Page, count: number): Promise<void> {
  await open(page, URL);
  await newSession(page, "many-turns");
  const replies = page.locator('[data-testid="conv-message"]', { hasText: /fixture ready/i });
  for (let i = 0; i < count; i += 1) {
    await say(page, `Turn ${i}, please.`);
    await expect(replies).toHaveCount(i + 1, { timeout: 20_000 });
    await expect(page.getByTestId("conv-send")).toBeVisible();
  }
}

const ticks = (page: Page) => page.getByTestId("conv-turnrail-tick");

test("a tick per turn, the last one current, a preview on pointing, and a click lands on the turn", TAGS, async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await open(page, URL);
  await newSession(page, "many-turns");
  await say(page, "Turn 0, please.");
  await expect(page.locator('[data-testid="conv-message"]', { hasText: /fixture ready/i })).toHaveCount(1, { timeout: 20_000 });
  // One turn: nothing to go between.
  await expect(page.getByTestId("conv-turnrail")).toHaveCount(0);

  // Enough that the transcript scrolls a turn near the start up to its top.
  for (let i = 1; i < 10; i += 1) {
    await say(page, `Turn ${i}, please.`);
    await expect(page.locator('[data-testid="conv-message"]', { hasText: /fixture ready/i })).toHaveCount(i + 1, { timeout: 20_000 });
    await expect(page.getByTestId("conv-send")).toBeVisible();
  }
  const rail = page.getByTestId("conv-turnrail");
  await expect(rail).toBeVisible();
  await expect(rail).toHaveAttribute("aria-label", /轮次|Turns/);
  await expect(ticks(page)).toHaveCount(10);
  await expect(ticks(page).last()).toHaveAttribute("aria-current", "true");

  await ticks(page).nth(1).hover();
  const preview = page.getByTestId("conv-turnrail-preview");
  await expect(preview).toBeVisible();
  await expect(page.getByTestId("conv-turnrail-prompt")).toHaveText("Turn 1, please.");
  await expect(page.getByTestId("conv-turnrail-reply")).toContainText(/fixture ready/i);

  await ticks(page).nth(1).click();
  const anchor = page.locator("[data-turn-anchor]").nth(1);
  await expect(anchor).toContainText("Turn 1, please.");
  await expect.poll(async () => {
    const flow = await page.getByTestId("conv-flow").boundingBox();
    const box = await anchor.locator(":scope > *").first().boundingBox();
    return flow && box ? Math.abs(box.y - flow.y) : Infinity;
  }).toBeLessThan(60);
  await expect(ticks(page).nth(1)).toHaveAttribute("aria-current", "true");
});

test("⌥↑ goes to the turn before the one being read, and a composer with text in it keeps the key", TAGS, async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await turns(page, 10);
  await expect(ticks(page)).toHaveCount(10);
  // From the transcript, the focus in no field.
  await page.getByTestId("conv-flow").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Alt+ArrowUp");
  await expect(ticks(page).nth(8)).toHaveAttribute("aria-current", "true");
  await page.keyboard.press("Alt+ArrowUp");
  await expect(ticks(page).nth(7)).toHaveAttribute("aria-current", "true");
  await expect(page.locator("[data-turn-anchor]").nth(7)).toBeInViewport();
  await page.keyboard.press("Alt+ArrowDown");
  await expect(ticks(page).nth(8)).toHaveAttribute("aria-current", "true");

  const input = page.getByTestId("conv-input");
  await input.fill("line one\nline two");
  await input.press("Alt+ArrowUp");
  await expect(ticks(page).nth(8)).toHaveAttribute("aria-current", "true");
  await expect(input).toHaveValue("line one\nline two");
});

test("the rail is a list of labelled ticks the arrows move along, and is not drawn where the conversation is narrow", TAGS, async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await turns(page, 3);
  const last = ticks(page).last();
  await expect(last).toHaveAttribute("aria-label", /Turn 2, please\./);
  await last.focus();
  await page.keyboard.press("ArrowUp");
  await expect(ticks(page).nth(1)).toBeFocused();
  await expect(page.getByTestId("conv-turnrail-prompt")).toHaveText("Turn 1, please.");
  await page.keyboard.press("Enter");
  await expect(ticks(page).nth(1)).toHaveAttribute("aria-current", "true");

  await page.setViewportSize({ width: 500, height: 760 });
  await expect(page.getByTestId("conv-turnrail")).toBeHidden();
});
