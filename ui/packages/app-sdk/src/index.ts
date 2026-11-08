/**
 * `@harnix/app-sdk`: an application's page in the shell's isolated frame, talking to the shell.
 *
 * ```js
 * const app = await HarnixApp.connect();          // or: import { connect } from "@harnix/app-sdk"
 * const notes = await app.call("app.acme.notes.list");
 * const off = app.subscribe("app.acme.notes.changed", {}, (value) => draw(value));
 * if (await app.confirm("Delete every note?", { danger: true })) await app.call("app.acme.notes.clear");
 * app.onEnv((env) => document.documentElement.dataset.theme = env.theme);
 * ```
 *
 * No framework: plain promises and callbacks, so a page in React, in anything else or in nothing
 * uses it the same way. What it does by itself once connected:
 *
 * - answers the shell's heartbeat (a page that stops answering is shown as failed);
 * - reports its height when the shell sizes the frame to its content (`home.card`, a settings
 *   section, a modal) — `autoHeight`;
 * - hands the shell the keys that are the shell's: its shortcuts (⌘K, ⌘\, ⌘/) always, and Escape
 *   when the frame is inside one of the shell's dialogs, so a key lands in one place only. While
 *   a dialog of the shell is over the frame, no key reaches the page at all;
 * - sends a link to another site to the shell (`openLink`), the only way a page outside opens;
 * - reports errors the page did not catch, for the shell's diagnostics.
 *
 * `./mock` is a host to test a page against without Harnix (`createMockHost`).
 */

import {
  PROTOCOL,
  inDialog,
  shellKey,
  type DetailItem,
  type Environment,
  type ErrorCode,
  type ErrorData,
  type FrameEvents,
  type FrameRequests,
  type KeyPress,
  type Message,
  type SessionRef,
  type SurfaceInfo,
  type Welcome,
} from "./protocol.ts";

export * from "./protocol.ts";

/** This SDK's version, told to the shell at `hello`. */
export const SDK_VERSION = "0.1.0";

/** A request the shell answered with an error. */
export class BridgeError extends Error {
  readonly code: ErrorCode;
  readonly data: unknown;

  constructor(error: ErrorData) {
    super(error.message);
    this.name = "BridgeError";
    this.code = error.code;
    this.data = error.data;
  }
}

/** A port, as a `MessagePort` is one (a test may hand any object that behaves the same). */
export interface Port {
  postMessage(message: unknown, transfer?: Transferable[]): void;
  addEventListener(type: "message", listener: (event: MessageEvent) => void): void;
  removeEventListener(type: "message", listener: (event: MessageEvent) => void): void;
  start?(): void;
  close?(): void;
}

export interface ConnectOptions {
  /** The shell's port. Given by a test (a mock host's); else the one the shell hands this window. */
  port?: Port;
  /** How long to wait for the shell, in ms. 10 000 by default. */
  timeoutMs?: number;
  /** Report the content's height (default: when the shell sizes this frame to it). */
  autoHeight?: boolean;
  /** Hand the shell its keys (default true). */
  keys?: boolean;
}

/** A subscription: call it to end it. `ready` settles when the shell has taken it (or refused it). */
export type Subscription = (() => void) & { ready: Promise<void> };

/* ── the port the shell hands this window ─────────────────────────────── */

let handed: MessagePort | null = null;
const waiting: ((port: MessagePort) => void)[] = [];

/**
 * Listened for as soon as this module runs, because the shell hands the port when the page has
 * loaded — maybe before the page gets round to `connect()`. Only the parent's message counts:
 * another frame cannot hand this one a port of its own.
 */
if (typeof window !== "undefined" && window.parent !== window) {
  const take = (event: MessageEvent) => {
    const data = event.data as { protocol?: unknown; type?: unknown } | null;
    if (event.source !== window.parent || data?.protocol !== PROTOCOL || data.type !== "connect" || !event.ports[0]) return;
    window.removeEventListener("message", take);
    handed = event.ports[0];
    for (const fn of waiting.splice(0)) fn(handed);
  };
  window.addEventListener("message", take);
}

function shellPort(timeoutMs: number): Promise<Port> {
  if (handed) return Promise.resolve(handed);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      const at = waiting.indexOf(done);
      if (at >= 0) waiting.splice(at, 1);
      reject(new BridgeError({ code: "timeout", message: "no Harnix shell handed this page a bridge: is it open outside Harnix?" }));
    }, timeoutMs);
    const done = (port: MessagePort) => {
      clearTimeout(timer);
      resolve(port);
    };
    waiting.push(done);
  });
}

/** Connect to the shell: resolves once it has said what this frame is. */
export async function connect(options: ConnectOptions = {}): Promise<HarnixApp> {
  const timeoutMs = options.timeoutMs ?? 10_000;
  const port = options.port ?? (await shellPort(timeoutMs));
  const app = new HarnixApp(port);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const welcome = await Promise.race([
    app.request("hello", { sdk: SDK_VERSION }),
    new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new BridgeError({ code: "timeout", message: "the shell did not answer hello" })), timeoutMs);
    }),
  ]).finally(() => clearTimeout(timer));
  app.welcome(welcome, options);
  return app;
}

/* ── the application's side of the bridge ─────────────────────────────── */

type Listener<T> = (value: T) => void;

/** An application page's handle on the shell. Made by `connect()`. */
export class HarnixApp {
  private next = 1;
  private readonly pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
  private readonly topics = new Map<number, Listener<unknown>>();
  private readonly envListeners = new Set<(env: Environment, changed: (keyof Environment)[]) => void>();
  private readonly sessionListeners = new Set<Listener<SessionRef | null>>();
  private readonly paramListeners = new Set<Listener<unknown>>();
  private info: SurfaceInfo | null = null;
  private environment: Environment | null = null;
  private currentSession: SessionRef | null = null;
  private readonly cleanups: (() => void)[] = [];
  private closed = false;

  /** @internal Use `connect()`. */
  constructor(private readonly port: Port) {
    port.addEventListener("message", this.receive);
    port.start?.();
  }

  /** What this frame is. */
  get surface(): SurfaceInfo {
    return this.info!;
  }

  /** The shell's look and state now. */
  get env(): Environment {
    return this.environment!;
  }

  /** Follow the environment: theme, tokens, locale, narrow, visible, modal. Called on every change. */
  onEnv(fn: (env: Environment, changed: (keyof Environment)[]) => void): () => void {
    this.envListeners.add(fn);
    return () => this.envListeners.delete(fn);
  }

  /** New params for this frame (a modal's, a secondary panel's, a details panel's item). */
  onParams(fn: (params: unknown) => void): () => void {
    this.paramListeners.add(fn);
    return () => this.paramListeners.delete(fn);
  }

  /* ── data ── */

  /** A method: the application's own `app.<id>.*`, or one its `[interface] hub` lists. */
  call<T = unknown>(method: string, params?: unknown): Promise<T> {
    return this.request("call", { method, params }) as Promise<T>;
  }

  /** A topic's values, as they are published; a refusal rejects `ready` and calls nothing. */
  subscribe(topic: string, params: unknown, fn: (value: unknown) => void): Subscription {
    let sub: number | null = null;
    let ended = false;
    const ready = this.request("subscribe", { topic, params }).then((r) => {
      sub = r.sub;
      if (ended) void this.request("unsubscribe", { sub }).catch(() => undefined);
      else this.topics.set(sub, fn);
    });
    const off = (() => {
      ended = true;
      if (sub === null) return;
      this.topics.delete(sub);
      void this.request("unsubscribe", { sub }).catch(() => undefined);
    }) as Subscription;
    off.ready = ready;
    return off;
  }

  /** The application's space's sessions, as the shell has them open. */
  readonly session = {
    current: (): SessionRef | null => this.currentSession,
    open: (ref: SessionRef): Promise<void> => this.request("session.open", ref).then(() => undefined),
    create: (spec: { scene?: string; options?: Record<string, unknown> } = {}): Promise<SessionRef> => this.request("session.create", spec),
    onChange: (fn: (current: SessionRef | null) => void): (() => void) => {
      this.sessionListeners.add(fn);
      return () => this.sessionListeners.delete(fn);
    },
  };

  /* ── the shell's hands ── */

  readonly spaces = {
    switch: (space: string): Promise<void> => this.request("spaces.switch", { space }).then(() => undefined),
  };

  readonly details = {
    open: (tab?: string): Promise<void> => this.request("details.open", { tab }).then(() => undefined),
    show: (item: DetailItem): Promise<void> => this.request("details.show", { item }).then(() => undefined),
  };

  readonly secondary = {
    open: (params?: unknown, title?: string): Promise<void> => this.request("secondary.open", { params, title }).then(() => undefined),
    close: (): Promise<void> => this.request("secondary.close", {}).then(() => undefined),
  };

  readonly settings = {
    open: (section?: string): Promise<void> => this.request("settings.open", { section }).then(() => undefined),
  };

  readonly composer = {
    insert: (text: string): Promise<void> => this.request("composer.insert", { text }).then(() => undefined),
  };

  /** A badge on the application's space entry; null, 0 or "" clears it. */
  badge(value: string | number | null): Promise<void> {
    return this.request("badge", { value }).then(() => undefined);
  }

  notify(text: string, level: "info" | "error" = "info"): Promise<void> {
    return this.request("notify", { text, level }).then(() => undefined);
  }

  /** Text onto the clipboard, written by the shell: a frame has no clipboard of its own. */
  copy(text: string): Promise<void> {
    return this.request("copy", { text }).then(() => undefined);
  }

  /** One of the application's modals; resolves with what it closed with, null if it was dismissed. */
  openModal<T = unknown>(id: string, params?: unknown): Promise<T | null> {
    return this.request("openModal", { id, params }) as Promise<T | null>;
  }

  /** In a modal: close it, answering its opener with `result`. */
  close(result?: unknown): void {
    this.event("close", { result });
  }

  /** The shell's own dialog, which no page can draw: true when the person confirmed. */
  confirm(text: string, options: { title?: string; confirm?: string; danger?: boolean } = {}): Promise<boolean> {
    return this.request("confirm", { text, ...options });
  }

  /** A web page, outside Harnix, after the person agrees (`[interface] links`); true when opened. */
  openLink(url: string): Promise<boolean> {
    return this.request("openLink", { url });
  }

  /** A file uploaded by the shell (a frame has no network): its path, for a message's attachments. */
  async upload(file: Blob & { name?: string }): Promise<{ path: string }> {
    const data = await file.arrayBuffer();
    return this.request("upload", { name: file.name ?? "upload", type: file.type, data }, [data]);
  }

  /** Stop talking to the shell (the page is going away). */
  disconnect(): void {
    if (this.closed) return;
    this.closed = true;
    for (const fn of this.cleanups.splice(0)) fn();
    this.port.removeEventListener("message", this.receive);
    this.port.close?.();
    for (const [, p] of this.pending) p.reject(new BridgeError({ code: "closed", message: "the bridge was closed" }));
    this.pending.clear();
  }

  /* ── the wire ── */

  /** @internal */
  request<M extends keyof FrameRequests>(method: M, params: FrameRequests[M]["params"], transfer: Transferable[] = []): Promise<FrameRequests[M]["result"]> {
    if (this.closed) return Promise.reject(new BridgeError({ code: "closed", message: "the bridge was closed" }));
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.port.postMessage({ kind: "request", id, method, params } satisfies Message, transfer);
    });
  }

  private event<N extends keyof FrameEvents>(name: N, data: FrameEvents[N]): void {
    if (!this.closed) this.port.postMessage({ kind: "event", name, data } satisfies Message);
  }

  private readonly receive = (event: MessageEvent): void => {
    const message = event.data as Message | null;
    if (!message || typeof message !== "object") return;
    if (message.kind === "response") {
      const waiting = this.pending.get(message.id);
      if (!waiting) return;
      this.pending.delete(message.id);
      if (message.error) waiting.reject(new BridgeError(message.error));
      else waiting.resolve(message.result);
    } else if (message.kind === "request") {
      // The shell asks one thing: is this page alive.
      const known = message.method === "ping";
      this.port.postMessage({
        kind: "response",
        id: message.id,
        ...(known ? { result: "pong" } : { error: { code: "unknown", message: `no request \`${message.method}\` here` } }),
      } satisfies Message);
    } else if (message.kind === "event") {
      this.onEvent(message.name, message.data);
    }
  };

  private onEvent(name: string, data: unknown): void {
    if (name === "topic") {
      const { sub, value } = data as { sub: number; value: unknown };
      this.topics.get(sub)?.(value);
    } else if (name === "topic.end") {
      this.topics.delete((data as { sub: number }).sub);
    } else if (name === "env") {
      const next = data as Environment;
      const before = this.environment;
      this.environment = next;
      const changed = (Object.keys(next) as (keyof Environment)[]).filter((k) => JSON.stringify(before?.[k]) !== JSON.stringify(next[k]));
      if (changed.length) for (const fn of [...this.envListeners]) fn(next, changed);
    } else if (name === "session") {
      this.currentSession = (data as { current: SessionRef | null }).current;
      for (const fn of [...this.sessionListeners]) fn(this.currentSession);
    } else if (name === "params") {
      const params = (data as { params: unknown }).params;
      if (this.info) this.info = { ...this.info, params };
      for (const fn of [...this.paramListeners]) fn(params);
    }
  }

  /** @internal What `hello` answered: start following the shell. */
  welcome(welcome: Welcome, options: ConnectOptions): void {
    this.info = welcome.surface;
    this.environment = welcome.env;
    this.currentSession = welcome.current;
    if (typeof document === "undefined") return;
    if (options.keys !== false) this.cleanups.push(this.followKeys());
    if (options.autoHeight ?? welcome.surface.autoHeight) this.cleanups.push(this.reportHeight());
    this.cleanups.push(this.followLinks(), this.reportErrors());
  }

  /** Keys: the shell's go to it, and while one of its dialogs is over this frame, none go anywhere. */
  private followKeys(): () => void {
    const onKey = (event: KeyboardEvent) => {
      if (this.environment?.modal) {
        event.preventDefault();
        event.stopImmediatePropagation();
        return;
      }
      const press: KeyPress = { key: event.key, code: event.code, meta: event.metaKey, ctrl: event.ctrlKey, shift: event.shiftKey, alt: event.altKey };
      const which = shellKey(press);
      // Escape is the page's own, unless the frame is inside a dialog of the shell: then it closes that.
      if (!which || (which === "escape" && !inDialog(this.surface.point))) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      this.event("key", press);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }

  /** The content's height, whenever it changes, at most once a frame. */
  private reportHeight(): () => void {
    let last = -1;
    let frame = 0;
    const measure = () => {
      frame = 0;
      const body = document.body;
      if (!body) return;
      const margin = parseFloat(getComputedStyle(body).marginBottom) || 0;
      const height = Math.ceil(body.getBoundingClientRect().bottom + margin + (window.scrollY || 0));
      if (height !== last) {
        last = height;
        this.event("resize", { height });
      }
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(measure);
    };
    const observer = typeof ResizeObserver === "function" ? new ResizeObserver(schedule) : null;
    if (document.body) observer?.observe(document.body);
    window.addEventListener("load", schedule);
    schedule();
    return () => {
      observer?.disconnect();
      window.removeEventListener("load", schedule);
      if (frame) cancelAnimationFrame(frame);
    };
  }

  /**
   * A link to another site goes through the shell; this frame does not navigate itself (to the
   * shell, a frame whose page went away has failed). A link within the page — `#…` — is the
   * page's own.
   */
  private followLinks(): () => void {
    const onClick = (event: MouseEvent) => {
      const anchor = (event.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
      if (!anchor || event.defaultPrevented) return;
      // The page's own origin is opaque: compare against where it was loaded from.
      const here = new URL(location.href);
      const url = new URL(anchor.getAttribute("href") ?? "", here);
      if (url.hash && url.href.split("#")[0] === here.href.split("#")[0]) return;
      event.preventDefault();
      const web = url.protocol === "https:" || url.protocol === "http:";
      if (web && url.origin !== here.origin && this.surface.links) void this.openLink(url.href).catch(() => undefined);
    };
    document.addEventListener("click", onClick);
    return () => document.removeEventListener("click", onClick);
  }

  /** What the page did not catch, told to the shell — a few a second at most. */
  private reportErrors(): () => void {
    let budget = 5;
    const refill = setInterval(() => (budget = 5), 1000);
    const report = (message: string) => {
      if (budget-- > 0) this.event("error", { message: message.slice(0, 500) });
    };
    const onError = (event: ErrorEvent) => report(event.message || String(event.error));
    const onRejection = (event: PromiseRejectionEvent) => report(String((event.reason as Error)?.message ?? event.reason));
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);
    return () => {
      clearInterval(refill);
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
    };
  }
}
