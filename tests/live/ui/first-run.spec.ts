/**
 * The first run, on a fresh profile (docs/harnix-app.md §4.1): the new conversation's empty page has
 * three steps — a model (the settings'), a project and the scene and permissions (the Harnix
 * app's) — each drawn until it is done or closed. Done is read from what is really there: a
 * provider configured, a project listed. A step done while the page is open shows as done; one
 * already done when the page comes up is not drawn. Nothing of trust is said here: that is the
 * banner over a conversation's.
 *
 * Its own backend: no key in the environment and nothing in the settings, so no model; `code`
 * served beside the default scene, as the `harnix` binary does when no profile is named. No turn
 * is run: nothing here is replayed.
 */

import type { Page } from "@playwright/test";
import { type Backend, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { connected, open, settingsSection } from "./page";

const FIRST_RUN = { tag: ["@feature:session-space", "@layer:ui"] };

const MODEL = "settings.setup.model";
const PROJECT = "harnix-app.setup.project";
const SCENE = "harnix-app.setup.scene";

let fresh: Backend;
test.beforeAll(async () => {
  fresh = await startHarnix(launchFor("replay", { env: { ANTHROPIC_API_KEY: "", ANTHROPIC_AUTH_TOKEN: "" }, profile: '[engine]\nscenes = ["code"]\n' }));
});
test.afterAll(async () => {
  await fresh?.dispose();
});

const step = (page: Page, id: string) => page.locator(`[data-testid="setup-step"][data-step="${id}"]`);

const call = (page: Page, method: string, params: Record<string, unknown>) =>
  page.evaluate(([method, params]) => (window as any).harnix.app.call(method, params), [method, params] as const);

test("a fresh profile's empty page has the three steps; the scene step says the defaults, offers code, and once closed stays closed", FIRST_RUN, async ({ page }) => {
  await open(page, fresh.url);
  await expect(step(page, MODEL)).toHaveAttribute("data-state", "todo");
  await expect(step(page, PROJECT)).toHaveAttribute("data-state", "todo");
  const scene = step(page, SCENE);
  await expect(scene).toHaveAttribute("data-state", "todo");
  // In the order the steps are taken: the model, a project, then what a conversation starts with.
  await expect(page.getByTestId("home-cards").locator("[data-testid=setup-step]")).toHaveCount(3);
  const order = await page.getByTestId("home-cards").locator("[data-testid=setup-step]").evaluateAll((all) => all.map((e) => (e as HTMLElement).dataset.step));
  expect(order).toEqual([MODEL, PROJECT, SCENE]);

  // The default scene stays `harnix`; the mode is the engine's default; `code` is offered beside it.
  await expect(scene.getByTestId("setup-scene-default")).toHaveAttribute("data-scene", "harnix");
  await expect(scene.getByTestId("setup-scene-mode")).toHaveAttribute("data-mode", "default");
  await expect(scene.getByTestId("setup-scene-offer")).toBeVisible();
  await scene.getByTestId("setup-scene-code").click();
  await expect(page.getByTestId("new-scene")).toHaveValue("code");
  await page.keyboard.press("Escape");
  await expect(page.locator("#modal")).toHaveCount(0);

  // "Change the defaults" is the settings' engine section, where the permission mode is.
  await scene.getByTestId("setup-scene-settings").click();
  await expect(settingsSection(page, "engine")).toHaveAttribute("data-state", "current");
  await page.keyboard.press("Escape");
  await expect(page.locator("#modal")).toHaveCount(0);

  await scene.getByTestId("setup-scene-got").click();
  await expect(scene).toHaveCount(0);
  await page.reload();
  await connected(page);
  await expect(step(page, MODEL)).toBeVisible();
  await expect(step(page, SCENE)).toHaveCount(0);
});

test("the model step opens Settings · Models, and is done once a provider is added there", FIRST_RUN, async ({ page }) => {
  await open(page, fresh.url);
  const model = step(page, MODEL);
  await expect(model).toHaveAttribute("data-state", "todo");
  await model.getByTestId("setup-model-open").click();
  await expect(settingsSection(page, "models")).toHaveAttribute("data-state", "current");
  try {
    const list = page.getByTestId("providers");
    await list.getByTestId("provider-add").click();
    const form = page.getByTestId("provider-form");
    await form.getByTestId("provider-provider_id").fill("first-run");
    await form.getByTestId("provider-type-anthropic").click();
    await form.getByTestId("provider-base_url").fill("http://127.0.0.1:9/");
    await form.getByTestId("provider-api_key").fill("sk-live-test-first-run");
    await form.getByTestId("provider-default_model").fill("model-first-run");
    await form.getByTestId("provider-save").click();
    await expect(list.locator('[data-provider="first-run"]')).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator("#modal")).toHaveCount(0);

    // Read again when the panel closed: done, naming the model new sessions run on.
    await expect(model).toHaveAttribute("data-state", "done");
    await expect(model).toContainText("model-first-run");
    // The note over the composer agrees.
    await expect(page.getByTestId("model-setup")).toHaveCount(0);

    // Done before the page came up: not drawn at all.
    await page.reload();
    await connected(page);
    await expect(step(page, PROJECT)).toBeVisible();
    await expect(step(page, MODEL)).toHaveCount(0);
  } finally {
    await call(page, "settings.setProvider", { provider_id: "first-run", delete: true }).catch(() => undefined);
  }
});

test("the project step adds one, is done once it is listed, opens it in the projects space, and says nothing of trust", FIRST_RUN, async ({ page }) => {
  await open(page, fresh.url);
  const project = step(page, PROJECT);
  await expect(project).toHaveAttribute("data-state", "todo");
  await project.getByTestId("setup-project-add").click();
  const name = `first-run-${Date.now()}`;
  await page.getByRole("dialog").getByRole("textbox").first().fill(name);
  await page.getByTestId("project-add").click();
  await expect(page.locator("#modal")).toHaveCount(0);
  try {
    await expect(project).toHaveAttribute("data-state", "done");
    await expect(project).toContainText(name);
    // The new project is not trusted, and the empty page says nothing of it: trust is the banner's,
    // over a conversation in it.
    await expect(project).not.toContainText(/trust|信任/i);
    await expect(page.getByTestId("trust-banner")).toHaveCount(0);

    await project.getByTestId("setup-project-open").click();
    await expect(page.locator("#main")).toHaveAttribute("data-space", "projects");
    await expect(page.getByTestId("projects").locator("[data-project]", { hasText: name })).toHaveAttribute("data-open", "true");

    // There is a project when the page comes up: the step is not drawn.
    await page.reload();
    await connected(page);
    await expect(step(page, MODEL)).toBeVisible();
    await expect(step(page, PROJECT)).toHaveCount(0);
  } finally {
    const { workspaces } = (await call(page, "harnix-app.workspaces.list", {})) as { workspaces: { id: string; title?: string; path: string }[] };
    for (const w of workspaces.filter((w) => (w.title ?? w.path).includes(name))) await call(page, "harnix-app.workspaces.remove", { id: w.id });
  }
});
