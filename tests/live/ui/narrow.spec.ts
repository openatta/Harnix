/**
 * A narrow window (Helm §1; review G2), at 900px and at 600px, against a real backend with turns
 * replayed.
 *
 * At 960px and below the sidebar is the rail: nothing of its column shows in it or spills out of
 * it, and its entry opens the front space's sessions as a drawer over the page. A session picked
 * there opens and the drawer closes; Escape and the veil close it too. An open right column lies
 * over the conversation from the right, which keeps its width, and Escape closes it. The person's
 * own toggle in a wide window is theirs: the rail does not undo it.
 */

import type { Page } from "@playwright/test";
import { dropSessions, expect, test } from "@harnix/testkit";

import { newSession, open, reply, say, sessionRow } from "./page";

const SHELL = { tag: ["@feature:shell", "@layer:ui"] };

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});
// The worker's backend is shared by every file that uses it: a test leaves no session behind.
test.afterEach(async ({ api }) => {
  await dropSessions(api);
});

const frame = (page: Page) => page.locator("#frame");
const row = sessionRow;
const current = (page: Page) => page.evaluate(() => ((window as any).harnix.app.snapshot.current?.sessionId as string | undefined) ?? null);

/** What of the rail is drawn outside it, and the text it shows: neither should there be. */
function leaks(page: Page) {
  return page.evaluate(() => {
    const side = document.getElementById("side")!;
    const edge = side.getBoundingClientRect().right;
    const out = [...side.querySelectorAll<HTMLElement>("*")]
      .filter((el) => el.checkVisibility() && el.getBoundingClientRect().width > 0 && el.getBoundingClientRect().right > edge + 1)
      .map((el) => `${el.tagName.toLowerCase()}.${[...el.classList].join(".")}`);
    // What a reader sees in it, the avatar's initials aside.
    const avatar = side.querySelector(".avatar")?.textContent ?? "";
    const text = side.innerText.replace(avatar, "").trim();
    return { width: Math.round(side.getBoundingClientRect().width), out, text };
  });
}

/** Two sessions that have said something, the second one open. */
async function twoSessions(page: Page): Promise<[string, string]> {
  const first = await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  const second = await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  return [first, second];
}

for (const width of [900, 600]) {
  test.describe(`${width}px wide`, () => {
    test.use({ viewport: { width, height: 760 } });

    test("the sidebar is a rail, and nothing of its column shows in it or spills out of it", SHELL, async ({ page }) => {
      await open(page, URL);
      await twoSessions(page);
      await expect(frame(page)).toHaveAttribute("data-sidebar", "collapsed");
      await expect(page.getByTestId("rail-sessions")).toBeVisible();
      await expect(page.locator('[data-testid="session-row"]').first()).toBeHidden();
      const seen = await leaks(page);
      expect(seen.width).toBeLessThanOrEqual(57);
      expect(seen.out).toEqual([]);
      expect(seen.text).toBe("");
      // The conversation keeps the width, with 16px gutters: centred while it is wider than its column
      // (720px), at the gutter once it is not (the old 24px clearance put it at 48).
      const { gutter, room } = await page.getByTestId("conv-input").evaluate((input) => {
        const main = document.getElementById("main")!.getBoundingClientRect();
        return { gutter: Math.round(input.closest('[data-testid="conv-composer"]')!.getBoundingClientRect().left - main.left), room: main.width };
      });
      expect(gutter).toBeLessThanOrEqual(Math.max(32, (room - 720) / 2 + 8));
    });

    test("the rail's entry opens the sessions in a drawer; one picked there opens, and the drawer closes", SHELL, async ({ page }) => {
      await open(page, URL);
      const [first, second] = await twoSessions(page);
      expect(await current(page)).toBe(second);
      await page.getByTestId("rail-sessions").click();
      await expect(frame(page)).toHaveAttribute("data-drawer", "");
      await expect(frame(page)).toHaveAttribute("data-sidebar", "expanded");
      await expect(page.getByTestId("drawer-veil")).toBeVisible();
      // Over the page, not beside it: the conversation keeps its place under the veil.
      const side = await page.locator("#side").boundingBox();
      expect(side!.width).toBeGreaterThan(200);
      await expect(row(page, first)).toBeVisible();
      await row(page, first).click();
      await expect.poll(() => current(page)).toBe(first);
      await expect(frame(page)).not.toHaveAttribute("data-drawer", "");
      await expect(frame(page)).toHaveAttribute("data-sidebar", "collapsed");
      await expect(page.getByTestId("drawer-veil")).toHaveCount(0);

      // Escape closes it, and so does the veil.
      await page.getByTestId("rail-sessions").click();
      await expect(frame(page)).toHaveAttribute("data-drawer", "");
      await page.keyboard.press("Escape");
      await expect(frame(page)).not.toHaveAttribute("data-drawer", "");
      await page.getByTestId("sidebar-toggle").click();
      await expect(frame(page)).toHaveAttribute("data-drawer", "");
      await page.getByTestId("drawer-veil").click({ position: { x: width - 20, y: 400 } });
      await expect(frame(page)).not.toHaveAttribute("data-drawer", "");
    });

    test("an open right column lies over the conversation from the right, and Escape closes it", SHELL, async ({ page }) => {
      await open(page, URL);
      await newSession(page, "says-ready");
      await page.getByTestId("details-toggle").click();
      await expect(frame(page)).toHaveAttribute("data-details", "open");
      const details = await page.locator("#details").boundingBox();
      const main = await page.locator("#main").boundingBox();
      // The conversation is not pushed aside or hidden: the column is over it, at the right edge.
      expect(Math.round(details!.x + details!.width)).toBe(width);
      expect(main!.width).toBeGreaterThan(width - 60);
      await expect(page.locator("#main")).toBeVisible();
      await expect(page.getByTestId("drawer-veil")).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(frame(page)).toHaveAttribute("data-details", "closed");
      await expect(page.getByTestId("drawer-veil")).toHaveCount(0);
    });
  });
}

test("a sidebar collapsed in a wide window stays collapsed through a narrow one", SHELL, async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await open(page, URL);
  await expect(frame(page)).toHaveAttribute("data-sidebar", "expanded");
  await page.getByTestId("sidebar-toggle").click();
  await expect(frame(page)).toHaveAttribute("data-sidebar", "collapsed");
  await page.setViewportSize({ width: 800, height: 760 });
  await expect(frame(page)).toHaveAttribute("data-sidebar", "collapsed");
  await page.setViewportSize({ width: 1280, height: 760 });
  await expect(frame(page)).toHaveAttribute("data-sidebar", "collapsed");
  // And one expanded comes back expanded after the rail.
  await page.getByTestId("sidebar-toggle").click();
  await page.setViewportSize({ width: 800, height: 760 });
  await expect(frame(page)).toHaveAttribute("data-sidebar", "collapsed");
  await page.setViewportSize({ width: 1280, height: 760 });
  await expect(frame(page)).toHaveAttribute("data-sidebar", "expanded");
});
