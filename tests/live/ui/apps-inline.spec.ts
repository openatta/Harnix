/**
 * What the shell draws for a runtime application from its manifest alone, in the browser: the
 * showcase (`examples/apps/showcase`) installed from a file. Its tool's row is drawn as its
 * `[[inline.tool_row]]` says; a card its backend appends to a session is drawn as its
 * `[[inline.flow_block]]` says, and its buttons call the backend (asking first where it says so),
 * with what came back shown in the card; the composer's `/` and `@` menus offer the backend's
 * candidates; its chats space's sessions are grouped as the backend answers (its first space draws
 * its own left column, which no grouping reaches). Its space in the left column
 * carries the application mark, which the settings turn off and on; and Settings → Applications
 * clears what it keeps.
 */

import type { Page } from "@playwright/test";
import { type Backend, type TestClient, connect, expect, test, topology } from "@harnix/testkit";

import { SHOWCASE_CHATS, SHOWCASE_ID, showcasePackage, startApps } from "../api/apps-host";
import { newSession, open, openSettings, reply, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:apps-inline", "@layer:ui"] };

const M = (name: string) => `app.${SHOWCASE_ID}.${name}`;

let backend: Backend;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  test.setTimeout(600_000);
  const { path } = showcasePackage();
  backend = await startApps({ topologies: process.env.HARNIX_TOPOLOGY ? [topology()] : undefined });
  await withApi((api) => api.call("harnix.apps.installFile", { path }));
});
test.afterAll(async () => { await backend?.dispose(); });

async function withApi<T>(f: (api: TestClient) => Promise<T>): Promise<T> {
  const api = await connect(backend, { topology: topology() });
  try {
    return await f(api);
  } finally {
    api.close();
  }
}

/** The page's open session. */
const current = (page: Page) =>
  page.evaluate(() => {
    const c = (window as any).harnix.app.snapshot.current;
    return { instance: c.instance as string, session_id: c.sessionId as string };
  });

test("a tool's row is drawn as its manifest says", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await newSession(page, "app-echo");
  await say(page, "Use the showcase's echo tool");
  const ask = page.getByTestId("conv-ask").last();
  await expect(ask).toBeVisible({ timeout: 30_000 });
  // The card names the tool as its row does, not by its wire name (review G7).
  await expect(ask).toContainText(/Showcase echo|示例应用回声/);
  await expect(ask).not.toContainText("app__");
  // Said aloud by the same name.
  const said = page.getByTestId("ask-announce");
  await expect(said).toContainText(/Showcase echo|示例应用回声/);
  await expect(said).not.toContainText("app__");
  await ask.getByTestId("conv-ask-allow").click();
  const row = page.locator(`[data-inline="tool.row"][data-app="${SHOWCASE_ID}"][data-tool="echo"]`);
  await expect(row).toBeVisible({ timeout: 30_000 });
  await expect(row.getByTestId("conv-tool-name")).toHaveText(/Showcase echo · showcase-echo-42|示例应用回声 · showcase-echo-42/);
  await expect(row.getByTestId("conv-tool-summary")).toHaveText("showcase-echo-42", { timeout: 30_000 });
  await expect(row.getByTestId("conv-tool")).toHaveAttribute("data-state", "ok");
  // Declared unfolded: the result is shown without a click.
  await expect(row.getByTestId("conv-tool-result")).toContainText("showcase-echo-42");
  // The shell's own glyph, by the name the manifest gave.
  await expect(row.locator(".app-inline-icon svg")).toHaveCount(1);
  await expect(reply(page)).toBeVisible({ timeout: 30_000 });
  expect(errors).toEqual([]);
});

test("a card the backend appends is drawn, and its buttons call the backend", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, backend.url);
  await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  const s = await current(page);
  await withApi((api) => api.call(M("card.post"), { ...s, title: "A card for the browser", body: "Some **bold** words" }));

  const card = page.locator(`[data-testid="app-card"][data-app="${SHOWCASE_ID}"][data-kind="card"]`);
  await expect(card).toBeVisible({ timeout: 15_000 });
  await expect(card.getByTestId("app-card-title")).toHaveText("A card for the browser");
  await expect(card).toContainText(/from Showcase|来自 Showcase/);
  await expect(card.getByTestId("app-card-field")).toHaveCount(2);
  // The body is Markdown, as the shell renders it.
  await expect(card.getByTestId("app-card-body").locator("strong")).toHaveText("bold");

  // Asks first, in the shell's own dialog: cancelled, nothing is called.
  const before = (await withApi((api) => api.call(M("state")))).counter;
  await card.locator('[data-method="counter.bump"]').click();
  await expect(page.locator("#modal")).toContainText(/Count one more on the showcase's counter\?|在示例应用的计数器上再数一个？/);
  await page.locator("#modal").getByRole("button", { name: /Cancel|取消/ }).click();
  expect((await withApi((api) => api.call(M("state")))).counter).toBe(before);
  await card.locator('[data-method="counter.bump"]').click();
  await page.getByTestId("confirm").click();
  await expect(card.getByTestId("app-card-done")).toHaveText(new RegExp(`(The counter is at|计数器到了) ${before + 1}`));

  // Without a confirm: called at once, with the payload in its params.
  await card.locator('[data-method="notes.add"]').click();
  await expect(card.getByTestId("app-card-done")).toHaveText(/notes on the board|条留言/);
  const notes = (await withApi((api) => api.call(M("notes.list")))).notes.map((n: any) => n.text);
  expect(notes).toContain("A card for the browser");
  expect(errors).toEqual([]);
});

test("the composer's menus offer the backend's candidates", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await withApi((api) => api.call(M("notes.add"), { text: "zebra crossing" }));
  await open(page, backend.url);
  await newSession(page, "says-ready");
  const input = page.getByTestId("conv-input");
  const menu = page.getByTestId("conv-menu");

  await input.click();
  await input.pressSequentially("/showcase");
  await expect(menu).toBeVisible();
  await expect(menu.locator('[data-id="showcase-bump"]')).toBeVisible();
  await expect(menu.locator('[data-id="showcase-clear"]')).toBeVisible();
  // Chosen, it runs the backend's method: nothing is inserted, and the backend's words are shown.
  const before = (await withApi((api) => api.call(M("state")))).counter;
  await menu.locator('[data-id="showcase-bump"]').click();
  await expect(input).toHaveValue("");
  await expect(page.locator("#banner")).toContainText(/Counted one more|又数了一个/);
  expect((await withApi((api) => api.call(M("state")))).counter).toBe(before + 1);

  // The `@` source: the backend's notes for what is typed.
  await input.click();
  await input.pressSequentially("look at @zebra");
  const item = menu.locator('[data-id="notes:zebra crossing"]');
  await expect(item).toBeVisible({ timeout: 10_000 });
  await item.click();
  await expect(input).toHaveValue("look at “zebra crossing” ");
  expect(errors).toEqual([]);
});

test("its chats space's sessions are grouped as its backend answers", TAGS, async ({ page }) => {
  const [carded, plain] = await withApi(async (api) => {
    const made: { instance: string; session_id: string }[] = [];
    for (let i = 0; i < 2; i += 1) {
      const created = await api.call("session.create", { space: SHOWCASE_CHATS, options: { recorder: { name: "says-ready" } } });
      const s = { instance: created.instance, session_id: created.session_id };
      await api.call("harnix.attach", s);
      await api.call("harnix.send", { ...s, message: "Say exactly: fixture ready" });
      await api.waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id, { timeout: 60_000, describe: "the turn" });
      made.push(s);
    }
    await api.call(M("card.post"), { ...made[0]!, title: "grouped" });
    return made;
  });
  await open(page, backend.url);
  await page.locator(`#side [data-space="${SHOWCASE_CHATS}"]`).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", SHOWCASE_CHATS);
  const cards = page.locator('#sessions [data-testid="space-group"][data-group="m:cards"]');
  await expect(cards).toBeVisible({ timeout: 15_000 });
  await expect(cards).toContainText(/With a card|有卡片的/);
  await expect(cards.locator(`[data-sid="${carded!.session_id}"]`)).toBeVisible();
  const other = page.locator('#sessions [data-testid="space-group"][data-group="other"]');
  await expect(other).toContainText(/Without a card|没有卡片的/);
  await expect(other.locator(`[data-sid="${plain!.session_id}"]`)).toBeVisible();
  await expect(cards.locator(`[data-sid="${plain!.session_id}"]`)).toHaveCount(0);
  // Its first space's left column is its own frame: nothing of the shell's list, grouped or not.
  await page.locator(`#side [data-space="${SHOWCASE_ID}"]`).click();
  await expect(page.locator(`iframe.hx-frame[data-app="${SHOWCASE_ID}"][data-point="space.sidebar"]`)).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('#sessions [data-testid="space-group"]')).toHaveCount(0);
});

async function applications(page: Page): Promise<void> {
  await openSettings(page);
  await page.locator('.settings-nav [data-section="apps"]').click();
  await expect(page.locator(".settings-page")).toBeVisible();
}

test("its space carries the application mark, which the settings turn off and on", TAGS, async ({ page }) => {
  await open(page, backend.url);
  const mark = page.locator(`#side [data-space="${SHOWCASE_ID}"] [data-testid="space-app-mark"]`);
  await expect(mark).toBeVisible({ timeout: 15_000 });
  await expect(mark).toHaveAttribute("data-trust", "unsigned");
  // The product's own spaces carry none.
  await expect(page.locator('#side [data-space="sessions"] [data-testid="space-app-mark"]')).toHaveCount(0);

  await applications(page);
  const toggle = page.getByTestId("apps-mark-toggle");
  await expect(toggle).toBeChecked();
  await toggle.uncheck();
  await expect(mark).toHaveCount(0);
  // Kept by this browser.
  await open(page, backend.url);
  await expect(page.locator(`#side [data-space="${SHOWCASE_ID}"]`)).toBeVisible({ timeout: 15_000 });
  await expect(mark).toHaveCount(0);
  await applications(page);
  await page.getByTestId("apps-mark-toggle").check();
  await expect(mark).toBeVisible();
});

test("Settings → Applications shows its permissions, and clears what it keeps", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await withApi((api) => api.call(M("notes.add"), { text: "to be cleared" }));
  await open(page, backend.url);
  await applications(page);
  const row = page.locator(`[data-testid="app-installed"][data-app="${SHOWCASE_ID}"]`);
  await expect(row).toHaveAttribute("data-state", "running");
  const permissions = row.getByTestId("app-permissions");
  await expect(permissions.locator('[data-capability="hub"]', { hasText: "harnix.entries.append" })).toBeVisible();
  await expect(permissions.locator('[data-capability="inline"]', { hasText: "/showcase" })).toBeVisible();
  await expect(permissions.locator('[data-inline="flow_block"]')).toContainText("card");
  const data = row.getByTestId("app-data");
  expect(Number(await data.getAttribute("data-bytes"))).toBeGreaterThan(0);

  await row.getByTestId("app-clear").click();
  const asked = page.getByTestId("app-clear-ask");
  await expect(asked).toBeVisible();
  // Escape closes the question, and only the question.
  await page.keyboard.press("Escape");
  await expect(asked).toHaveCount(0);
  await expect(page.locator(".settings-page")).toBeVisible();
  expect((await withApi((api) => api.call(M("notes.list")))).notes.length).toBeGreaterThan(0);

  await row.getByTestId("app-clear").click();
  await page.getByTestId("app-clear-ask-confirm").click();
  await expect(row.getByTestId("app-data")).toHaveAttribute("data-bytes", "0", { timeout: 15_000 });
  await expect(row.getByTestId("app-clear")).toBeDisabled();
  await expect(page.locator("#banner")).toContainText(/cleared|清掉了/);
  expect((await withApi((api) => api.call(M("notes.list")))).notes).toEqual([]);
  await expect(row).toHaveAttribute("data-state", "running");
  expect(errors).toEqual([]);
});
