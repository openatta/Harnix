/**
 * The rest of the conversation, driven as a person drives it: the ask's "allow here" answer, the
 * attach menu, the context ring, copying a reply, the thinking row, markdown in a reply, an image
 * sent along, a banner and its dismissal.
 *
 * Replayed: `says-ready`, `says-markdown` (a reply with a code block, a table and a link),
 * `asks-permission`.
 */

import { cpSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { dropSessions, expect, RECORDINGS, ROOT, test } from "@harnix/testkit";

import { newSession, open, reply, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:sessions", "@layer:ui"] };

let URL = "";
let SCRATCH = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
  SCRATCH = harnix.scratch;
});
// The worker's backend is shared by every file that uses it: a test leaves no session behind
// for the next one to count.
test.afterEach(async ({ api }) => {
  await dropSessions(api);
});

/** A conversation with one replayed turn behind it. */
async function oneTurn(page: import("@playwright/test").Page, fixture = "says-ready"): Promise<string> {
  const id = await newSession(page, fixture);
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("conv-send")).toBeVisible();
  return id;
}

test("allowing an ask for this project answers it, and the call runs", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "asks-permission");
  await say(page, "Use the Bash tool to run exactly: touch harnix-permission-fixture");
  const ask = page.getByTestId("conv-ask");
  await expect(ask).toBeVisible({ timeout: 30_000 });
  await ask.getByTestId("conv-ask-allow-local").click();
  await expect(ask).toHaveAttribute("data-state", "answered");
  await expect(page.locator('[data-testid="conv-tool"]').first()).toHaveAttribute("data-state", "ok", { timeout: 30_000 });
});

test("the attach menu picks a file, which can be taken off again, and puts an @ to mention one", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "says-ready");
  await page.getByTestId("conv-attach").click();
  await expect(page.getByTestId("conv-attach-menu")).toBeVisible();
  const [chooser] = await Promise.all([page.waitForEvent("filechooser"), page.getByTestId("conv-attach-file").click()]);
  await chooser.setFiles({ name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("attached for the test") });
  const chip = page.getByTestId("conv-attachment");
  await expect(chip).toContainText("notes.txt", { timeout: 15_000 });
  await chip.getByTestId("conv-attachment-remove").click();
  await expect(page.getByTestId("conv-attachment")).toHaveCount(0);

  await page.getByTestId("conv-attach").click();
  await page.getByTestId("conv-attach-mention").click();
  await expect(page.getByTestId("conv-input")).toHaveValue(/@$/);
  await expect(page.getByTestId("conv-menu")).toBeVisible();
});

test("the context ring says how full the context is, and opens to the numbers", TAGS, async ({ page }) => {
  await open(page, URL);
  await oneTurn(page);
  const ring = page.getByTestId("conv-context-ring");
  await expect(ring).toBeVisible({ timeout: 15_000 });
  await ring.click();
  const menu = page.getByTestId("conv-context-menu");
  await expect(menu).toBeVisible();
  await expect(menu).toContainText(/\d/);
  await expect(menu.getByTestId("conv-context-note")).toContainText(/\d+%/);
});

test("a reply is copied whole; regenerating is shown but not offered", TAGS, async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await open(page, URL);
  await oneTurn(page);
  await page.getByTestId("conv-copy").last().click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toMatch(/fixture ready/i);
  await expect(page.getByTestId("conv-retry").last()).toBeDisabled();
});

test("what the model thought is a row of its own, opened on a click", TAGS, async ({ page }) => {
  await open(page, URL);
  await oneTurn(page);
  const think = page.getByTestId("conv-think").first();
  await expect(think).toHaveAttribute("data-state", "done");
  await expect(think).toHaveAttribute("aria-expanded", "false");
  await think.click();
  await expect(think).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator(".hx-cv-thtext").first()).toContainText(/fixture ready/i);
});

test("a reply's markdown is drawn: a code block, a table, a link", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await oneTurn(page, "says-markdown");
  const answer = page.locator('[data-testid="conv-message"]', { hasText: "Here it is" });
  await expect(answer.locator("pre code")).toContainText("echo fixture ready");
  await expect(answer.locator("table td").first()).toHaveText("answer");
  await expect(answer.locator("a[href='https://example.com/docs']")).toHaveText("the docs");
  expect(errors).toEqual([]);
});

test("an image sent along is drawn with the message when the session is opened anew", TAGS, async ({ page }) => {
  await open(page, URL);
  const id = await newSession(page, "says-ready");
  const png = readFileSync(join(ROOT, "templates", "desktop", "src-tauri", "icons", "32x32.png"));
  await page.getByTestId("conv-attach").click();
  const [chooser] = await Promise.all([page.waitForEvent("filechooser"), page.getByTestId("conv-attach-image").click()]);
  await chooser.setFiles({ name: "dot.png", mimeType: "image/png", buffer: png });
  await expect(page.getByTestId("conv-attachment")).toBeVisible({ timeout: 15_000 });
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toBeVisible({ timeout: 30_000 });
  // What was sent is drawn from the transcript, which the hub serves once the turn has settled.
  const running = () =>
    page.evaluate(async (sid) => {
      const { sessions } = await (window as any).harnix.app.call("harnix.instances.sessions", { instance: "system" });
      return sessions.find((s: any) => s.session_id === sid)?.running;
    }, id);
  await expect.poll(running, { timeout: 30_000 }).toBe(false);
  await page.reload();
  await expect(page.locator("#conn.on")).toBeVisible({ timeout: 15_000 });
  await page.evaluate((sid) => (window as any).harnix.app.open({ instance: "system", sessionId: sid }), id);
  await expect(page.getByTestId("conv-image").first()).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("conv-image").first().locator("img")).toHaveAttribute("src", /^(data:image|blob:|\/)/);
});

test("a call that fails says so in a banner, which goes when dismissed", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "says-ready");
  // Editing a queued message that is not in the queue: refused, and the composer's session says why.
  await page.evaluate(() => (window as any).harnix.app.controller.editQueued("no-such-item", "x"));
  const banner = page.getByTestId("conv-banner");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText(/no-such-item/);
  await page.getByTestId("conv-banner-dismiss").click();
  await expect(page.getByTestId("conv-banner")).toHaveCount(0);
});

test("what the model was sent apart from the messages is a row of its own, read at length beside the conversation", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  const id = await oneTurn(page);
  // A replayed session records nothing of its own: the envelope is read from a recording put in its place.
  cpSync(join(RECORDINGS, "calls-a-tool"), join(SCRATCH, "agix", "recordings", id), { recursive: true });
  await page.reload();
  await expect(page.locator("#conn.on")).toBeVisible({ timeout: 15_000 });
  await page.evaluate((sid) => (window as any).harnix.app.open({ instance: "system", sessionId: sid }), id);
  const row = page.getByTestId("conv-request").first();
  await expect(row).toBeVisible({ timeout: 15_000 });
  await row.getByTestId("conv-request-toggle").click();
  await row.getByTestId("open-in-details").click();
  const details = page.getByTestId("details-request");
  await expect(details).toBeVisible();
  await expect(details).toContainText("Bash");
  expect(errors).toEqual([]);
});
