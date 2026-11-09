/**
 * The SDK against its own mock host: what a page's tests would do, and what the shell relies on —
 * the hello, calls and their refusals, topics, the environment, sessions, the heartbeat, keys,
 * the auto-height reporter and modals closing themselves.
 */

import { afterEach, describe, expect, it } from "vitest";

import { BridgeError, PROTOCOL, admits, connect, inDialog, shellKey, type HarnixApp } from "../src/index.ts";
import { FRAME_HUB } from "../src/protocol.ts";
import { createMockHost, type MockHost } from "../src/mock.ts";

let open: { host: MockHost; app: HarnixApp }[] = [];

async function connected(options: Parameters<typeof createMockHost>[0] = {}) {
  const host = createMockHost(options);
  const app = await connect({ port: host.port, timeoutMs: 2000 });
  open.push({ host, app });
  return { host, app };
}

afterEach(() => {
  for (const { host, app } of open) {
    app.disconnect();
    host.close();
  }
  open = [];
});

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("the bridge", () => {
  it("says hello and learns what it is", async () => {
    const { host, app } = await connected({ app: "acme.notes", point: "home.card", autoHeight: true, env: { theme: "dark", locale: "zh-CN" } });
    expect(PROTOCOL).toBe("harnix-ui-bridge@1");
    expect(host.requests[0]).toEqual({ method: "hello", params: { sdk: "0.1.0" } });
    expect(app.surface).toMatchObject({ app: "acme.notes", point: "home.card", autoHeight: true });
    expect(app.env).toMatchObject({ theme: "dark", locale: "zh-CN", visible: true, modal: false });
  });

  it("calls its own methods, and is refused another's, with the reason", async () => {
    const { host, app } = await connected({ methods: { "app.acme.notes.list": () => ({ notes: ["a"] }) }, hub: ["harnix.sessions"] });
    expect(await app.call("app.acme.notes.list")).toEqual({ notes: ["a"] });
    const refused = await app.call("harnix.apps.uninstall", { id: "acme.notes" }).catch((e: unknown) => e);
    expect(refused).toBeInstanceOf(BridgeError);
    expect((refused as BridgeError).code).toBe("refused");
    expect(host.refused.map((r) => r.name)).toEqual(["harnix.apps.uninstall"]);
    // A name in the hub goes through to the host (which has no such method here: a failure, not a refusal).
    const failed = (await app.call("harnix.sessions").catch((e: unknown) => e)) as BridgeError;
    expect(failed.code).toBe("failed");
  });

  it("follows a topic until it unsubscribes, and is refused a topic not its own", async () => {
    const { host, app } = await connected();
    const seen: unknown[] = [];
    const off = app.subscribe("app.acme.notes.changed", {}, (v) => seen.push(v));
    await off.ready;
    host.publish("app.acme.notes.changed", { n: 1 });
    host.publish("app.acme.other.changed", { n: 9 });
    await tick();
    expect(seen).toEqual([{ n: 1 }]);
    off();
    await tick();
    host.publish("app.acme.notes.changed", { n: 2 });
    await tick();
    expect(seen).toEqual([{ n: 1 }]);
    expect(host.requests.some((r) => r.method === "unsubscribe")).toBe(true);
    const bad = app.subscribe("harnix.sessions.status", {}, () => undefined);
    await expect(bad.ready).rejects.toMatchObject({ code: "refused" });
  });

  it("follows the environment and says what changed", async () => {
    const { host, app } = await connected({ env: { tokens: { "--accent": "#2563eb" } } });
    const changes: string[][] = [];
    app.onEnv((_env, changed) => changes.push(changed));
    host.setEnv({ theme: "dark", tokens: { "--accent": "#4f8cff" } });
    await tick();
    expect(app.env.theme).toBe("dark");
    expect(app.env.tokens["--accent"]).toBe("#4f8cff");
    expect(changes).toEqual([["theme", "tokens"]]);
  });

  it("knows its space's open session, and follows it", async () => {
    const { host, app } = await connected({ current: { instance: "system", sessionId: "s1" } });
    expect(app.session.current()).toEqual({ instance: "system", sessionId: "s1" });
    const seen: unknown[] = [];
    app.session.onChange((c) => seen.push(c));
    host.setCurrent(null);
    await tick();
    expect(app.session.current()).toBeNull();
    expect(seen).toEqual([null]);
  });

  it("answers the heartbeat", async () => {
    const { host } = await connected();
    expect(await host.ping()).toBe("pong");
  });

  it("asks the shell for its dialogs and answers what they answered", async () => {
    const { app } = await connected({
      requests: {
        confirm: (p) => (p as { text: string }).text === "Delete it?",
        openModal: (p) => ({ opened: (p as { id: string }).id }),
      },
    });
    expect(await app.confirm("Delete it?", { danger: true })).toBe(true);
    expect(await app.confirm("Something else?")).toBe(false);
    expect(await app.openModal("edit", { n: 1 })).toEqual({ opened: "edit" });
  });

  it("closes a modal with its result, and takes new params", async () => {
    const { host, app } = await connected({ point: "modal", id: "edit", params: { n: 1 } });
    expect(app.surface.params).toEqual({ n: 1 });
    const seen: unknown[] = [];
    app.onParams((p) => seen.push(p));
    host.setParams({ n: 2 });
    await tick();
    expect(seen).toEqual([{ n: 2 }]);
    expect(app.surface.params).toEqual({ n: 2 });
    app.close({ saved: true });
    expect(await host.next("close")).toEqual({ result: { saved: true } });
  });

  it("uploads through the shell, the bytes moved over the port", async () => {
    let got: { name?: string; size?: number } = {};
    const { app } = await connected({
      requests: {
        upload: (p) => {
          const { name, data } = p as { name: string; data: ArrayBuffer };
          got = { name, size: data.byteLength };
          return { path: `/uploads/${name}` };
        },
      },
    });
    const file = new File(["hello"], "note.txt", { type: "text/plain" });
    expect(await app.upload(file)).toEqual({ path: "/uploads/note.txt" });
    expect(got).toEqual({ name: "note.txt", size: 5 });
  });

  it("rejects what is pending when it disconnects", async () => {
    const { app } = await connected({ requests: { confirm: () => new Promise(() => undefined) } });
    const asked = app.confirm("never answered");
    app.disconnect();
    await expect(asked).rejects.toMatchObject({ code: "closed" });
  });
});

describe("keys", () => {
  const press = (key: string, init: KeyboardEventInit = {}) => {
    const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
    document.body.dispatchEvent(event);
    return event;
  };

  it("hands the shell its shortcuts, and keeps Escape the page's outside a dialog", async () => {
    const { host } = await connected({ point: "space" });
    const page: string[] = [];
    const listen = (e: KeyboardEvent) => page.push(e.key);
    window.addEventListener("keydown", listen);
    try {
      expect(press("k", { metaKey: true }).defaultPrevented).toBe(true);
      expect(press("Escape").defaultPrevented).toBe(false);
      expect(press("a").defaultPrevented).toBe(false);
      await tick();
      expect(host.events.filter((e) => e.name === "key").map((e) => (e.data as { key: string }).key)).toEqual(["k"]);
      expect(page).toEqual(["Escape", "a"]);
    } finally {
      window.removeEventListener("keydown", listen);
    }
  });

  it("in a dialog, gives Escape to the shell and to nobody else", async () => {
    const { host } = await connected({ point: "modal", id: "edit" });
    const page: string[] = [];
    const listen = (e: KeyboardEvent) => page.push(e.key);
    document.addEventListener("keydown", listen);
    try {
      expect(press("Escape").defaultPrevented).toBe(true);
      await tick();
      expect(host.events.map((e) => e.name)).toContain("key");
      expect(page).toEqual([]);
    } finally {
      document.removeEventListener("keydown", listen);
    }
  });

  it("passes no key at all while a dialog of the shell is over the frame", async () => {
    const { host } = await connected({ point: "space" });
    host.setEnv({ modal: true });
    await tick();
    const page: string[] = [];
    const listen = (e: KeyboardEvent) => page.push(e.key);
    document.addEventListener("keydown", listen);
    try {
      press("a");
      press("Escape");
      press("k", { ctrlKey: true });
      await tick();
      expect(page).toEqual([]);
      expect(host.events.filter((e) => e.name === "key")).toEqual([]);
    } finally {
      document.removeEventListener("keydown", listen);
    }
  });

  it("knows which keys are the shell's", () => {
    const key = (k: string, mods: Partial<{ meta: boolean; ctrl: boolean; alt: boolean }> = {}) =>
      shellKey({ key: k, code: "", meta: false, ctrl: false, shift: false, alt: false, ...mods });
    expect(key("Escape")).toBe("escape");
    expect(key("K", { ctrl: true })).toBe("new");
    expect(key("\\", { meta: true })).toBe("sidebar");
    expect(key("/", { meta: true })).toBe("composer");
    expect(key("k")).toBeNull();
    expect(key("k", { meta: true, alt: true })).toBeNull();
    expect(inDialog("modal") && inDialog("settings.section") && !inDialog("space")).toBe(true);
  });
});

describe("what a frame may reach", () => {
  it("is its own names and its hub, nothing like them", () => {
    expect(admits("acme.notes", [], "app.acme.notes.list", false)).toBe(true);
    expect(admits("acme.notes", [], "app.acme.notesx.list", false)).toBe(false);
    expect(admits("acme.notes", ["harnix.sessions"], "harnix.sessions", false)).toBe(true);
    expect(admits("acme.notes", ["harnix.sessions"], "harnix.sessions", true)).toBe(false);
    expect(admits("acme.notes", ["topic:harnix.sessions.tasks"], "harnix.sessions.tasks", true)).toBe(true);
  });

  it("is never a name the frames' allowlist does not have, whatever a hub lists", () => {
    expect(admits("acme.notes", ["harnix.apps.list"], "harnix.apps.list", false)).toBe(false);
    expect(admits("acme.notes", ["topic:harnix.sessions.status"], "harnix.sessions.status", true)).toBe(false);
    expect(FRAME_HUB["harnix.send"]).toBe("session");
    expect(FRAME_HUB["harnix.sessions"]).toBe("space");
  });
});

describe("the auto-height reporter", () => {
  it("reports the content's height when the shell sizes the frame to it", async () => {
    const { host } = await connected({ point: "home.card", autoHeight: true });
    const reported = (await host.next("resize")) as { height: number };
    expect(typeof reported.height).toBe("number");
  });
});
