/**
 * The cards beside the permission card, driven as a person drives them: the model's question
 * (choose an option, or say it in your own words), a plan's review (read the plan, ask for changes,
 * approve), and a question answered on another device.
 *
 * Replayed (`asks-a-question`, `reviews-a-plan`, made as `api/recorded.ts` says). Both tools are the
 * `code` scene's, so this backend activates it beside the default.
 */

import type { Page } from "@playwright/test";

import { type Backend, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { PLAN_FEEDBACK } from "../api/recorded";
import { open, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:sessions", "@layer:ui"] };

let backend: Backend;

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay", { profile: '\n[engine]\nscenes = ["code"]\n' }));
});
test.afterAll(async () => {
  await backend?.dispose();
});

/** A `code` session replaying `recording`, opened. */
async function codeSession(page: Page, recording: string, options: Record<string, unknown> = {}): Promise<string> {
  await page.evaluate(
    ({ name, options }) => (window as any).harnix.app.create({ scene: "code", options: { recorder: { name }, ...options } }),
    { name: recording, options },
  );
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  return page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId as string);
}

test("a model's question is answered by choosing an option, and the card says what was chosen", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await codeSession(page, "asks-a-question");
  await say(page, "Ask me which color to use.");
  const card = page.locator('[data-testid="conv-ask"][data-kind="question"]');
  await expect(card).toBeVisible({ timeout: 30_000 });
  await expect(card.getByTestId("conv-ask-option")).toHaveCount(2);
  await expect(card.getByTestId("conv-ask-submit")).toBeDisabled();
  // By keyboard: the options are a radio group.
  await card.locator('[data-testid="conv-ask-option"][data-id="r"] input').focus();
  await page.keyboard.press("ArrowDown");
  await expect(card.locator('[data-testid="conv-ask-option"][data-id="b"] input')).toBeChecked();
  await card.getByTestId("conv-ask-submit").click();
  await expect(card).toHaveAttribute("data-state", "answered");
  await expect(card).toHaveAttribute("data-answer", "replied");
  await expect(card).toContainText("blue");
  await expect(page.locator('[data-testid="conv-message"]', { hasText: /blue/i }).last()).toBeVisible({ timeout: 30_000 });
  expect(errors).toEqual([]);
});

test("a question can be answered in one's own words", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await codeSession(page, "asks-a-question");
  await say(page, "Ask me which color to use.");
  const card = page.locator('[data-testid="conv-ask"][data-kind="question"]');
  await expect(card).toBeVisible({ timeout: 30_000 });
  await card.getByTestId("conv-ask-other").click();
  const text = card.getByTestId("conv-ask-other-text");
  await text.fill("teal, please");
  await text.press("Enter");
  await expect(card).toHaveAttribute("data-answer", "replied");
  await expect(card).toContainText("teal, please");
});

test("a question answered on another device shows that answer here", TAGS, async ({ page }) => {
  await open(page, backend.url);
  const sid = await codeSession(page, "asks-a-question");
  const other = await connect(backend);
  try {
    await other.call("harnix.session.attach", { session_id: sid });
    await say(page, "Ask me which color to use.");
    const card = page.locator('[data-testid="conv-ask"][data-kind="question"]');
    await expect(card).toBeVisible({ timeout: 30_000 });
    const ask = await other.waitFor((f) => f.params?.session_id === sid && f.params?.event?.kind === "prompt", { describe: "the question" });
    await other.call("harnix.session.respond", { session_id: sid, prompt_id: ask.params.event.prompt_id, prompt_type: "question", answer: { type: "answer", text: "r" } });
    await expect(card).toHaveAttribute("data-answer", "replied");
    await expect(card).toContainText("red");
    await expect(card).toContainText(/别处|elsewhere/);
    await expect(card.locator("button")).toHaveCount(0);
  } finally {
    other.close();
  }
});

test("a plan is read as markdown, sent back with changes, and the next one approved", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await codeSession(page, "reviews-a-plan", { permission_mode: "plan" });
  await say(page, "Plan a file.");
  const first = page.locator('[data-testid="conv-ask"][data-kind="plan"]').first();
  await expect(first).toBeVisible({ timeout: 30_000 });
  // The plan is the reply before the call, drawn as markdown: its heading is a heading.
  await expect(first.getByTestId("conv-ask-plan").locator("h1, h2, h3").first()).toContainText(/plan/i);
  await first.getByTestId("conv-ask-plan-changes").click();
  await expect(first.getByTestId("conv-ask-plan-send")).toBeDisabled();
  await first.getByTestId("conv-ask-plan-feedback").fill(PLAN_FEEDBACK);
  await first.getByTestId("conv-ask-plan-send").click();
  await expect(first).toHaveAttribute("data-answer", "deny");
  await expect(first).toContainText(PLAN_FEEDBACK);

  const second = page.locator('[data-testid="conv-ask"][data-kind="plan"][data-state="pending"]');
  await expect(second).toBeVisible({ timeout: 30_000 });
  await second.getByTestId("conv-ask-plan-approve").click();
  await expect(page.locator('[data-testid="conv-ask"][data-kind="plan"]').last()).toHaveAttribute("data-answer", "allow");
  await expect(page.locator('[data-testid="conv-message"]', { hasText: /plan approved/i }).last()).toBeVisible({ timeout: 30_000 });
  expect(errors).toEqual([]);
});
