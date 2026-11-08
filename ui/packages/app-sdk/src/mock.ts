/**
 * A host to test an application's page against, without Harnix: the shell's side of the bridge,
 * in memory, over a real `MessageChannel`.
 *
 * ```ts
 * import { connect } from "@harnix/app-sdk";
 * import { createMockHost } from "@harnix/app-sdk/mock";
 *
 * const host = createMockHost({
 *   app: "acme.notes",
 *   methods: { "app.acme.notes.list": () => ({ notes: [] }) },
 * });
 * const app = await connect({ port: host.port });
 * expect(await app.call("app.acme.notes.list")).toEqual({ notes: [] });
 * host.publish("app.acme.notes.changed", { notes: [{ text: "hi" }] });   // to its subscribers
 * host.setEnv({ theme: "dark" });                                        // as the shell pushes it
 * expect(host.requests.map((r) => r.method)).toContain("call");
 * ```
 *
 * It refuses what the shell would — a method or topic outside the application's own `app.<id>.*`
 * and the `hub` it is given — so a page's tests see the same refusals the shell gives it. Each
 * request is recorded (`requests`), and so is each event the page sent (`events`).
 */

import {
  admits,
  type Environment,
  type FrameRequests,
  type Message,
  type SessionRef,
  type SurfaceInfo,
  type SurfacePoint,
} from "./protocol.ts";

type Handler = (params: unknown) => unknown | Promise<unknown>;

export interface MockHostOptions {
  /** The application's id (`acme.notes` by default). */
  app?: string;
  point?: SurfacePoint;
  id?: string;
  space?: string | null;
  params?: unknown;
  autoHeight?: boolean;
  /** `[interface] links`. */
  links?: boolean;
  /** `[interface] hub`: what the page may reach beyond its own names. */
  hub?: string[];
  env?: Partial<Environment>;
  current?: SessionRef | null;
  /** The methods the page may call, by name: what each answers (a throw is a failure). */
  methods?: Record<string, Handler>;
  /** Any other request of the bridge (`confirm`, `openModal`, `notify`, …): what it answers. */
  requests?: Partial<Record<keyof FrameRequests, Handler>>;
}

const DEFAULT_ENV: Environment = {
  theme: "light",
  tokens: {},
  locale: "en",
  narrow: false,
  visible: true,
  modal: false,
};

export interface MockHost {
  /** The page's end: `connect({ port: host.port })`. */
  readonly port: MessagePort;
  /** Every request the page made, in order. */
  readonly requests: { method: string; params: unknown }[];
  /** Every event the page sent: `resize`, `key`, `close`, `error`. */
  readonly events: { name: string; data: unknown }[];
  /** What was refused, and why. */
  readonly refused: { name: string; reason: string }[];
  readonly env: Environment;
  /** A value on a topic, to every subscription of it. */
  publish(topic: string, value: unknown): void;
  /** Change the environment, as the shell does on a theme switch, a resize, a dialog. */
  setEnv(change: Partial<Environment>): void;
  setCurrent(current: SessionRef | null): void;
  /** Push new params, as the shell does when a modal or panel is given another item. */
  setParams(params: unknown): void;
  /** The heartbeat: resolves "pong" when the page is alive. */
  ping(): Promise<unknown>;
  /** Until an event of this name arrives (or one that already did). */
  next(name: string): Promise<unknown>;
  close(): void;
}

export function createMockHost(options: MockHostOptions = {}): MockHost {
  const app = options.app ?? "acme.notes";
  const hub = options.hub ?? [];
  const channel = new MessageChannel();
  const shell = channel.port1;
  const env: Environment = { ...DEFAULT_ENV, ...options.env };
  let current = options.current ?? null;
  const surface: SurfaceInfo = {
    app,
    point: options.point ?? "space",
    id: options.id ?? options.point ?? "space",
    space: options.space === undefined ? app : options.space,
    params: options.params ?? null,
    autoHeight: options.autoHeight ?? false,
    links: options.links ?? false,
  };
  const subs = new Map<number, string>();
  let nextSub = 1;
  let nextPing = 1;
  const pings = new Map<number, (v: unknown) => void>();
  const requests: MockHost["requests"] = [];
  const events: MockHost["events"] = [];
  const refused: MockHost["refused"] = [];
  const awaiting: { name: string; resolve: (v: unknown) => void }[] = [];

  const send = (message: Message) => shell.postMessage(message);
  const refuse = (name: string, reason: string) => {
    refused.push({ name, reason });
    return { code: "refused" as const, message: reason };
  };

  const answer = async (method: string, params: unknown): Promise<{ result?: unknown; error?: { code: "refused" | "failed" | "unknown"; message: string } }> => {
    const p = (params ?? {}) as Record<string, unknown>;
    switch (method) {
      case "hello":
        return { result: { surface, env, current } };
      case "call": {
        const name = String(p.method);
        if (!admits(app, hub, name, false)) return { error: refuse(name, `\`${name}\` is not ${app}'s to call`) };
        const handler = options.methods?.[name];
        if (!handler) return { error: { code: "failed", message: `no method \`${name}\` in this mock` } };
        try {
          return { result: await handler(p.params) };
        } catch (e) {
          return { error: { code: "failed", message: e instanceof Error ? e.message : String(e) } };
        }
      }
      case "subscribe": {
        const topic = String(p.topic);
        if (!admits(app, hub, topic, true)) return { error: refuse(topic, `\`${topic}\` is not ${app}'s to subscribe to`) };
        const sub = nextSub++;
        subs.set(sub, topic);
        return { result: { sub } };
      }
      case "unsubscribe":
        subs.delete(Number(p.sub));
        return { result: null };
      case "session.current":
        return { result: current };
      default: {
        const handler = options.requests?.[method as keyof FrameRequests];
        if (handler) {
          try {
            return { result: (await handler(params)) ?? null };
          } catch (e) {
            return { error: { code: "failed", message: e instanceof Error ? e.message : String(e) } };
          }
        }
        // What the shell does without saying anything back: a notification, a badge, a switch.
        if (["notify", "badge", "copy", "spaces.switch", "details.open", "details.show", "secondary.open", "secondary.close", "settings.open", "composer.insert", "session.open"].includes(method)) {
          return { result: null };
        }
        if (method === "confirm" || method === "openLink") return { result: false };
        if (method === "openModal") return { result: null };
        return { error: { code: "unknown", message: `no request \`${method}\`` } };
      }
    }
  };

  shell.onmessage = (event: MessageEvent) => {
    const message = event.data as Message;
    if (message.kind === "request") {
      requests.push({ method: message.method, params: message.params });
      void answer(message.method, message.params).then((r) => send({ kind: "response", id: message.id, ...r }));
    } else if (message.kind === "event") {
      events.push({ name: message.name, data: message.data });
      for (const w of awaiting.filter((a) => a.name === message.name)) {
        awaiting.splice(awaiting.indexOf(w), 1);
        w.resolve(message.data);
      }
    } else if (message.kind === "response") {
      pings.get(message.id)?.(message.result ?? message.error);
      pings.delete(message.id);
    }
  };

  return {
    port: channel.port2,
    requests,
    events,
    refused,
    get env() {
      return env;
    },
    publish(topic, value) {
      for (const [sub, name] of subs) if (name === topic) send({ kind: "event", name: "topic", data: { sub, value } });
    },
    setEnv(change) {
      Object.assign(env, change);
      send({ kind: "event", name: "env", data: { ...env } });
    },
    setCurrent(next) {
      current = next;
      send({ kind: "event", name: "session", data: { current } });
    },
    setParams(params) {
      surface.params = params;
      send({ kind: "event", name: "params", data: { params } });
    },
    ping() {
      const id = nextPing++;
      return new Promise((resolve) => {
        pings.set(id, resolve);
        send({ kind: "request", id, method: "ping", params: {} });
      });
    },
    next(name) {
      const seen = events.find((e) => e.name === name);
      if (seen) return Promise.resolve(seen.data);
      return new Promise((resolve) => awaiting.push({ name, resolve }));
    },
    close() {
      shell.close();
    },
  };
}
