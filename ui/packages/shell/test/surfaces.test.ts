/**
 * The shell's end of a frame's bridge, against a frame played by the test over a real
 * `MessageChannel`: the port is handed once, at the first load; hello says what the frame is;
 * calls and topics outside the application's own names and its hub are refused and recorded; a
 * second load, no hello, and a stopped heartbeat are failures; the shell's keys are acted on as
 * if pressed on the page; a notice is under the application's name. And the page-local record:
 * three failures in a row set a surface aside.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HarnixClient } from "@harnix/core";
import { Registry } from "@harnix/react";
import { PROTOCOL, type Message } from "@harnix/app-sdk/protocol";

import { App } from "../src/app";
import { FrameBridge, HELLO_MS, MISSES, PING_MS, type FrameHost } from "../src/surfaces/bridge";
import { interfacesOf, type AppInterface, type SurfaceDecl } from "../src/surfaces/model";
import { surfaces } from "../src/surfaces/store";

class Client {
  status = "open";
  calls: { method: string; params: Record<string, unknown> }[] = [];
  call(method: string, params: Record<string, unknown> = {}): Promise<unknown> {
    this.calls.push({ method, params });
    return Promise.resolve(method === "harnix.subscribe" ? { value: null } : { ok: method });
  }
  onNotify(): () => void {
    return () => undefined;
  }
  onStatus(): () => void {
    return () => undefined;
  }
  connect(): void {}
}

const IFACE: AppInterface = { id: "acme.notes", title: "Notes", trust: "store", hub: ["harnix.sessions"], links: false, spaces: ["acme.notes"], surfaces: [] };
const decl = (point: SurfaceDecl["point"], id: string = point): SurfaceDecl => ({
  app: "acme.notes", point, id, frame: "/apps/acme.notes/ui/a.html", title: null, space: point.startsWith("space") ? "acme.notes" : null, sessions: "acme.notes", icon: null, order: null, center: null, kinds: [], wide: false,
});

/** An iframe as far as the bridge sees one: load events, and a window to hand a port to. */
function fakeFrame() {
  const listeners = new Set<() => void>();
  const handed: { message: unknown; port: MessagePort }[] = [];
  const iframe = {
    addEventListener: (_: string, fn: () => void) => listeners.add(fn),
    removeEventListener: (_: string, fn: () => void) => listeners.delete(fn),
    contentWindow: { postMessage: (message: unknown, _origin: string, ports: MessagePort[]) => handed.push({ message, port: ports[0]! }) },
  } as unknown as HTMLIFrameElement;
  return { iframe, handed, load: () => [...listeners].forEach((fn) => fn()) };
}

/** The frame's side: requests, answered in order. */
function framePort(port: MessagePort) {
  let id = 1;
  const answers = new Map<number, (m: Message) => void>();
  const received: Message[] = [];
  port.onmessage = (e) => {
    const m = e.data as Message;
    received.push(m);
    if (m.kind === "response") answers.get(m.id)?.(m);
  };
  return {
    received,
    ask: (method: string, params: unknown = {}) =>
      new Promise<Message & { kind: "response" }>((resolve) => {
        const n = id++;
        answers.set(n, resolve as (m: Message) => void);
        port.postMessage({ kind: "request", id: n, method, params });
      }),
    event: (name: string, data: unknown) => port.postMessage({ kind: "event", name, data }),
  };
}

let app: App;
let client: Client;
let outcome: { ready: number; failed: string[]; closed: unknown[] };
let bridges: FrameBridge[] = [];

function bridgeFor(d: SurfaceDecl) {
  const f = fakeFrame();
  const host: FrameHost = {
    app,
    iface: IFACE,
    decl: d,
    space: "acme.notes",
    params: { n: 1 },
    ready: () => (outcome.ready += 1),
    failed: (reason) => outcome.failed.push(reason),
    resize: () => undefined,
    close: (r) => outcome.closed.push(r),
    openModal: () => Promise.resolve(null),
    secondary: () => undefined,
    badge: () => undefined,
  };
  const bridge = new FrameBridge(f.iframe, host);
  bridges.push(bridge);
  return { ...f, bridge };
}

const settle = () => new Promise((r) => setTimeout(r, 5));

beforeEach(() => {
  client = new Client();
  app = new App(client as unknown as HarnixClient, new Registry());
  outcome = { ready: 0, failed: [], closed: [] };
});

afterEach(() => {
  for (const b of bridges) b.dispose();
  bridges = [];
  vi.useRealTimers();
});

describe("a frame's bridge", () => {
  it("hands the port once, at the first load, and says what the frame is at hello", async () => {
    const f = bridgeFor(decl("space"));
    f.load();
    expect(f.handed).toHaveLength(1);
    expect(f.handed[0]!.message).toEqual({ protocol: PROTOCOL, type: "connect" });
    const frame = framePort(f.handed[0]!.port);
    const hello = await frame.ask("hello", { sdk: "test" });
    expect(hello.result).toMatchObject({ surface: { app: "acme.notes", point: "space", params: { n: 1 }, autoHeight: false, links: false }, env: { modal: false } });
    expect(outcome.ready).toBe(1);
    // A second load is the page going away.
    f.load();
    expect(f.handed).toHaveLength(1);
    expect(outcome.failed).toHaveLength(1);
  });

  it("calls the application's own methods and its hub's, and refuses the rest on the record", async () => {
    const f = bridgeFor(decl("space"));
    f.load();
    const frame = framePort(f.handed[0]!.port);
    await frame.ask("hello");
    expect((await frame.ask("call", { method: "app.acme.notes.list" })).result).toEqual({ ok: "app.acme.notes.list" });
    expect((await frame.ask("call", { method: "harnix.sessions", params: { space: "acme.notes" } })).result).toEqual({ ok: "harnix.sessions" });
    const refused = await frame.ask("call", { method: "harnix.apps.uninstall", params: { id: "acme.notes" } });
    expect(refused.error).toMatchObject({ code: "refused" });
    expect(client.calls.map((c) => c.method)).not.toContain("harnix.apps.uninstall");
    const topic = await frame.ask("subscribe", { topic: "harnix.sessions.status" });
    expect(topic.error?.code).toBe("refused");
    expect(surfaces.summary("acme.notes").refused.map((r) => r.name)).toEqual(expect.arrayContaining(["harnix.apps.uninstall", "harnix.sessions.status"]));
    expect(app.registry.refusals().filter((r) => r.point === "bridge").map((r) => r.id)).toEqual(["harnix.apps.uninstall", "harnix.sessions.status"]);
    // What is not a request at all.
    expect((await frame.ask("frobnicate")).error?.code).toBe("unknown");
  });

  it("refuses what acts on the front space while the application's is not in front", async () => {
    const f = bridgeFor(decl("home.card", "card"));
    f.load();
    const frame = framePort(f.handed[0]!.port);
    await frame.ask("hello");
    expect((await frame.ask("composer.insert", { text: "hi" })).error?.code).toBe("refused");
    expect((await frame.ask("openModal", { id: "nope" })).error?.code).toBe("refused");
    expect((await frame.ask("openLink", { url: "https://example.com/" })).error?.code).toBe("refused");
  });

  it("puts a notice under the application's name", async () => {
    const f = bridgeFor(decl("space"));
    f.load();
    const frame = framePort(f.handed[0]!.port);
    await frame.ask("hello");
    await frame.ask("notify", { text: "saved", level: "error" });
    expect(app.snapshot.banner).toEqual({ text: expect.stringContaining("Notes"), tone: "error" });
    expect(app.snapshot.banner?.text).toContain("saved");
  });

  it("fails a page that does not say hello in time", () => {
    vi.useFakeTimers();
    const f = bridgeFor(decl("space"));
    f.load();
    vi.advanceTimersByTime(HELLO_MS + 1);
    expect(outcome.failed).toHaveLength(1);
  });

  it("fails a page that stops answering the heartbeat", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
    const f = bridgeFor(decl("space"));
    f.load();
    const frame = framePort(f.handed[0]!.port);
    await frame.ask("hello");
    // The frame answers nothing from here on.
    f.handed[0]!.port.onmessage = null;
    vi.advanceTimersByTime(PING_MS * (MISSES + 1));
    expect(outcome.failed).toHaveLength(1);
  });

  it("acts on the shell's keys as if pressed on the page: Escape only from inside a dialog", async () => {
    const seen: string[] = [];
    const listen = (e: KeyboardEvent) => seen.push(`${e.metaKey ? "⌘" : ""}${e.key}`);
    window.addEventListener("keydown", listen);
    try {
      const space = bridgeFor(decl("space"));
      space.load();
      const a = framePort(space.handed[0]!.port);
      await a.ask("hello");
      a.event("key", { key: "Escape", code: "Escape", meta: false, ctrl: false, shift: false, alt: false });
      a.event("key", { key: "k", code: "KeyK", meta: true, ctrl: false, shift: false, alt: false });
      a.event("key", { key: "a", code: "KeyA", meta: false, ctrl: false, shift: false, alt: false });
      const modal = bridgeFor(decl("modal", "edit"));
      modal.load();
      const b = framePort(modal.handed[0]!.port);
      await b.ask("hello");
      b.event("key", { key: "Escape", code: "Escape", meta: false, ctrl: false, shift: false, alt: false });
      b.event("close", { result: { saved: true } });
      await settle();
      expect(seen).toEqual(["⌘k", "Escape"]);
      expect(outcome.closed).toEqual([{ saved: true }]);
    } finally {
      window.removeEventListener("keydown", listen);
    }
  });
});

describe("the page's record of its frames", () => {
  it("sets a surface aside after three failures in a row, until it is asked for again", () => {
    const status = { key: "acme.notes/space//t", app: "acme.notes", point: "space", id: "t" };
    expect(surfaces.fail({ ...status, reason: "1" }, 1000)).toBe("failed");
    surfaces.reload(status.key);
    expect(surfaces.fail({ ...status, reason: "2" }, 2000)).toBe("failed");
    expect(surfaces.fail({ ...status, reason: "3" }, 3000)).toBe("aside");
    expect(surfaces.isAside(status.key)).toBe(true);
    surfaces.retry(status.key);
    expect(surfaces.get(status.key)?.state).toBe("loading");
    expect(surfaces.fail({ ...status, reason: "4" }, 4000)).toBe("failed");
  });
});

describe("what hello says", () => {
  it("is read into each application's surfaces; a frame outside its own files is dropped", () => {
    const { apps, refused } = interfacesOf({
      contributions: [
        { plugin: "acme.notes", point: "home.card", api_version: 2, refused: "card.js", reason: "in the page" },
        { plugin: "acme.notes", point: "space", api_version: 2, frame: "/apps/acme.notes/ui/a.html", surface: { id: "space", space: "acme.notes", title: { en: "Notes" } }, interface: { hub: ["harnix.sessions"], links: true, trust: "store", title: "Notes", spaces: ["acme.notes"] } },
        { plugin: "acme.notes", point: "home.card", api_version: 2, frame: "/apps/other.app/ui/x.html", surface: { id: "card" }, interface: {} },
        { plugin: "pkg", point: "space", api_version: 2, module: "/plugins/pkg/ui/x.js" },
      ],
    });
    expect(refused).toEqual([{ app: "acme.notes", point: "home.card", module: "card.js", reason: "in the page" }]);
    expect(apps).toHaveLength(1);
    expect(apps[0]).toMatchObject({ id: "acme.notes", hub: ["harnix.sessions"], links: true });
    expect(apps[0]!.surfaces.map((s) => [s.point, s.frame])).toEqual([
      ["space", "/apps/acme.notes/ui/a.html"],
      ["home.card", null],
    ]);
  });
});
