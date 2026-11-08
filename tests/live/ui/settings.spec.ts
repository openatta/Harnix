/**
 * Settings that change something, driven from the page: a model provider added, edited, made the
 * default and deleted (Settings · Models), and a paired device revoked (Settings · Devices).
 *
 * On a backend of its own: the providers are the instance's settings, and another suite's sessions
 * should not route through one this suite made up.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, openSettings } from "./page";

const TAGS = { tag: ["@feature:settings", "@layer:ui"] };
const AUTHZ = { tag: ["@feature:authz", "@layer:ui"] };

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => { await backend?.dispose(); });

async function section(page: Page, id: string): Promise<void> {
  await openSettings(page);
  await page.locator(`.settings-nav [data-section="${id}"]`).click();
}

/** What the engine's settings say about providers, read the way the page reads them. */
const providers = (page: Page) =>
  page.evaluate(async () => {
    const { app } = (window as any).harnix;
    const described = await app.call("harnix.settings.describe", {});
    return described.providers ?? {};
  });

test("a provider is added, edited, made the default and deleted from Settings · Models", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await section(page, "models");
  const list = page.getByTestId("providers");

  // Add one: the key is written, and never shown back.
  await list.getByTestId("provider-add").click();
  const form = page.getByTestId("provider-form");
  await form.getByTestId("provider-provider_id").fill("first");
  await form.getByTestId("provider-type-anthropic").click();
  await form.getByTestId("provider-base_url").fill("http://127.0.0.1:9/");
  await form.getByTestId("provider-api_key").fill("sk-live-test-first");
  await form.getByTestId("provider-default_model").fill("model-one");
  await form.getByTestId("provider-save").click();
  const first = list.locator('[data-provider="first"]');
  await expect(first).toBeVisible();
  await expect(first).toContainText("model-one");
  await expect(page.locator(".settings-page")).not.toContainText("sk-live-test-first");
  expect((await providers(page)).providers.first.default_model).toBe("model-one");

  // Edit it: the key is kept when the field is left empty.
  await first.getByTestId("provider-edit").click();
  await page.getByTestId("provider-form").getByTestId("provider-default_model").fill("model-two");
  await page.getByTestId("provider-form").getByTestId("provider-save").click();
  await expect(first).toContainText("model-two");
  const edited = (await providers(page)).providers.first;
  expect(edited.default_model).toBe("model-two");
  expect(edited.api_key, "the key survived an edit that left it empty").toBeTruthy();

  // A second one, then make it the default.
  await list.getByTestId("provider-add").click();
  await page.getByTestId("provider-form").getByTestId("provider-provider_id").fill("second");
  await page.getByTestId("provider-form").getByTestId("provider-default_model").fill("model-three");
  await page.getByTestId("provider-form").getByTestId("provider-save").click();
  const second = list.locator('[data-provider="second"]');
  await expect(second).toBeVisible();
  await second.getByTestId("provider-default").click();
  await expect(second).toHaveClass(/default/);
  await expect(first).not.toHaveClass(/default/);
  expect((await providers(page)).default_provider).toBe("second");

  // Delete asks once more on the same button; then it is gone.
  await first.getByTestId("provider-delete").click();
  await expect(first).toBeVisible();
  await first.getByTestId("provider-delete").click();
  await expect(first).toHaveCount(0);
  expect(Object.keys((await providers(page)).providers)).toEqual(["second"]);
});

test("a paired device is listed in Settings · Devices and revoked from there", AUTHZ, async ({ page }) => {
  await open(page, backend.url);
  const device = await page.evaluate(async () => {
    const { app } = (window as any).harnix;
    const grant = await app.call("harnix.devices.pair.begin", { label: "the test laptop" });
    const paired = await app.call("harnix.devices.pair.complete", { code: grant.code, public_key: "dGVzdC1rZXk=" });
    return paired.device.id as string;
  });
  await section(page, "devices");
  await expect(page.locator(".settings-page")).toContainText("the test laptop");
  const revoke = page.locator(`[data-testid="device-revoke"][data-device="${device}"]`);
  // It asks first: dismissed, nothing happens.
  page.once("dialog", (d) => void d.dismiss());
  await revoke.click();
  await expect(revoke).toBeVisible();
  page.once("dialog", (d) => void d.accept());
  await revoke.click();
  await expect(revoke).toHaveCount(0);
  const listed = await page.evaluate(async () => (await (window as any).harnix.app.call("harnix.devices.list", {})).devices);
  expect(listed.some((d: any) => d.id === device)).toBe(false);
});
