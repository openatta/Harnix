/** The system space's view: what is selected, and the hub topics that keep the page current. */

import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import type { HostApi, HostSessionRef as SessionKey, Registry } from "@harnix/react";

import { PAGE } from "./constants.ts";
import { failureText } from "./failure.tsx";
import type { ConsoleTabId, Scope, SessionRowView } from "./types.ts";

/** Where this browser keeps the sessions that finished unseen, and how many at most. */
const UNREAD_KEY = "harnix.system.unread";
const UNREAD_KEPT = 200;

/** What the right column is about. `space`: one space's sessions, across instances, the one a
 *  person came from (`current`) first. */
export type Selection =
  | { kind: "space"; space: string; current?: SessionKey }
  | { kind: "instance"; instance: string }
  /** `parent`: a sub-agent's session, under that main Agent's (its calls are in the parent's recording).
   *  `tool`: a tool call's id the person came for. */
  | { kind: "session"; instance: string; sessionId: string; parent?: string; tool?: string }
  | { kind: "task"; instance: string; sessionId: string; taskId: string };

/** A topic and its params, as one key. */
export function topicKey(topic: string, params?: Record<string, unknown>): string {
  return `${topic}\u0000${JSON.stringify(params ?? {})}`;
}

/**
 * The selection, and the hub topics this space watches. A topic is subscribed once however many
 * parts of the page read it, and given up when the last one stops; the host shares one subscription
 * per topic with the rest of the page.
 */
export class View {
  private picked: Selection | null = null;
  /** The console's tab, kept while the page is open: back from a contributed tab is back on it. */
  consoleTab: ConsoleTabId = "active";
  /** Each Agents tab's search, filter and length, kept while the page is open: a tab left and
   *  come back to is as it was. */
  readonly agentLists: Record<Scope, AgentListState> = { active: { query: "", who: "all", limit: PAGE }, all: { query: "", who: "all", limit: PAGE } };
  private readonly listeners = new Set<() => void>();
  /** Each topic watched here: its last value, its readers, and the host's subscription. */
  private readonly topics = new Map<string, { value: unknown; readers: Set<() => void>; off: () => void }>();
  /** Sessions whose turn ended while they were neither open nor picked here (`instance/session`):
   *  DSH's "completed", until someone looks. */
  private readonly unread = new Set<string>();
  /** The sessions each instance's list last showed running. */
  private readonly wasRunning = new Map<string, Set<string>>();

  constructor(
    readonly host: HostApi,
    readonly registry: Registry,
  ) {
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
    const open = this.host.session.current();
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

  select(selection: Selection): void {
    this.picked = selection;
    if ((selection.kind === "session" || selection.kind === "task") && this.unread.delete(`${selection.instance}/${selection.sessionId}`)) this.keepUnread();
    for (const fn of [...this.listeners]) fn();
    // The right column is where a selection is shown; drawn again with the new tabs.
    this.host.details.open();
    this.host.redraw();
  }

  /**
   * The console's tab in front changes. A contributed tab has no row to pick for the right column:
   * what was picked is let go, and the right column says what that tab is (`ConsoleAbout`), not
   * what was picked on another tab.
   */
  showTab(id: ConsoleTabId): void {
    this.consoleTab = id;
    if (id !== "active" && id !== "all") {
      this.picked = null;
      for (const fn of [...this.listeners]) fn();
    }
    this.host.redraw();
  }

  watch(topic: string, params: Record<string, unknown> | undefined, fn: () => void): () => void {
    const key = topicKey(topic, params);
    let entry = this.topics.get(key);
    if (!entry) {
      const made: { value: unknown; readers: Set<() => void>; off: () => void } = { value: undefined, readers: new Set(), off: () => undefined };
      this.topics.set(key, made);
      made.off = this.host.subscribe(topic, params ?? {}, (value) => {
        made.value = value;
        if (topic === "harnix.instances.sessions" && typeof params?.instance === "string") this.noteRunning(params.instance, value);
        for (const reader of [...made.readers]) reader();
      });
      entry = made;
    }
    const watched = entry;
    // Each reader is its own, even when one function reads twice.
    const reader = () => fn();
    watched.readers.add(reader);
    return () => {
      watched.readers.delete(reader);
      if (watched.readers.size || this.topics.get(key) !== watched) return;
      this.topics.delete(key);
      watched.off();
    };
  }

  value(topic: string, params?: Record<string, unknown>): unknown {
    return this.topics.get(topicKey(topic, params))?.value;
  }

  /** The sessions running a turn now (`instance/session`), as `harnix.sessions.status` last said. */
  running(): Set<string> {
    const rows = (this.value("harnix.sessions.status") as { sessions?: { instance: string; session_id: string }[] } | undefined)?.sessions ?? [];
    return new Set(rows.map((row) => `${row.instance}/${row.session_id}`));
  }

  /** The space a session's own space lists it under, among the interface's spaces: the one whose
   *  backend space is `space`, not this one. */
  spaceFor(space: string, self: string): string {
    return this.registry.at("space").find((c) => (c.sessions ?? c.id) === space && c.id !== self)?.id ?? space;
  }
}

/** How an Agents tab was left: its search, whose Agents, and how many of them are shown. */
export interface AgentListState {
  query: string;
  who: "all" | "user" | "harnix";
  limit: number;
}

export function useSelection(view: View): Selection | null {
  return useSyncExternalStore(view.subscribe, view.selected);
}

/** Whether `row` is what is selected: drawn again only when that changes, not on every pick. */
export function useIsSelected(view: View, row: Selection): boolean {
  const key = JSON.stringify(row);
  const read = useCallback(() => isSelected(view.selected(), row), [view, key]);
  return useSyncExternalStore(view.subscribe, read);
}

export function useTopic<T>(view: View, topic: string, params?: Record<string, unknown>): T | undefined {
  const key = topicKey(topic, params);
  const [, bump] = useState(0);
  useEffect(() => view.watch(topic, params, () => bump((n) => n + 1)), [view, key]);
  return view.value(topic, params) as T | undefined;
}

/** The sessions running a turn now, kept current: `harnix.sessions.status`. */
export function useRunning(view: View): Set<string> {
  useTopic(view, "harnix.sessions.status");
  return view.running();
}

/** One clock per pace, shared: what reads every three seconds reads together, and a timer runs
 *  only while something counts on it. */
const clocks = new Map<number, { tick: number; listeners: Set<() => void>; timer: ReturnType<typeof setInterval> }>();

function onTick(every: number, fn: () => void): () => void {
  let clock = clocks.get(every);
  if (!clock) {
    const made: { tick: number; listeners: Set<() => void>; timer: ReturnType<typeof setInterval> } = {
      tick: 0,
      listeners: new Set(),
      timer: setInterval(() => {
        made.tick += 1;
        for (const listener of [...made.listeners]) listener();
      }, every),
    };
    clock = made;
    clocks.set(every, clock);
  }
  const shared = clock;
  shared.listeners.add(fn);
  return () => {
    shared.listeners.delete(fn);
    if (shared.listeners.size || clocks.get(every) !== shared) return;
    clearInterval(shared.timer);
    clocks.delete(every);
  };
}

/**
 * How many beats the shared clock that goes every `every` ms has struck since something began
 * reading it (0 while `every` is not given): what reads it is drawn again on each beat. Kept to the parts that change with it — a time on one row, not
 * the list it is in.
 */
export function useTick(every: number | null): number {
  const subscribe = useCallback((fn: () => void) => (every ? onTick(every, fn) : () => undefined), [every]);
  return useSyncExternalStore(subscribe, () => (every ? (clocks.get(every)?.tick ?? 0) : 0));
}

/** `useTopic`, watching only while `on`. */
export function useTopicIf<T>(view: View, topic: string, params: Record<string, unknown>, on: boolean): T | undefined {
  const key = topicKey(topic, params);
  const [, bump] = useState(0);
  useEffect(() => (on ? view.watch(topic, params, () => bump((n) => n + 1)) : undefined), [view, key, on]);
  return on ? (view.value(topic, params) as T | undefined) : undefined;
}

/** A call made from this space, with its failure said in the banner (in the page's words, `failure.tsx`). */
export async function act(host: HostApi, run: () => Promise<unknown>): Promise<boolean> {
  try {
    await run();
    return true;
  } catch (e) {
    host.shell.notice(failureText(e), "error");
    return false;
  }
}

export function isSelected(picked: Selection | null, row: Selection): boolean {
  if (!picked || picked.kind === "space" || picked.kind !== row.kind || picked.instance !== row.instance) return false;
  if (row.kind === "instance") return true;
  if (picked.kind === "instance" || picked.sessionId !== (row as { sessionId: string }).sessionId) return false;
  return row.kind === "session" || (picked as { taskId: string }).taskId === (row as { taskId: string }).taskId;
}
