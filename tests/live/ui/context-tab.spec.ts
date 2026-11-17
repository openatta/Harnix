/**
 * The context tab (harnix-app.md §6.1), @harnix/observability's `pinned` tab: it stands at the end
 * of every session's strip — there without being opened, after whatever the person opened, and
 * with no way to close it — and shows what the session's next call carries: the report (measured
 * by the provider apart from estimated), the skills, hooks and MCP servers. A session in the middle
 * of a turn is not laid out; the tab says so in words.
 *
 * Replayed: `says-ready`, `asks-permission` (a turn waiting on its Bash call's permission).
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { newSession, reply, open, say, sessionRow, watchConsole } from "./page";

const TAGS = { tag: ["@feature:session-space", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => {
  await backend?.dispose();
});

const chips = (page: Page) => page.locator('#details [data-testid="sidebar-tab"]');
const chip = (page: Page, kind: string) => page.locator(`#details [data-testid="sidebar-tab"][data-kind="${kind}"]`);
const kinds = (page: Page) => chips(page).evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.kind));

async function expand(page: Page): Promise<void> {
  if ((await page.locator("#frame").getAttribute("data-details")) !== "open") await page.getByTestId("details-toggle").click();
  await expect(page.locator("#frame")).toHaveAttribute("data-details", "open");
}

/** A session that has run one replayed turn, open in the session space. */
async function withTurn(page: Page): Promise<string> {
  const sid = await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  await expect(page.getByTestId("conv-send")).toBeVisible();
  return sid;
}

test("in every session's strip, last: there unopened, after what was opened, and it cannot be closed", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const first = await withTurn(page);
  await expand(page);
  // Standing at the end, without taking the front: the guide is what a new surface shows.
  await expect(chip(page, "context")).toHaveAttribute("data-pinned", "true");
  await expect.poll(() => kinds(page)).toEqual(expect.arrayContaining(["context"]));
  expect((await kinds(page)).at(-1)).toBe("context");
  await expect(chip(page, "context")).toHaveAttribute("data-state", "open");
  // Not offered by the guide: it is always there.
  await expect(page.locator('[data-testid="sidebar-guide-entry"][data-kind="context"]')).toHaveCount(0);
  // A tab opened from the guide goes before it.
  await page.locator('[data-testid="sidebar-guide-entry"][data-kind="usage"]').click();
  await expect(chip(page, "usage")).toHaveAttribute("data-state", "current");
  expect((await kinds(page)).at(-1)).toBe("context");

  // No close: no button, a middle click leaves it, and the host's close refuses it.
  await expect(chip(page, "context").getByTestId("sidebar-tab-close")).toHaveCount(0);
  await chip(page, "context").getByRole("tab").click({ button: "middle" });
  await expect(chip(page, "context")).toHaveCount(1);
  const id = await chip(page, "context").getAttribute("data-tab");
  await page.evaluate((tab) => (window as any).harnix.host.sidebar.close(tab), id);
  await page.evaluate((tab) => (window as any).harnix.host.sidebar.close(tab), await chip(page, "usage").getAttribute("data-tab"));
  // The other one went; this one stays.
  await expect(chip(page, "usage")).toHaveCount(0);
  await expect(chip(page, "context")).toHaveCount(1);
  // Brought to the front, the session's context.
  await chip(page, "context").getByRole("tab").click();
  await expect(chip(page, "context")).toHaveAttribute("data-state", "current");
  await expect(page.locator('#details [data-testid="context-tab"] [data-testid="context-report"]')).toBeVisible();

  // A second session has its own, at the end of its own strip.
  const second = await withTurn(page);
  expect(second).not.toBe(first);
  await expand(page);
  await expect(chip(page, "context")).toHaveCount(1);
  expect((await kinds(page)).at(-1)).toBe("context");
  await expect(chip(page, "usage")).toHaveCount(0);
  // And the first's is as it was left.
  await sessionRow(page, first).click();
  await expect(chip(page, "context")).toHaveAttribute("data-state", "current");
  expect((await kinds(page)).at(-1)).toBe("context");
  expect(errors).toEqual([]);
});

test("the report: measured apart from estimated, its blocks, and the skills, hooks and MCP servers beside it", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await withTurn(page);
  await expand(page);
  await chip(page, "context").getByRole("tab").click();
  const tab = page.locator('#details [data-testid="context-tab"]');
  // The bar is as long as what the provider counted; the estimates that divide it say they are.
  await expect(tab.getByTestId("context-stack")).toHaveAttribute("data-measured", /^[1-9]\d*$/, { timeout: 15_000 });
  await expect(tab.locator(".legend2")).toContainText("~");
  await expect(tab.getByTestId("system-block").first()).toBeVisible();
  for (const part of ["context-skills", "context-hooks", "context-mcp"]) await expect(tab.getByTestId(part)).toBeVisible();
  expect(errors).toEqual([]);
});

test("a session in the middle of a turn: the tab says so in words, the engine's own in the title, and reads it once the turn is over", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await newSession(page, "asks-permission");
  await say(page, "Use the Bash tool to run exactly: touch harnix-permission-fixture");
  // Waiting on its ask, the turn runs: the engine will not lay out a context it is in the middle of.
  await expect(page.getByTestId("conv-ask")).toBeVisible({ timeout: 30_000 });
  await expand(page);
  await chip(page, "context").getByRole("tab").click();
  const failed = page.locator('#details [data-testid="context-report-failed"]');
  await expect(failed).toHaveText(/running a turn|正在跑一轮/);
  // The engine's own words ("session is busy"), with its code.
  await expect(failed).toHaveAttribute("title", /busy.*\(-32015\)/);
  await expect(failed).toHaveAttribute("data-code", "-32015");
  // Answered, the turn ends, and the tab reads the context again.
  await page.getByTestId("conv-ask").getByTestId("conv-ask-allow").click();
  await expect(page.getByTestId("conv-send")).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('#details [data-testid="context-tab"] [data-testid="context-report"]')).toBeVisible({ timeout: 15_000 });
});
