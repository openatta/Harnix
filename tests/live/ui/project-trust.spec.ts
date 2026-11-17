/**
 * A project's trust in the projects space (docs/harnix-app.md §3): its menu offers to trust it and
 * says first what that lets in; a trusted project carries a shield; a conversation open in a
 * project that is not trusted has a banner over it offering to — closed, it stays closed for that
 * project. No turn is run: nothing here needs a model.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { dropSessions, expect, test } from "@harnix/testkit";

import { open, sessionRow, spaceEntry } from "./page";

const PROJECTS = { tag: ["@feature:projects", "@layer:ui"] };

let URL = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
});
test.afterEach(async ({ api }) => {
  await dropSessions(api);
});

/** A new project whose own settings wait on trust (a looser permission mode, an MCP server of its
 *  own), shown in the projects space: its directory, and its row. */
async function untrustedProject(page: Page, name: string) {
  const path = await page.evaluate(async (name) => {
    const { app } = (window as any).harnix;
    return (await app.call("harnix-app.projects.create", { name })).path as string;
  }, `${name}-${Date.now()}`);
  mkdirSync(join(path, ".harnix"), { recursive: true });
  writeFileSync(join(path, ".harnix", "settings.json"), JSON.stringify({ permission_mode: "bypassPermissions", mcp_servers: { demo: { command: "true" } } }));
  await page.evaluate(() => (window as any).harnix.app.refreshSessions());
  await spaceEntry(page, "projects").click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
  const project = page.getByTestId("projects").locator("[data-project]", { hasText: name });
  return { path, project };
}

const menuItem = (page: Page, item: string) => page.locator(`[data-testid="host-menu"] [data-item="${item}"]`);

test("a project is trusted from its menu after being told what that lets in, and carries a shield until it is no longer", PROJECTS, async ({ page }) => {
  await open(page, URL);
  const { path, project } = await untrustedProject(page, "trust-menu");
  await expect(project.getByTestId("project-trusted")).toHaveCount(0);

  await project.getByRole("treeitem", { level: 1 }).click({ button: "right" });
  await expect(menuItem(page, "untrust")).toHaveCount(0);
  await menuItem(page, "trust").click();

  // What trust lets in, by kind, before anything is written.
  await expect(page.getByTestId("trust-intro")).toContainText(path);
  await expect(page.getByTestId("trust-categories").locator("li")).toHaveCount(2);
  await expect(page.getByTestId("trust-categories").locator('[data-category="mcp"]')).toContainText("demo");
  await expect(page.getByTestId("trust-categories").locator('[data-category="permissions"]')).toBeVisible();
  await page.getByTestId("trust-confirm").click();
  await expect(page.locator("#modal")).toHaveCount(0);
  await expect(project.getByTestId("project-trusted")).toBeVisible();

  // The engine says so too.
  const trust = () => page.evaluate(async (root) => (await (window as any).harnix.app.call("harnix-app.projects.status", { project_root: root })).trust, path);
  expect(await trust()).toBe("trusted");

  // No longer: straight from the menu, and the shield goes.
  await project.getByRole("treeitem", { level: 1 }).click({ button: "right" });
  await menuItem(page, "untrust").click();
  await expect(project.getByTestId("project-trusted")).toHaveCount(0);
  expect(await trust()).toBe("untrusted");
});

test("a conversation in an untrusted project offers trust over it, until it is closed for that project", PROJECTS, async ({ page }) => {
  await open(page, URL);
  const { project } = await untrustedProject(page, "trust-banner");
  await project.getByRole("treeitem", { level: 1 }).click({ button: "right" });
  await menuItem(page, "new").click();
  await page.getByTestId("new-mode").selectOption({ index: 0 });
  await page.getByTestId("new-create").click();
  await expect(page.locator("#modal")).toHaveCount(0);
  const id = await page.evaluate(() => (window as any).harnix.app.snapshot.current?.sessionId as string);
  expect(id).toBeTruthy();

  const banner = page.locator("#main").getByTestId("trust-banner");
  await expect(banner).toBeVisible({ timeout: 15_000 });
  await expect(banner).toContainText("MCP");

  // Its action is the menu's: the same dialog, which can be left without trusting.
  await banner.getByTestId("trust-banner-trust").click();
  await expect(page.getByTestId("trust-categories")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator("#modal")).toHaveCount(0);
  await expect(banner).toBeVisible();

  await banner.getByTestId("trust-banner-dismiss").click();
  await expect(banner).toHaveCount(0);
  // Closed for that project: not back when the conversation is opened again after a reload.
  await open(page, URL);
  await spaceEntry(page, "projects").click();
  if ((await project.getAttribute("data-open")) !== "true") await project.getByRole("treeitem", { level: 1 }).click();
  await sessionRow(page, id).click();
  await expect(page.getByTestId("conv-input")).toBeVisible({ timeout: 15_000 });
  await expect(page.locator("#main").getByTestId("trust-banner")).toHaveCount(0);
});
