/**
 * The right column as the open session's docking surface (harnix-ext.md §22.7), in a real browser
 * against a real backend: the guide as the default page, the expand control at the header's corner,
 * a tab type registered from the page and opened through `host.sidebar`, the surface remembered
 * across a reload, a space's own `details.tabs` still reachable beside the session's tabs, and the
 * narrow drawer carrying the same strip.
 *
 * Tab types are registered from inside the page, on the shell's own registry, the way a module's
 * `register(registry, host)` would — React comes from the page's import map.
 */

import type { Page } from "@playwright/test";
import { dropSessions, expect, test } from "@harnix/testkit";

import { newSession, open, tab, watchConsole } from "./page";

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
const details = (page: Page) => page.locator("#details");
/** A session tab's chip, by the kind of its type. */
const chip = (page: Page, kind: string) => page.locator(`#details [data-testid="sidebar-tab"][data-kind="${kind}"]`);

/** A preview type claiming `harnix-resource://file/**`, and a page type with a guide entry, registered on the page as an
 *  extension's are: over the product's own types of the same kinds. */
async function registerTypes(page: Page): Promise<void> {
  await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    const React = await import("react" as string);
    const { useTabInfo } = await import("@harnix/react" as string);
    const body = (testId: string) =>
      React.createElement(() => {
        const { tab } = useTabInfo();
        return React.createElement("div", { "data-testid": testId, "data-address": tab.address ?? "", "data-revision": tab.navigation.revision }, JSON.stringify(tab.navigation.params));
      });
    app.registry.register(
      "sidebar.tab",
      {
        id: "test.preview",
        kind: "preview",
        // Above the Harnix app's own preview (a fallback of the same kind): the page's wins while it is registered.
        priority: "extension",
        patterns: ["harnix-resource://file/**"],
        title: (address: string | undefined) => (address ? address.split("/").pop() : "Preview"),
        render: () => body("test-preview"),
      },
      "test",
    );
    app.registry.register(
      "sidebar.tab",
      {
        id: "test.usage",
        kind: "usage",
        priority: "extension",
        // `title` is required of every type: without it the registration is refused.
        title: () => "Usage",
        guide: [{ id: "usage", title: () => "Usage", description: () => "What this session spent" }],
        render: () => body("test-usage"),
      },
      "test",
    );
  });
}

test("a new session's column opens from the header's corner onto the guide, which lists the registered entries", SHELL, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await registerTypes(page);
  await newSession(page, "says-ready");
  const toggle = page.getByTestId("details-toggle");
  await expect(toggle).toHaveAttribute("data-state", "collapsed");
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await toggle.click();
  await expect(frame(page)).toHaveAttribute("data-details", "open");
  await expect(toggle).toHaveAttribute("data-state", "expanded");
  // The default page: the guide, current, with one card per guide entry; no add control while it is open.
  await expect(chip(page, "guide")).toHaveAttribute("data-state", "current");
  await expect(page.getByTestId("sidebar-guide")).toBeVisible();
  await expect(page.locator('[data-testid="sidebar-guide-entry"][data-kind="usage"]')).toContainText("Usage");
  await expect(page.getByTestId("sidebar-add")).toHaveCount(0);
  // A card opens its page in the guide's place.
  await page.locator('[data-testid="sidebar-guide-entry"][data-kind="usage"]').click();
  await expect(page.getByTestId("test-usage")).toBeVisible();
  await expect(chip(page, "guide")).toHaveCount(0);
  await expect(chip(page, "usage")).toHaveAttribute("data-state", "current");
  // The add control brings the guide back; the chip's close takes it away again.
  await page.getByTestId("sidebar-add").click();
  await expect(chip(page, "guide")).toHaveAttribute("data-state", "current");
  await chip(page, "guide").getByTestId("sidebar-tab-close").click();
  await expect(chip(page, "guide")).toHaveCount(0);
  await expect(chip(page, "usage")).toHaveAttribute("data-state", "current");
  // The header's button collapses it again.
  await toggle.click();
  await expect(frame(page)).toHaveAttribute("data-details", "closed");
  expect(errors).toEqual([]);
});

test("host.sidebar opens a resource with the type that claims it, dedupes, delivers params, and the surface survives a reload", SHELL, async ({ page }) => {
  await open(page, URL);
  await registerTypes(page);
  const sid = await newSession(page, "says-ready");
  const first = await page.evaluate(() => (window as any).harnix.host.sidebar.openResource("harnix-resource://file/src/a.ts", { params: { line: 3 } }));
  await expect(frame(page)).toHaveAttribute("data-details", "open");
  await expect(page.getByTestId("test-preview")).toHaveAttribute("data-address", "harnix-resource://file/src/a.ts");
  await expect(page.getByTestId("test-preview")).toHaveText('{"line":3}');
  await expect(chip(page, "preview").locator('[role="tab"]')).toHaveText("a.ts");
  // The same address again: the same tab, the new params, the revision up.
  const again = await page.evaluate(() => (window as any).harnix.host.sidebar.openResource("harnix-resource://file/src/a.ts", { params: { line: 9 } }));
  expect(again).toBe(first);
  await expect(chip(page, "preview")).toHaveCount(1);
  await expect(page.getByTestId("test-preview")).toHaveText('{"line":9}');
  await expect(page.getByTestId("test-preview")).toHaveAttribute("data-revision", "2");
  // An address no type claims is a wiring error.
  const refused = await page.evaluate(() => {
    try {
      (window as any).harnix.host.sidebar.openResource("harnix-resource://nobody/x");
      return null;
    } catch (e) {
      return String(e);
    }
  });
  expect(refused).toMatch(/harnix-resource:\/\/nobody\/x/);
  // Stored under the session's key, beside the context tab that stands in every surface; back after
  // a reload, the type registered again.
  const stored = await page.evaluate((id) => localStorage.getItem(`harnix.sidebar.v1.main/${id}`), sid);
  const standing = await chip(page, "context").getAttribute("data-tab");
  expect([...JSON.parse(stored!).layout.tabs].sort()).toEqual([standing, first].sort());
  await page.reload();
  await open(page, URL);
  await registerTypes(page);
  if ((await frame(page).getAttribute("data-details")) !== "open") await page.getByTestId("details-toggle").click();
  await expect(chip(page, "preview")).toHaveAttribute("data-state", "current");
  await expect(page.getByTestId("test-preview")).toHaveAttribute("data-address", "harnix-resource://file/src/a.ts");
  // Not stored: the params. A fresh page starts the navigation over.
  await expect(page.getByTestId("test-preview")).toHaveText("{}");
});

test("a space's own details tabs stay in the strip beside the session's tabs, and details.tab() still names them", SHELL, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await registerTypes(page);
  await page.evaluate(() => {
    const { app } = (window as any).harnix;
    const t = (id: string) => ({ id, title: () => id.toUpperCase(), render: () => `body of ${id}` });
    app.registry.register("space", { id: "tabbed", icon: "chat", title: () => "Tabbed", sessions: "sessions", details: { tabs: [t("t1"), t("t2")] } }, "test");
  });
  await page.locator('#side button[data-space="tabbed"]').click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "tabbed");
  await newSession(page, "says-ready");
  await page.evaluate(() => (window as any).harnix.host.sidebar.openTab("usage"));
  await expect(frame(page)).toHaveAttribute("data-details", "open");
  await expect(page.getByTestId("test-usage")).toBeVisible();
  // Both sides in one strip: the space's two, the session's one.
  await expect(tab(details(page), "t1")).toBeVisible();
  await expect(tab(details(page), "t2")).toBeVisible();
  expect(await page.evaluate(() => (window as any).harnix.host.details.tab())).toBe("t1");
  await tab(details(page), "t2").click();
  await expect(details(page).locator(".body")).toContainText("body of t2");
  expect(await page.evaluate(() => (window as any).harnix.host.details.tab())).toBe("t2");
  await expect(page.getByTestId("test-usage")).toHaveCount(0);
  // And back to the session's tab by its chip.
  await chip(page, "usage").locator('[role="tab"]').click();
  await expect(page.getByTestId("test-usage")).toBeVisible();
  expect(await page.evaluate(() => (window as any).harnix.host.details.tab())).toBe("t2");
  // reveal(item) brings the details tab in front, as before.
  await page.evaluate(() => (window as any).harnix.host.reveal({ kind: "tool", name: "Bash", id: "c1", input: "ls" }));
  await expect(tab(details(page), "details")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("details-tool")).toBeVisible();
  expect(errors).toEqual([]);
});

test.describe("800px wide", () => {
  test.use({ viewport: { width: 800, height: 700 } });

  test("the drawer carries the session's surface: the guide inside, Escape closes it", SHELL, async ({ page }) => {
    await open(page, URL);
    await registerTypes(page);
    await newSession(page, "says-ready");
    await page.getByTestId("details-toggle").click();
    await expect(frame(page)).toHaveAttribute("data-details", "open");
    const box = await details(page).boundingBox();
    expect(Math.round(box!.x + box!.width)).toBe(800);
    await expect(page.getByTestId("drawer-veil")).toBeVisible();
    await expect(chip(page, "guide")).toHaveAttribute("data-state", "current");
    await expect(page.getByTestId("sidebar-guide")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(frame(page)).toHaveAttribute("data-details", "closed");
    // Collapsed, the surface is kept: the guide is still there when it opens again.
    await page.getByTestId("details-toggle").click();
    await expect(chip(page, "guide")).toHaveAttribute("data-state", "current");
  });
});
