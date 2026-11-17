/**
 * Headless sessions in the browser (concept §3.2): sessions a module drives from the server side
 * are ordinary sessions. Placed nowhere else, they are the session space's (its backend space is
 * the default one), so its list shows them, and opening one shows everything the module made it say.
 */

import { type Backend, expect, test } from "@harnix/testkit";

import { startTestApp } from "../api/testapp";
import { open, reply, sessionRow, toSessions } from "./page";

const TAGS = { tag: ["@feature:headless", "@layer:ui"] };

let backend: Backend;

test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(async () => { await backend?.dispose(); });

test("a module's headless sessions are in the session space's list, and open like any other", TAGS, async ({ page }) => {
  await open(page, backend.url);
  const started: string[] = await page.evaluate(async (root) => {
    const { app } = (window as any).harnix;
    const result = await app.call("fleet.headless", {
      count: 2,
      project_root: root,
      recording: "says-ready",
      message: "Say exactly: fixture ready",
    });
    return result.started.map((s: any) => s.session_id);
  }, backend.project);
  expect(started).toHaveLength(2);

  await toSessions(page);
  for (const sid of started) await expect(sessionRow(page, sid)).toBeVisible({ timeout: 30_000 });
  await sessionRow(page, started[0]!).click();
  await expect(sessionRow(page, started[0]!)).toHaveAttribute("data-state", "current");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
});
