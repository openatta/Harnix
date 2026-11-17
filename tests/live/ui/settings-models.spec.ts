/**
 * Model settings driven from the page: a model parameter written from Settings · Models, a task
 * routed to a provider and a model of it from Settings · Models by task (the conversation's own
 * route among them), a provider's key said detected as the engine says and shown only as its last
 * four characters, and its models edited as rows with their limits.
 *
 * On a backend of its own, as the other settings suite: the providers are the instance's settings.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, openSettings, settingsSection } from "./page";

const TAGS = { tag: ["@feature:settings", "@layer:ui"] };

const PROVIDER = "routed";
const KEY = "sk-live-test-route-9876wxyz";

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startHarnix(launchFor("replay"));
});
test.afterAll(async () => { await backend?.dispose(); });

async function section(page: Page, id: string): Promise<void> {
  await openSettings(page);
  await settingsSection(page, id).click();
}

/** A field as `settings.describe` says it, read the way the page reads it. */
const field = (page: Page, key: string) =>
  page.evaluate(async (key) => {
    const described = await (window as any).harnix.app.call("settings.describe", {});
    return described.fields.find((f: { key: string }) => f.key === key);
  }, key);

const call = (page: Page, method: string, params: Record<string, unknown>) =>
  page.evaluate(async ([method, params]) => (window as any).harnix.app.call(method, params), [method, params] as const);

/** A provider to route to; the engine builds its client without calling it. */
const addProvider = (page: Page) =>
  call(page, "settings.setProvider", {
    provider_id: PROVIDER,
    config: { api_type: "openai_compatible", base_url: "http://127.0.0.1:9/v1", api_key: KEY, default_model: "route-default", models: ["route-default", "route-other"] },
  });

test("a model parameter is written from Settings · Models, then cleared", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await section(page, "models");
  const effort = page.getByTestId("settings-model.effort");
  await expect(effort).toBeVisible();
  await effort.selectOption("low");
  await expect.poll(async () => (await field(page, "model.effort")).tiers.global).toBe("low");
  await page.getByTestId("settings-model.effort-clear").click();
  await expect.poll(async () => (await field(page, "model.effort")).tiers.global).toBeNull();
});

test("a task is routed to a provider and one of its models from Settings · Models by task", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await addProvider(page);
  try {
    await section(page, "task-models");
    const provider = page.getByTestId("settings-task_models.compact");
    await expect(provider).toBeVisible();
    await provider.selectOption(PROVIDER);
    await expect.poll(async () => (await field(page, "task_models.compact")).tiers.global).toBe(PROVIDER);
    // The provider's default model is what a blank model means; another is written with it.
    const model = page.getByTestId("settings-task_models.compact-model");
    await expect(model).toHaveAttribute("placeholder", "route-default");
    await model.fill("route-other");
    await model.blur();
    await expect.poll(async () => (await field(page, "task_models.compact")).tiers.global).toEqual({ provider: PROVIDER, model: "route-other" });
    await page.getByTestId("settings-task_models.compact-clear").click();
    await expect.poll(async () => (await field(page, "task_models.compact")).tiers.global).toBeNull();
  } finally {
    await call(page, "settings.write", { tier: "global", changes: { "task_models.compact": null } });
    await call(page, "settings.setProvider", { provider_id: PROVIDER, delete: true });
  }
});

test("a provider's key is shown as its last four characters, never whole", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await addProvider(page);
  try {
    await section(page, "models");
    const key = page.locator(`[data-provider="${PROVIDER}"]`).getByTestId("provider-key");
    await expect(key).toHaveAttribute("data-state", "present");
    await expect(key).toHaveAttribute("data-source", "stored");
    await expect(key).toHaveAttribute("title", /\*\*\*wxyz/);
    await expect(page.getByTestId("settings-page")).not.toContainText(KEY);
    // Editing it shows the same four, and leaves the key as it is when nothing is typed.
    await page.locator(`[data-provider="${PROVIDER}"]`).getByTestId("provider-edit").click();
    await expect(page.getByTestId("provider-form").getByTestId("provider-api_key")).toHaveAttribute("placeholder", /\*\*\*wxyz/);
  } finally {
    await call(page, "settings.setProvider", { provider_id: PROVIDER, delete: true });
  }
});

test("the conversation's route is set in Settings · Models by task", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await addProvider(page);
  try {
    await section(page, "task-models");
    const provider = page.getByTestId("settings-task_models.main");
    await expect(provider).toBeVisible();
    await provider.selectOption(PROVIDER);
    await expect.poll(async () => (await field(page, "task_models.main")).tiers.global).toBe(PROVIDER);
    // The endpoint is not there: asking it again says so, and a model is still typed in.
    const refresh = page.getByTestId("settings-task_models.main-models");
    await refresh.click();
    await expect(refresh).toHaveAttribute("data-state", "failed");
    await page.getByTestId("settings-task_models.main-model").fill("route-other");
    await page.getByTestId("settings-task_models.main-model").blur();
    await expect.poll(async () => (await field(page, "task_models.main")).tiers.global).toEqual({ provider: PROVIDER, model: "route-other" });
  } finally {
    await call(page, "settings.write", { tier: "global", changes: { "task_models.main": null } });
    await call(page, "settings.setProvider", { provider_id: PROVIDER, delete: true });
  }
});

test("a provider's models are edited as rows, each with its own limits", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await addProvider(page);
  try {
    await section(page, "models");
    await page.locator(`[data-provider="${PROVIDER}"]`).getByTestId("provider-edit").click();
    const form = page.getByTestId("provider-form");
    await expect(form.getByTestId("provider-model-id")).toHaveCount(2);
    await form.getByTestId("provider-model-max_output").nth(1).fill("4096");
    await form.getByTestId("provider-model-context_window").nth(1).fill("64000");
    await form.getByTestId("provider-save").click();
    await expect(form).toHaveCount(0);
    const models = await page.evaluate(async (id) => (await (window as any).harnix.app.call("settings.describe", {})).providers.providers[id].models, PROVIDER);
    expect(models).toEqual(["route-default", { id: "route-other", max_output: 4096, context_window: 64000 }]);
  } finally {
    await call(page, "settings.setProvider", { provider_id: PROVIDER, delete: true });
  }
});
