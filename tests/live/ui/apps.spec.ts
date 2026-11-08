/**
 * A runtime application's interface module in the browser: the showcase (`examples/apps/showcase`),
 * installed from a file on the apps-enabled binary, puts its `home.card` in front of a new
 * conversation — imported from `/apps/<id>/ui/`, on the shell's own React, with words from its own
 * language pack — and the page reports nothing. Disabled, the card is gone after a reload.
 */

import { type Backend, connect, expect, test } from "@harnix/testkit";

import { SHOWCASE_ID, showcasePackage, startApps } from "../api/apps-host";
import { open, watchConsole } from "./page";

const TAGS = { tag: ["@feature:apps", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  test.setTimeout(600_000);
  const { path } = showcasePackage();
  backend = await startApps();
  const api = await connect(backend);
  try {
    await api.call("harnix.apps.installFile", { path });
  } finally {
    api.close();
  }
});
test.afterAll(async () => { await backend?.dispose(); });

test("an application's interface module draws its card, signed with its id", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  const violations: string[] = [];
  page.on("console", (m) => {
    if (/Content Security Policy|Refused to/i.test(m.text())) violations.push(m.text());
  });
  await open(page, backend.url);
  await expect
    .poll(() => page.evaluate(() => (window as any).harnix.app.registry.at("home.card").map((c: any) => c.owner)), { timeout: 15_000 })
    .toContain(SHOWCASE_ID);
  const card = page.getByTestId("showcase-card");
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card).toContainText(/Showcase|示例应用/);
  await expect(card).toContainText(/Six tools|六个工具/);
  expect(violations).toEqual([]);
  expect(errors).toEqual([]);
});

test("disabled, its card is gone from the page", TAGS, async ({ page }) => {
  const api = await connect(backend);
  try {
    await api.call("harnix.apps.disable", { id: SHOWCASE_ID });
  } finally {
    api.close();
  }
  await open(page, backend.url);
  await expect(page.locator("#hero")).toBeVisible();
  const owners = await page.evaluate(() => (window as any).harnix.app.registry.at("home.card").map((c: any) => c.owner));
  expect(owners).not.toContain(SHOWCASE_ID);
  await expect(page.getByTestId("showcase-card")).toHaveCount(0);
});
