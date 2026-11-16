/**
 * Settings that change something, driven from the page: an engine setting written from the form
 * its declaration draws (Settings · Engine); a risky value confirmed first, a write over settings
 * changed elsewhere kept and reviewed, a permission rule added from its editor (Settings ·
 * Permissions); the sandbox's and the network's fields (Settings · Sandbox, Network and web); a
 * model provider added, edited, made the default and deleted (Settings · Models), and a paired
 * device revoked (Settings · Devices).
 *
 * On a backend of its own: the providers are the instance's settings, and another suite's sessions
 * should not route through one this suite made up.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { open, openSettings, settingsSection } from "./page";

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
  await settingsSection(page, id).click();
}

/** What the engine's settings say about providers, read the way the page reads them. */
const providers = (page: Page) =>
  page.evaluate(async () => {
    const { app } = (window as any).harnix;
    const described = await app.call("settings.describe", {});
    return described.providers ?? {};
  });

/** A field as `settings.describe` says it, read the way the page reads it. */
const field = (page: Page, key: string) =>
  page.evaluate(async (key) => {
    const { app } = (window as any).harnix;
    const described = await app.call("settings.describe", {});
    return described.fields.find((f: { key: string }) => f.key === key);
  }, key);

/** Written behind the page's back, as another tab or a hand edit would. */
const writeElsewhere = (page: Page, changes: Record<string, unknown>) =>
  page.evaluate(async (changes) => {
    await (window as any).harnix.app.call("settings.write", { tier: "global", changes });
  }, changes);

test("an engine setting is written from its form into the tier chosen, then cleared", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await section(page, "engine");
  const memory = page.getByTestId("settings-memory_enabled");
  await expect(memory).toBeVisible();
  const was = await memory.isChecked();
  await memory.click();
  await expect.poll(async () => (await field(page, "memory_enabled")).tiers.global).toBe(!was);
  // Set in this tier now: the row says so, and can give it back.
  await expect(page.locator('[data-setting="memory_enabled"]')).toContainText(/Set in this tier|在这一层设置了/);
  await page.getByTestId("settings-memory_enabled-clear").click();
  await expect.poll(async () => (await field(page, "memory_enabled")).tiers.global).toBeNull();
});

test("full access is confirmed before it is written", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await section(page, "permissions");
  const mode = page.getByTestId("settings-permission_mode");
  await mode.selectOption("bypassPermissions");
  const risk = page.getByTestId("settings-risk");
  await expect(risk).toBeVisible();
  // It says what full access lets happen, and what still holds.
  await expect(risk).toContainText(/deny rules included|包括 deny/);
  // Called off: nothing is written.
  await risk.getByTestId("settings-risk-cancel").click();
  await expect(risk).toHaveCount(0);
  expect((await field(page, "permission_mode")).tiers.global).toBeNull();
  // Confirmed, once the risk is acknowledged.
  await mode.selectOption("bypassPermissions");
  await expect(page.getByTestId("settings-risk-confirm")).toBeDisabled();
  await page.getByTestId("settings-risk-ack").check();
  await page.getByTestId("settings-risk-confirm").click();
  await expect.poll(async () => (await field(page, "permission_mode")).tiers.global).toBe("bypassPermissions");
  await page.getByTestId("settings-permission_mode-clear").click();
  await expect.poll(async () => (await field(page, "permission_mode")).tiers.global).toBeNull();
});

test("a change made over settings changed elsewhere is kept, reviewed and then written", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await section(page, "permissions");
  const widen = page.getByTestId("settings-allow_client_permission_override");
  await expect(widen).toBeVisible();
  const was = await widen.isChecked();
  // The page read the settings; then they change behind it.
  await writeElsewhere(page, { permission_mode: "plan" });
  try {
    await widen.click();
    await expect(page.getByTestId("settings-conflict")).toBeVisible();
    expect((await field(page, "allow_client_permission_override")).tiers.global, "written over settings it did not read").toBeNull();
    await page.getByTestId("settings-conflict-keep").click();
    await expect(page.getByTestId("settings-review")).toBeVisible();
    // Mine over theirs: the kept change is shown as a draft, and the one made elsewhere is there.
    await expect(page.locator('[data-setting="allow_client_permission_override"]').first()).toHaveAttribute("data-state", "draft");
    await expect(page.getByTestId("settings-permission_mode")).toHaveValue("plan");
    await page.getByTestId("settings-review-save").click();
    await expect.poll(async () => (await field(page, "allow_client_permission_override")).tiers.global).toBe(!was);
    expect((await field(page, "permission_mode")).tiers.global).toBe("plan");
  } finally {
    await writeElsewhere(page, { allow_client_permission_override: null, permission_mode: null });
  }
});

test("a permission rule is added from its editor as a tool and a pattern, and one the engine cannot match is refused", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await section(page, "permissions");
  const editor = page.getByTestId("settings-permission_rules");
  await expect(editor).toBeVisible();
  await expect(editor).toContainText(/Effect|效果/);
  try {
    await editor.getByTestId("settings-rule-add").click();
    const row = editor.locator("tbody tr").last();
    await row.getByTestId("settings-rule-tool").fill("Bash");
    await row.getByTestId("settings-rule-content").fill("git push:*");
    await row.locator("select").first().selectOption("deny");
    await editor.getByTestId("settings-rules-apply").click();
    // The engine's shape: the pattern inside the tool's parentheses.
    await expect.poll(async () => (await field(page, "permission_rules")).tiers.global).toEqual([{ tool: "Bash(git push:*)", action: "deny" }]);

    // A wildcard in a tool's name matches nothing in the engine: said at the row, not written.
    const again = page.getByTestId("settings-permission_rules");
    await again.getByTestId("settings-rule-tool").first().fill("mcp__github__*");
    await again.getByTestId("settings-rules-apply").click();
    await expect(page.locator('[data-setting="permission_rules"]').getByTestId("setting-problem")).toContainText(/wildcard|通配符/);
    expect((await field(page, "permission_rules")).tiers.global).toEqual([{ tool: "Bash(git push:*)", action: "deny" }]);
  } finally {
    await writeElsewhere(page, { permission_rules: null });
  }
});

test("the sandbox's and the network's fields are drawn from their declarations and written", TAGS, async ({ page }) => {
  await open(page, backend.url);
  await section(page, "sandbox");
  const mode = page.getByTestId("settings-sandbox.network_mode");
  await expect(mode).toBeVisible();
  // Options by their names, not the engine's words.
  await expect(mode.locator("option", { hasText: /No network|禁止联网/ })).toHaveCount(1);
  await expect(page.getByTestId("settings-sandbox.read_directories")).toBeVisible();
  await expect(page.getByTestId("settings-sandbox.require_enforcement")).toBeVisible();

  await section(page, "network");
  await expect(page.getByTestId("settings-network.use_system_proxy")).toBeVisible();
  await expect(page.getByTestId("settings-task_models.web_search-provider")).toBeVisible();
  const domains = page.getByTestId("settings-sandbox.allowed_domains");
  try {
    await domains.fill("example.com\nexample.org");
    await domains.blur();
    await expect.poll(async () => (await field(page, "sandbox.allowed_domains")).tiers.global).toEqual(["example.com", "example.org"]);
  } finally {
    await writeElsewhere(page, { "sandbox.allowed_domains": null });
  }
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
  await expect(page.getByTestId("settings-page")).not.toContainText("sk-live-test-first");
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
  // With a key: the engine tries a provider before it is written, and one with no key does not route.
  await page.getByTestId("provider-form").getByTestId("provider-api_key").fill("sk-live-test-second");
  await page.getByTestId("provider-form").getByTestId("provider-default_model").fill("model-three");
  await page.getByTestId("provider-form").getByTestId("provider-save").click();
  const second = list.locator('[data-provider="second"]');
  await expect(second).toBeVisible();
  await second.getByTestId("provider-default").click();
  await expect(second).toHaveAttribute("data-state", "default");
  await expect(first).not.toHaveAttribute("data-state", "default");
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
    const grant = await app.call("devices.pair.begin", { label: "the test laptop" });
    const paired = await app.call("devices.pair.complete", { code: grant.code, public_key: "dGVzdC1rZXk=" });
    return paired.device.id as string;
  });
  await section(page, "devices");
  await expect(page.getByTestId("settings-page")).toContainText("the test laptop");
  const revoke = page.locator(`[data-testid="device-revoke"][data-device="${device}"]`);
  // It asks first: dismissed, nothing happens.
  await revoke.click();
  await page.getByTestId("confirm-cancel").click();
  await expect(revoke).toBeVisible();
  await revoke.click();
  await page.getByTestId("confirm").click();
  await expect(revoke).toHaveCount(0);
  const listed = await page.evaluate(async () => (await (window as any).harnix.app.call("devices.list", {})).devices);
  expect(listed.some((d: any) => d.id === device)).toBe(false);
});
