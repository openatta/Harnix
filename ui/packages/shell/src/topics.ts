/**
 * Topic subscriptions, counted per client. Every host on a page — and the shell itself — shares one
 * connection, and the server keeps one subscription per connection and `(topic, params)`: so the
 * first subscriber sends `harnix.subscribe`, the last one to leave sends `harnix.unsubscribe`, a
 * later subscriber is handed the value already held, and after a reconnect every key still wanted
 * is subscribed again. The server handles each frame in its own task, so a `subscribe` sent while an
 * `unsubscribe` of the same key is still on its way could be applied first and then undone: a key
 * is only subscribed again once its unsubscribe has been answered.
 *
 * A subscription a page in a frame asks for carries its extension (`"$ext"`), as its calls do, and
 * the server narrows it to what that extension's interface lists: it is asked as that extension
 * (`subscribeAs`), and again as that extension after a reconnect. One key held for the page and
 * for frames is still one subscription on the connection.
 */

import type { HarnixClient } from "@harnix/core";

/** `{a, b}` and `{b, a}` are one subscription. */
export function topicKey(topic: string, params: unknown): string {
  return `${topic}\u0000${canonical(params ?? {})}`;
}

function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record)
      .sort()
      .filter((k) => record[k] !== undefined)
      .map((k) => `${JSON.stringify(k)}:${canonical(record[k])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}

interface Entry {
  topic: string;
  params: Record<string, unknown>;
  /** Each listener, and the extension whose frame it is for (null: the page's own). */
  listeners: Map<(value: unknown) => void, string | null>;
  /** The value last seen, once there is one. */
  last?: { value: unknown };
  /** Whether this connection holds the subscription now (false after a disconnect, until renewed). */
  held: boolean;
}

/** One client's topic subscriptions, by key. Every host on a page shares it. */
export class Topics {
  private readonly entries = new Map<string, Entry>();
  /** The `harnix.unsubscribe` still unanswered, by key. */
  private readonly leaving = new Map<string, Promise<unknown>>();

  constructor(private readonly client: HarnixClient) {
    client.onNotify((method, frame) => {
      if (method !== "harnix.topic") return;
      const entry = this.entries.get(topicKey(String(frame.topic), frame.params));
      if (entry) this.deliver(entry, frame.value);
    });
    client.onStatus((status) => {
      if (status === "open") {
        for (const entry of this.entries.values()) if (!entry.held) this.hold(entry);
      } else if (status === "closed" || status === "error" || status === "connecting") {
        for (const entry of this.entries.values()) entry.held = false;
      }
    });
  }

  /**
   * A subscription for a page in a frame of extension `ext`: the server is asked as that extension
   * first, and refuses what its interface does not list — the promise rejects with why. Settles
   * with the unsubscribe.
   */
  async subscribeAs(ext: string, topic: string, params: Record<string, unknown>, fn: (value: unknown) => void): Promise<() => void> {
    await this.client.call("harnix.subscribe", { topic, params, $ext: ext });
    return this.subscribe(topic, params, fn, ext);
  }

  subscribe(topic: string, params: Record<string, unknown>, fn: (value: unknown) => void, ext: string | null = null): () => void {
    const key = topicKey(topic, params);
    let entry = this.entries.get(key);
    if (!entry) {
      entry = { topic, params, listeners: new Map(), held: false };
      this.entries.set(key, entry);
      entry.listeners.set(fn, ext);
      if (this.client.status === "open") this.hold(entry);
    } else {
      entry.listeners.set(fn, ext);
      const held = entry;
      if (held.last) {
        queueMicrotask(() => {
          if (held.listeners.has(fn) && held.last) fn(held.last.value);
        });
      }
    }
    let alive = true;
    return () => {
      if (!alive) return;
      alive = false;
      const current = this.entries.get(key);
      if (!current) return;
      current.listeners.delete(fn);
      if (current.listeners.size) return;
      this.entries.delete(key);
      if (current.held && this.client.status === "open") {
        const gone = this.client
          .call("harnix.unsubscribe", { topic: current.topic, params: current.params })
          .catch(() => undefined)
          .then(() => {
            if (this.leaving.get(key) === gone) this.leaving.delete(key);
          });
        this.leaving.set(key, gone);
      }
    };
  }

  /** How many subscribe to this key (for tests and the diagnostics). */
  count(topic: string, params: unknown): number {
    return this.entries.get(topicKey(topic, params))?.listeners.size ?? 0;
  }

  private hold(entry: Entry): void {
    entry.held = true;
    const key = topicKey(entry.topic, entry.params);
    const leaving = this.leaving.get(key);
    // Still the entry wanted, and still connected, once the unsubscribe before it has been answered.
    const wanted = () => this.entries.get(key) === entry && entry.held && this.client.status === "open";
    // As the page, and as each extension whose frame listens: what the server narrows by.
    const send = (ext: string | null) =>
      wanted() ? this.client.call("harnix.subscribe", { topic: entry.topic, params: entry.params, ...(ext ? { $ext: ext } : {}) }) : undefined;
    const tags = [...new Set(entry.listeners.values())];
    for (const ext of tags) {
      (leaving ? leaving.then(() => send(ext)) : Promise.resolve(send(ext)))
        .then((r) => {
          if (r && this.entries.get(key) === entry) this.deliver(entry, (r as { value: unknown }).value);
        })
        .catch(() => {
          if (ext === null) {
            entry.held = false;
            return;
          }
          // Refused for that extension now: its frames' listeners go; the page's stay.
          for (const [fn, tag] of [...entry.listeners]) if (tag === ext) entry.listeners.delete(fn);
          if (!entry.listeners.size && this.entries.get(key) === entry) this.entries.delete(key);
        });
    }
  }

  private deliver(entry: Entry, value: unknown): void {
    entry.last = { value };
    for (const fn of [...entry.listeners.keys()]) fn(value);
  }
}

const TABLES = new WeakMap<HarnixClient, Topics>();

/** The page's one table for this client. */
export function topicsOf(client: HarnixClient): Topics {
  let table = TABLES.get(client);
  if (!table) {
    table = new Topics(client);
    TABLES.set(client, table);
  }
  return table;
}
