/**
 * The frame, in a real browser against a real backend: what a fake DOM cannot see — a panel four
 * pixels wide, text the colour of its background, an icon filling its row — and the chrome around
 * the conversation: the sidebar, search, the settings panel, the theme, the language.
 */

import { dropSessions, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { connected, flowReference, newSession, open, openFlow, openSettings, say, sessionRow, settingsSection, toFlows, watchConsole } from "./page";

const TAGS = { tag: ["@feature:session-space", "@layer:ui"] };
const SETTINGS = { tag: ["@feature:settings", "@layer:ui"] };
const CONNECTION = { tag: ["@feature:connection", "@layer:ui"] };

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});
// The worker's backend is shared by every file that uses it: a test leaves no session behind
// for the next one to count.
test.afterEach(async ({ api }) => {
  await dropSessions(api);
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
  await connected(page);
  const before = await page.evaluate(() => document.documentElement.dataset.theme);
  await page.getByTestId("theme-toggle").click();
  await expect.poll(() => page.evaluate(() => document.documentElement.dataset.theme)).not.toBe(before);
});

test("the theme chosen in settings applies at once, survives a reload, and follows the system when set back", SETTINGS, async ({ page }) => {
  await open(page, URL);
  const theme = () => page.evaluate(() => document.documentElement.dataset.theme);
  const body = () => page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  await openSettings(page);
  const select = page.getByTestId("settings-theme");
  await select.selectOption("dark");
  await expect.poll(theme).toBe("dark");
  const dark = await body();
  await select.selectOption("light");
  await expect.poll(theme).toBe("light");
  expect(await body(), "the two themes paint the page differently").not.toBe(dark);

  await page.reload();
  await connected(page);
  await expect.poll(theme).toBe("light");

  // Back to the system's: the page follows what the system prefers.
  await page.emulateMedia({ colorScheme: "dark" });
  await openSettings(page);
  await page.getByTestId("settings-theme").selectOption("system");
  await expect.poll(theme).toBe("dark");
  await page.emulateMedia({ colorScheme: "light" });
  await expect.poll(theme).toBe("light");
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
  const items = page.getByTestId("settings-nav").getByRole("button");
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
    await expect(items.nth(i)).toHaveAttribute("data-state", "current");
    await expect(page.getByTestId("settings-page")).toBeVisible();
    await expect(page.getByTestId("settings-page").getByTestId("settings-failed")).toHaveCount(0);
    expect(await size(), `section ${i}`).toBe(first);
  }
  await page.keyboard.press("Escape");
  await expect(page.locator("#veil")).toHaveAttribute("data-state", "closed");
  expect(errors).toEqual([]);
});

test("the language switches everything at once, and comes back", SETTINGS, async ({ page }) => {
  await open(page, URL);
  await openSettings(page);
  const language = page.getByTestId("settings-language");
  await language.selectOption("en");
  await expect(page.locator("#new")).toContainText("New session");
  await expect(page.getByTestId("settings-nav").getByRole("button").first()).toHaveText("General");
  await language.selectOption("zh-CN");
  await expect(page.locator("#new")).toContainText("新会话");
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  await language.selectOption("system");
});

test("the sidebar collapses to a rail and back", TAGS, async ({ page }) => {
  await open(page, URL);
  await page.getByTestId("sidebar-toggle").click();
  await expect(page.locator("#frame")).toHaveAttribute("data-sidebar", "collapsed");
  await page.getByTestId("sidebar-toggle").click();
  await expect(page.locator("#frame")).toHaveAttribute("data-sidebar", "expanded");
  await page.keyboard.press(process.platform === "darwin" ? "Meta+\\" : "Control+\\");
  await expect(page.locator("#frame")).toHaveAttribute("data-sidebar", "collapsed");
});

test("a new conversation starts empty, and its first message makes it and lists it", TAGS, async ({ page }) => {
  await open(page, URL);
  await page.locator("#new").click();
  await expect(page.locator("#main")).toHaveAttribute("data-state", "empty");
  await page.getByTestId("conv-input").fill("Say exactly: fixture ready");
  await page.getByTestId("conv-input").press("Enter");
  await expect(page.locator("#main")).toHaveAttribute("data-state", "open");
  await expect(page.locator('[data-testid="session-row"][data-state="current"]')).toBeVisible();
});

test("search filters the list by name, and Enter searches the transcripts", TAGS, async ({ page }) => {
  await open(page, URL);
  const id = await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(sessionRow(page, id)).toBeVisible();
  await page.locator("#search").fill("no session is called this");
  await expect(page.getByTestId("session-search")).toHaveAttribute("data-state", "searching");
  await expect(sessionRow(page, id)).toHaveCount(0);
  await page.locator("#search").fill("fixture");
  await page.locator("#search").press("Enter");
  await expect(page.locator('[data-testid="session-hit"]').first()).toBeVisible({ timeout: 10_000 });
  await page.locator("#search").press("Escape");
  await expect(page.getByTestId("session-search")).toHaveAttribute("data-state", "idle");
});

test("a session renamed from its menu is renamed in the list", TAGS, async ({ page }) => {
  await open(page, URL);
  const id = await newSession(page, "says-ready");
  const row = sessionRow(page, id);
  await row.click({ button: "right" });
  await page.locator('[data-testid="session-menu"] [data-item="rename"]').click();
  const input = page.locator("#modal input");
  await input.fill("renamed in the test");
  await input.press("Enter");
  await expect(row.locator(".name")).toHaveText("renamed in the test");
  await expect(page.locator("#head .title")).toHaveText("renamed in the test");
});

test("a device is paired from settings with a code shown once, counting down", { tag: ["@feature:authz", "@layer:ui"] }, async ({ page }) => {
  await open(page, URL);
  await openSettings(page);
  await settingsSection(page, "devices").click();
  await page.getByTestId("settings-page").locator("button", { hasText: /pair|配对/i }).last().click();
  await expect(page.locator(".pairing-code")).toHaveText(/\S{4,}/);
  await expect(page.getByTestId("settings-page")).toContainText(/\d+:\d{2}/);
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
  await connected(page);
  await expect.poll(async () => (await side.boundingBox())!.width).toBeGreaterThan(before + 40);
  await page.getByTestId("resize-side").dblclick();
  await expect.poll(async () => Math.round((await side.boundingBox())!.width)).toBe(Math.round(before));
});

test("the right column is a tab strip with no title bar: it maximizes over the centre, restores, and closes", SHELL, async ({ page }) => {
  // Widths are compared, so the columns are measured where they end up: with no 300 ms transition,
  // a slow moment cannot catch one part way and compare the restored width against that.
  await page.emulateMedia({ reducedMotion: "reduce" });
  await open(page, URL);
  await page.getByTestId("details-toggle").click();
  const details = page.locator("#details");
  await expect(details.locator('[role="tab"]').first()).toBeVisible();
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

test("narrower than 960 the right column opens over the centre, its buttons reachable, and closes back to it", SHELL, async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 700 });
  await open(page, URL);
  await page.getByTestId("details-toggle").click();
  const details = page.locator("#details");
  // A drawer from the right edge, over the conversation, which stays where it was.
  await expect.poll(async () => (await details.boundingBox())?.width ?? 0).toBeGreaterThan(300);
  await expect.poll(async () => Math.round(((await details.boundingBox())?.x ?? 0) + ((await details.boundingBox())?.width ?? 0))).toBe(800);
  await expect(page.locator("#main")).toBeVisible();
  await page.getByTestId("details-close").click();
  await expect.poll(async () => (await details.boundingBox())?.width ?? 0).toBeLessThan(2);
  await expect(page.locator("#main")).toBeVisible();
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
  // The lights end 80px in, centred in the 52px band; the toggle is right of them, open or rail.
  const toggle = page.getByTestId("sidebar-toggle");
  const at = async () => (await toggle.boundingBox())!;
  expect((await at()).x).toBeGreaterThanOrEqual(88);
  await toggle.click();
  await expect(page.locator("#frame")).toHaveAttribute("data-sidebar", "collapsed");
  await expect.poll(async () => (await at()).x).toBeGreaterThanOrEqual(88);
  const toggleEnd = (await at()).x + (await at()).width;
  expect((await page.locator("#head .title").boundingBox())!.x).toBeGreaterThanOrEqual(toggleEnd);
  await toggle.click();
  await expect(page.locator("#frame")).toHaveAttribute("data-sidebar", "expanded");
});

test("outside the desktop shell there is no window chrome to make room for", SHELL, async ({ page }) => {
  await open(page, URL);
  await expect(page.locator("html")).not.toHaveAttribute("data-chrome", "overlay");
  expect((await page.getByTestId("sidebar-toggle").boundingBox())!.y).toBeLessThan(40);
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
  await expect(page.getByTestId("settings-nav")).toBeVisible();
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
    await expect(page.getByTestId("space-list")).toHaveCount(0);
    await expect(page.locator('#side [data-space="system"]')).toHaveCount(0);
    await expect(page.getByTestId("avatar")).toBeVisible();
  } finally {
    await bare.dispose();
  }
});

test("at a phone's width nothing scrolls sideways: a conversation, the system space, a flow in it", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await open(page, URL);
  const sideways = () => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  await newSession(page, "says-markdown");
  await say(page, "Say exactly: fixture ready");
  await expect(page.locator('[data-testid="conv-group"] table')).toBeVisible({ timeout: 30_000 });
  expect(await sideways(), "the conversation").toBeLessThanOrEqual(0);
  await expect(page.getByTestId("conv-input")).toBeInViewport();
  for (const space of ["system", "flow"]) {
    await page.evaluate((id) => (window as any).harnix.app.switchSpace(id), space);
    await expect(page.locator("#main")).toHaveAttribute("data-space", space);
    // Polled: the columns animate into place when a space opens its right column.
    await expect.poll(sideways, { message: `the ${space} space` }).toBeLessThanOrEqual(0);
  }
  // A flow entered, its details beside the conversation.
  await page.evaluate(() => (window as any).harnix.app.switchSpace("system"));
  await page.locator('[data-testid="system-console"] [role="tab"][data-tab="flows"]').click();
  await expect.poll(sideways, { message: "the Flows tab" }).toBeLessThanOrEqual(0);
  await page.getByTestId("flow-def-example-review").click();
  await expect(page.getByTestId("flow-context")).toHaveAttribute("data-flow", "example-review");
  await expect.poll(sideways, { message: "a flow" }).toBeLessThanOrEqual(0);
  expect(errors).toEqual([]);
});

test("in Chinese the system space has no string left untranslated or showing its key", SETTINGS, async ({ page }) => {
  await open(page, URL);
  await openSettings(page);
  await page.getByTestId("settings-language").selectOption("zh-CN");
  await page.keyboard.press("Escape");
  await page.locator('#side [data-space="system"]').click();
  await expect(page.getByTestId("system-console")).toBeVisible();
  // The system space's strings are `system.*`; one drawn as its key has no translation. Method
  // names (`harnix.system.context`) are not keys: a dot before is not a key's start.
  const leaked = () => page.evaluate(() => document.body.innerText.match(/(?<![\w.])system\.[a-z][\w.]*[a-z]\b/g) ?? []);
  for (const tab of ["active", "all"]) {
    const button = page.locator(`[data-testid="system-console"] [role="tab"][data-tab="${tab}"]`);
    await expect(button, `the ${tab} tab's title`).toHaveText(/[一-鿿]/);
    await button.click();
    expect(await leaked(), `the ${tab} tab`).toEqual([]);
  }
  await expect(page.locator('[data-testid="system-strip"] [data-kpi="threads"] .l')).toHaveText(/[一-鿿]/);
  await openSettings(page);
  await page.getByTestId("settings-language").selectOption("system");
  await page.keyboard.press("Escape");
});

test("in Chinese the flows have no string left untranslated or showing its key", SETTINGS, async ({ page }) => {
  await open(page, URL);
  await openSettings(page);
  await page.getByTestId("settings-language").selectOption("zh-CN");
  await page.keyboard.press("Escape");
  await toFlows(page);
  await expect(page.locator('[data-testid="system-console"] [role="tab"][data-tab="flows"]')).toHaveText("流程");
  const leaked = () =>
    page.evaluate(() => {
      const text = document.body.innerText;
      // The flow interface's strings are `flowui.*`; a key that is drawn is one without a translation.
      return text.match(/\bflowui\.[a-z][\w.]*[a-z]\b/g) ?? [];
    });
  expect(await leaked(), "the Flows tab").toEqual([]);
  await openFlow(page, "example-review");
  expect(await leaked(), "a flow's context and details").toEqual([]);
  await expect(page.getByTestId("flow-back")).toHaveText(/[一-鿿]/);
  await expect(page.locator('#secondary [data-testid="flow-detail"]')).toContainText(/[一-鿿]/);
  await flowReference(page);
  await expect(page.locator('#details [role="tab"][data-tab="flow.reference"]')).toHaveText(/[一-鿿]/);
  expect(await leaked(), "the reference").toEqual([]);
  // The general conversation's context.
  await toFlows(page);
  await page.getByTestId("flow-add").click();
  await expect(page.getByTestId("flow-context-title")).toHaveText("通用会话");
  expect(await leaked(), "the general conversation").toEqual([]);
  await openSettings(page);
  await page.getByTestId("settings-language").selectOption("system");
});
