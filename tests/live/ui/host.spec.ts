/**
 * What a space can do through its host (`useNest()`) and what it can say about itself
 * (`SpaceContribution`), in a real browser against a real backend.
 *
 * The spaces here are registered from inside the page, on the shell's own registry, the way a
 * module's `register(registry, host)` would — React comes from the page's import map, so a
 * component drawn here runs on the shell's React and sees its host. Each test keeps what its space
 * did in `window.__t`.
 */

import type { Page } from "@playwright/test";
import { expect, test } from "@harnix/testkit";

import { newSession, open, say, watchConsole } from "./page";

const SHELL = { tag: ["@feature:shell", "@layer:ui"] };

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});

/** Bring a space to the front the way a person does: its entry in the left column. */
async function inSpace(page: Page, id: string): Promise<void> {
  await page.locator(`#side [data-space="${id}"]`).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", id);
}

test("spaces.switch brings another space to the front, drawn, before it settles", SHELL, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await page.evaluate(() => {
    const { app } = (window as any).harnix;
    app.registry.register("space", { id: "beta", icon: "chat", title: () => "Beta", sessions: "sessions" }, "test");
  });
  const after = await page.evaluate(async () => {
    const { host } = (window as any).harnix;
    await host.spaces.switch("beta");
    // Straight away: the composer is beta's.
    host.composer.insert("x");
    return { current: host.spaces.current(), main: document.querySelector("#main")!.getAttribute("data-space") };
  });
  expect(after).toEqual({ current: "beta", main: "beta" });
  await expect(page.getByTestId("conv-input")).toHaveValue("x");
  const refused = await page.evaluate(() =>
    (window as any).harnix.host.spaces.switch("nope").then(
      () => "switched",
      (e: Error) => e.message,
    ),
  );
  expect(refused).toMatch(/nope/);
  expect(errors).toEqual([]);
});

test("session.open with switchSpace opens a session in the space the hub has it in", SHELL, async ({ page }) => {
  await open(page, URL);
  const mine = await newSession(page, "says-ready");
  // A session of the system space, made the way that space makes them.
  const other = await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    const created = await app.call("session.create", { space: "system", scene: "coding", options: { recorder: { name: "says-ready" } } });
    return { instance: created.instance ?? "system", sessionId: created.session_id as string };
  });
  // Without the option, nothing changes: the front space's own session is opened there.
  await page.evaluate((ref) => (window as any).harnix.host.session.open(ref), { instance: "system", sessionId: mine });
  await expect(page.locator("#main")).toHaveAttribute("data-space", "sessions");
  await page.evaluate((ref) => (window as any).harnix.host.session.open(ref, { switchSpace: true }), other);
  await expect(page.locator("#main")).toHaveAttribute("data-space", "system");
  const state = await page.evaluate(() => {
    const { app } = (window as any).harnix;
    return { current: app.snapshot.current, sessions: app.snapshot.currentBySpace.sessions };
  });
  expect(state.current).toEqual(other);
  expect(state.sessions.sessionId).toBe(mine);
  const missing = await page.evaluate(() =>
    (window as any).harnix.host.session.open({ instance: "system", sessionId: "no-such-session" }, { switchSpace: true }).then(
      () => "opened",
      (e: Error) => e.message,
    ),
  );
  expect(missing).toMatch(/no-such-session/);
});

test("details: redraw adds a tab, open selects it, clicks are heard, and each space keeps its own", SHELL, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await page.evaluate(() => {
    const { app, host } = (window as any).harnix;
    const t = ((window as any).__t = { count: 0, tabs: [] as string[] });
    host.details.onTab((tab: string) => t.tabs.push(tab));
    const tab = (id: string) => ({ id, title: () => id.toUpperCase(), render: () => `body of ${id}` });
    app.registry.register(
      "space",
      { id: "tabbed", icon: "chat", title: () => "Tabbed", sessions: "sessions", tabs: () => (t.count > 0 ? [tab("t1"), tab("t2")] : [tab("t1")]) },
      "test",
    );
  });
  await inSpace(page, "tabbed");
  const details = page.locator("#details");
  await expect(details.locator("[data-tab]")).toHaveCount(1);
  await page.evaluate(() => {
    (window as any).__t.count = 1;
    (window as any).harnix.host.redraw();
  });
  await expect(details.locator('[data-tab="t2"]')).toHaveCount(1);
  await page.evaluate(() => (window as any).harnix.host.details.open("t2"));
  await expect(details).toHaveAttribute("aria-hidden", "false");
  await expect(details.locator('[data-tab="t2"]')).toHaveAttribute("aria-selected", "true");
  await expect(details.locator(".body")).toContainText("body of t2");
  expect(await page.evaluate(() => (window as any).harnix.host.details.tab())).toBe("t2");
  await details.locator('[data-tab="t1"]').click();
  await expect.poll(() => page.evaluate(() => (window as any).__t.tabs)).toEqual(["t2", "t1"]);
  await page.evaluate(() => {
    const { host } = (window as any).harnix;
    host.details.open("t2");
    host.details.close();
  });
  await expect(details).toHaveAttribute("aria-hidden", "true");
  await inSpace(page, "sessions");
  await inSpace(page, "tabbed");
  await expect(details).toHaveAttribute("aria-hidden", "true");
  expect(await page.evaluate(() => (window as any).harnix.host.details.tab())).toBe("t2");
  expect(errors).toEqual([]);
});

test("useCurrentSession: the right column follows the session picked in the left one", SHELL, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  const first = await newSession(page, "says-ready");
  const second = await newSession(page, "says-ready");
  await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    const React = await import("react");
    const { useCurrentSession } = await import("@harnix/react");
    function Current() {
      const current = useCurrentSession();
      return React.createElement("div", { "data-testid": "current-session" }, current ? current.sessionId : "none");
    }
    app.registry.register(
      "space",
      {
        id: "follows",
        icon: "chat",
        title: () => "Follows",
        sessions: "sessions",
        details: "open",
        tabs: [{ id: "now", title: () => "Now", render: () => React.createElement(Current) }],
      },
      "test",
    );
  });
  await inSpace(page, "follows");
  const shown = page.getByTestId("current-session");
  await page.locator(`#side .srow[data-sid="${first}"]`).click();
  await expect(shown).toHaveText(first);
  await page.locator(`#side .srow[data-sid="${second}"]`).click();
  await expect(shown).toHaveText(second);
  expect(errors).toEqual([]);
});

test("two subscribers of one topic: the first leaving does not take it from the second", SHELL, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "says-ready");
  await page.evaluate(() => {
    const { host } = (window as any).harnix;
    const t = ((window as any).__t = { one: [] as unknown[], two: [] as unknown[], off: null as null | (() => void) });
    t.off = host.subscribe("harnix.sessions.status", {}, (v: unknown) => t.one.push(v));
    host.subscribe("harnix.sessions.status", {}, (v: unknown) => t.two.push(v));
  });
  await expect.poll(() => page.evaluate(() => (window as any).__t.two.length)).toBeGreaterThan(0);
  const before = await page.evaluate(() => {
    const t = (window as any).__t;
    t.off();
    return t.two.length;
  });
  await say(page, "Say exactly: fixture ready");
  // A turn running changes the topic: the second still hears it, and so does the shell's own list.
  await expect.poll(() => page.evaluate(() => (window as any).__t.two.length), { timeout: 30_000 }).toBeGreaterThan(before);
});
