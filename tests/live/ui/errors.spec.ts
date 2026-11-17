/**
 * A failure told to a person (harnix-app.md, "错误呈现"): a turn that fails says what happened and
 * what to do next in the conversation's own note, with the engine's words and code in its tooltip,
 * and for the model's own problems (no model, a key the service rejects, a wrong model name) a way
 * to the settings where it is put right.
 *
 * Two backends of their own, neither replaying: one with no key and no provider, and one whose
 * provider is a stub that fails on purpose (`api/stub-provider.ts`). The engine retries a rate
 * limit or a dropped connection before it gives up, so those turns are given time.
 */

import type { Page } from "@playwright/test";
import { type Backend, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

import { FAULTS, stubProvider, useStub, type Fault, type StubProvider } from "../api/stub-provider";
import { open, say, settingsSection, toSessions } from "./page";

const TAGS = { tag: ["@feature:session-space", "@layer:ui"] };
const MODELS = /模型与凭据|Models & credentials/;

let bare: Backend;
let failing: Backend;
let stub: StubProvider;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  stub = await stubProvider();
  bare = await startHarnix(launchFor("none", { env: { ANTHROPIC_API_KEY: "", ANTHROPIC_AUTH_TOKEN: "" } }));
  failing = await startHarnix(launchFor("none", { env: { ANTHROPIC_API_KEY: "", ANTHROPIC_AUTH_TOKEN: "" } }));
});
test.afterAll(async () => {
  await stub?.close();
  await bare?.dispose();
  await failing?.dispose();
});

/** A session on the engine's own model (no recorder), open in the session space. */
async function plainSession(page: Page): Promise<void> {
  await toSessions(page);
  await page.evaluate(() => (window as any).harnix.app.create({ scene: "harnix" }));
}

/** The failure's note: the turn's, or where there is no session to hold it the banner's. */
const told = (page: Page) => page.locator('[data-testid="conv-note"][data-state="error"], #banner[data-tone="error"]').last();

test("no model: a message is answered with where to add one, and the way there opens it", TAGS, async ({ page }) => {
  await open(page, bare.url);
  await plainSession(page);
  await say(page, "hello");
  const note = told(page);
  await expect(note).toContainText(MODELS, { timeout: 60_000 });
  // Not a bare code or a stack.
  await expect(note).not.toContainText(/-32\d{3}|at .*\(.*:\d+:\d+\)/);
  // The engine sent nothing without a key, and said so: its kind is in the tooltip.
  await expect(note).toHaveAttribute("title", /no_credentials/);
  const action = page.getByTestId("conv-note-action");
  await expect(action).toBeVisible();
  await action.click();
  await expect(settingsSection(page, "models")).toHaveAttribute("data-state", "current");
});

test.describe("a provider that fails", () => {
  test.beforeAll(async () => {
    const api = await connect(failing);
    try {
      await useStub(api, stub);
    } finally {
      api.close();
    }
  });

  const CASES: [string, Fault, RegExp, string, boolean][] = [
    ["a key the service rejects", FAULTS.auth, /拒绝了密钥|rejected the key/, "invalid x-api-key", true],
    ["a model name it does not know", FAULTS.wrongModel, /模型名|model name/, "no-such-model", true],
    // The engine keeps the service's own words about a 429: summed up after the interface's.
    ["a rate limit", FAULTS.rateLimited, /(限流|rate limiting).*rate limit exceeded/, "rate_limited: rate limit exceeded", false],
    ["a service that cannot be reached", FAULTS.drop, /连不上模型服务|Cannot reach the model service/, "", false],
  ];
  for (const [name, fault, words, raw, toSettings] of CASES) {
    test(`${name}: said in words, the engine's own in the tooltip${toSettings ? ", with the way to the settings" : ""}`, TAGS, async ({ page }) => {
      stub.fail(fault);
      await open(page, failing.url);
      await plainSession(page);
      await say(page, "hello");
      const note = told(page);
      await expect(note).toContainText(words, { timeout: 120_000 });
      await expect(note).not.toContainText(/at .*\(.*:\d+:\d+\)/);
      if (raw) await expect(note).toHaveAttribute("title", new RegExp(raw));
      const action = page.getByTestId("conv-note-action");
      if (toSettings) {
        await action.click();
        await expect(settingsSection(page, "models")).toHaveAttribute("data-state", "current");
      } else {
        await expect(action).toHaveCount(0);
      }
      // The message box works again: a failed turn is over, not stuck.
      await expect(page.getByTestId("conv-send")).toBeVisible();
    });
  }
});
