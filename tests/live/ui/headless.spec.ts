/**
 * Headless sessions in the browser (design §6.1, §9.2): sessions a module drives from the server
 * side are ordinary sessions, so the system space's tree shows them under their instance, and
 * opening one shows everything the module made it say.
 */

import { type Backend, expect, test } from "@harnix/testkit";

import { startTestApp } from "../api/testapp";
import { open, reply } from "./page";

const TAGS = { tag: ["@feature:headless", "@layer:ui"] };

let backend: Backend;

test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(() => backend?.stop());

test("a module's headless sessions are in the instance tree, and open like any other", TAGS, async ({ page }) => {
  await open(page, backend.url);
  const started: string[] = await page.evaluate(async (root) => {
    const { app } = (window as any).harnix;
    const result = await app.call("app.fleet.headless", {
      count: 2,
      project_root: root,
      recording: "says-ready",
      message: "Say exactly: fixture ready",
    });
    return result.started.map((s: any) => s.session_id);
  }, backend.project);
  expect(started).toHaveLength(2);

  await page.locator('#side .spaces [data-space="system"]').click();
  await page.locator('#side .system-tree .inst[data-instance="system"]').click();
  for (const sid of started) await expect(page.locator(`#side .inst-sessions [data-sid="${sid}"]`)).toBeVisible({ timeout: 30_000 });
  await page.locator(`#side .inst-sessions [data-sid="${started[0]}"]`).click();
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
});
