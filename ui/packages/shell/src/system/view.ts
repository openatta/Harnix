/** The system space's view: what is selected, and the hub topics that keep the page current. */

import { useEffect, useState, useSyncExternalStore } from "react";
import { errorText } from "@harnix/core";

import type { App, SessionKey } from "../app.ts";
import { topicKey, topicsOf, type Topics } from "../topics.ts";
import type { ConsoleTab, SessionRowView } from "./types.ts";

/** Where this browser keeps the sessions that finished unseen, and how many at most. */
const UNREAD_KEY = "harnix.system.unread";
const UNREAD_KEPT = 200;

/** What the right column is about. `space`: one space's sessions, across instances, the one a
 *  person came from (`current`) first. */
export type Selection =
  | { kind: "space"; space: string; current?: SessionKey }
  | { kind: "instance"; instance: string }
  /** `parent`: a sub-agent's session, under that main Agent's (its calls are in the parent's recording). */
  | { kind: "session"; instance: string; sessionId: string; parent?: string }
  | { kind: "task"; instance: string; sessionId: string; taskId: string };

/**
 * The selection, and the hub topics this space watches. A topic is subscribed once however many
 * parts of the page read it, and given up when the last one stops.
 */
export class View {
  private picked: Selection | null = null;
  /** The console's tab, kept while the page is open: back from a flow is back on the Flows tab. */
  consoleTab: ConsoleTab = "active";
  /** A tool call to pick out in the calls tab, once: what a jump from the conversation asked for. */
  private tool: string | null = null;
  private readonly listeners = new Set<() => void>();
  /** The page's one table of subscriptions, shared with the shell: one per connection and key. */
  private readonly table: Topics;
  /** The last value of each topic watched here, while something here watches it. */
  private readonly values = new Map<string, { value: unknown; readers: number }>();
  /** Sessions whose turn ended while they were neither open nor picked here (`instance/session`):
   *  DSH's "completed", until someone looks. */
  private readonly unread = new Set<string>();
  /** The sessions each instance's list last showed running. */
  private readonly wasRunning = new Map<string, Set<string>>();

  constructor(private readonly app: App) {
    this.table = topicsOf(app.client);
    try {
      for (const key of JSON.parse(localStorage.getItem(UNREAD_KEY) ?? "[]") as string[]) this.unread.add(key);
    } catch {
      // No storage, or nothing kept: nothing is unread yet.
    }
  }

  private keepUnread(): void {
    try {
      localStorage.setItem(UNREAD_KEY, JSON.stringify([...this.unread].slice(-UNREAD_KEPT)));
    } catch {
      // Without storage the marks last as long as the page.
    }
  }

  isUnread(key: string): boolean {
    return this.unread.has(key);
  }

  /** A session list arrived: a turn that was running there and is not now, and that nobody here is
   *  looking at, finished unseen. */
  private noteRunning(instance: string, value: unknown): void {
    const rows = (value as { sessions?: SessionRowView[] } | undefined)?.sessions ?? [];
    const now = new Set(rows.filter((r) => r.running).map((r) => r.session_id));
    const before = this.wasRunning.get(instance);
    this.wasRunning.set(instance, now);
    if (!before) return;
    const open = this.app.snapshot.current;
    const picked = this.picked && this.picked.kind === "session" ? `${this.picked.instance}/${this.picked.sessionId}` : null;
    for (const id of before) {
      const key = `${instance}/${id}`;
      if (now.has(id) || key === picked || (open && open.instance === instance && open.sessionId === id)) continue;
      this.unread.add(key);
      this.keepUnread();
    }
  }

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly selected = (): Selection | null => this.picked;

  focusTool(id: string): void {
    this.tool = id;
  }

  /** The tool call to pick out, given up once read. */
  takeTool(): string | null {
    const id = this.tool;
    this.tool = null;
    return id;
  }

  select(selection: Selection): void {
    this.picked = selection;
    if ((selection.kind === "session" || selection.kind === "task") && this.unread.delete(`${selection.instance}/${selection.sessionId}`)) this.keepUnread();
    for (const fn of [...this.listeners]) fn();
    // The right column is where a selection is shown; redraw it with the new tabs.
    this.app.set({ detail: null });
    this.app.toggleDetails(true);
  }

  watch(topic: string, params: Record<string, unknown> | undefined, fn: () => void): () => void {
    const key = topicKey(topic, params);
    const entry = this.values.get(key) ?? { value: undefined, readers: 0 };
    this.values.set(key, entry);
    entry.readers += 1;
    const off = this.table.subscribe(topic, params ?? {}, (value) => {
      entry.value = value;
      if (topic === "harnix.instances.sessions" && typeof params?.instance === "string") this.noteRunning(params.instance, value);
      fn();
    });
    return () => {
      off();
      entry.readers -= 1;
      if (entry.readers === 0 && this.values.get(key) === entry) this.values.delete(key);
    };
  }

  value(topic: string, params?: Record<string, unknown>): unknown {
    return this.values.get(topicKey(topic, params))?.value;
  }
}

export function useSelection(view: View): Selection | null {
  return useSyncExternalStore(view.subscribe, view.selected);
}

export function useTopic<T>(view: View, topic: string, params?: Record<string, unknown>): T | undefined {
  const key = topicKey(topic, params);
  const [, bump] = useState(0);
  useEffect(() => view.watch(topic, params, () => bump((n) => n + 1)), [view, key]);
  return view.value(topic, params) as T | undefined;
}

/** Draw again every `every` ms while given: for clocks that count on their own. */
export function useNow(every: number | null): void {
  const [, bump] = useState(0);
  useEffect(() => {
    if (!every) return;
    const timer = setInterval(() => bump((n) => n + 1), every);
    return () => clearInterval(timer);
  }, [every]);
}

/** `useTopic`, watching only while `on`. */
export function useTopicIf<T>(view: View, topic: string, params: Record<string, unknown>, on: boolean): T | undefined {
  const key = topicKey(topic, params);
  const [, bump] = useState(0);
  useEffect(() => (on ? view.watch(topic, params, () => bump((n) => n + 1)) : undefined), [view, key, on]);
  return on ? (view.value(topic, params) as T | undefined) : undefined;
}

/** A call made from this space, with its failure said in the banner. */
export async function act(app: App, run: () => Promise<unknown>): Promise<boolean> {
  try {
    await run();
    return true;
  } catch (e) {
    app.notice(errorText(e), "error");
    return false;
  }
}

export function isSelected(picked: Selection | null, row: Selection): boolean {
  if (!picked || picked.kind === "space" || picked.kind !== row.kind || picked.instance !== row.instance) return false;
  if (row.kind === "instance") return true;
  if (picked.kind === "instance" || picked.sessionId !== (row as { sessionId: string }).sessionId) return false;
  return row.kind === "session" || (picked as { taskId: string }).taskId === (row as { taskId: string }).taskId;
}
