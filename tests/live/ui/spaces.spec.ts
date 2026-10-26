/**
 * Spaces in the browser (design §8), against `examples/hello-app` — an application built the way
 * applications are, with two spaces of its own beside Harnix's built-in ones:
 *
 *   greeter  a tab and a home card, a backend module, its own in-process instance
 *   notes    interface and Agent only, on `system`
 *
 * The invariant under test is the correspondence of §8.5: whatever space is in front, the centre,
 * the composer and the right column show and act on that space's own session, and coming back to
 * a space restores the one it had.
 */

import { existsSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { type Backend, ROOT, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, reply, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:spaces", "@layer:ui"] };
const PROJECTS = { tag: ["@feature:projects", "@layer:ui"] };
const BRAND = { tag: ["@feature:brand", "@layer:ui"] };
const MODULE = { tag: ["@feature:modules", "@layer:ui"] };

const APP = join(ROOT, "examples", "hello-app");
const BIN = join(APP, "target", "release", "hello-app");
const UI = join(APP, "target", "ui");

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  test.skip(!existsSync(BIN) || !existsSync(join(UI, "index.html")), "examples/hello-app is not built: examples/hello-app/scripts/build.sh");
  backend = await startHarnix(launchFor("replay", { bin: BIN, uiDir: UI }));
});
test.afterAll(() => backend?.stop());

async function inSpace(page: Page, space: string): Promise<void> {
  await page.locator(`#side .spaces [data-space="${space}"]`).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", space);
}

/** Start a session in the front space that replays `says-ready`, and return its key. */
async function startReplaying(page: Page): Promise<{ instance: string; sessionId: string }> {
  return page.evaluate(async () => {
    const { app } = (window as any).harnix;
    return app.create({ options: { recorder: { name: "says-ready" } } });
  });
}

test("the application's spaces are listed, each with its own title", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const ids = await page.locator("#side .spaces .space").evaluateAll((nodes) => nodes.map((n) => (n as HTMLElement).dataset.space));
  expect(ids).toEqual(["sessions", "projects", "greeter", "notes"]);
  // The system space is entered from the bottom, beside the avatar.
  await expect(page.locator('#side .l-bottom [data-space="system"]')).toBeVisible();
  await inSpace(page, "greeter");
  await expect(page.getByTestId("space-pill")).toContainText("Greeter");
  expect(errors).toEqual([]);
});

test("a space's session runs on the space's own instance", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await inSpace(page, "greeter");
  const key = await startReplaying(page);
  expect(key.instance).toBe("space-greeter");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  const owner = await page.evaluate(async () => {
    const { instances } = await (window as any).harnix.app.call("harnix.instances.list", {});
    return instances.find((i: any) => i.id === "space-greeter")?.owner;
  });
  expect(owner).toBe("space:greeter");
});

test("each space keeps its own session and draft; coming back restores them", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await inSpace(page, "greeter");
  const greeter = await startReplaying(page);
  await page.getByTestId("conv-input").fill("half a greeting");

  await inSpace(page, "notes");
  const notes = await startReplaying(page);
  expect(notes.instance).toBe("system");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });

  await inSpace(page, "greeter");
  await expect(page.getByTestId("conv-input")).toHaveValue("half a greeting");
  await expect(page.locator(`#side .srow[data-sid="${greeter.sessionId}"]`)).toHaveClass(/on/);
  // The notes message is not here.
  await expect(page.locator('[data-testid="conv-message"]')).toHaveCount(0);

  await inSpace(page, "notes");
  await expect(page.locator(`#side .srow[data-sid="${notes.sessionId}"]`)).toHaveClass(/on/);
  await expect(reply(page)).toContainText(/fixture ready/i);
});

test("a deleted current session falls back to its own space's list, not another's", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await inSpace(page, "notes");
  const notes = await startReplaying(page);
  await page.evaluate(async (key) => {
    const { app } = (window as any).harnix;
    await app.call("session.delete", { instance: key.instance, session_id: key.sessionId, space: "notes" });
  }, notes);
  await inSpace(page, "greeter");
  await inSpace(page, "notes");
  await expect(page.locator("#hero")).toBeVisible();
});

test("a session cannot be reached from another space", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await inSpace(page, "greeter");
  const greeter = await startReplaying(page);
  const refused = await page.evaluate(async (key) => {
    const { client } = (window as any).harnix;
    try {
      await client.call("harnix.attach", { instance: key.instance, session_id: key.sessionId, space: "notes" });
      return null;
    } catch (e: any) {
      return { code: e.code, message: e.message };
    }
  }, greeter);
  expect(refused?.message).toContain("greeter");
});

test("two tabs in two spaces do not cross", TAGS, async ({ page, context }) => {
  await open(page, backend.url);
  await inSpace(page, "greeter");
  const greeter = await startReplaying(page);
  const other = await context.newPage();
  await open(other, backend.url);
  await inSpace(other, "notes");
  const notes = await startReplaying(other);
  await say(other, "Say exactly: fixture ready");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  await expect(reply(other)).toContainText(/fixture ready/i, { timeout: 30_000 });
  const lists = await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    const ids = async (space: string) => (await app.call("harnix.sessions", { space })).sessions.map((s: any) => s.session_id);
    return { greeter: await ids("greeter"), notes: await ids("notes") };
  });
  expect(lists.greeter).toContain(greeter.sessionId);
  expect(lists.greeter).not.toContain(notes.sessionId);
  expect(lists.notes).toContain(notes.sessionId);
  expect(lists.notes).not.toContain(greeter.sessionId);
  await other.close();
});

test("a space's tab calls its module, and the home cards under a new conversation show what it published", MODULE, async ({ page }) => {
  await open(page, backend.url);
  await inSpace(page, "greeter");
  await page.getByTestId("details-toggle").click();
  await page.locator('#details .tabs [data-tab="greet"]').click();
  await page.getByTestId("greet").click();
  await expect(page.getByTestId("greeting")).toContainText(/hello, world \(\d+\)/);
  await inSpace(page, "sessions");
  await page.locator("#new").click();
  await expect(page.getByTestId("home-cards")).toBeVisible();
  await expect(page.getByTestId("greeted-count")).toContainText(/[1-9]\d* greeted/);
});

test("the projects space lists the session space's conversations by project, one project open at a time", PROJECTS, async ({ page }) => {
  await open(page, backend.url);
  const roots = await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    const made: string[] = [];
    for (const name of [`alpha-${Date.now()}`, `beta-${Date.now()}`]) made.push((await app.call("harnix.projects.create", { name })).path);
    await app.refreshSessions();
    return made;
  });
  await inSpace(page, "projects");
  const projects = page.getByTestId("projects");
  const alpha = projects.locator(".project", { hasText: "alpha-" });
  const beta = projects.locator(".project", { hasText: "beta-" });
  await alpha.locator(".prj").click();
  await expect(alpha).toHaveAttribute("data-open", "true");
  // A conversation started here runs in the open project, and is listed under it.
  await page.getByTestId("conv-input").fill("Say exactly: fixture ready");
  await page.getByTestId("conv-input").press("Enter");
  await expect(alpha.locator(".pgroup .srow")).toHaveCount(1, { timeout: 15_000 });
  const root = await page.evaluate(() => {
    const { app } = (window as any).harnix;
    const key = app.snapshot.current;
    return app.snapshot.sessions.find((s: any) => s.session_id === key.sessionId)?.project_root;
  });
  expect(roots).toContain(root);
  // Opening the other closes this one.
  await beta.locator(".prj").click();
  await expect(alpha).toHaveAttribute("data-open", "false");
  await expect(alpha.locator(".pgroup")).toHaveCount(0);
});

test("brand: the product's name, icon, colours and words, without a fork of ui/", BRAND, async ({ page }) => {
  const branded = await startHarnix(
    launchFor("replay", {
      bin: BIN,
      uiDir: UI,
      profile: `\n[brand]\nname = "Hello"\nproduct_id = "hello-app"\nicon = "${join(APP, "profiles", "brand.svg")}"\n`,
    }),
  );
  try {
    const errors = watchConsole(page);
    await open(page, branded.url);
    await expect(page).toHaveTitle("Hello");
    await expect(page.getByTestId("brand-name")).toHaveText("Hello");
    await expect(page.locator("#side .brand img.brand-icon")).toHaveAttribute("src", "/brand/icon.svg");
    const accent = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--accent").trim());
    expect(accent.toLowerCase()).toBe("#2f6fb3");
    await expect(page.locator('#side .spaces [data-space="greeter"]')).toContainText("Greeter");
    // The module's language pack follows the language.
    await page.getByTestId("avatar").click();
    await page.getByTestId("menu-language").click();
    const zh = await page.evaluate(() => document.documentElement.lang);
    await expect(page.locator('#side .spaces [data-space="greeter"]')).toContainText(zh === "en" ? "Greeter" : "问候");
    expect(branded.state.endsWith("/.hello-app")).toBe(true);
    expect(errors).toEqual([]);
  } finally {
    branded.stop();
  }
});

const ENTRIES = { tag: ["@feature:entries", "@layer:ui"] };
const ROUTES = { tag: ["@feature:routes", "@layer:ui"] };

test("entry-card: a long job's progress arrives on its topic, and its result is a card in the conversation", ENTRIES, async ({ page }) => {
  await open(page, backend.url);
  await inSpace(page, "greeter");
  const key = await startReplaying(page);
  await page.getByTestId("details-toggle").click();
  await page.locator('#details .tabs [data-tab="greet"]').click();
  await page.getByTestId("job-start").click();
  await expect(page.getByTestId("job-progress")).toHaveAttribute("data-finished", "true", { timeout: 15_000 });
  await expect(page.getByTestId("job-progress")).toContainText("20/20");
  const card = page.getByTestId("job-done-card");
  await expect(card).toContainText("counted to 20");

  // It is in the session's log: a refresh draws it again, once.
  await page.reload();
  await expect(page.locator("#conn.on")).toBeVisible({ timeout: 15_000 });
  await expect(page.locator("#main")).toHaveAttribute("data-space", "greeter");
  await expect(page.getByTestId("job-done-card")).toHaveCount(1);

  // And it is not the model's.
  const context = await page.evaluate(
    (key) => (window as any).harnix.app.call("session.history", { instance: key.instance, session_id: key.sessionId, space: "greeter" }),
    key,
  );
  expect(JSON.stringify(context.messages)).not.toContain("counted to");
});

test("a module's own link: its route and its socket, with the host's credential", ROUTES, async ({ page }) => {
  await open(page, backend.url);
  await inSpace(page, "greeter");
  await page.getByTestId("details-toggle").click();
  await page.locator('#details .tabs [data-tab="greet"]').click();
  await page.getByTestId("route-fetch").click();
  await expect(page.getByTestId("route-said")).toHaveText("200 hello from the greeter's own route");
  await page.getByTestId("route-echo").click();
  await expect(page.getByTestId("route-echoed")).toHaveText("echo: ping");
  // A path cannot leave the module's prefix.
  const escaped = await page.evaluate(async () => {
    try {
      await (window as any).harnix.host.space("greeter").fetch("../../ws");
      return "fetched";
    } catch (e: any) {
      return e.message;
    }
  });
  expect(escaped).toContain("not a path of module greeter");
});

const APPROVAL = { tag: ["@feature:entries", "@layer:ui"] };

/** Ask for approval in the greeter's open session, as its tab does, and return the ticket. */
async function askApproval(page: Page, timeoutMs?: number): Promise<{ id: string }> {
  return page.evaluate(async (timeout_ms) => {
    const { app } = (window as any).harnix;
    const current = app.snapshot.currentBySpace.greeter;
    return app.call("app.greeter.request_approval", {
      instance: current.instance,
      session_id: current.sessionId,
      title: "Greet everyone at once",
      ...(timeout_ms ? { timeout_ms } : {}),
    });
  }, timeoutMs);
}

test("an approval: two people see one card, and the first answer stands", APPROVAL, async ({ page, context }) => {
  await open(page, backend.url);
  await inSpace(page, "greeter");
  const key = await startReplaying(page);
  const other = await context.newPage();
  await open(other, backend.url);
  await inSpace(other, "greeter");
  await other.evaluate((key) => (window as any).harnix.app.open(key), key);

  await page.getByTestId("details-toggle").click();
  await page.locator('#details .tabs [data-tab="greet"]').click();
  await page.getByTestId("approval-request").click();
  const mine = page.getByTestId("approval-card");
  const theirs = other.getByTestId("approval-card");
  await expect(mine).toHaveAttribute("data-state", "pending");
  await expect(theirs).toHaveAttribute("data-state", "pending");

  await theirs.getByTestId("approval-approve").click();
  await expect(mine).toHaveAttribute("data-state", "approved");
  await expect(theirs).toHaveAttribute("data-state", "approved");
  // The decision is written down once, where both can see it.
  await expect(page.getByTestId("approval-decided")).toHaveCount(1);
  await expect(other.getByTestId("approval-decided")).toHaveCount(1);

  // Answering again is refused, and nothing changes.
  const ticket = await mine.getAttribute("data-ticket");
  const late = await page.evaluate(async (ticket) => {
    try {
      await (window as any).harnix.app.call("app.greeter.approve", { ticket, decision: "reject" });
      return null;
    } catch (e: any) {
      return e.message;
    }
  }, ticket);
  expect(late).toContain("already approved");
  await expect(mine).toHaveAttribute("data-state", "approved");
  await other.close();
});

test("an approval nobody answers is a no, and a module cannot answer for a person", APPROVAL, async ({ page }) => {
  await open(page, backend.url);
  await inSpace(page, "greeter");
  await startReplaying(page);
  const pending = await askApproval(page);
  const byModule = await page.evaluate(
    (ticket) => (window as any).harnix.app.call("app.greeter.answer_for_a_person", { ticket }),
    pending.id,
  );
  expect(byModule.refused, JSON.stringify(byModule)).toBeTruthy();
  await expect(page.locator(`[data-testid="approval-card"][data-ticket="${pending.id}"]`)).toHaveAttribute("data-state", "pending");

  const quick = await askApproval(page, 300);
  const card = page.locator(`[data-testid="approval-card"][data-ticket="${quick.id}"]`);
  await expect(card).toHaveAttribute("data-state", "rejected", { timeout: 10_000 });
  await expect(card.getByTestId("approval-outcome")).toHaveText("rejected by timeout");

  // Still there, decided, after a refresh: the card and its decision are in the session's log.
  await page.reload();
  await expect(page.locator("#conn.on")).toBeVisible({ timeout: 15_000 });
  await expect(page.locator(`[data-testid="approval-card"][data-ticket="${quick.id}"]`)).toHaveAttribute("data-state", "rejected");
  await expect(page.locator(`[data-testid="approval-decided"][data-ticket="${quick.id}"]`)).toHaveCount(1);
});
