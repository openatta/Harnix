/**
 * What a person does to a session and a project from their menus, and the answers to an ask other
 * than "allow once" — against a real backend, turns replayed.
 *
 * The session menu opens on a right click of a row; the project menu on a right click of a project
 * in the projects space. Their items are found by `data-item`, not by position or wording.
 */

import { readFileSync } from "node:fs";
import type { Page } from "@playwright/test";
import { expect, test } from "@harnix/testkit";

import { newSession, open, reply, say } from "./page";

const SESSIONS = { tag: ["@feature:sessions", "@layer:ui"] };
const SPACE = { tag: ["@feature:session-space", "@layer:ui"] };
const PROJECTS = { tag: ["@feature:projects", "@layer:ui"] };

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});

const row = (page: Page, id: string) => page.locator(`#side .srow[data-sid="${id}"]`);

/** Right-click a session's row and pick an item of its menu. */
async function fromMenu(page: Page, id: string, item: string): Promise<void> {
  await row(page, id).click({ button: "right" });
  await page.locator(`#modal .menu-list button[data-item="${item}"]`).click();
}

/** A session that has had one replayed turn, listed in the sidebar. */
async function withTurn(page: Page): Promise<string> {
  const id = await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  await expect(row(page, id)).toBeVisible();
  return id;
}

test("denying an ask answers it, and the call it was about is not run", SESSIONS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "asks-permission");
  await say(page, "Use the Bash tool to run exactly: touch harnix-permission-fixture");
  const ask = page.getByTestId("conv-ask");
  await expect(ask).toBeVisible({ timeout: 30_000 });
  await ask.getByTestId("conv-ask-deny").click();
  await expect(ask).toHaveAttribute("data-state", "answered");
  await expect(ask.getByTestId("conv-ask-deny")).toHaveCount(0);
  const tool = page.locator('[data-testid="conv-tool"]').first();
  await expect(tool).toHaveAttribute("data-state", "error", { timeout: 30_000 });
  await expect(page.getByTestId("conv-send")).toBeVisible({ timeout: 30_000 });
});

test("allowing an ask for the session answers it, and the call runs", SESSIONS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "asks-permission");
  await say(page, "Use the Bash tool to run exactly: touch harnix-permission-fixture");
  const ask = page.getByTestId("conv-ask");
  await expect(ask).toBeVisible({ timeout: 30_000 });
  await ask.getByTestId("conv-ask-allow-session").click();
  await expect(ask).toHaveAttribute("data-state", "answered");
  await expect(page.locator('[data-testid="conv-tool"]').first()).toHaveAttribute("data-state", "ok", { timeout: 30_000 });
});

test("a session archived from its menu leaves the list, comes back behind the toggle, and is unarchived", SPACE, async ({ page }) => {
  await open(page, URL);
  const id = await withTurn(page);
  await fromMenu(page, id, "archive");
  await expect(row(page, id)).toHaveCount(0);
  const toggle = page.getByTestId("archive-toggle");
  await expect(toggle).toBeVisible();
  await toggle.click();
  await expect(row(page, id)).toBeVisible();
  // Unarchived, it is an ordinary row again: nothing is left behind the toggle.
  await fromMenu(page, id, "archive");
  await expect(toggle).toHaveCount(0);
  await expect(row(page, id)).toBeVisible();
});

test("a fork from the menu is a new session with the same conversation, and opens", SPACE, async ({ page }) => {
  await open(page, URL);
  const id = await withTurn(page);
  await fromMenu(page, id, "fork");
  await expect.poll(() => page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId)).not.toBe(id);
  const forked = await page.evaluate(() => (window as any).harnix.app.snapshot.current.sessionId as string);
  await expect(row(page, forked)).toBeVisible();
  await expect(row(page, id)).toBeVisible();
  await expect(page.locator('[data-testid="conv-message"]').first()).toContainText("Say exactly: fixture ready");
});

test("delete asks first; cancelled it keeps the session, confirmed it removes it", SPACE, async ({ page }) => {
  await open(page, URL);
  const id = await withTurn(page);
  await fromMenu(page, id, "delete");
  await expect(page.getByTestId("confirm")).toBeVisible();
  await page.locator("#modal .btn.outline").first().click();
  await expect(row(page, id)).toBeVisible();

  await fromMenu(page, id, "delete");
  await page.getByTestId("confirm").click();
  await expect(row(page, id)).toHaveCount(0);
  const gone = await page.evaluate(async (sid) => {
    const { app } = (window as any).harnix;
    try {
      await app.call("session.get", { instance: "system", session_id: sid });
      return false;
    } catch {
      return true;
    }
  }, id);
  expect(gone, "the engine still has the session").toBe(true);
});

test("a new session from a project's menu: the dialog offers the active scenes, and the session lands in that project", PROJECTS, async ({ page }) => {
  await open(page, URL);
  const path = await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    const made = await app.call("harnix.projects.create", { name: `dialog-${Date.now()}` });
    await app.refreshSessions();
    return made.path as string;
  });
  await page.locator('#side .spaces [data-space="projects"]').click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
  const project = page.getByTestId("projects").locator(".project", { hasText: "dialog-" });
  await project.locator(".prj").click({ button: "right" });
  await page.locator('#modal .menu-list button[data-item="new"]').click();

  const scene = page.getByTestId("new-scene");
  await expect(scene).toHaveValue("harnix");
  const offered = await scene.locator("option").evaluateAll((os) => os.map((o) => (o as HTMLOptionElement).value));
  expect(offered).toContain("harnix");
  await expect(page.getByTestId("new-root")).toContainText(path);
  await page.getByTestId("new-mode").selectOption({ index: 0 });
  await page.getByTestId("new-create").click();

  // The dialog closes on the new session, opened and listed.
  await expect(page.locator("#modal")).toHaveCount(0);
  const made = () =>
    page.evaluate(() => {
      const { app } = (window as any).harnix;
      const key = app.snapshot.current;
      const row = key && app.snapshot.sessions.find((s: any) => s.session_id === key.sessionId);
      return row ? { project_root: row.project_root, scene: row.scene } : null;
    });
  await expect.poll(made, { timeout: 15_000 }).toEqual({ project_root: path, scene: "harnix" });
});

test("a project's session is renamed and deleted from its menu in the projects space, which claims the session space", PROJECTS, async ({ page }) => {
  await open(page, URL);
  await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    await app.call("harnix.projects.create", { name: `menu-${Date.now()}` });
    await app.refreshSessions();
  });
  await page.locator('#side .spaces [data-space="projects"]').click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
  const project = page.getByTestId("projects").locator(".project", { hasText: "menu-" });
  await project.locator(".prj").click({ button: "right" });
  await page.locator('#modal .menu-list button[data-item="new"]').click();
  await page.getByTestId("new-mode").selectOption({ index: 0 });
  await page.getByTestId("new-create").click();
  await expect(page.locator("#modal")).toHaveCount(0);
  const id = await page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId as string);
  expect(id).toBeTruthy();
  if ((await project.getAttribute("data-open")) !== "true") await project.locator(".prj").click();
  await expect(row(page, id)).toBeVisible();

  // The row is the session space's session: its menu claims that space, not the projects one.
  await fromMenu(page, id, "rename");
  await page.locator("#modal input").fill("renamed in a project");
  await page.locator("#modal .btn.primary").click();
  await expect(row(page, id)).toContainText("renamed in a project");
  await expect(page.locator("#banner")).not.toContainText(/SPACE_MISMATCH|another space|-31003/);

  await fromMenu(page, id, "delete");
  await page.getByTestId("confirm").click();
  await expect(row(page, id)).toHaveCount(0);
});

test("a session is exported from its menu: markdown to read, or the engine's archive without the recording unless asked", SPACE, async ({ page }) => {
  await open(page, URL);
  const id = await withTurn(page);
  const download = async (pick: () => Promise<void>) => {
    await fromMenu(page, id, "export");
    await pick();
    const [file] = await Promise.all([page.waitForEvent("download"), page.getByTestId("export-go").click()]);
    const bytes = readFileSync((await file.path())!);
    return { name: file.suggestedFilename(), text: bytes.toString("utf8"), bytes };
  };

  const md = await download(async () => {
    await expect(page.getByTestId("export-markdown")).toBeChecked();
    await expect(page.getByTestId("export-recording"), "a recording only goes with an archive").toBeDisabled();
  });
  expect(md.name).toMatch(/\.md$/);
  expect(md.text).toContain("Say exactly: fixture ready");

  const archive = await download(async () => {
    await page.getByTestId("export-archive").check();
    await expect(page.getByTestId("export-recording")).toBeEnabled();
    await expect(page.getByTestId("export-recording")).not.toBeChecked();
  });
  expect(archive.name).toMatch(/\.zip$/);
  // A zip, as the engine wrote it: its manifest and the session's own transcript inside.
  expect(archive.bytes.subarray(0, 4).toString("binary")).toBe("PK\x03\x04");
  expect(archive.bytes.includes("manifest.json")).toBe(true);
  expect(archive.bytes.includes(`sessions/${id}/`)).toBe(true);
  await expect(page.locator("#modal")).toHaveCount(0);
});
