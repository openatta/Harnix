/**
 * A person's message changed from its menu (harnix-app.md §5.1), in a real browser against a real
 * backend: 「改写这条消息」 rewrites it in place (`observability.messages.edit`) and 「从上下文中去掉」
 * takes it out of the context (`observability.messages.drop`), each found by its turn and changed
 * against the count of messages the history had. The engine sends nothing for it; the hub has the
 * conversation read again, so what is drawn is what the engine now holds, before and after a
 * reload. While a turn runs, both are offered greyed.
 *
 * Replayed: `says-ready`, `runs-a-while`.
 */

import type { Page } from "@playwright/test";
import { dropSessions, expect, test } from "@harnix/testkit";

import { connected, newSession, open, reply, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:sessions", "@layer:ui"] };
const ASKED = "Say exactly: fixture ready";

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});
test.afterEach(async ({ api }) => {
  await dropSessions(api);
});

const said = (page: Page) => page.locator('[data-testid="conv-message"][data-role="user"]').first();

/** The person's message, its menu opened. */
async function messageMenu(page: Page) {
  const bubble = said(page).locator("xpath=..");
  await bubble.hover();
  await bubble.getByTestId("conv-message-menu").click();
  const menu = page.getByTestId("message-menu");
  await expect(menu).toBeVisible();
  return menu;
}

/** A session with one turn over: the person's message, and the reply. */
async function answered(page: Page): Promise<string> {
  await open(page, URL);
  const sid = await newSession(page, "says-ready");
  await say(page, ASKED);
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  await expect(page.getByTestId("conv-send")).toBeVisible({ timeout: 30_000 });
  return sid;
}

/** What the engine's history holds as the person's first message, as text. */
async function firstSaid(page: Page, sid: string): Promise<string> {
  return page.evaluate(async (sid) => {
    const { app } = (window as any).harnix;
    const history = await app.call("engine.session.history", { instance: "main", session_id: sid, space: "sessions" });
    const message = history.messages.find((m: any) => m.role === "user");
    const content = message?.content;
    return typeof content === "string" ? content : (content ?? []).map((b: any) => b.text ?? "").join("");
  }, sid);
}

test("a person's message is rewritten from its menu: the conversation and the history both say the new words", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  const sid = await answered(page);
  const menu = await messageMenu(page);
  const item = menu.locator('button[data-item="action:observability.message.edit"]');
  await expect(item).toBeEnabled();
  await item.click();
  const text = page.getByTestId("message-edit-text");
  await expect(text).toHaveValue(ASKED);
  // The same words change nothing.
  await expect(page.getByTestId("message-edit-save")).toBeDisabled();
  await text.fill("Say exactly: rewritten here");
  await page.getByTestId("message-edit-save").click();
  await expect(page.locator("#modal")).toHaveCount(0);
  // Drawn again from what the engine holds now.
  await expect(said(page)).toContainText("rewritten here", { timeout: 15_000 });
  expect(await firstSaid(page, sid)).toContain("rewritten here");
  // A reload draws the same.
  await page.reload();
  await connected(page);
  await expect(said(page)).toContainText("rewritten here", { timeout: 15_000 });
  expect(errors).toEqual([]);
});

test("a person's message is taken out of the context once confirmed: it stays in the conversation, marked removed", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  const sid = await answered(page);
  let menu = await messageMenu(page);
  await menu.locator('button[data-item="action:observability.message.drop"]').click();
  // Asked first: declined, nothing changes.
  await page.getByTestId("confirm-cancel").click();
  await expect(page.locator("#modal")).toHaveCount(0);
  expect(await firstSaid(page, sid)).toContain("fixture ready");
  menu = await messageMenu(page);
  await menu.locator('button[data-item="action:observability.message.drop"]').click();
  await expect(page.getByTestId("confirm")).toHaveText(/Take it out|去掉/);
  await page.getByTestId("confirm").click();
  await expect(said(page)).toContainText("removed by the host", { timeout: 15_000 });
  expect(await firstSaid(page, sid)).toContain("removed by the host");
  // The reply is as it was.
  await expect(reply(page)).toContainText(/fixture ready/i);
  expect(errors).toEqual([]);
});

test("while a turn runs, rewriting and taking out are offered greyed, with the reason", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "runs-a-while");
  await say(page, "Use the Bash tool to wait a few seconds");
  await expect(page.getByTestId("conv-ask")).toBeVisible({ timeout: 15_000 });
  const menu = await messageMenu(page);
  for (const id of ["observability.message.edit", "observability.message.drop"]) {
    await expect(menu.locator(`button[data-item="action:${id}"]`)).toBeDisabled();
  }
  await expect(menu).toContainText(/Not while a turn runs or waits|有一轮在跑或在排队时不能改/);
  await page.keyboard.press("Escape");
  await page.getByTestId("conv-ask").getByTestId("conv-ask-allow").click();
});
