/**
 * The frame, in a real browser against a real backend: what a fake DOM cannot see — a panel four
 * pixels wide, text the colour of its background, an icon filling its row — and the chrome around
 * the conversation: the sidebar, search, the settings panel, the theme, the language.
 */

import { expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { newSession, open, openSettings, say, watchConsole } from "./page";

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
  await page.getByTestId("theme-toggle").click();
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

test("the settings panel opens, every section draws, and the panel keeps one size", SETTINGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await openSettings(page);
  const items = page.locator(".settings-nav .nav-item");
  expect(await items.count()).toBeGreaterThan(4);
  const size = async () => {
    const box = (await page.locator("#modal").boundingBox())!;
    return `${Math.round(box.width)}x${Math.round(box.height)}`;
  };
  // Measured after the opening animation, which scales the panel in.
  await page.locator("#modal").evaluate((el) => Promise.all(el.getAnimations().map((a) => a.finished)));
  const first = await size();
  for (let i = 0; i < (await items.count()); i += 1) {
    await items.nth(i).click();
    await expect(items.nth(i)).toHaveClass(/on/);
    await expect(page.locator(".settings-page")).toBeVisible();
    await expect(page.locator(".settings-page .hint.err")).toHaveCount(0);
    expect(await size(), `section ${i}`).toBe(first);
  }
  await page.keyboard.press("Escape");
  await expect(page.locator("#veil")).not.toHaveClass(/on/);
  expect(errors).toEqual([]);
});

test("the language switches everything at once, and comes back", SETTINGS, async ({ page }) => {
  await open(page, URL);
  await openSettings(page);
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

test("a new conversation starts empty, and its first message makes it and lists it", TAGS, async ({ page }) => {
  await open(page, URL);
  await page.locator("#new").click();
  await expect(page.locator("#main")).toHaveClass(/empty/);
  await page.getByTestId("conv-input").fill("Say exactly: fixture ready");
  await page.getByTestId("conv-input").press("Enter");
  await expect(page.locator("#main")).not.toHaveClass(/empty/);
  await expect(page.locator("#side .srow.on")).toBeVisible();
});

test("search filters the list by name, and Enter searches the transcripts", TAGS, async ({ page }) => {
  await open(page, URL);
  const id = await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(page.locator(`#side .srow[data-sid="${id}"]`)).toBeVisible();
  await page.locator("#search").fill("no session is called this");
  await expect(page.locator(".section-head")).toHaveClass(/searching/);
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
  await openSettings(page);
  await page.locator(".settings-nav .nav-item").nth(5).click();
  await page.locator(".settings-page button", { hasText: /pair|配对/i }).last().click();
  await expect(page.locator(".pairing-code")).toHaveText(/\S{4,}/);
  await expect(page.locator(".settings-page")).toContainText(/\d+:\d{2}/);
});

const SHELL = { tag: ["@feature:shell", "@layer:ui"] };

test("the side columns are dragged wider, remember it, and a double-click puts them back", SHELL, async ({ page }) => {
  await open(page, URL);
  const side = page.locator("#side");
  const before = (await side.boundingBox())!.width;
  const edge = page.getByTestId("resize-side");
  const box = (await edge.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + 200);
  await page.mouse.down();
  await page.mouse.move(box.x + 60, box.y + 200, { steps: 5 });
  await page.mouse.up();
  await expect.poll(async () => (await side.boundingBox())!.width).toBeGreaterThan(before + 40);
  await page.reload();
  await expect(page.locator("#conn.on")).toBeVisible({ timeout: 15_000 });
  await expect.poll(async () => (await side.boundingBox())!.width).toBeGreaterThan(before + 40);
  await page.getByTestId("resize-side").dblclick();
  await expect.poll(async () => Math.round((await side.boundingBox())!.width)).toBe(Math.round(before));
});

test("the right column is a tab strip with no title bar: it maximizes over the centre, restores, and closes", SHELL, async ({ page }) => {
  await open(page, URL);
  await page.getByTestId("details-toggle").click();
  const details = page.locator("#details");
  await expect(details.locator(".strip .tabs [role=tab]").first()).toBeVisible();
  await expect(details.locator(".head")).toHaveCount(0);
  await expect.poll(async () => (await details.boundingBox())?.width ?? 0).toBeGreaterThan(300);
  const normal = (await details.boundingBox())!.width;
  await page.getByTestId("details-max").click();
  await expect.poll(async () => (await details.boundingBox())!.width).toBeGreaterThan(normal + 300);
  await expect(page.locator("#main")).toBeHidden();
  await page.getByTestId("details-max").click();
  await expect.poll(async () => Math.round((await details.boundingBox())!.width)).toBe(Math.round(normal));
  await expect(page.locator("#main")).toBeVisible();
  await page.getByTestId("details-close").click();
  await expect.poll(async () => (await details.boundingBox())?.width ?? 0).toBeLessThan(2);
});

test("in the desktop shell on macOS the top bars leave the traffic lights room and are where the window is dragged", SHELL, async ({ page }) => {
  // What the shell (templates/desktop) looks like from inside the page: Tauri's internals, a Mac.
  await page.addInitScript(() => {
    (window as any).__TAURI_INTERNALS__ = {};
    Object.defineProperty(navigator, "userAgent", { get: () => "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)" });
  });
  await open(page, URL);
  await expect(page.locator("html")).toHaveAttribute("data-chrome", "overlay");
  for (const bar of ["#head", "#side .l-top", "#details .strip"]) {
    await expect(page.locator(bar)).toHaveAttribute("data-tauri-drag-region", "deep");
  }
  // The lights take the first 78px of the 52px band; the toggle is right of them, open or rail.
  const toggle = page.locator("#side .l-top .collapse");
  const at = async () => (await toggle.boundingBox())!;
  expect((await at()).x).toBeGreaterThanOrEqual(78);
  await toggle.click();
  await expect(page.locator("#frame")).toHaveAttribute("data-sidebar", "collapsed");
  await expect.poll(async () => (await at()).x).toBeGreaterThanOrEqual(78);
  const toggleEnd = (await at()).x + (await at()).width;
  expect((await page.locator("#head .title").boundingBox())!.x).toBeGreaterThanOrEqual(toggleEnd);
  await toggle.click();
  await expect(page.locator("#frame")).toHaveAttribute("data-sidebar", "expanded");
});

test("outside the desktop shell there is no window chrome to make room for", SHELL, async ({ page }) => {
  await open(page, URL);
  await expect(page.locator("html")).not.toHaveAttribute("data-chrome", "overlay");
  expect((await page.locator("#side .l-top .collapse").boundingBox())!.y).toBeLessThan(40);
});

test("the avatar is the settings: its ring says the connection, its menu opens settings and switches", SHELL, async ({ page }) => {
  await open(page, URL);
  const avatar = page.getByTestId("avatar");
  await expect(avatar).toHaveAttribute("data-conn", "ok");
  await avatar.click();
  await expect(page.getByTestId("avatar-menu")).toBeVisible();
  await page.getByTestId("menu-connection").click();
  await expect(page.getByTestId("connection-details")).toContainText(/single_duplex|split_streams/);
  await page.keyboard.press("Escape");
  await avatar.click();
  await page.getByTestId("open-settings").click();
  await expect(page.locator(".settings-nav")).toBeVisible();
  await page.keyboard.press("Escape");
  // Gone, and the ring says so.
  await page.evaluate(() => (window as any).harnix.client.close());
  await expect(avatar).not.toHaveAttribute("data-conn", "ok");
});

test("with every built-in space removed, the frame is the chrome alone and says so", SHELL, async ({ page }) => {
  const bare = await startHarnix(launchFor("replay", { profile: "\n[shell]\nspaces = []\n" }));
  try {
    await open(page, bare.url);
    await expect(page.getByTestId("no-spaces")).toBeVisible();
    await expect(page.locator("#side .spaces")).toHaveCount(0);
    await expect(page.locator('#side [data-space="system"]')).toHaveCount(0);
    await expect(page.getByTestId("avatar")).toBeVisible();
  } finally {
    bare.stop();
  }
});
