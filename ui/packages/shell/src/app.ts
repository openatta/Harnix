/**
 * 外壳的状态与动作：连接、hello、前台 space 的会话列表、当前会话、右栏、横幅、模态框、搜索框、主题。
 *
 * 一份不可变快照，`subscribe` / `getSnapshot` 对上 `useSyncExternalStore`。会话本身的状态不在这里：
 * 它在 `@harnix/core/state` 的控制器里，`ChatPanel` 打开会话时建。这里只记"开的是哪个"。
 *
 * 会话的全名是 `(instance, session)`，而且属于一个 space（概念与架构 §5.4）：没有全局的「当前会话」，记的是
 * 每个 space 各自的当前会话；前台 space 换了，中区、输入框、右栏就换成那个 space 自己的那一项，切回来
 * 恢复它记住的。外壳发出的每个会话调用都带上前台 space，中枢对着账本核对。
 */

import type { ReactNode } from "react";
import { TOKEN_STALE, modelConfigured, type ConnectionStatus, type ExtensionEntry, type HarnixClient, type Hello, type SessionRow, type UiView } from "@harnix/core";
import { MAIN_INSTANCE, type SessionController } from "@harnix/core/state";
import { CONVERSATION_VIEW, sidebarTypes, t, tabsOf, type ComposerApi, type Contributions, type DetailItem, type Registry, type ShellContext, type SidebarTabType, type SpaceContribution, type SpaceTab, explainError } from "@harnix/react";

import { SurfaceStore, type SidebarTab } from "./sidebar/surface.ts";
import { topicsOf } from "./topics.ts";
import { Shortcuts } from "./shortcuts.tsx";

export interface SessionKey {
  instance: string;
  sessionId: string;
}

/** `harnix.sessions.list` 的一行，实例补齐。 */
export interface ShellSession extends SessionRow {
  instance: string;
}

/** 一个模态框：标题，和画 `.body` 与 `.foot` 的内容（见 `dialogs.tsx` 的 `Body` / `Foot`）。 */
export interface Modal {
  title: string;
  content: ReactNode;
  wide?: boolean;
  /** Opened again while it is on top, a modal with the same key takes its place rather than going over it. */
  key?: string;
}

/** A modal on the shell's stack: what was opened, and which opening it was. */
export interface OpenModal extends Modal {
  readonly id: number;
}

/** One line of the banner: info goes by itself after a while, an error stays longer. */
export interface Notice {
  text: string;
  tone?: "info" | "error";
}

/** How long a notice stays in front before the next takes its place; a sticky one stays. */
const NOTICE_MS = { info: 6_000, error: 12_000 } as const;
/** At most this many wait behind the one shown: a burst of failures is not a queue to read through. */
const NOTICE_QUEUE = 5;

export type ThemeChoice = "auto" | "light" | "dark";

/** The side columns' widths, as the reader dragged them: the left one, the right one, and a
 *  space's second side panel (xops-04 §7). */
export interface Widths {
  side: number;
  details: number;
  secondary: number;
}

export const WIDTHS: { side: [number, number, number]; details: [number, number, number]; secondary: [number, number, number] } = {
  // min, default, max
  side: [200, 260, 360],
  details: [320, 360, 1600], // the right column's real widest is `detailsMax`
  secondary: [300, 440, 760],
};

/** The defaults, with what the reader dragged over them. */
function widthsWith(dragged: Partial<Widths>): Widths {
  return { side: WIDTHS.side[1], details: WIDTHS.details[1], secondary: WIDTHS.secondary[1], ...dragged };
}

/** The second side panel's width: what the reader dragged, else what the space asked for, else the default. */
export function secondaryWidthOf(s: Pick<ShellState, "dragged">, asked: number | undefined): number {
  const [min, def, max] = WIDTHS.secondary;
  if (s.dragged.secondary !== undefined) return s.dragged.secondary;
  if (typeof asked !== "number" || !Number.isFinite(asked)) return def;
  return Math.round(Math.min(max, Math.max(min, asked)));
}

/** The centre's narrowest beside an open right column: the right column gives way first. */
export const CENTRE_MIN = 400;

/** The right column's widest: 70% of the window, and never so wide that the centre beside the left
 *  column would go under `CENTRE_MIN`. The layout holds the same bound when the window shrinks. */
export function detailsMax(side: number, viewport: number = typeof window === "undefined" ? 1600 : window.innerWidth): number {
  return Math.max(WIDTHS.details[0], Math.min(Math.round(viewport * 0.7), viewport - side - CENTRE_MIN));
}

/** The right column's width: what the reader dragged, else what the front space asked for, else the
 *  default. A space's width under 1 is a share of the window (DSH opens its right column at 0.45). */
export function detailsWidthOf(
  s: Pick<ShellState, "widths" | "dragged">,
  space: { details?: { width?: number } } | undefined,
  viewport: number = typeof window === "undefined" ? 1600 : window.innerWidth,
): number {
  const min = WIDTHS.details[0];
  const max = detailsMax(s.widths.side, viewport);
  const within = (w: number) => Math.round(Math.min(max, Math.max(min, w)));
  if (s.dragged.details !== undefined) return within(s.dragged.details);
  const asked = space?.details?.width;
  if (typeof asked !== "number" || !Number.isFinite(asked) || asked <= 0) return within(s.widths.details);
  return within(asked < 1 ? asked * viewport : asked);
}

export interface ShellState {
  /** The space in front. */
  space: string;
  /** Each space's own current session. */
  currentBySpace: Record<string, SessionKey | null>;
  connection: ConnectionStatus;
  hello: Hello | null;
  /** Every extension and whether it is enabled: hello's, then `harnix.ui`'s as it changes. */
  extensions: ExtensionEntry[];
  reachable: string[];
  /** The front space's sessions, as the hub lists them for the backend space it claims. */
  sessions: ShellSession[];
  /** 正在跑的会话，`instance/session` → true（`harnix.sessions.status` 主题） */
  running: Record<string, boolean>;
  /** The front space's current session: `currentBySpace[space]`, kept in step by `set`. */
  current: SessionKey | null;
  detail: DetailItem | null;
  /** The right column, open or not, per space: each space keeps its own (unset → its default). */
  detailsBySpace: Record<string, boolean>;
  /** The right column takes the centre's place too (it closes back to its own width). */
  detailsMax: boolean;
  /** A space's second side panel takes the whole centre, per space; remembered (xops-04 §7). */
  secondaryMaxBySpace: Record<string, boolean>;
  /** The sidebar as the person left it in a wide window (⌘\ there, the toggle). */
  sidebar: "expanded" | "collapsed";
  /** The window is 960px wide or less (`Shell` follows the media query). The sidebar is a rail then,
   *  and the right column a drawer over the conversation. */
  narrow: boolean;
  /** Narrow, the sidebar is open as a drawer over the page: the person asked for it (the toggle,
   *  the rail's entry, ⌘\); picking a session, another space, Escape or the veil close it. */
  drawer: boolean;
  /** The notice in front: `notices[0]`, kept in step by `set`. */
  banner: Notice | null;
  /** What the banner has to say, in order: the one in front first. */
  notices: Notice[];
  /** The shell's dialogs, bottom first: each opened goes over the last, Escape closes the top one. */
  modals: OpenModal[];
  /** The one on top: `modals`' last, kept in step by `set`. */
  modal: OpenModal | null;
  /** The page title's count prefix (`host.shell.badge`, ipdrun-33); null ⇒ none. */
  titleBadge: number | null;
  /** The line under the brand (`host.shell.context`, ipdrun-49); null ⇒ none. */
  shellContext: ShellContext | null;
  /** The left column's search box, drawn while the front space gives `sidebar.search`. */
  search: { query: string };
  theme: ThemeChoice;
  widths: Widths;
  /** The widths the reader set by dragging; a column not in here has its default (or its space's). */
  dragged: Partial<Widths>;
  /** The avatar's menu. */
  menu: boolean;
  /** The right column's selected tab, per space. `asked`: chosen through the host API, so it is kept
   *  when the space is entered again; a tab the reader clicked goes back to the first, as before
   *  (kept too for a space that gives `tab()`). */
  tabBySpace: Record<string, { id: string; asked: boolean }>;
  /** Which side of the right column's strip is in front: the front space's tabs (`details.tabs`,
   *  the details, the `scope: "space"` types — `tabBySpace` says which), or the open session's
   *  surface (`sidebar`, whose active tab it is). Opening or clicking a session tab brings that side
   *  in front; selecting a space tab or revealing a detail brings the space's. */
  stripSide: "space" | "session";
  /** Each session's chosen view of the centre (a `conversation.view` id), by session key; one not
   *  here is shown as its conversation. Kept in this browser. */
  viewBySession: Record<string, string>;
  /** Bumped by `redraw()`: the shell reads the front space's functions again. */
  drawn: number;
}

const THEME_KEY = "harnix.theme";
const SPACE_KEY = "harnix.space";
const CURRENT_KEY = "harnix.space.current";
const WIDTHS_KEY = "harnix.widths";
const DETAILS_KEY = "harnix.details";
const SECONDARY_MAX_KEY = "harnix.secondaryMax";
/** Which view of the centre each session is shown in (`conversation.view`), by session key; the
 *  conversation when absent. */
const VIEW_KEY = "harnix.view";
/** In the tab's own storage: this tab has landed once, so a reload keeps the space in front. */
const LANDED_KEY = "harnix.landed";

/** Whether this tab is opened afresh rather than reloaded: true the first time it is asked in a tab,
 *  and every time where the tab keeps no storage of its own. */
function firstOpen(): boolean {
  try {
    if (sessionStorage.getItem(LANDED_KEY)) return false;
    sessionStorage.setItem(LANDED_KEY, "1");
  } catch {
    // No storage of the tab's: each load is a first one.
  }
  return true;
}

/** The page's storage, if it has one that can be touched at all. */
function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

function stored<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw == null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}

function remember(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* private mode: it does not survive a reload */
  }
}

export const keyOf = (instance: string, sessionId: string) => `${instance}/${sessionId}`;

/** How long the start waits for an answer it can do without: `harnix.reachable`, the built-ins. */
export const BOOT_WAIT_MS = 5_000;
export const TIMED_OUT: unique symbol = Symbol("timed out");

/** The promise's value, or `TIMED_OUT` once `ms` have passed; the promise itself goes on. */
export function within<T>(promise: Promise<T>, ms: number): Promise<T | typeof TIMED_OUT> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<typeof TIMED_OUT>((resolve) => {
    timer = setTimeout(() => resolve(TIMED_OUT), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

function sameKey(a: SessionKey | null, b: SessionKey | null): boolean {
  return a === b || (a != null && b != null && a.instance === b.instance && a.sessionId === b.sessionId);
}

/** The tabs as the strip draws them: the `end` ones after the others. */
export function orderTabs<T extends { align?: "start" | "end" }>(tabs: T[]): T[] {
  return [...tabs.filter((x) => x.align !== "end"), ...tabs.filter((x) => x.align === "end")];
}

/** A tab of the strip's space side: one of the space's own, or a `scope: "space"` tab type in
 *  force, which stands in the strip for every space and is drawn with a tab domain of its own. */
export type StripTab = SpaceTab & { type?: SidebarTabType & { owner: string } };

/** A space's tabs — a `tabs()` that throws gives none, and is counted as failed — then the
 *  `scope: "space"` tab types in force, each as a tab whose id is its kind. */
export function safeTabs(registry: Registry, space: (SpaceContribution & { owner?: string }) | undefined): StripTab[] {
  const own: StripTab[] = ask(registry, space, () => tabsOf(space), []);
  const types = sidebarTypes(registry.at("sidebar.tab")).filter((type) => type.scope === "space");
  return [...own, ...types.map((type): StripTab => ({ id: type.kind, title: () => type.title(), render: () => type.render(), type }))];
}

/** One of a space's functions, read while drawing: one that throws gives `fallback`, and the space
 *  is counted as failed (after the draw) — the rest of the page draws on. */
export function ask<T>(registry: Registry, space: (SpaceContribution & { owner?: string }) | undefined, read: () => T, fallback: T): T {
  try {
    return read();
  } catch (e) {
    if (space) later(() => registry.fail("space", { id: space.id, owner: space.owner ?? "?" }, e));
    return fallback;
  }
}

/** Why a space cannot be entered for now (`SpaceContribution.disabled`, ipdrun-48): its string, or null
 *  when it can — no `disabled()`, a non-string answer, or a throw (noted as the space's failure). */
export function disabledOf(registry: Registry, space: (SpaceContribution & { owner?: string }) | undefined): string | null {
  const disabled = space?.sidebar?.disabled;
  if (!disabled) return null;
  const why = ask(registry, space, () => disabled(), null);
  return typeof why === "string" ? why : null;
}

/** Report after the current draw: a registry change made while drawing would draw again at once. */
export function later(fn: () => void): void {
  queueMicrotask(fn);
}

function storedTheme(): ThemeChoice {
  try {
    const value = localStorage.getItem(THEME_KEY);
    return value === "light" || value === "dark" ? value : "auto";
  } catch {
    return "auto";
  }
}

export class App {
  readonly conn: HarnixClient;
  readonly registry: Registry;
  private state: ShellState;
  private readonly listeners = new Set<() => void>();
  private booted = false;
  /** The open session's controller and composer, for the host API. Not state: nothing redraws on them. */
  controller: SessionController | null = null;
  composer: ComposerApi | null = null;
  private builtins: ((app: App) => void | Promise<void>) | null = null;
  private interfaces: ((app: App, view: UiView) => Promise<void>) | null = null;
  /** Each session's docking surface in the right column (`sidebar/surface.ts`). */
  readonly sidebar: SurfaceStore;
  /** The `shortcut` commands and the keys they have now (`shortcuts.tsx`). */
  readonly shortcuts: Shortcuts;

  constructor(client: HarnixClient, registry: Registry) {
    this.conn = client;
    this.registry = registry;
    this.sidebar = new SurfaceStore(storage());
    this.sidebar.failed = (tabId, e) => registry.fail("sidebar.tab", { id: tabId, owner: "page" }, e);
    this.shortcuts = new Shortcuts(registry, { storage: storage() });
    // Until hello says which space this product starts in, the one left in front last time.
    const space = stored<string>(SPACE_KEY, "");
    const currentBySpace = stored<Record<string, SessionKey | null>>(CURRENT_KEY, {});
    this.state = {
      space,
      currentBySpace,
      connection: client.status,
      hello: null,
      extensions: [],
      reachable: [],
      sessions: [],
      running: {},
      current: currentBySpace[space] ?? null,
      detail: null,
      detailsBySpace: stored<Record<string, boolean>>(DETAILS_KEY, {}),
      detailsMax: false,
      secondaryMaxBySpace: stored<Record<string, boolean>>(SECONDARY_MAX_KEY, {}),
      sidebar: "expanded",
      narrow: false,
      drawer: false,
      banner: null,
      notices: [],
      modals: [],
      modal: null,
      titleBadge: null,
      shellContext: null,
      search: { query: "" },
      theme: storedTheme(),
      widths: widthsWith(stored<Partial<Widths>>(WIDTHS_KEY, {})),
      dragged: stored<Partial<Widths>>(WIDTHS_KEY, {}),
      menu: false,
      tabBySpace: {},
      stripSide: "space",
      viewBySession: stored<Record<string, string>>(VIEW_KEY, {}),
      drawn: 0,
    };
  }

  /* ── 观察 ─────────────────────────────────────────────────────────── */

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly getSnapshot = (): ShellState => this.state;

  get snapshot(): ShellState {
    return this.state;
  }

  set(change: Partial<ShellState>): void {
    const before = this.state;
    const next = { ...this.state, ...change };
    next.current = next.currentBySpace[next.space] ?? null;
    next.modal = next.modals[next.modals.length - 1] ?? null;
    next.banner = next.notices[0] ?? null;
    this.state = next;
    if (before.banner !== next.banner) this.timeNotice(next.banner);
    for (const fn of [...this.listeners]) fn();
    if ((before.modal == null) !== (next.modal == null)) {
      for (const fn of [...this.modalListeners]) fn(next.modal != null);
    }
    if (!sameKey(before.current, next.current)) {
      const ref = next.current ? { instance: next.current.instance, sessionId: next.current.sessionId } : null;
      for (const fn of [...this.currentListeners]) fn(ref);
    }
    if (!sameKey(before.current, next.current) || before.viewBySession !== next.viewBySession) {
      const was = this.conversationView(before.current, before);
      const now = this.conversationView(next.current, next);
      if (was !== now) for (const fn of [...this.viewListeners]) fn(now);
    }
    if (before.search.query !== next.search.query) this.search(next.search.query);
    // The settings panel closed: what it changed may be the model providers the composer offers.
    if (before.modals.some((m) => m.key === "settings") && !next.modals.some((m) => m.key === "settings")) void this.refreshEngine();
  }

  /** Tell the front space what the left column's search box says (`sidebar.search`); `submit` when
   *  Enter was pressed in it. */
  search(query: string, submit = false): void {
    const space = this.registry.at("space").find((c) => c.id === this.state.space);
    const search = space?.sidebar?.search;
    if (search) ask(this.registry, space, () => search(query, submit || undefined), undefined);
  }

  private readonly currentListeners = new Set<(current: SessionKey | null) => void>();

  /** The front space's current session changed — opened, created, closed, deleted, or another space in front. */
  onCurrent(fn: (current: SessionKey | null) => void): () => void {
    this.currentListeners.add(fn);
    return () => this.currentListeners.delete(fn);
  }

  /* ── the centre's view of a session ───────────────────────────────── */

  private readonly viewListeners = new Set<(view: string) => void>();

  /** The open session's view changed: switched, or another session is open. */
  onView(fn: (view: string) => void): () => void {
    this.viewListeners.add(fn);
    return () => this.viewListeners.delete(fn);
  }

  /** The `conversation.view` registrations that apply to a session, in the switch's order; a
   *  `match` that throws drops its view. */
  viewsFor(key: SessionKey | null): (Contributions["conversation.view"] & { owner: string })[] {
    if (!key) return [];
    const ref = { instance: key.instance, sessionId: key.sessionId };
    return [...this.registry.at("conversation.view")]
      .filter((view) => {
        if (!view.match) return true;
        try {
          return view.match(ref) === true;
        } catch (e) {
          later(() => this.registry.fail("conversation.view", view, e));
          return false;
        }
      })
      .sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
  }

  /** Which view the centre shows a session in: its remembered choice while that view still applies
   *  to it, else the conversation. */
  conversationView(key: SessionKey | null, state: ShellState = this.state): string {
    if (!key) return CONVERSATION_VIEW;
    const chosen = state.viewBySession[keyOf(key.instance, key.sessionId)];
    if (!chosen || chosen === CONVERSATION_VIEW) return CONVERSATION_VIEW;
    return this.viewsFor(key).some((view) => view.id === chosen) ? chosen : CONVERSATION_VIEW;
  }

  /** Show a session in a view, and remember it for the session; the conversation forgets the entry. */
  setConversationView(key: SessionKey, view: string): void {
    const id = keyOf(key.instance, key.sessionId);
    const viewBySession = { ...this.state.viewBySession };
    if (view === CONVERSATION_VIEW) delete viewBySession[id];
    else viewBySession[id] = view;
    remember(VIEW_KEY, viewBySession);
    this.set({ viewBySession });
  }

  private readonly modalListeners = new Set<(open: boolean) => void>();

  /** The shell's own modal layer opened or closed. */
  onModal(fn: (open: boolean) => void): () => void {
    this.modalListeners.add(fn);
    return () => this.modalListeners.delete(fn);
  }

  /* ── composers ────────────────────────────────────────────────────── */

  /** Each open session's composer while it is on the page, by session key. */
  private readonly composers = new Map<string, ComposerApi>();
  /** Text for a session whose composer is not on the page yet: it goes in when that one is bound. */
  private readonly pendingInserts = new Map<string, string>();

  /** A session's composer is on the page: text waiting for it goes in now. */
  composerBound(key: string, api: ComposerApi): void {
    this.composers.set(key, api);
    const pending = this.pendingInserts.get(key);
    if (pending === undefined) return;
    this.pendingInserts.delete(key);
    api.insert(pending);
  }

  /** A session's composer left the page; only if it is still the one bound. */
  composerGone(key: string, api: ComposerApi): void {
    if (this.composers.get(key) === api) this.composers.delete(key);
  }

  /** Text into a composer: whichever is on the page; with `session`, that session's — now if it is
   *  on the page, else when it is (appended to text already waiting). */
  insert(text: string, session?: SessionKey): void {
    if (!session) {
      this.composer?.insert(text);
      return;
    }
    const key = keyOf(session.instance, session.sessionId);
    const api = this.composers.get(key);
    if (api) api.insert(text);
    else this.pendingInserts.set(key, (this.pendingInserts.get(key) ?? "") + text);
  }

  /* ── drawing ──────────────────────────────────────────────────────── */

  private drawWaiters: (() => void)[] = [];

  /** Read the front space's functions again (tabs, title, badge…): a redraw, the registry untouched. */
  redraw(): void {
    this.set({ drawn: this.state.drawn + 1 });
    this.followSpaceTab();
  }

  /** The front space's own say on its selected tab (`SpaceContribution.tab`): taken when it names one
   *  of its tabs, or "details". A space without `tab()` is left alone. */
  private followSpaceTab(): void {
    const space = this.registry.at("space").find((c) => c.id === this.state.space);
    const tab = space?.details?.tab;
    if (!tab) return;
    const id = ask(this.registry, space, () => tab(), undefined);
    if (typeof id !== "string" || id === this.shownTab()) return;
    if (id === "details" || safeTabs(this.registry, space).some((x) => x.id === id)) this.selectTab(id, true);
  }

  /** Resolves once the page has drawn the state as it is now. The shell says so (`drew`); a page
   *  with no shell drawing it resolves after a moment rather than never. */
  afterDraw(): Promise<void> {
    return new Promise((resolve) => {
      const timer = setTimeout(done, 2000);
      function done() {
        clearTimeout(timer);
        resolve();
      }
      this.drawWaiters.push(done);
      this.redraw();
    });
  }

  /** The shell has committed a draw. */
  drew(): void {
    const waiting = this.drawWaiters;
    this.drawWaiters = [];
    for (const fn of waiting) fn();
  }

  /* ── 启动 ─────────────────────────────────────────────────────────── */

  /**
   * 连上就跑一遍：hello、可调方法、内建贡献与 hello 里启用的扩展的 UI 包（一页一次，首屏之前）、会话列表。重连后再跑一遍。
   * 之后跟着 `harnix.ui`：它的第一个值与每次变化交给 `follow`（补注册、整批撤下、进页面的模块与面）。
   */
  start(activate: (app: App) => void | Promise<void>, follow?: (app: App, view: UiView) => Promise<void>): void {
    this.builtins = activate;
    this.interfaces = follow ?? null;
    this.conn.onStatus((status, detail) => {
      this.set({ connection: status });
      if (status === "open") {
        const restarted = t("conn.restarted");
        if (this.state.notices.some((n) => n.text === restarted)) this.set({ notices: this.state.notices.filter((n) => n.text !== restarted) });
        void this.boot();
      }
      // Reconnecting, and no new token to be had: only a reload gets this page back.
      if (status === "closed" && detail === TOKEN_STALE) this.notice(t("conn.restarted"), "error", { sticky: true });
      if (status === "incompatible") this.notice(t("banner.incompatible", { reason: detail ?? "" }), "error", { sticky: true });
    });
    this.conn.onNotification((method, params) => this.handle(method, params));
    this.conn.connect();
  }

  private async boot(): Promise<void> {
    try {
      const hello = (await this.conn.call("harnix.hello", {})) as Hello;
      this.set({ hello, extensions: hello.extensions ?? this.state.extensions });
      if (!this.booted) {
        this.booted = true;
        // Not waited for past a bound: the rest of the start (sessions, the front space) does not hang
        // on it. Late, it is taken when it comes; what was registered meanwhile went without it.
        const asked = this.conn.call("harnix.reachable", {}).catch(() => ({ methods: [] })) as Promise<{ methods?: string[] }>;
        const reachable = await within(asked, BOOT_WAIT_MS);
        if (reachable === TIMED_OUT) void asked.then((r) => this.set({ reachable: r.methods ?? [] }));
        else this.set({ reachable: reachable.methods ?? [] });
        // Through the page's counted table: a space subscribing to the same topic and leaving again
        // does not take it from the shell.
        topicsOf(this.conn).subscribe("harnix.sessions.status", {}, (value) => this.applyStatus(value));
        // A session made, renamed, closed or deleted — by this page or another — is pushed for its
        // instance: the front space's list is read again.
        topicsOf(this.conn).subscribe("harnix.instances.sessions", { instance: this.defaultInstance() }, () => void this.refreshSessions());
        // Before the stored front space is looked for: the spaces are the UI packages'. Bounded
        // (`within`): an extension's start that awaits the backend and never hears back does not
        // stop the shell's.
        const registering = Promise.resolve(this.builtins?.(this)).catch((e: unknown) => this.notice(explainError(e), "error"));
        await within(registering, BOOT_WAIT_MS);
        // Every registration of the start is in: what went into a point nobody defined goes now.
        this.registry.settle();
        this.landOnASpace();
        this.followUi();
      }
      await this.refreshSessions();
    } catch (e) {
      this.notice(explainError(e), "error");
    }
  }

  /** Each `harnix.ui` value, one after another: what the page draws for whom follows it. */
  private uiChain: Promise<void> = Promise.resolve();

  private followUi(): void {
    topicsOf(this.conn).subscribe("harnix.ui", {}, (value) => {
      const view = value as UiView | null;
      if (!view || !Array.isArray(view.extensions)) return;
      const before = this.state.extensions;
      this.set({ extensions: view.extensions });
      if (JSON.stringify(before) !== JSON.stringify(view.extensions)) for (const fn of [...this.extensionListeners]) fn();
      this.uiChain = this.uiChain
        .then(() => this.interfaces?.(this, view))
        .catch((e: unknown) => this.notice(explainError(e), "error"));
    });
  }

  private readonly extensionListeners = new Set<() => void>();

  /** An extension was enabled, disabled, registered or withdrawn. */
  onExtensions(fn: () => void): () => void {
    this.extensionListeners.add(fn);
    return () => this.extensionListeners.delete(fn);
  }

  /** Ask `harnix.reachable` again: an extension package registered since the start adds methods. */
  async refreshReachable(): Promise<void> {
    try {
      const r = (await this.conn.call("harnix.reachable", {})) as { methods?: string[] };
      this.set({ reachable: r.methods ?? [] });
    } catch {
      // What was known stays.
    }
  }

  /** Read what hello says of the engine again — its model providers — after the settings panel,
   *  which may have changed one, closes. */
  async refreshEngine(): Promise<void> {
    const hello = (await this.conn.call("harnix.hello", {}).catch(() => null)) as Hello | null;
    if (hello?.engine && this.state.hello) this.set({ hello: { ...this.state.hello, engine: hello.engine } });
  }

  /**
   * Whether a turn has a model to go to: a provider, or the `ANTHROPIC_*` key the engine started
   * with. Unknown (hello could not read the providers) counts as yes, so the page does not cry wolf.
   */
  modelReady(): boolean {
    return modelConfigured(this.state.hello?.engine);
  }

  /** The model a new session runs on: the default provider's, else the engine's; none while no
   *  model is configured (`modelReady()`), whatever name the engine started with. */
  defaultModel(): string | null {
    if (!this.modelReady()) return null;
    const engine = this.state.hello?.engine;
    const providers = engine?.providers;
    const chosen = providers?.default_provider ? providers.providers[providers.default_provider] : undefined;
    return chosen?.default_model || (typeof engine?.model === "string" ? engine.model : null);
  }

  /** A page opened afresh: the space that says it is where the page lands (`landing`). Else the
   *  stored front space, if this product still has it; else the first one it does have. */
  private landOnASpace(): void {
    const spaces = [...this.registry.at("space")].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
    if (!spaces.length) return;
    const landing = spaces.find((c) => c.landing && !c.within && disabledOf(this.registry, c) === null);
    if (landing && firstOpen()) {
      if (landing.id !== this.state.space) this.set({ space: landing.id });
      return;
    }
    const here = spaces.find((c) => c.id === this.state.space);
    if (here && disabledOf(this.registry, here) === null) return;
    // Nothing kept, or gone: the product's own first space, as hello names it, if it can be entered.
    const named = this.state.hello?.defaults?.space;
    const first = named ? spaces.find((c) => !c.within && (c.id === named || c.sessions === named) && disabledOf(this.registry, c) === null) : undefined;
    if (first) {
      if (first.id !== this.state.space) this.set({ space: first.id });
      return;
    }
    // The stored space is gone or cannot be entered for now (ipdrun-48): the first that can, by
    // order — one shown inside another (`within`) is not landed on by itself; when none can, stay
    // where we are if that space is still here, else the first.
    const space = (spaces.find((c) => !c.within && disabledOf(this.registry, c) === null) ?? here ?? spaces[0])!.id;
    if (space !== this.state.space) this.set({ space });
  }

  /** Whether the shell offers to start a session at all: `[shell] new_session` (ipdrun-46), on unless
   *  the profile says false. A space's own `newSession` applies on top. */
  startsSessions(): boolean {
    return this.state.hello?.shell?.new_session !== false;
  }

  /** `[shell] narrow_details` (ipdrun-50): what an open right column does at ≤ 960px. */
  narrowDetails(): "over" | "beside" {
    return this.state.hello?.shell?.narrow_details === "beside" ? "beside" : "over";
  }

  /** The backend space a front space's sessions belong to (`SpaceContribution.sessions`). */
  claim(space = this.state.space): string {
    return this.registry.at("space").find((c) => c.id === space)?.sessions ?? space;
  }

  /** The instance a session that names none runs on: hello's `defaults`. */
  defaultInstance(): string {
    return this.state.hello?.defaults?.instance ?? MAIN_INSTANCE;
  }

  /* ── 服务端通知 ───────────────────────────────────────────────────── */

  private handle(method: string, params: Record<string, unknown>): void {
    if (method === "harnix.event" && (params.event as { kind?: string } | undefined)?.kind === "skills_changed") {
      // The command list is the engine's; a skill installed mid-session adds one.
      void this.conn
        .call("engine.commands.list", {})
        .then((r) => {
          const hello = this.state.hello;
          if (hello) this.set({ hello: { ...hello, commands: (r as { commands?: unknown[] }).commands ?? hello.commands } });
        })
        .catch(() => undefined);
    } else if (method === "harnix.turn_settled") {
      void this.refreshSessions();
    } else if (method === "harnix.host_event" && params.kind === "mcp_connect_failed") {
      this.notice(t("banner.mcpFailed", { server: String(params.server ?? "?"), error: String(params.error ?? "") }), "error");
    }
  }

  private applyStatus(value: unknown): void {
    const rows = ((value as { sessions?: { instance: string; session_id: string }[] } | null)?.sessions ?? []);
    const running: Record<string, boolean> = {};
    for (const row of rows) running[keyOf(row.instance, row.session_id)] = true;
    this.set({ running });
  }

  /* ── 会话列表 ─────────────────────────────────────────────────────── */

  /** Which `refreshSessions` is the latest: an earlier one answering later must not undo it. */
  private refreshes = 0;

  async refreshSessions(): Promise<void> {
    const mine = ++this.refreshes;
    const space = this.state.space;
    if (!space) return;
    try {
      const listed = (await this.conn.call("harnix.sessions.list", { space: this.claim(space) })) as { sessions: SessionRow[] };
      const sessions: ShellSession[] = (listed.sessions ?? []).map((row) => ({
        ...row,
        instance: (row.instance as string | undefined) ?? this.defaultInstance(),
      }));
      if (mine !== this.refreshes || space !== this.state.space) return;
      this.set({ sessions });
    } catch (e) {
      if (mine === this.refreshes) this.notice(explainError(e), "error");
    }
  }

  /* ── 动作 ─────────────────────────────────────────────────────────── */

  /** Open a session in the front space — or in `space`, which must be the one it belongs to. */
  open(key: SessionKey | null, space = this.state.space): void {
    const currentBySpace = { ...this.state.currentBySpace, [space]: key };
    remember(CURRENT_KEY, currentBySpace);
    this.set({ currentBySpace, notices: [] });
  }

  /**
   * Bring a space to the front, with its own current session. One that has gone since — deleted,
   * its instance stopped — falls back to the space's list, never to another space's session.
   */
  async switchSpace(space: string): Promise<void> {
    remember(SPACE_KEY, space);
    // Each space starts on its first tab, unless the host API chose one for it — or the space says
    // which tab is current (`tab()`): then the one clicked last is kept too, and `tab()` has its say.
    const tabBySpace = { ...this.state.tabBySpace };
    const declares = this.registry.at("space").find((c) => c.id === space)?.details?.tab !== undefined;
    if (space !== this.state.space && tabBySpace[space] && !tabBySpace[space]!.asked && !declares) delete tabBySpace[space];
    const changed = space !== this.state.space;
    this.set({ space, detail: null, menu: false, tabBySpace, ...(changed ? { search: { query: "" }, sessions: [] } : {}) });
    this.followSpaceTab();
    if (changed) void this.refreshSessions();
    const key = this.state.currentBySpace[space];
    if (!key) return;
    try {
      await this.conn.call("engine.session.get", { space: this.claim(space), instance: key.instance, session_id: key.sessionId });
    } catch {
      if (this.state.currentBySpace[space] === key) this.open(null, space);
    }
  }

  /** Forget a session wherever it was current: it was deleted. Its surface in the right column goes too. */
  forget(key: { instance: string; sessionId: string }): void {
    this.sidebar.drop(key);
    const currentBySpace = { ...this.state.currentBySpace };
    for (const [space, current] of Object.entries(currentBySpace)) {
      if (current && current.instance === key.instance && current.sessionId === key.sessionId) currentBySpace[space] = null;
    }
    remember(CURRENT_KEY, currentBySpace);
    const viewBySession = { ...this.state.viewBySession };
    delete viewBySession[keyOf(key.instance, key.sessionId)];
    remember(VIEW_KEY, viewBySession);
    this.set({ currentBySpace, viewBySession });
  }

  /** New sessions belong to the front space and land on its default instance unless one is named. */
  async create(params: { scene?: string; project_root?: string | null; options?: Record<string, unknown>; instance?: string }): Promise<SessionKey> {
    const space = this.state.space;
    // What the space starts its conversations with (the project open in it, say), under what was given.
    const contribution = this.registry.at("space").find((c) => c.id === space);
    const asked = contribution?.conversation?.newSessionDefaults;
    const defaults = asked ? ask(this.registry, contribution, () => asked(), undefined) : undefined;
    const given = Object.fromEntries(Object.entries(params).filter(([, v]) => v !== undefined));
    const created = (await this.conn.call("harnix.session.create", { ...(defaults ?? {}), ...given, space: this.claim(space) })) as { session_id: string; instance?: string };
    const key = { instance: created.instance ?? this.defaultInstance(), sessionId: created.session_id };
    await this.refreshSessions();
    this.open(key, space);
    return key;
  }

  /** A call about one session, claiming the front space. */
  async sessionCall<T = unknown>(method: string, key: { instance: string; session_id: string }, params: Record<string, unknown> = {}): Promise<T> {
    return this.call<T>(method, { ...params, ...key, space: this.claim() });
  }

  /** Whether the front space shows its right column: what it was left at, else its default. */
  detailsOpen(state: ShellState = this.state): boolean {
    const kept = state.detailsBySpace[state.space];
    if (kept !== undefined) return kept;
    const space = [...this.registry.at("space")].find((c) => c.id === state.space);
    return space?.details?.initial === "open";
  }

  private keepDetails(open: boolean): Record<string, boolean> {
    const detailsBySpace = { ...this.state.detailsBySpace, [this.state.space]: open };
    remember(DETAILS_KEY, detailsBySpace);
    return detailsBySpace;
  }

  showDetail(item: DetailItem): void {
    // A detail revealed from the flow is what the reader wants to see now.
    this.set({ detail: item, detailsBySpace: this.keepDetails(true) });
    this.selectTab("details");
  }

  /**
   * What the right column shows for the front space: a tab of the space's side (`shownTab()`), or
   * the open session's tab in front. The session's side is shown when it was brought in front last
   * (`stripSide`) and has a tab — or when the space's side has nothing at all to show.
   */
  stripShown(state: ShellState = this.state): { side: "space"; id: string } | { side: "session"; tab: SidebarTab } {
    const active = state.current ? this.sidebar.active(state.current) : null;
    if (active) {
      if (state.stripSide === "session") return { side: "session", tab: active };
      const space = this.registry.at("space").find((c) => c.id === state.space);
      if (!safeTabs(this.registry, space).length && !state.detail) return { side: "session", tab: active };
    }
    return { side: "space", id: this.shownTab(state) };
  }

  /** Bring a tab of the open session's surface to the front, and that side of the strip with it. */
  focusSessionTab(id: string): void {
    const current = this.state.current;
    if (!current) return;
    this.sidebar.select(current, id);
    if (this.state.stripSide !== "session") this.set({ stripSide: "session" });
  }

  /** The tab the right column shows for the front space: the one chosen while it is still there,
   *  else the first. "details" stands while there is a detail to show. */
  shownTab(state: ShellState = this.state): string {
    const space = this.registry.at("space").find((c) => c.id === state.space);
    const tabs = orderTabs(safeTabs(this.registry, space));
    const chosen = state.tabBySpace[state.space]?.id;
    if (chosen && tabs.some((x) => x.id === chosen)) return chosen;
    if (chosen === "details" && state.detail) return "details";
    return tabs[0]?.id ?? "details";
  }

  private readonly tabListeners = new Set<(tab: string) => void>();

  /** Select a tab of the front space's right column; `asked` when the host API asked for it. */
  selectTab(id: string, asked = false): void {
    const before = this.shownTab();
    this.set({ tabBySpace: { ...this.state.tabBySpace, [this.state.space]: { id, asked } }, stripSide: "space" });
    const after = this.shownTab();
    if (after !== before) for (const fn of [...this.tabListeners]) fn(after);
  }

  onTab(fn: (tab: string) => void): () => void {
    this.tabListeners.add(fn);
    return () => this.tabListeners.delete(fn);
  }

  toggleDetails(open?: boolean): void {
    const detailsOpen = open ?? !this.detailsOpen();
    this.set({ detailsBySpace: this.keepDetails(detailsOpen), detailsMax: detailsOpen && this.state.detailsMax });
  }

  /** Whether the front space's second side panel takes the whole centre. */
  secondaryMax(state: ShellState = this.state): boolean {
    return state.secondaryMaxBySpace[state.space] === true;
  }

  /** The front space's second side panel takes the whole centre, or goes back beside the
   *  conversation at the width it had. Remembered per space. A maximized right column gives way. */
  toggleSecondaryMax(max?: boolean): void {
    const next = max ?? !this.secondaryMax();
    const secondaryMaxBySpace = { ...this.state.secondaryMaxBySpace, [this.state.space]: next };
    if (!next) delete secondaryMaxBySpace[this.state.space];
    remember(SECONDARY_MAX_KEY, secondaryMaxBySpace);
    this.set({ secondaryMaxBySpace, detailsMax: next ? false : this.state.detailsMax });
  }

  toggleDetailsMax(max?: boolean): void {
    this.set({ detailsBySpace: this.keepDetails(true), detailsMax: max ?? !this.state.detailsMax });
  }

  /** A side column's width, kept within its bounds and remembered; the right column's widest is
   *  `detailsMax`. Null: back to the default. */
  setWidth(column: keyof Widths, width: number | null): void {
    const [min, , max] = WIDTHS[column];
    const widest = column === "details" ? detailsMax(this.state.widths.side) : max;
    const dragged = { ...this.state.dragged };
    if (width === null) delete dragged[column];
    else dragged[column] = Math.round(Math.min(widest, Math.max(min, width)));
    remember(WIDTHS_KEY, dragged);
    this.set({ dragged, widths: widthsWith(dragged) });
  }

  toggleMenu(open?: boolean): void {
    this.set({ menu: open ?? !this.state.menu });
  }

  /** The sidebar as drawn: in a wide window what the person left it at; narrow, the rail, or the
   *  drawer while it is open. Styles read only this (`#frame[data-sidebar]`). */
  sidebarShown(state: ShellState = this.state): "expanded" | "collapsed" {
    return state.narrow ? (state.drawer ? "expanded" : "collapsed") : state.sidebar;
  }

  /** Expand or collapse the sidebar: the wide choice in a wide window, the drawer in a narrow one.
   *  Each is the person's own and stays when the window crosses 960px and back. */
  toggleSidebar(): void {
    if (this.state.narrow) this.set({ drawer: !this.state.drawer });
    else this.set({ sidebar: this.state.sidebar === "expanded" ? "collapsed" : "expanded" });
  }

  /** The window crossed 960px. A drawer left open closes: the wide sidebar is back, or the rail is. */
  setNarrow(narrow: boolean): void {
    if (narrow !== this.state.narrow) this.set({ narrow, drawer: false });
  }

  closeDrawer(): void {
    if (this.state.drawer) this.set({ drawer: false });
  }

  /**
   * Say something in the banner. Notices queue: one at a time, each in front for a while (an error
   * longer) unless `sticky` — said of a state the page stays in until it changes, such as a backend
   * that has to be reloaded. The same line already waiting is not queued twice.
   */
  notice(text: string, tone: "info" | "error" = "info", options: { sticky?: boolean } = {}): void {
    const queued = this.state.notices;
    if (queued.some((n) => n.text === text && (n.tone ?? "info") === tone)) return;
    const notice: Notice = { text, tone };
    if (options.sticky) this.sticky.add(notice);
    // Over the cap, the oldest waiting goes; the one in front stays until its time is up.
    const waiting = [...queued.slice(1), notice].slice(-NOTICE_QUEUE);
    this.set({ notices: queued.length ? [queued[0]!, ...waiting] : [notice] });
  }

  /** The notice in front goes; the next, if any, takes its place. */
  dismissBanner(): void {
    this.set({ notices: this.state.notices.slice(1) });
  }

  private readonly sticky = new WeakSet<Notice>();
  private noticeTimer: ReturnType<typeof setTimeout> | null = null;

  /** The notice now in front goes by itself after its time, if it is still the one in front then. */
  private timeNotice(front: Notice | null): void {
    if (this.noticeTimer) clearTimeout(this.noticeTimer);
    this.noticeTimer = null;
    if (!front || this.sticky.has(front)) return;
    this.noticeTimer = setTimeout(() => {
      if (this.state.banner === front) this.dismissBanner();
    }, NOTICE_MS[front.tone ?? "info"]);
  }

  private modalIds = 0;

  /**
   * Open a dialog over what is on the page — over another dialog too: the shell's dialogs are a
   * stack, and Escape, the veil and `closeModal()` take the top one off. Its focus, the inert page
   * under it and the keys are the layers' (`@harnix/react`'s `layers.ts`).
   */
  openModal(modal: Modal): void {
    const modals = this.state.modals;
    const top = modals[modals.length - 1];
    const open: OpenModal = { ...modal, id: ++this.modalIds };
    this.set({ modals: modal.key && top?.key === modal.key ? [...modals.slice(0, -1), open] : [...modals, open] });
  }

  /** Close the top dialog, or the one given (its id, or what `snapshot.modal` was). */
  closeModal(which?: OpenModal | number): void {
    const modals = this.state.modals;
    if (which == null) {
      if (modals.length) this.set({ modals: modals.slice(0, -1) });
      return;
    }
    const id = typeof which === "number" ? which : which.id;
    if (modals.some((m) => m.id === id)) this.set({ modals: modals.filter((m) => m.id !== id) });
  }

  /** `host.shell.badge`: a whole number above 0 is the title's prefix; anything else clears it. */
  setTitleBadge(n: number | null): void {
    const badge = typeof n === "number" && Number.isFinite(n) && n > 0 ? Math.floor(n) : null;
    if (badge !== this.snapshot.titleBadge) this.set({ titleBadge: badge });
  }

  /** `host.shell.context`: the line under the brand; null or an empty label clears it. */
  setShellContext(c: ShellContext | null): void {
    this.set({ shellContext: c && typeof c.label === "string" && c.label !== "" ? { ...c } : null });
  }

  setTheme(choice: ThemeChoice): void {
    try {
      if (choice === "auto") localStorage.removeItem(THEME_KEY);
      else localStorage.setItem(THEME_KEY, choice);
    } catch {
      /* private mode: the choice does not survive a reload */
    }
    this.set({ theme: choice });
  }

  /** A same-origin path with this page's credential on it: for what the browser fetches itself
   *  and cannot be given a header, such as a package's module (`import()`). */
  credentialed(path: string): string {
    return this.conn.withToken(path);
  }

  /** 一次调用；失败时说出来并把错误交回去。 */
  async call<T = unknown>(method: string, params?: unknown): Promise<T> {
    return (await this.conn.call(method, params)) as T;
  }

  /** 上传一个文件：先要一张票，再把文件原样 POST 过去（`harnix.bulk.upload`）。 */
  async upload(file: File): Promise<string> {
    const limit = this.state.hello?.limits?.max_upload_bytes;
    if (limit && file.size > limit) {
      throw new Error(t("composer.uploadTooLarge", { name: file.name, limit: (limit / 1048576).toFixed(0) }));
    }
    const grant = await this.call<{ url: string; path: string }>("harnix.bulk.upload", { name: file.name, bytes: file.size });
    const response = await fetch(grant.url, { method: "POST", body: file });
    if (!response.ok) throw new Error(`${response.status} ${await response.text()}`);
    const answer = (await response.json().catch(() => ({}))) as { path?: string };
    return answer.path ?? grant.path;
  }
}
