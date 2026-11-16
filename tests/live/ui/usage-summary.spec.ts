/**
 * The system space's 用量 tab in the browser (system-space.md §6.2): every session's recording of
 * the last 30 days summed — totals, a bar a day, tables by project, model and day that sort by a
 * header — read again on asking, with the flows' section below it.
 *
 * Replayed sessions record nothing, so the recordings are put in place as the trajectory's tests do
 * (`recorded`): `calls-a-tool` (two calls) and `says-ready` (one), their times moved to now so they
 * fall in the window whenever this runs.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, recorded, watchConsole } from "./page";

const TAGS = { tag: ["@feature:system", "@layer:ui"] };
const MODEL = "claude-sonnet-4-6";

let backend: Backend;
test.describe.configure({ mode: "serial" });
test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => {
  await backend?.dispose();
});

/** A recording's lines with every time moved by the same amount, the first call now. */
function now(lines: string[]): string[] {
  const parsed = lines.map((l) => JSON.parse(l) as Record<string, unknown>);
  const first = parsed.find((l) => l.type === "call")?.ts as number;
  const by = Date.now() - first;
  return parsed.map((l) => {
    for (const key of ["ts", "ts0", "created_at"]) if (typeof l[key] === "number") l[key] = (l[key] as number) + by;
    return JSON.stringify(l);
  });
}

async function toUsage(page: Page): Promise<void> {
  await open(page, backend.url);
  await page.locator('#side [data-space="system"]').click();
  await expect(page.getByTestId("system-console")).toBeVisible();
  await page.locator('[data-testid="system-console"] [role="tab"][data-tab="observability.summary"]').click();
  await expect(page.getByTestId("usage-summary")).toBeVisible();
}

test("usage-summary: the recordings of the window summed by project, model and day, a header sorting its table, read again on asking", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await recorded(page, backend, { fixture: "calls-a-tool", edit: now });
  await toUsage(page);
  const summary = page.getByTestId("usage-summary");
  await expect(summary).not.toHaveAttribute("data-loading", "true", { timeout: 20_000 });
  // The right column says what the tab is.
  await expect(page.locator('#details [data-testid="system-about"]')).toHaveAttribute("data-tab", "observability.summary");

  const totals = page.getByTestId("usage-summary-totals");
  await expect(totals.locator('[data-cell="sessions"] .v')).toHaveText("1");
  await expect(totals.locator('[data-cell="calls"] .v')).toHaveText(/^2/);
  await expect(page.getByTestId("usage-summary-bars").locator(".col")).toHaveCount(30);
  await expect(page.getByTestId("usage-summary-bars").locator('.col:not([data-value="0"])')).toHaveCount(1);
  const models = page.getByTestId("usage-by-model");
  await expect(models.locator(`tbody tr[data-key="${MODEL}"]`)).toHaveCount(1);
  // A session of no project is listed as such.
  await expect(page.getByTestId("usage-by-project").locator('tbody tr[data-key=""]')).toHaveCount(1);
  await expect(page.getByTestId("usage-by-day").locator("tbody tr")).toHaveCount(1);

  // A second session: not there until it is read again.
  await recorded(page, backend, { fixture: "says-ready", edit: now });
  await toUsage(page);
  await expect(totals.locator('[data-cell="sessions"] .v')).toHaveText("2", { timeout: 20_000 });
  await expect(totals.locator('[data-cell="calls"] .v')).toHaveText(/^3/);
  await expect(page.getByTestId("usage-by-project").locator('tbody tr[data-key=""] td[data-column="sessions"]')).toHaveText("2");

  // A header sorts by its column, and again turns it.
  const header = models.locator('th[data-column="calls"]');
  await header.locator("button").click();
  await expect(header).toHaveAttribute("aria-sort", "descending");
  await header.locator("button").click();
  await expect(header).toHaveAttribute("aria-sort", "ascending");

  await page.getByTestId("usage-summary-refresh").click();
  await expect(summary).not.toHaveAttribute("data-loading", "true", { timeout: 20_000 });
  await expect(totals.locator('[data-cell="sessions"] .v')).toHaveText("2");

  // The flows add their section: nothing started in these days.
  const flows = summary.locator('[data-section="flow.usage"]');
  await expect(flows).toBeVisible();
  await expect(flows.getByTestId("usage-by-flow-none")).toBeVisible();
  expect(errors).toEqual([]);
});
