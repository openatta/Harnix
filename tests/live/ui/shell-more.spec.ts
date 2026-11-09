/**
 * The shell's own controls not covered elsewhere: the avatar's menu, a project added by name or by
 * picking its directory and then renamed and removed, the session menu from the title, the count of
 * running sessions on the system entry, and the note over the composer while no model is configured.
 *
 * Replayed: `says-ready`, `runs-a-while` (a turn that is still running four seconds on).
 */

import type { Page } from "@playwright/test";
import { type Backend, dropSessions, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { newSession, open, reply, say, settingsSection, spaceEntry, watchConsole } from "./page";

const SHELL = { tag: ["@feature:session-space", "@layer:ui"] };
const PROJECTS = { tag: ["@feature:projects", "@layer:ui"] };
const SETTINGS = { tag: ["@feature:settings", "@layer:ui"] };

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});
// The worker's backend is shared by every file that uses it: a test leaves no session behind
// for the next one to count.
test.afterEach(async ({ api }) => {
  await dropSessions(api);
});

async function toProjects(page: Page): Promise<void> {
  await spaceEntry(page, "projects").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
}

test("the avatar's menu cycles the theme and opens About", SHELL, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  const theme = () => page.evaluate(() => document.documentElement.dataset.theme);
  const seen = new Set<string | undefined>([await theme()]);
  for (let i = 0; i < 3; i += 1) {
    await page.getByTestId("avatar").click();
    await expect(page.getByTestId("avatar-menu")).toBeVisible();
    await page.getByTestId("menu-theme").click();
    await expect(page.getByTestId("avatar-menu")).toHaveCount(0);
    seen.add(await theme());
  }
  // System, light, dark: both drawn themes come round.
  expect([...seen].sort()).toEqual(["dark", "light"]);

  await page.getByTestId("avatar").click();
  await page.getByTestId("menu-about").click();
  await expect(settingsSection(page, "about")).toHaveAttribute("data-state", "current");
  expect(errors).toEqual([]);
});

test("a project added by name is listed; renamed from its menu it says so; removed it is gone", PROJECTS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await toProjects(page);
  const name = `added-${Date.now()}`;
  await page.getByTestId("add-project").click();
  await page.getByRole("dialog").getByRole("textbox").first().fill(name);
  await page.getByTestId("project-add").click();
  await expect(page.locator("#modal")).toHaveCount(0);
  const project = page.getByTestId("projects").locator("[data-project]", { hasText: name });
  await expect(project).toBeVisible();

  await project.getByRole("treeitem", { level: 1 }).click({ button: "right" });
  await page.locator('[data-testid="project-menu"] [data-item="rename"]').click();
  const renamed = `${name}-renamed`;
  await page.locator("#modal input").fill(renamed);
  await page.getByTestId("rename-save").click();
  const after = page.getByTestId("projects").locator("[data-project]", { hasText: renamed });
  await expect(after).toBeVisible();

  await after.getByRole("treeitem", { level: 1 }).click({ button: "right" });
  await page.locator('[data-testid="project-menu"] [data-item="remove"]').click();
  await expect(page.locator("#modal")).toHaveCount(0);
  await expect(page.getByTestId("projects").locator("[data-project]", { hasText: renamed })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("a project added by picking its directory: walked into, used, listed by its path", PROJECTS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  // A directory of the test project's to pick: made where the picker starts.
  const start = await page.evaluate(async () => (await (window as any).harnix.app.call("harnix.listDirectory", {})).path as string);
  const made = await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    return (await app.call("harnix.projects.create", { name: `picked-${Date.now()}` })).path as string;
  });
  // That made it a project already; take it off so the picker is what adds it back.
  await page.evaluate(async (path) => {
    const { app } = (window as any).harnix;
    const { workspaces } = await app.call("harnix.workspaces.list", {});
    const row = workspaces.find((w: any) => w.path === path);
    if (row) await app.call("harnix.workspaces.remove", { id: row.id });
    await app.refreshSessions();
  }, made);
  await toProjects(page);
  await expect(page.getByTestId("projects").locator("[data-project]", { hasText: made.split("/").pop()! })).toHaveCount(0);

  await page.getByTestId("add-project").click();
  const picker = page.locator("#modal .picker");
  // Down from where it starts to the directory, one entry at a time.
  const rest = made.startsWith(start) ? made.slice(start.replace(/\/$/, "").length + 1).split("/") : [];
  expect(rest.length, `${made} is under ${start}`).toBeGreaterThan(0);
  for (const part of rest) await picker.locator(".list .entry", { hasText: part }).first().click();
  await expect(picker.locator(".crumbs")).toContainText(rest[rest.length - 1]!);
  await page.getByTestId("picker-use").click();
  await expect(page.locator("#modal .picked")).toHaveText(made);
  await page.getByTestId("project-add").click();
  await expect(page.locator("#modal")).toHaveCount(0);
  await expect(page.getByTestId("projects").locator("[data-project]", { hasText: made.split("/").pop()! })).toBeVisible();
  expect(errors).toEqual([]);
});

test("the title opens the session's menu", SHELL, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  await page.getByTestId("head-title").click();
  const menu = page.getByTestId("session-menu");
  await expect(menu).toBeVisible();
  for (const item of ["rename", "fork", "archive", "export", "delete"]) await expect(menu.locator(`button[data-item="${item}"]`)).toBeVisible();
});

test("the system entry counts the running sessions while they run", SHELL, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "runs-a-while");
  const count = page.getByTestId("running-count");
  await expect(count).toHaveCount(0);
  await say(page, "Use the Bash tool to wait a few seconds");
  await expect(count).toHaveText("1", { timeout: 15_000 });
  await page.getByTestId("conv-ask").getByTestId("conv-ask-allow").click();
  await expect(count).toHaveCount(0, { timeout: 30_000 });
});

test.describe("without a model", () => {
  let bare: Backend;
  test.beforeAll(async () => {
    bare = await startHarnix(launchFor("replay", { env: { ANTHROPIC_API_KEY: "", ANTHROPIC_AUTH_TOKEN: "" } }));
  });
  test.afterAll(async () => {
    await bare?.dispose();
  });

  test("the composer says no model is set up, and its button opens Settings · Models", SETTINGS, async ({ page }) => {
    await open(page, bare.url);
    const note = page.getByTestId("model-setup");
    await expect(note).toBeVisible();
    // The model button agrees with the note: no model named, nothing to pick (review G8).
    const model = page.getByTestId("conv-model");
    await expect(model).toHaveAttribute("data-state", "unset");
    await expect(model).toBeDisabled();
    await expect(model).toHaveText(/Not configured|未配置/);
    await page.getByTestId("model-setup-open").click();
    await expect(settingsSection(page, "models")).toHaveAttribute("data-state", "current");
    await expect(page.getByTestId("providers-none")).toBeVisible();
  });
});
