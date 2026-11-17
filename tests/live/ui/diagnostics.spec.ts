/**
 * The settings' diagnostics (harnix-app.md §12): after the shell's own content, the blocks other
 * packages put there (`settings.section` `within: "diagnostics"`) — the packages as they run
 * (@harnix/packages, §12.1) and the extensions' log (@harnix/observability, §12.2) — neither an
 * entry of the navigation of its own; one named to `host.settings.open` opens the diagnostics.
 *
 * Against the `harnix` binary with no package installed: the packages block says there is none,
 * the log reads the first-party extensions' lines. With a package installed, `packages.spec.ts`,
 * `packages-idle.spec.ts` and `packages-extlog.spec.ts` look at its row and its lines.
 */

import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { diagnosticsBlock, open, settingsSection, watchConsole } from "./page";

const TAGS = { tag: ["@feature:settings", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => {
  await backend?.dispose();
});

test("the diagnostics end with the packages block and the extension log, each under its title, neither in the navigation", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const packages = await diagnosticsBlock(page, "packages.runtime");
  const extlog = await diagnosticsBlock(page, "observability.extlog");
  const order = await page.getByTestId("settings-within").evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.section));
  expect(order).toEqual(["packages.runtime", "observability.extlog"]);
  for (const block of [packages, extlog]) await expect(block.locator("h3").first()).not.toBeEmpty();
  for (const id of ["packages.runtime", "observability.extlog"]) await expect(settingsSection(page, id)).toHaveCount(0);

  // Nothing installed: the packages block says so, and only looks — no buttons to act with.
  await expect(packages.getByTestId("packages-runtime-empty")).toBeVisible();
  await expect(packages.locator("button")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("the extension log: read, narrowed by extension and level, live by default and stopped on asking", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  const log = (await diagnosticsBlock(page, "observability.extlog")).getByTestId("extlog");
  // Read: lines, or the word that there are none — never a failure.
  await expect(log.getByTestId("extlog-line").first().or(log.getByTestId("extlog-empty"))).toBeVisible({ timeout: 15_000 });
  await expect(log.getByTestId("extlog-failed")).toHaveCount(0);
  const offered = await log.getByTestId("extlog-extension").locator("option").evaluateAll((os) => os.map((o) => (o as HTMLOptionElement).value).filter(Boolean));
  if (offered.length) {
    // One extension: only its lines.
    await log.getByTestId("extlog-extension").selectOption(offered[0]!);
    await expect(log.locator(`[data-testid="extlog-line"]:not([data-extension="${offered[0]}"])`)).toHaveCount(0);
    await expect(log.getByTestId("extlog-line").first()).toBeVisible();
    await log.getByTestId("extlog-extension").selectOption("");
  }
  await log.getByTestId("extlog-level").selectOption("error");
  await expect(log.locator('[data-testid="extlog-line"]:not([data-level="error"])')).toHaveCount(0);
  await log.getByTestId("extlog-level").selectOption("");
  await expect(log.getByTestId("extlog-live")).toHaveAttribute("aria-pressed", "true");
  await log.getByTestId("extlog-live").click();
  await expect(log.getByTestId("extlog-live")).toHaveAttribute("aria-pressed", "false");
  expect(errors).toEqual([]);
});

test("a block's id given to host.settings.open opens the diagnostics it is in", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await page.evaluate(() => (window as any).harnix.host.settings.open("packages.runtime"));
  await expect(settingsSection(page, "diagnostics")).toHaveAttribute("data-state", "current");
  await expect(page.locator('[data-testid="settings-within"][data-section="packages.runtime"]')).toBeVisible();
  expect(errors).toEqual([]);
});
