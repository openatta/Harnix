/**
 * The frame, in a real browser against a real backend: what a fake DOM cannot see — a panel four
 * pixels wide, text the colour of its background, an icon filling its row — and the chrome around
 * the conversation: the sidebar, search, the settings panel, the theme, the language.
 */

import { expect, test } from "@harnix/testkit";

import { newSession, open, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:session-space", "@layer:ui"] };
const SETTINGS = { tag: ["@feature:settings", "@layer:ui"] };
const CONNECTION = { tag: ["@feature:connection", "@layer:ui"] };

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});

test("the page loads, connects, and lays out its columns", CONNECTION, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  expect((await page.locator("#side").boundingBox())!.width).toBeGreaterThan(120);
  expect((await page.locator("#main").boundingBox())!.width).toBeGreaterThan(320);
  expect((await page.locator("#details").boundingBox())?.width ?? 0).toBeLessThan(2);
  await page.getByTestId("details-toggle").click();
  await expect.poll(async () => (await page.locator("#details").boundingBox())?.width ?? 0).toBeGreaterThan(200);
  await expect(page.locator("#hero")).toBeVisible();
  expect(errors).toEqual([]);
});

test("nothing scrolls the page sideways", TAGS, async ({ page }) => {
  await open(page, URL);
  for (const width of [1440, 1024, 720]) {
    await page.setViewportSize({ width, height: 800 });
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow, `at ${width}px`).toBeLessThanOrEqual(0);
  }
});

test("text is readable in both themes, and the theme button switches between them", TAGS, async ({ page }) => {
  await open(page, URL);
  for (const theme of ["light", "dark"]) {
    await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
    const { color, background } = await page.evaluate(() => {
      const style = getComputedStyle(document.body);
      return { color: style.color, background: style.backgroundColor };
    });
    expect(color, theme).not.toBe(background);
    expect(background, theme).not.toBe("rgba(0, 0, 0, 0)");
  }
  await page.reload();
  await expect(page.locator("#conn.on")).toBeVisible({ timeout: 15_000 });
  const before = await page.evaluate(() => document.documentElement.dataset.theme);
  await page.locator("#side .foot button").nth(1).click();
  await expect.poll(() => page.evaluate(() => document.documentElement.dataset.theme)).not.toBe(before);
});

test("icons are sized by a class, not left to fill the row", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "says-ready");
  const largest = await page.evaluate(() =>
    Math.max(0, ...[...document.querySelectorAll("svg")].map((s) => Math.max(s.getBoundingClientRect().width, s.getBoundingClientRect().height))),
  );
  expect(largest).toBeLessThanOrEqual(64);
});

test("the settings panel opens and every section draws", SETTINGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await page.locator("#side .foot button").first().click();
  await expect(page.locator(".settings-nav")).toBeVisible();
  const items = page.locator(".settings-nav .nav-item");
  expect(await items.count()).toBeGreaterThan(4);
  for (let i = 0; i < (await items.count()); i += 1) {
    await items.nth(i).click();
    await expect(items.nth(i)).toHaveClass(/on/);
    await expect(page.locator(".settings-page")).toBeVisible();
    await expect(page.locator(".settings-page .hint.err")).toHaveCount(0);
  }
  await page.keyboard.press("Escape");
  await expect(page.locator("#veil")).not.toHaveClass(/on/);
  expect(errors).toEqual([]);
});

test("the language switches everything at once, and comes back", SETTINGS, async ({ page }) => {
  await open(page, URL);
  await page.locator("#side .foot button").first().click();
  const language = page.locator(".settings-page select").nth(1);
  await language.selectOption("en");
  await expect(page.locator("#new")).toContainText("New session");
  await expect(page.locator(".settings-nav .nav-item").first()).toHaveText("General");
  await language.selectOption("zh-CN");
  await expect(page.locator("#new")).toContainText("新会话");
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  await language.selectOption("system");
});

test("the sidebar collapses to a rail and back", TAGS, async ({ page }) => {
  await open(page, URL);
  await page.locator("#side .collapse").click();
  await expect(page.locator("#frame")).toHaveAttribute("data-sidebar", "collapsed");
  await page.locator("#side .collapse").click();
  await expect(page.locator("#frame")).toHaveAttribute("data-sidebar", "expanded");
  await page.keyboard.press(process.platform === "darwin" ? "Meta+\\" : "Control+\\");
  await expect(page.locator("#frame")).toHaveAttribute("data-sidebar", "collapsed");
});

test("a new session from the dialog opens and is listed", TAGS, async ({ page }) => {
  await open(page, URL);
  await page.locator("#new").click();
  await expect(page.locator("#veil")).toHaveClass(/on/);
  await expect(page.getByTestId("new-scene")).toBeVisible();
  await expect(page.getByTestId("new-mode")).toBeVisible();
  await page.getByTestId("new-create").click();
  await expect(page.locator("#veil")).not.toHaveClass(/on/);
  await expect(page.getByTestId("conv-input")).toBeEnabled();
  await expect(page.locator("#side .srow.on")).toBeVisible();
});

test("search filters the list by name, and Enter searches the transcripts", TAGS, async ({ page }) => {
  await open(page, URL);
  const id = await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(page.locator(`#side .srow[data-sid="${id}"]`)).toBeVisible();
  await page.locator(".section-head .icon-btn").first().click();
  await expect(page.locator(".section-head")).toHaveClass(/searching/);
  await page.locator("#search").fill("no session is called this");
  await expect(page.locator(`#side .srow[data-sid="${id}"]`)).toHaveCount(0);
  await page.locator("#search").fill("fixture");
  await page.locator("#search").press("Enter");
  await expect(page.locator("#side .srow.hit").first()).toBeVisible({ timeout: 10_000 });
  await page.locator("#search").press("Escape");
  await expect(page.locator(".section-head")).not.toHaveClass(/searching/);
});

test("a session renamed from its menu is renamed in the list", TAGS, async ({ page }) => {
  await open(page, URL);
  const id = await newSession(page, "says-ready");
  const row = page.locator(`#side .srow[data-sid="${id}"]`);
  await row.click({ button: "right" });
  await page.locator("#modal .menu-list button").nth(1).click();
  const input = page.locator("#modal input");
  await input.fill("renamed in the test");
  await input.press("Enter");
  await expect(row.locator(".name")).toHaveText("renamed in the test");
  await expect(page.locator("#head .title")).toHaveText("renamed in the test");
});

test("a device is paired from settings with a code shown once, counting down", { tag: ["@feature:authz", "@layer:ui"] }, async ({ page }) => {
  await open(page, URL);
  await page.locator("#side .foot button").first().click();
  await page.locator(".settings-nav .nav-item").nth(5).click();
  await page.locator(".settings-page button", { hasText: /pair|配对/i }).last().click();
  await expect(page.locator(".pairing-code")).toHaveText(/\S{4,}/);
  await expect(page.locator(".settings-page")).toContainText(/\d+:\d{2}/);
});
