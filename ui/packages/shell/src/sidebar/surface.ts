/**
 * Each session's docking surface (harnix-ext.md §22.7): the tabs open in the right column for that
 * session, which is in front, and the layout they sit in. One pane for now; the layout is a tree so
 * that splits and floating panes can come without changing what is stored.
 *
 * Persisted per session in localStorage, `harnix.sidebar.v1.<instance>/<session>`, read back when
 * the session is first looked at: what is stored is checked (`validateSurface`), and a stored value
 * that does not hold up is dropped — that session's key alone. Storage that fails leaves the surface
 * in memory for as long as the page is open. What is not stored: how a tab was last navigated to
 * (its params, its revision), its abort signal, its close handlers — those live with the page.
 *
 * No action edits a surface in place: each computes the next surface, keeps it, and tells the
 * listeners, so a draw always sees a whole state.
 */

/** A session's full name, `(instance, session)`, as `app.ts` writes it too. */
export interface SessionKey {
  instance: string;
  sessionId: string;
}

const keyOf = (instance: string, sessionId: string) => `${instance}/${sessionId}`;

export interface SidebarTab {
  /** Minted here, `tab-<n>`; stable while the tab is open. */
  id: string;
  kind: string;
  /** The resource it shows; null for a page opened by kind. A `multiple` type's page gets an
   *  address of its own (`harnix-tab://<kind>/<n>`) so that each opening is its own tab. */
  address: string | null;
  /** The chip's text as it was when the tab opened: what is shown when its type has gone. */
  title: string;
}

/** The one layout node there is for now: a pane with its tabs in order and the one in front. */
export interface Pane {
  type: "pane";
  id: string;
  tabs: string[];
  active: string | null;
}

/** The layout tree; a split or a floating pane is a node type to add here. */
export type Layout = Pane;

export interface SurfaceState {
  version: 1;
  layout: Layout;
  tabs: SidebarTab[];
  /** The next id to mint: carried with the surface so that ids never come back. */
  nextId: number;
}

/** How a tab was last navigated to; with the page, not stored. */
export interface Navigation {
  params: Record<string, unknown>;
  revision: number;
}

export const STORAGE_PREFIX = "harnix.sidebar.v1.";

export function storageKey(session: SessionKey): string {
  return `${STORAGE_PREFIX}${keyOf(session.instance, session.sessionId)}`;
}

export function emptySurface(): SurfaceState {
  return { version: 1, layout: { type: "pane", id: "p1", tabs: [], active: null }, tabs: [], nextId: 1 };
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const isString = (v: unknown): v is string => typeof v === "string" && v !== "";

/** A stored surface as it must be, or null for anything that is not: wrong fields, a pane naming a
 *  tab that is not there, a tab in no pane, a selection that is not in its pane, a counter below an id. */
export function validateSurface(raw: unknown): SurfaceState | null {
  if (!isRecord(raw) || raw.version !== 1 || !Array.isArray(raw.tabs) || !isRecord(raw.layout)) return null;
  const tabs: SidebarTab[] = [];
  const ids = new Set<string>();
  for (const tab of raw.tabs) {
    if (!isRecord(tab) || !isString(tab.id) || !isString(tab.kind) || typeof tab.title !== "string") return null;
    if (tab.address !== null && typeof tab.address !== "string") return null;
    if (ids.has(tab.id)) return null;
    ids.add(tab.id);
    tabs.push({ id: tab.id, kind: tab.kind, address: tab.address, title: tab.title });
  }
  const layout = raw.layout;
  if (layout.type !== "pane" || !isString(layout.id) || !Array.isArray(layout.tabs)) return null;
  const inPane = new Set<string>();
  for (const id of layout.tabs) {
    if (!isString(id) || !ids.has(id) || inPane.has(id)) return null;
    inPane.add(id);
  }
  if (inPane.size !== ids.size) return null;
  if (layout.active !== null && (!isString(layout.active) || !inPane.has(layout.active))) return null;
  if (layout.tabs.length && layout.active === null) return null;
  const minted = Math.max(0, ...tabs.map((t) => Number(/^tab-(\d+)$/.exec(t.id)?.[1] ?? 0)));
  if (typeof raw.nextId !== "number" || !Number.isInteger(raw.nextId) || raw.nextId < 1 || raw.nextId <= minted) return null;
  return { version: 1, layout: { type: "pane", id: layout.id, tabs: [...(layout.tabs as string[])], active: layout.active }, tabs, nextId: raw.nextId };
}

/** What `open` is given: the decided type's facts and the opener's options. */
export interface OpenSpec {
  kind: string;
  /** The resource; null for a page. */
  address: string | null;
  title: string;
  params?: Record<string, unknown>;
  /** A page of a `multiple` type: opened by kind, never deduplicated; at an address, one tab an address. */
  multiple?: boolean;
  /** False: the same content already open is not focused; a tab of its own opens. */
  revealIfOpened?: boolean;
  /** The tab the new one takes the place of, which closes. */
  replaceTab?: string;
  /** False: opened without coming to the front (a standing tab the shell adds), unless nothing is in front. */
  focus?: boolean;
}

interface Live {
  navigation: Navigation;
  abort: AbortController;
  closers: Set<() => void>;
  /** Its body has been in front at least once: a `keepMounted` body stays from then on. */
  visited: boolean;
}

/** The storage the store writes to: `localStorage`'s three methods, or none. */
export interface SurfaceStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export class SurfaceStore {
  private readonly surfaces = new Map<string, SurfaceState>();
  private readonly live = new Map<string, Map<string, Live>>();
  private readonly listeners = new Set<() => void>();
  private version = 0;

  constructor(private readonly storage: SurfaceStorage | null) {}

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly getVersion = (): number => this.version;

  /** The sessions whose surfaces are in memory, by key. */
  loaded(): string[] {
    return [...this.surfaces.keys()];
  }

  /** A session's surface: from memory, else from storage (checked), else empty. */
  surface(session: SessionKey): SurfaceState {
    const key = keyOf(session.instance, session.sessionId);
    const held = this.surfaces.get(key);
    if (held) return held;
    const state = this.read(storageKey(session)) ?? emptySurface();
    this.surfaces.set(key, state);
    return state;
  }

  /** The tabs as the strip draws them: in pane order. */
  tabs(session: SessionKey): SidebarTab[] {
    const s = this.surface(session);
    return s.layout.tabs.map((id) => s.tabs.find((t) => t.id === id)!).filter(Boolean);
  }

  /** The tab in front, if any. */
  active(session: SessionKey): SidebarTab | null {
    const s = this.surface(session);
    return s.layout.active ? (s.tabs.find((t) => t.id === s.layout.active) ?? null) : null;
  }

  tab(session: SessionKey, tabId: string): SidebarTab | null {
    return this.surface(session).tabs.find((t) => t.id === tabId) ?? null;
  }

  navigation(session: SessionKey, tabId: string): Navigation {
    return this.liveOf(session, tabId).navigation;
  }

  signal(session: SessionKey, tabId: string): AbortSignal {
    return this.liveOf(session, tabId).abort.signal;
  }

  visited(session: SessionKey, tabId: string): boolean {
    return this.liveOf(session, tabId).visited;
  }

  /** A tab's body was drawn in front: a `keepMounted` body is kept from now on. */
  visit(session: SessionKey, tabId: string): void {
    const live = this.liveOf(session, tabId);
    if (live.visited) return;
    live.visited = true;
    this.changed();
  }

  /** Run before the tab is closed or replaced. Returns the unsubscribe. */
  onClose(session: SessionKey, tabId: string, handler: () => void): () => void {
    const live = this.liveOf(session, tabId);
    live.closers.add(handler);
    return () => live.closers.delete(handler);
  }

  /**
   * Open content in a session's surface and bring it to the front; answers the tab's id. The same
   * content already open (same kind and address) is focused instead, unless `revealIfOpened` is
   * false or a `multiple` type is opened by kind (no address). Either way the tab's navigation
   * moves on: the params given, the revision up by one.
   */
  open(session: SessionKey, spec: OpenSpec): string {
    const s = this.surface(session);
    // A `multiple` type's resource is still one tab an address: an output tab names its task.
    const found = spec.multiple && spec.address === null ? undefined : s.tabs.find((t) => t.kind === spec.kind && t.address === spec.address);
    if (found && spec.revealIfOpened !== false) {
      this.navigate(session, found.id, spec.params);
      if (s.layout.active !== found.id) this.commit(session, { ...s, layout: { ...s.layout, active: found.id } });
      else this.changed();
      return found.id;
    }
    const id = `tab-${s.nextId}`;
    const address = spec.multiple && spec.address === null ? `harnix-tab://${spec.kind}/${s.nextId}` : spec.address;
    const tab: SidebarTab = { id, kind: spec.kind, address, title: spec.title };
    let order = [...s.layout.tabs, id];
    let tabs = [...s.tabs, tab];
    // Taking another tab's place: it closes, unless a close handler of its keeps it — then the new
    // one opens beside it.
    const replaced = spec.replaceTab !== undefined && s.layout.tabs.includes(spec.replaceTab) && this.runClosers(session, spec.replaceTab) ? spec.replaceTab : null;
    if (replaced) {
      order = s.layout.tabs.map((x) => (x === replaced ? id : x));
      tabs = tabs.filter((t) => t.id !== replaced);
      this.forget(session, replaced);
    }
    this.navigate(session, id, spec.params);
    const active = spec.focus === false && s.layout.active !== null && order.includes(s.layout.active) ? s.layout.active : id;
    this.commit(session, { version: 1, layout: { ...s.layout, tabs: order, active }, tabs, nextId: s.nextId + 1 });
    return id;
  }

  /** Bring a tab to the front. */
  select(session: SessionKey, tabId: string): void {
    const s = this.surface(session);
    if (!s.layout.tabs.includes(tabId) || s.layout.active === tabId) return;
    this.commit(session, { ...s, layout: { ...s.layout, active: tabId } });
  }

  /**
   * Close a tab: its close handlers run first, and one that throws keeps it (answers false). The
   * one in front closing hands over to its neighbour on the left, else the right.
   */
  close(session: SessionKey, tabId: string): boolean {
    const s = this.surface(session);
    const at = s.layout.tabs.indexOf(tabId);
    if (at < 0) return false;
    if (!this.runClosers(session, tabId)) return false;
    const order = s.layout.tabs.filter((x) => x !== tabId);
    const active = s.layout.active === tabId ? (order[Math.max(0, at - 1)] ?? null) : s.layout.active;
    this.forget(session, tabId);
    this.commit(session, { ...s, layout: { ...s.layout, tabs: order, active }, tabs: s.tabs.filter((t) => t.id !== tabId) });
    return true;
  }

  /** The session is gone (deleted): its surface, its storage and its live state with it. */
  drop(session: SessionKey): void {
    const key = keyOf(session.instance, session.sessionId);
    for (const live of this.live.get(key)?.values() ?? []) live.abort.abort();
    this.live.delete(key);
    this.surfaces.delete(key);
    try {
      this.storage?.removeItem(storageKey(session));
    } catch {
      /* storage that cannot be written was not read from either */
    }
    this.changed();
  }

  private runClosers(session: SessionKey, tabId: string): boolean {
    for (const handler of [...this.liveOf(session, tabId).closers]) {
      try {
        handler();
      } catch (e) {
        this.failed?.(tabId, e);
        return false;
      }
    }
    return true;
  }

  /** Told of a close handler that threw (the shell files it in the diagnostics). */
  failed: ((tabId: string, error: unknown) => void) | null = null;

  private navigate(session: SessionKey, tabId: string, params: Record<string, unknown> | undefined): void {
    const live = this.liveOf(session, tabId);
    live.navigation = { params: params ?? {}, revision: live.navigation.revision + 1 };
  }

  private forget(session: SessionKey, tabId: string): void {
    const key = keyOf(session.instance, session.sessionId);
    const live = this.live.get(key)?.get(tabId);
    if (!live) return;
    live.abort.abort();
    this.live.get(key)!.delete(tabId);
  }

  private liveOf(session: SessionKey, tabId: string): Live {
    const key = keyOf(session.instance, session.sessionId);
    let bySession = this.live.get(key);
    if (!bySession) this.live.set(key, (bySession = new Map()));
    let live = bySession.get(tabId);
    if (!live) bySession.set(tabId, (live = { navigation: { params: {}, revision: 0 }, abort: new AbortController(), closers: new Set(), visited: false }));
    return live;
  }

  private commit(session: SessionKey, next: SurfaceState): void {
    this.surfaces.set(keyOf(session.instance, session.sessionId), next);
    try {
      this.storage?.setItem(storageKey(session), JSON.stringify(next));
    } catch {
      /* private mode, or full: the surface lives in memory for as long as the page does */
    }
    this.changed();
  }

  private read(key: string): SurfaceState | null {
    let raw: string | null = null;
    try {
      raw = this.storage?.getItem(key) ?? null;
    } catch {
      return null;
    }
    if (raw === null) return null;
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      parsed = undefined;
    }
    const state = validateSurface(parsed);
    if (!state) {
      try {
        this.storage?.removeItem(key);
      } catch {
        /* nothing more to do: it was not readable either */
      }
    }
    return state;
  }

  private changed(): void {
    this.version += 1;
    for (const fn of [...this.listeners]) fn();
  }
}
