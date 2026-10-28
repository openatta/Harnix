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
import { expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { newSession, open, reply, say, watchConsole } from "./page";

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

test("settings.open opens the panel at a section, a built-in one too, and the first for one that is not there", SHELL, async ({ page }) => {
  await open(page, URL);
  await page.evaluate(() => {
    const { app } = (window as any).harnix;
    app.registry.register("settings.section", { id: "probe", label: () => "Probe", render: () => "the probe's settings" }, "test");
    app.registry.register("space", { id: "configured", icon: "settings", title: () => "Configured", sessions: "sessions" }, "test");
  });
  await inSpace(page, "configured");
  const on = page.locator(".settings-nav .nav-item.on");
  await page.evaluate(() => (window as any).harnix.host.settings.open("probe"));
  await expect(on).toHaveText("Probe");
  await expect(page.locator(".settings-page")).toContainText("the probe's settings");
  await page.keyboard.press("Escape");
  await page.evaluate(() => (window as any).harnix.host.settings.open("models"));
  await expect(on).toHaveText(/Models|模型/);
  await page.keyboard.press("Escape");
  await page.evaluate(() => (window as any).harnix.host.settings.open("nope"));
  await expect(on).toHaveText(await page.locator(".settings-nav .nav-item").first().innerText());
});

test("badge: a space's entry shows its count, not when it is zero, also on the rail; one that throws is noted", SHELL, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await page.evaluate(() => {
    const { app } = (window as any).harnix;
    const t = ((window as any).__t = { count: 0 });
    app.registry.register("space", { id: "counted", icon: "chat", title: () => "Counted", sessions: "sessions", badge: () => t.count }, "test");
    app.registry.register(
      "space",
      { id: "broken", icon: "chat", title: () => "Broken", sessions: "sessions", badge: () => { throw new Error("badge broke"); } },
      "test",
    );
  });
  const entry = page.locator('#side [data-space="counted"]');
  await expect(entry).toBeVisible();
  await expect(entry.getByTestId("space-badge")).toHaveCount(0);
  await page.evaluate(() => {
    (window as any).__t.count = 1;
    (window as any).harnix.host.redraw();
  });
  await expect(entry.getByTestId("space-badge")).toHaveText("1");
  await expect(entry.getByTestId("space-badge")).toHaveAttribute("aria-label", /Counted.*1/);
  await page.locator("#side .collapse").click();
  await expect(page.locator("#frame")).toHaveAttribute("data-sidebar", "collapsed");
  await expect(entry.getByTestId("space-badge")).toBeVisible();
  await expect(page.locator('#side [data-space="broken"]')).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => (window as any).harnix.app.registry.refusals().some((r: any) => r.id === "broken" && /badge broke/.test(r.reason))))
    .toBe(true);
  expect(errors).toEqual([]);
});

/** A space over the session space whose conversation-facing functions read `window.__t`. */
async function conversationSpace(page: Page): Promise<void> {
  await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    const React = await import("react");
    const t = ((window as any).__t = { readOnly: "", pinned: "", clicked: 0, opened: [] as string[] });
    app.registry.register(
      "space",
      {
        id: "talk",
        icon: "chat",
        title: () => "Talk",
        sessions: "sessions",
        notice: () => "first-screen notice",
        readOnly: (s: { sessionId: string }) => (s.sessionId === t.readOnly ? "Archived" : false),
        pinned: (s: { sessionId: string }) =>
          s.sessionId === t.pinned
            ? React.createElement(
                "div",
                { "data-testid": "pinned-block" },
                "waiting on you ",
                React.createElement("button", { type: "button", "data-testid": "pinned-button", onClick: () => (t.clicked += 1) }, "Approve"),
              )
            : null,
        composerNotice: () => "do not paste keys",
        ids: () => ({ pattern: /\bfixture\b/, onOpen: (id: string) => t.opened.push(id) }),
      },
      "test",
    );
  });
}

test("readOnly: a session the space calls read-only has no composer, only the words", SHELL, async ({ page }) => {
  await open(page, URL);
  await conversationSpace(page);
  await inSpace(page, "talk");
  const archived = await newSession(page, "says-ready");
  const open_ = await newSession(page, "says-ready");
  await page.evaluate((id) => ((window as any).__t.readOnly = id), archived);
  await page.evaluate((id) => (window as any).harnix.app.open({ instance: "system", sessionId: id }), archived);
  await expect(page.getByTestId("conv-readonly")).toContainText("Archived");
  await expect(page.getByTestId("conv-input")).toHaveCount(0);
  await page.evaluate((id) => (window as any).harnix.app.open({ instance: "system", sessionId: id }), open_);
  await expect(page.getByTestId("conv-input")).toBeVisible();
  await expect(page.getByTestId("conv-readonly")).toHaveCount(0);
});

test("pinned: a session's block stays above the flow and calls its space; other sessions have none", SHELL, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await conversationSpace(page);
  await inSpace(page, "talk");
  const other = await newSession(page, "says-ready");
  const pinned = await newSession(page, "says-ready");
  await page.evaluate((id) => ((window as any).__t.pinned = id), pinned);
  await page.evaluate(() => (window as any).harnix.host.redraw());
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  const block = page.getByTestId("conv-pinned");
  await expect(block).toContainText("waiting on you");
  const [top, first] = await Promise.all([block.boundingBox(), page.locator('[data-testid="conv-message"]').first().boundingBox()]);
  expect(top!.y).toBeLessThan(first!.y);
  await page.getByTestId("pinned-button").click();
  expect(await page.evaluate(() => (window as any).__t.clicked)).toBe(1);
  await page.evaluate((id) => (window as any).harnix.app.open({ instance: "system", sessionId: id }), other);
  await expect(page.getByTestId("conv-input")).toBeVisible();
  await expect(page.getByTestId("conv-pinned")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("composerNotice: the open session's composer has the space's line; the first screen keeps notice()", SHELL, async ({ page }) => {
  await open(page, URL);
  await conversationSpace(page);
  await inSpace(page, "talk");
  await expect(page.locator("#main .disc")).toHaveText("first-screen notice");
  await expect(page.getByTestId("space-notice")).toHaveCount(0);
  await newSession(page, "says-ready");
  await expect(page.getByTestId("space-notice")).toHaveText("do not paste keys");
  // A space that gives none adds nothing.
  await inSpace(page, "sessions");
  await newSession(page, "says-ready");
  await expect(page.getByTestId("space-notice")).toHaveCount(0);
});

test("ids: a pattern in the reply's text is a chip the space opens; without ids it is text", SHELL, async ({ page }) => {
  await open(page, URL);
  await conversationSpace(page);
  await inSpace(page, "talk");
  await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  const chip = reply(page).getByTestId("conv-chip");
  await expect(chip.first()).toHaveText(/fixture/i, { timeout: 30_000 });
  await chip.first().click();
  expect(await page.evaluate(() => (window as any).__t.opened)).toEqual(["fixture"]);
  // The pattern came without `g`: the shell added it and said so.
  expect(
    await page.evaluate(() => (window as any).harnix.app.registry.refusals().some((r: any) => r.id === "talk" && r.point === "space")),
  ).toBe(true);
  await inSpace(page, "sessions");
  await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  await expect(reply(page).getByTestId("conv-chip")).toHaveCount(0);
});

test("composer.steer: a space that asks for it has a deliver-now switch, and a message sent with it goes into the running turn", SHELL, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "says-ready");
  await expect(page.getByTestId("conv-steer")).toHaveCount(0);
  await page.evaluate(() => {
    const { app } = (window as any).harnix;
    app.registry.register("space", { id: "steered", icon: "chat", title: () => "Steered", sessions: "sessions", composer: { steer: true } }, "test");
  });
  await inSpace(page, "steered");
  await newSession(page, "asks-permission");
  const steer = page.getByTestId("conv-steer");
  await expect(steer).toBeDisabled();
  await say(page, "Use the Bash tool to run exactly: touch harnix-permission-fixture");
  await expect(page.getByTestId("conv-ask")).toBeVisible({ timeout: 30_000 });
  await expect(steer).toBeEnabled();
  await steer.click();
  await expect(steer).toHaveAttribute("aria-checked", "true");
  // What the composer sends, as the client sends it.
  await page.evaluate(() => {
    const { client } = (window as any).harnix;
    const t = ((window as any).__t = { busy: [] as string[] });
    const call = client.call.bind(client);
    client.call = (method: string, params: any) => {
      if (method === "harnix.send") t.busy.push(params.on_busy);
      return call(method, params);
    };
  });
  await say(page, "and this now");
  expect(await page.evaluate(() => (window as any).__t.busy)).toEqual(["steer"]);
  // Not queued behind the turn: it goes into it, once the step it waits on is over.
  await expect(page.getByTestId("conv-queue-item")).toHaveCount(0);
  await page.getByTestId("conv-ask").getByTestId("conv-ask-allow").click();
  await expect(page.locator('[data-testid="conv-message"]', { hasText: "and this now" })).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("conv-queue-item")).toHaveCount(0);
  // Delivered into the turn (user_message_delivered), not run as a turn of its own.
  const steered = await page.evaluate(
    () => (window as any).harnix.app.controller.getSnapshot().blocks.find((b: any) => b.kind === "user" && b.text === "and this now")?.steered,
  );
  expect(steered).toBe(true);
});

test("model_switch: without the system space the composer changes the model through harnix.sessions.setModel", SHELL, async ({ page }) => {
  const backend = await startHarnix(launchFor("replay", { profile: "[shell]\nspaces = [\"sessions\"]\nmodel_switch = true\n" }));
  try {
    await open(page, backend.url);
    const reachable = await page.evaluate(() => (window as any).harnix.app.snapshot.reachable as string[]);
    expect(reachable).toContain("harnix.sessions.setModel");
    expect(reachable).not.toContain("harnix.system.model");
    await newSession(page, "says-ready");
    await page.getByTestId("conv-model").click();
    await page.getByTestId("conv-model-custom").fill("claude-switched-model");
    await page.getByTestId("conv-model-custom").press("Enter");
    await expect(page.getByTestId("conv-model")).toContainText("claude-switched-model");
  } finally {
    await backend.dispose();
  }
  const without = await startHarnix(launchFor("replay", { profile: "[shell]\nspaces = [\"sessions\"]\n" }));
  try {
    await open(page, without.url);
    await newSession(page, "says-ready");
    await page.getByTestId("conv-model").click();
    await expect(page.getByTestId("conv-model-custom")).toHaveCount(0);
  } finally {
    await without.dispose();
  }
});

test("commands: one that runs instead of inserting, asked with the front space, and a space that hides an engine command", SHELL, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await page.evaluate(() => {
    const { app } = (window as any).harnix;
    const t = ((window as any).__t = { ran: [] as string[], asked: [] as string[] });
    app.registry.register(
      "command",
      {
        id: "probe.commands",
        complete: (_q: string, ctx: { space: string }) => {
          t.asked.push(ctx?.space);
          return [{ replacement: "/probe-run ", label: "/probe-run", description: "runs", run: (c: { space: string }) => t.ran.push(c.space) }];
        },
      },
      "test",
    );
    app.registry.register("space", { id: "quiet", icon: "chat", title: () => "Quiet", sessions: "sessions", hideCommands: ["clear"] }, "test");
  });
  const names = async () => {
    const input = page.getByTestId("conv-input");
    await input.press("ControlOrMeta+a");
    await input.press("Backspace");
    await input.pressSequentially("/");
    const menu = page.getByTestId("conv-menu");
    await expect(menu).toBeVisible();
    return menu.getByTestId("conv-menu-item").evaluateAll((nodes) => nodes.map((n) => (n as HTMLElement).dataset.id));
  };
  await inSpace(page, "quiet");
  await newSession(page, "says-ready");
  const inQuiet = await names();
  expect(inQuiet).toContain("probe-run");
  expect(inQuiet).not.toContain("clear");
  await page.getByTestId("conv-menu").locator('[data-id="probe-run"]').click();
  await expect(page.getByTestId("conv-input")).toHaveValue("");
  await expect.poll(() => page.evaluate(() => (window as any).__t.ran)).toEqual(["quiet"]);
  expect(await page.evaluate(() => (window as any).__t.asked)).toContain("quiet");
  await inSpace(page, "sessions");
  await newSession(page, "says-ready");
  expect(await names()).toContain("clear");
  expect(errors).toEqual([]);
});

test("@ takes an application's candidates after the shell's own, and the / menu does not", SHELL, async ({ page }) => {
  await open(page, URL);
  await page.evaluate(() => {
    const { app } = (window as any).harnix;
    app.registry.register(
      "command",
      { id: "probe.docs", trigger: "@", complete: (q: string) => (q === "ab" ? [{ replacement: "doc:ab ", label: "doc ab", description: "a document" }] : []) },
      "test",
    );
    app.registry.register("command", { id: "probe.broken", trigger: "@", complete: () => { throw new Error("mentions broke"); } }, "test");
  });
  await newSession(page, "says-ready");
  const input = page.getByTestId("conv-input");
  await input.click();
  await input.pressSequentially("see @ab");
  const item = page.getByTestId("conv-menu").locator('[data-id="doc ab"]');
  await expect(item).toBeVisible();
  await item.click();
  await expect(input).toHaveValue("see doc:ab ");
  await input.press("ControlOrMeta+a");
  await input.press("Backspace");
  await expect(input).toHaveValue("");
  await input.pressSequentially("/");
  await expect(page.getByTestId("conv-menu")).toBeVisible();
  expect(await page.getByTestId("conv-menu").locator('[data-id="doc ab"]').count()).toBe(0);
  expect(
    await page.evaluate(() => (window as any).harnix.app.registry.refusals().some((r: any) => r.id === "probe.broken" && /mentions broke/.test(r.reason))),
  ).toBe(true);
});

test("icons: a space's own icon, by its owner's name, drawn in the shell's frame at the size of the others", SHELL, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await page.evaluate(() => {
    const { app } = (window as any).harnix;
    app.registry.icons({ ship: "<path d='M3 3h18'/>" }, "test");
    app.registry.register("space", { id: "shipped", icon: "test.ship", title: () => "Shipped", sessions: "sessions" }, "test");
  });
  const glyph = page.locator('#side [data-space="shipped"] .glyph');
  await expect(glyph.locator('svg path[d="M3 3h18"]')).toHaveCount(1);
  await expect(glyph.locator("svg")).toHaveAttribute("stroke-width", "1.7");
  const size = async (selector: string) => {
    const box = (await page.locator(selector).boundingBox())!;
    return [Math.round(box.width), Math.round(box.height)];
  };
  expect(await size('#side .spaces [data-space="shipped"] .glyph')).toEqual(await size('#side .spaces [data-space="sessions"] > .glyph'));
  expect(errors).toEqual([]);
});

test("after the backend restarts on its port with a new token, the open page reconnects by itself", { tag: ["@feature:connection", "@layer:ui"] }, async ({ page }) => {
  const first = await startHarnix(launchFor("replay"));
  let backend = first;
  try {
    await open(page, first.url);
    backend = await first.restart();
    expect(backend.token).not.toBe(first.token);
    // The page now speaks with the new process's token — the old one is refused — without a reload.
    await expect.poll(() => page.evaluate(() => (window as any).harnix.client.token), { timeout: 30_000 }).toBe(backend.token);
    await expect(page.locator("#conn")).toHaveAttribute("data-state", "open", { timeout: 30_000 });
    await newSession(page, "says-ready");
    await say(page, "Say exactly: fixture ready");
    await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  } finally {
    await backend.dispose();
  }
});
