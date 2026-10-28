/**
 * Topic subscriptions, counted per client. Every host on a page — and the shell itself — shares one
 * connection, and the server keeps one subscription per connection and `(topic, params)`: so the
 * first subscriber sends `harnix.subscribe`, the last one to leave sends `harnix.unsubscribe`, a
 * later subscriber is handed the value already held, and after a reconnect every key still wanted
 * is subscribed again.
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
  listeners: Set<(value: unknown) => void>;
  /** The value last seen, once there is one. */
  last?: { value: unknown };
  /** Whether this connection holds the subscription now (false after a disconnect, until renewed). */
  held: boolean;
}

/** One client's topic subscriptions, by key. Every host on a page shares it. */
export class Topics {
  private readonly entries = new Map<string, Entry>();

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

  subscribe(topic: string, params: Record<string, unknown>, fn: (value: unknown) => void): () => void {
    const key = topicKey(topic, params);
    let entry = this.entries.get(key);
    if (!entry) {
      entry = { topic, params, listeners: new Set(), held: false };
      this.entries.set(key, entry);
      entry.listeners.add(fn);
      if (this.client.status === "open") this.hold(entry);
    } else {
      entry.listeners.add(fn);
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
        this.client.call("harnix.unsubscribe", { topic: current.topic, params: current.params }).catch(() => undefined);
      }
    };
  }

  /** How many subscribe to this key (for tests and the diagnostics). */
  count(topic: string, params: unknown): number {
    return this.entries.get(topicKey(topic, params))?.listeners.size ?? 0;
  }

  private hold(entry: Entry): void {
    entry.held = true;
    this.client
      .call("harnix.subscribe", { topic: entry.topic, params: entry.params })
      .then((r) => {
        if (this.entries.get(topicKey(entry.topic, entry.params)) === entry) this.deliver(entry, (r as { value: unknown }).value);
      })
      .catch(() => {
        entry.held = false;
      });
  }

  private deliver(entry: Entry, value: unknown): void {
    entry.last = { value };
    for (const fn of [...entry.listeners]) fn(value);
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
