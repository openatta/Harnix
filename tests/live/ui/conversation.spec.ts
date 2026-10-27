/**
 * The conversation, driven the way a person drives it, against a real backend.
 *
 * Turns are replayed from recorded fixtures, so the model does the same thing every run: the same
 * tool, the same arguments, the same permission ask. What is under test is everything between a
 * keystroke and a pixel — and the parity list the React shell had to meet before the old interface
 * could go (design §10.2).
 */

import { expect, test } from "@harnix/testkit";

import { newSession, open, reply, say, watchConsole } from "./page";

const TAGS = { tag: ["@feature:sessions", "@layer:ui"] };

let URL = "";
let PROJECT = "";
test.beforeEach(({ harnix }) => {
  URL = harnix.url;
  PROJECT = harnix.project;
});

test("typing a message and pressing Enter puts it in the flow, and the reply after it", TAGS, async ({ page }) => {
  const errors = watchConsole(page);
  await open(page, URL);
  await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(page.locator('[data-testid="conv-message"]').first()).toContainText("Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  // Markdown, not raw text.
  await expect(reply(page).locator("p").first()).toBeVisible();
  await expect(page.getByTestId("conv-input")).toHaveValue("");
  await expect(page.locator("#head [data-testid=head-tokens]")).toBeVisible();
  expect(errors).toEqual([]);
});

test("a tool call draws a row that expands, and opens in the details pane in full", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "calls-a-tool");
  await say(page, "Use the Bash tool to run exactly: echo harnix-fixture");
  const row = page.locator('[data-testid="conv-tool"]').filter({ hasText: /bash/i }).first();
  await expect(row).toBeVisible({ timeout: 30_000 });
  await expect(row).toHaveAttribute("data-state", "ok", { timeout: 30_000 });
  await row.getByTestId("conv-tool-toggle").click();
  await expect(row.getByTestId("conv-tool-input")).toContainText("echo harnix-fixture");
  await row.getByTestId("open-in-details").click();
  await expect.poll(async () => (await page.locator("#details").boundingBox())?.width ?? 0).toBeGreaterThan(200);
  await expect(page.getByTestId("details-tool")).toContainText(/bash/i);
});

test("a permission ask is answerable from the flow, with a countdown", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "asks-permission");
  await say(page, "Use the Bash tool to run exactly: touch harnix-permission-fixture");
  const ask = page.getByTestId("conv-ask");
  await expect(ask).toBeVisible({ timeout: 30_000 });
  await expect(ask.getByTestId("conv-ask-countdown")).not.toBeEmpty();
  await expect(ask.locator("button")).toHaveCount(4);
  await ask.getByTestId("conv-ask-allow").click();
  await expect(ask).toHaveAttribute("data-state", "answered");
});

test("a message sent during a turn queues, and can be withdrawn", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "asks-permission");
  await say(page, "Use the Bash tool to run exactly: touch harnix-permission-fixture");
  await expect(page.getByTestId("conv-ask")).toBeVisible({ timeout: 30_000 });
  // The turn is waiting on the ask, so this one waits behind it.
  await say(page, "and then this");
  const queued = page.getByTestId("conv-queue-item");
  await expect(queued).toContainText("and then this");
  await queued.getByTestId("conv-queue-remove").click();
  await expect(queued).toHaveCount(0);
});

test("stopping a turn says it was interrupted, not that it failed", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "asks-permission");
  await say(page, "Use the Bash tool to run exactly: touch harnix-permission-fixture");
  await expect(page.getByTestId("conv-ask")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("conv-interrupt").click();
  const note = page.getByTestId("conv-note").last();
  await expect(note).toContainText(/已中断|interrupted/i, { timeout: 20_000 });
  await expect(page.getByTestId("conv-send")).toBeVisible();
});

test("a sub-agent draws as a block of its own", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "spawns-agent");
  await say(
    page,
    "Use the Agent tool exactly once to launch a general-purpose sub-agent with the description 'list files' and the prompt 'Run ls in the current directory and reply with the file names only.' When it returns, reply with one line: 'sub-agent found:' followed by the names.",
  );
  await expect(page.getByTestId("conv-agent").first()).toBeVisible({ timeout: 60_000 });
  await expect(reply(page)).toBeVisible({ timeout: 60_000 });
});

test("/ offers the engine's commands, and choosing one fills the composer", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "says-ready");
  const input = page.getByTestId("conv-input");
  await input.click();
  await input.pressSequentially("/");
  const menu = page.getByTestId("conv-menu");
  await expect(menu).toBeVisible();
  const first = menu.getByTestId("conv-menu-item").first();
  const name = await first.getAttribute("data-id");
  await first.click();
  await expect(input).toHaveValue(`/${name} `);
});

test("@ offers the project's files, and choosing one references it", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "says-ready", PROJECT);
  const input = page.getByTestId("conv-input");
  await input.click();
  await input.pressSequentially("look at @READ");
  const item = page.getByTestId("conv-menu").getByTestId("conv-menu-item").first();
  await expect(item).toHaveAttribute("data-id", "README.md");
  await item.click();
  await expect(input).toHaveValue("look at @README.md ");
});

test("a file can be attached, shown, removed, and sent with the message", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "says-ready");
  const file = page.getByTestId("conv-file");
  await file.setInputFiles({ name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("hello") });
  await expect(page.getByTestId("conv-attachment")).toContainText("notes.txt");
  await page.getByTestId("conv-attachment-remove").click();
  await expect(page.getByTestId("conv-attachment")).toHaveCount(0);
  await file.setInputFiles({ name: "pixel.png", mimeType: "image/png", buffer: PNG });
  await expect(page.getByTestId("conv-attachment").locator("img")).toBeVisible();
  await say(page, "Say exactly: fixture ready");
  await expect(page.getByTestId("conv-attachment")).toHaveCount(0);
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
});

test("a draft is kept per session, and kept through a disconnect", TAGS, async ({ page }) => {
  await open(page, URL);
  const first = await newSession(page, "says-ready");
  await page.getByTestId("conv-input").fill("half a thought");
  await newSession(page, "says-ready");
  await expect(page.getByTestId("conv-input")).toHaveValue("");
  await page.locator(`#side .srow[data-sid="${first}"]`).click();
  await expect(page.getByTestId("conv-input")).toHaveValue("half a thought");
  // Offline: nothing can be sent, and nothing typed is lost.
  await page.evaluate(() => (window as unknown as { harnix: { client: { close(): void } } }).harnix.client.close());
  await expect(page.locator("#conn")).not.toHaveAttribute("data-state", "open");
  await expect(page.getByTestId("conv-input")).toBeDisabled();
  await expect(page.getByTestId("conv-input")).toHaveValue("half a thought");
});

test("a session reopens from the sidebar after a reload, with its history", TAGS, async ({ page }) => {
  await open(page, URL);
  const id = await newSession(page, "says-ready");
  await say(page, "Say exactly: fixture ready");
  await expect(reply(page)).toContainText(/fixture ready/i, { timeout: 30_000 });
  await page.reload();
  await expect(page.locator("#conn.on")).toBeVisible({ timeout: 15_000 });
  await page.locator(`#side .srow[data-sid="${id}"]`).click();
  await expect(reply(page)).toContainText(/fixture ready/i);
  await expect(page.locator('[data-testid="conv-message"]').first()).toContainText("Say exactly: fixture ready");
});

/** A one-pixel PNG. */
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=",
  "base64",
);

test("the host API inserts into the composer, reveals in details, and refuses what is not reachable", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "says-ready");
  await page.getByTestId("conv-input").fill("before ");
  await page.evaluate(() => (window as any).harnix.host.composer.insert("@inserted"));
  await expect(page.getByTestId("conv-input")).toHaveValue("before @inserted");
  await page.evaluate(() =>
    (window as any).harnix.host.reveal({ kind: "tool", name: "Bash", id: "t-1", input: { command: "ls" }, result: "ok", status: "ok" }),
  );
  await expect(page.getByTestId("details-tool")).toContainText("Bash");
  const refused = await page.evaluate(() =>
    (window as any).harnix.host.call("config.setProvider", {}).then(
      () => "answered",
      (e: Error) => e.message,
    ),
  );
  expect(refused).toMatch(/config\.setProvider/);
  const tasks = await page.evaluate(async () => {
    const { host } = (window as any).harnix;
    const current = host.session.current();
    return host.instances.tasks(current.instance, current.sessionId);
  });
  expect(tasks).toHaveProperty("tasks");
});

test("the composer's model menu switches the session's model", TAGS, async ({ page }) => {
  await open(page, URL);
  await newSession(page, "says-ready");
  await page.getByTestId("conv-model").click();
  await expect(page.getByTestId("conv-model-option").first()).toBeVisible();
  await page.getByTestId("conv-model-custom").fill("claude-switched-model");
  await page.getByTestId("conv-model-custom").press("Enter");
  await expect(page.getByTestId("conv-model")).toContainText("claude-switched-model");
  await expect(page.getByTestId("conv-model-menu")).toHaveCount(0);
});
