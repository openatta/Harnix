/**
 * Feedback on an answer, as a person gives it: the thumbs under a finished answer, the dialog that
 * asks what went wrong and takes a note, changing it and taking it back. What stands is read from
 * the session's log, so it is there after a refresh, and another page on the same session sees it
 * change; the export carries it.
 *
 * Replayed: `says-ready`.
 */

import { dropSessions, expect, test } from "@harnix/testkit";
import type { Page } from "@playwright/test";

import { connected, newSession, open, reply, say } from "./page";

const TAGS = { tag: ["@feature:sessions", "@layer:ui"] };

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});
test.afterEach(async ({ api }) => {
  await dropSessions(api);
});

/** A conversation with one replayed turn behind it, settled. */
async function oneTurn(page: Page): Promise<string> {
  const id = await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("conv-send")).toBeVisible();
  return id;
}

const thumb = (page: Page, rating: "up" | "down") => page.getByTestId(`conv-feedback-${rating}`).last();
const dialog = (page: Page) => page.getByTestId("conv-feedback-dialog");

async function reopen(page: Page, id: string): Promise<void> {
  await page.evaluate((sid) => (window as any).harnix.app.open({ instance: "main", sessionId: sid }), id);
  await expect(reply(page)).toBeVisible({ timeout: 15_000 });
}

test("a thumbs down asks what went wrong and takes a note; it stands after a refresh, and is changed and taken back", TAGS, async ({ page }) => {
  await open(page, URL);
  const id = await oneTurn(page);
  await page.getByTestId("conv-group").last().hover();
  await expect(thumb(page, "down")).toHaveAttribute("data-state", "off");

  await thumb(page, "down").click();
  await expect(dialog(page)).toBeVisible();
  await expect(dialog(page)).toHaveAttribute("data-rating", "down");
  await dialog(page).locator('[data-testid="conv-feedback-category"][data-id="inaccurate"]').click();
  await dialog(page).getByTestId("conv-feedback-note").fill("feedback-ui-marker");
  await dialog(page).getByTestId("conv-feedback-submit").click();
  await expect(dialog(page)).toHaveCount(0);
  await expect(thumb(page, "down")).toHaveAttribute("data-state", "on");
  // Given, the row stays without hovering.
  await page.mouse.move(0, 0);
  await expect(thumb(page, "down")).toBeVisible();

  await page.reload();
  await connected(page);
  await reopen(page, id);
  await expect(thumb(page, "down")).toHaveAttribute("data-state", "on");
  await thumb(page, "down").click();
  await expect(dialog(page)).toHaveAttribute("data-state", "rated");
  await expect(dialog(page).locator('[data-testid="conv-feedback-category"][data-id="inaccurate"]')).toHaveAttribute("aria-checked", "true");
  await expect(dialog(page).getByTestId("conv-feedback-note")).toHaveValue("feedback-ui-marker");
  await page.keyboard.press("Escape");
  await expect(dialog(page)).toHaveCount(0);

  // Changed to a thumbs up: no categories asked.
  await thumb(page, "up").click();
  await expect(dialog(page).getByTestId("conv-feedback-category")).toHaveCount(0);
  await dialog(page).getByTestId("conv-feedback-submit").click();
  await expect(thumb(page, "up")).toHaveAttribute("data-state", "on");
  await expect(thumb(page, "down")).toHaveAttribute("data-state", "off");

  // Taken back.
  await thumb(page, "up").click();
  await dialog(page).getByTestId("conv-feedback-retract").click();
  await expect(thumb(page, "up")).toHaveAttribute("data-state", "off");
  await page.reload();
  await connected(page);
  await reopen(page, id);
  await expect(thumb(page, "up")).toHaveAttribute("data-state", "off");
  await expect(thumb(page, "down")).toHaveAttribute("data-state", "off");
});

test("another page on the session sees the feedback change; the export has it under the answer", TAGS, async ({ page, context }) => {
  await open(page, URL);
  const id = await oneTurn(page);
  const other = await context.newPage();
  try {
    await open(other, URL);
    await reopen(other, id);
    await expect(thumb(other, "up")).toHaveAttribute("data-state", "off");

    await thumb(page, "up").click();
    await dialog(page).getByTestId("conv-feedback-note").fill("feedback-ui-shared");
    await dialog(page).getByTestId("conv-feedback-submit").click();
    await expect(thumb(other, "up")).toHaveAttribute("data-state", "on", { timeout: 15_000 });
  } finally {
    await other.close();
  }

  const md = await page.evaluate(
    (sid) => (window as any).harnix.app.call("harnix-app.sessions.export", { instance: "main", session_id: sid, space: "sessions", format: "markdown" }),
    id,
  );
  expect(md.content).toContain("> **Feedback**: up — feedback-ui-shared");
});
