/**
 * 外壳的状态与动作：连接、hello、会话列表与工作区、当前会话、右栏、横幅、模态框、搜索、主题。
 *
 * 一份不可变快照，`subscribe` / `getSnapshot` 对上 `useSyncExternalStore`。会话本身的状态不在这里：
 * 它在 `@harnix/core/state` 的控制器里，`ChatPanel` 打开会话时建。这里只记"开的是哪个"。
 *
 * 会话的全名是 `(instance, session)`，而且属于一个 space（设计 §8.5）：没有全局的「当前会话」，记的是
 * 每个 space 各自的当前会话；前台 space 换了，中区、输入框、右栏就换成那个 space 自己的那一项，切回来
 * 恢复它记住的。外壳发出的每个会话调用都带上前台 space，中枢对着账本核对。
 */

import type { ReactNode } from "react";
import { errorText, type ConnectionStatus, type HarnixClient, type Hello, type SessionRow } from "@harnix/core";
import { SYSTEM_INSTANCE, type SessionController } from "@harnix/core/state";
import { t, tabsOf, type ComposerApi, type DetailItem, type Registry, type SpaceContribution, type SpaceTab } from "@harnix/react";

import { topicsOf } from "./topics.ts";

export interface SessionKey {
  instance: string;
  sessionId: string;
}

export interface Workspace {
  id: string;
  path: string;
  title?: string | null;
  collapsed?: boolean;
  session_order?: string[];
}

/** `harnix.sessions` 的一行，加上工作区的覆盖（改名、归档、属于哪个工作区）。 */
export interface ShellSession extends SessionRow {
  instance: string;
  renamed?: boolean;
  archived?: boolean;
  workspace_id?: string | null;
}

export interface SearchHit {
  session_id: string;
  name?: string | null;
  snippet?: string;
}

/** 一个模态框：标题，和画 `.body` 与 `.foot` 的内容（见 `dialogs.tsx` 的 `Body` / `Foot`）。 */
export interface Modal {
  title: string;
  content: ReactNode;
  wide?: boolean;
}

export type ThemeChoice = "system" | "light" | "dark";

/** One configured model provider, as `harnix.settings.describe` reads it (the key redacted). */
export interface Provider {
  api_type?: string | null;
  base_url?: string | null;
  api_key?: string | null;
  default_model?: string | null;
  models?: string[];
}

/** The engine's providers: which there are, and which one a turn goes to. */
export interface Providers {
  providers: Record<string, Provider>;
  default_provider?: string | null;
}

/** The two side columns' widths, as the reader dragged them. */
export interface Widths {
  side: number;
  details: number;
}

export const WIDTHS: { side: [number, number, number]; details: [number, number, number] } = {
  // min, default, max
  side: [200, 260, 360],
  details: [320, 360, 480],
};

export interface ShellState {
  /** The space in front. */
  space: string;
  /** Each space's own current session. */
  currentBySpace: Record<string, SessionKey | null>;
  connection: ConnectionStatus;
  hello: Hello | null;
  reachable: string[];
  sessions: ShellSession[];
  workspaces: Workspace[];
  /** 正在跑的会话，`instance/session` → true（`harnix.sessions.status` 主题） */
  running: Record<string, boolean>;
  /** The front space's current session: `currentBySpace[space]`, kept in step by `set`. */
  current: SessionKey | null;
  detail: DetailItem | null;
  /** The right column, open or not, per space: each space keeps its own (unset → its default). */
  detailsBySpace: Record<string, boolean>;
  /** The right column takes the centre's place too (it closes back to its own width). */
  detailsMax: boolean;
  sidebar: "expanded" | "collapsed";
  banner: { text: string; tone?: "info" | "error" } | null;
  modal: Modal | null;
  search: { active: boolean; query: string; hits: SearchHit[] | null | "pending"; truncated: boolean };
  showArchived: boolean;
  expanded: Record<string, boolean>;
  theme: ThemeChoice;
  /** The project open in the projects space: one at a time. */
  project: string | null;
  widths: Widths;
  /** The avatar's menu. */
  menu: boolean;
  /** The model providers; null until read (or when this page may not read them). */
  providers: Providers | null;
  /** The right column's selected tab, per space. `asked`: chosen through the host API, so it is kept
   *  when the space is entered again; a tab the reader clicked goes back to the first, as before. */
  tabBySpace: Record<string, { id: string; asked: boolean }>;
  /** Bumped by `redraw()`: the shell reads the front space's functions again. */
  drawn: number;
}

const THEME_KEY = "harnix.theme";
const SPACE_KEY = "harnix.space";
const CURRENT_KEY = "harnix.space.current";
const PROJECT_KEY = "harnix.project";
const WIDTHS_KEY = "harnix.widths";
const DETAILS_KEY = "harnix.details";

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

function sameKey(a: SessionKey | null, b: SessionKey | null): boolean {
  return a === b || (a != null && b != null && a.instance === b.instance && a.sessionId === b.sessionId);
}

/** The tabs as the strip draws them: the `end` ones after the others. */
export function orderTabs<T extends { align?: "start" | "end" }>(tabs: T[]): T[] {
  return [...tabs.filter((x) => x.align !== "end"), ...tabs.filter((x) => x.align === "end")];
}

/** A space's tabs; a `tabs()` that throws gives none, and is counted as failed. */
export function safeTabs(registry: Registry, space: (SpaceContribution & { owner?: string }) | undefined): SpaceTab[] {
  try {
    return tabsOf(space);
  } catch (e) {
    if (space) later(() => registry.fail("space", { id: space.id, owner: space.owner ?? "?" }, e));
    return [];
  }
}

/** Report after the current draw: a registry change made while drawing would draw again at once. */
export function later(fn: () => void): void {
  queueMicrotask(fn);
}

function storedTheme(): ThemeChoice {
  try {
    const value = localStorage.getItem(THEME_KEY);
    return value === "light" || value === "dark" ? value : "system";
  } catch {
    return "system";
  }
}

export class App {
  readonly client: HarnixClient;
  readonly registry: Registry;
  private state: ShellState;
  private readonly listeners = new Set<() => void>();
  private booted = false;
  /** The open session's controller and composer, for the host API. Not state: nothing redraws on them. */
  controller: SessionController | null = null;
  composer: ComposerApi | null = null;
  private builtins: ((app: App) => void) | null = null;
  private packages: ((app: App, hello: Hello) => Promise<void>) | null = null;

  constructor(client: HarnixClient, registry: Registry) {
    this.client = client;
    this.registry = registry;
    const space = stored<string>(SPACE_KEY, "sessions");
    const currentBySpace = stored<Record<string, SessionKey | null>>(CURRENT_KEY, {});
    this.state = {
      space,
      currentBySpace,
      connection: client.status,
      hello: null,
      reachable: [],
      sessions: [],
      workspaces: [],
      running: {},
      current: currentBySpace[space] ?? null,
      detail: null,
      detailsBySpace: stored<Record<string, boolean>>(DETAILS_KEY, {}),
      detailsMax: false,
      sidebar: "expanded",
      banner: null,
      modal: null,
      search: { active: false, query: "", hits: null, truncated: false },
      showArchived: false,
      expanded: {},
      theme: storedTheme(),
      project: stored<string | null>(PROJECT_KEY, null),
      widths: { side: WIDTHS.side[1], details: WIDTHS.details[1], ...stored<Partial<Widths>>(WIDTHS_KEY, {}) },
      menu: false,
      providers: null,
      tabBySpace: {},
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
    this.state = next;
    for (const fn of [...this.listeners]) fn();
    if (!sameKey(before.current, next.current)) {
      const ref = next.current ? { instance: next.current.instance, sessionId: next.current.sessionId } : null;
      for (const fn of [...this.currentListeners]) fn(ref);
    }
  }

  private readonly currentListeners = new Set<(current: SessionKey | null) => void>();

  /** The front space's current session changed — opened, created, closed, deleted, or another space in front. */
  onCurrent(fn: (current: SessionKey | null) => void): () => void {
    this.currentListeners.add(fn);
    return () => this.currentListeners.delete(fn);
  }

  /* ── drawing ──────────────────────────────────────────────────────── */

  private drawWaiters: (() => void)[] = [];

  /** Read the front space's functions again (tabs, title, badge…): a redraw, the registry untouched. */
  redraw(): void {
    this.set({ drawn: this.state.drawn + 1 });
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

  /** 连上就跑一遍：hello、可调方法、内建贡献（一页一次）、会话列表。重连后再跑一遍。 */
  start(registerBuiltins: (app: App) => void, loadPackages?: (app: App, hello: Hello) => Promise<void>): void {
    this.builtins = registerBuiltins;
    this.packages = loadPackages ?? null;
    this.client.onStatus((status, detail) => {
      this.set({ connection: status });
      if (status === "open") void this.boot();
      if (status === "incompatible") this.notice(t("banner.incompatible", { reason: detail ?? "" }), "error");
    });
    this.client.onNotify((method, params) => this.handle(method, params));
    this.client.connect();
  }

  private async boot(): Promise<void> {
    try {
      const hello = (await this.client.call("harnix.hello", {})) as Hello;
      this.set({ hello });
      if (!this.booted) {
        this.booted = true;
        const reachable = (await this.client.call("harnix.reachable", {}).catch(() => ({ methods: [] }))) as {
          methods?: string[];
        };
        this.set({ reachable: reachable.methods ?? [] });
        void this.refreshProviders();
        // Through the page's counted table: a space subscribing to the same topic and leaving again
        // does not take it from the shell.
        topicsOf(this.client).subscribe("harnix.sessions.status", {}, (value) => this.applyStatus(value));
        this.builtins?.(this);
        this.landOnASpace();
        await this.packages?.(this, hello);
      }
      await this.refreshSessions();
    } catch (e) {
      this.notice(errorText(e), "error");
    }
  }

  /** Read the model providers again: at connect, and after the settings page changes one. */
  async refreshProviders(): Promise<void> {
    if (!this.state.reachable.includes("harnix.settings.describe")) return;
    const scene = this.state.hello?.engine?.active_scenes?.[0];
    const described = await this.client
      .call("harnix.settings.describe", { scene, project_root: null })
      .catch(() => null);
    const providers = (described as { providers?: Partial<Providers> } | null)?.providers;
    if (providers) this.set({ providers: { providers: providers.providers ?? {}, default_provider: providers.default_provider ?? null } });
  }

  /**
   * Whether a turn has a model to go to: a provider, or the `ANTHROPIC_*` key the engine started
   * with. Unknown (providers not read) counts as yes, so the page does not cry wolf.
   */
  modelReady(): boolean {
    const { providers, hello } = this.state;
    if (!providers) return true;
    return Object.keys(providers.providers).length > 0 || hello?.engine?.has_credentials === true;
  }

  /** The model a new session runs on: the default provider's, else the engine's. */
  defaultModel(): string | null {
    const { providers, hello } = this.state;
    const chosen = providers?.default_provider ? providers.providers[providers.default_provider] : undefined;
    return chosen?.default_model || (typeof hello?.engine?.model === "string" ? hello.engine.model : null);
  }

  /** The stored front space, if this product still has it; else the first one it does have. */
  private landOnASpace(): void {
    const ids = this.registry.at("space").map((c) => c.id);
    if (ids.includes(this.state.space) || !ids.length) return;
    const space = [...this.registry.at("space")].sort((a, b) => (a.order ?? 100) - (b.order ?? 100))[0]!.id;
    this.set({ space });
  }

  /** The backend space a front space's sessions belong to (`SpaceContribution.sessions`). */
  claim(space = this.state.space): string {
    return this.registry.at("space").find((c) => c.id === space)?.sessions ?? space;
  }

  /* ── 服务端通知 ───────────────────────────────────────────────────── */

  private handle(method: string, params: Record<string, unknown>): void {
    if (method === "harnix.event" && (params.event as { kind?: string } | undefined)?.kind === "skills_changed") {
      // The command list is the engine's; a skill installed mid-session adds one.
      void this.client
        .call("commands.list", {})
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

  async refreshSessions(): Promise<void> {
    try {
      const [listed, spaces] = await Promise.all([
        this.client.call("harnix.sessions", { space: "sessions" }) as Promise<{ sessions: SessionRow[] }>,
        this.client
          .call("harnix.workspaces.list", {})
          .catch(() => ({ workspaces: [], prefs: {}, sessions: {} })) as Promise<{
          workspaces: Workspace[];
          prefs?: Record<string, unknown>;
          sessions?: Record<string, { title?: string | null; archived?: boolean }>;
        }>,
      ]);
      const notes = spaces.sessions ?? {};
      const workspaces = spaces.workspaces ?? [];
      const sessions: ShellSession[] = (listed.sessions ?? []).map((row) => {
        const note = notes[row.session_id] ?? {};
        const workspace = workspaces.find((w) => row.project_root && w.path === row.project_root);
        return {
          ...row,
          instance: (row.instance as string | undefined) ?? SYSTEM_INSTANCE,
          name: note.title ?? row.name,
          renamed: Boolean(note.title),
          archived: Boolean(note.archived),
          workspace_id: workspace?.id ?? null,
        };
      });
      this.set({
        sessions,
        workspaces,
        showArchived: Boolean(spaces.prefs?.showArchived ?? this.state.showArchived),
      });
    } catch (e) {
      this.notice(errorText(e), "error");
    }
  }

  /* ── 动作 ─────────────────────────────────────────────────────────── */

  /** Open a session in the front space — or in `space`, which must be the one it belongs to. */
  open(key: SessionKey | null, space = this.state.space): void {
    const currentBySpace = { ...this.state.currentBySpace, [space]: key };
    remember(CURRENT_KEY, currentBySpace);
    this.set({ currentBySpace, banner: null });
  }

  /**
   * Bring a space to the front, with its own current session. One that has gone since — deleted,
   * its instance stopped — falls back to the space's list, never to another space's session.
   */
  async switchSpace(space: string): Promise<void> {
    remember(SPACE_KEY, space);
    // Each space starts on its first tab, unless the host API chose one for it.
    const tabBySpace = { ...this.state.tabBySpace };
    if (space !== this.state.space && tabBySpace[space] && !tabBySpace[space]!.asked) delete tabBySpace[space];
    this.set({ space, detail: null, menu: false, tabBySpace });
    const key = this.state.currentBySpace[space];
    if (!key) return;
    try {
      await this.client.call("session.get", { space: this.claim(space), instance: key.instance, session_id: key.sessionId });
    } catch {
      if (this.state.currentBySpace[space] === key) this.open(null, space);
    }
  }

  /** Forget a session wherever it was current: it was deleted. */
  forget(key: { instance: string; sessionId: string }): void {
    const currentBySpace = { ...this.state.currentBySpace };
    for (const [space, current] of Object.entries(currentBySpace)) {
      if (current && current.instance === key.instance && current.sessionId === key.sessionId) currentBySpace[space] = null;
    }
    remember(CURRENT_KEY, currentBySpace);
    this.set({ currentBySpace });
  }

  /** New sessions belong to the front space and land on its default instance unless one is named. */
  async create(params: { scene?: string; project_root?: string | null; options?: Record<string, unknown>; instance?: string }): Promise<SessionKey> {
    const space = this.state.space;
    // In the projects space a new conversation runs in the open project.
    if (space === "projects" && params.project_root === undefined && this.state.project) {
      const workspace = this.state.workspaces.find((w) => w.id === this.state.project);
      if (workspace) params = { ...params, project_root: workspace.path };
    }
    const created = (await this.client.call("session.create", { ...params, space: this.claim(space) })) as { session_id: string; instance?: string };
    const key = { instance: created.instance ?? SYSTEM_INSTANCE, sessionId: created.session_id };
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
    return space?.details === "open";
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
    this.set({ tabBySpace: { ...this.state.tabBySpace, [this.state.space]: { id, asked } } });
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

  toggleDetailsMax(max?: boolean): void {
    this.set({ detailsBySpace: this.keepDetails(true), detailsMax: max ?? !this.state.detailsMax });
  }

  /** Open one project in the projects space; the one open before closes. */
  openProject(id: string | null): void {
    remember(PROJECT_KEY, id);
    this.set({ project: id });
  }

  /** A side column's width, kept within its bounds and remembered. */
  setWidth(column: keyof Widths, width: number | null): void {
    const [min, fallback, max] = WIDTHS[column];
    const widths = { ...this.state.widths, [column]: width === null ? fallback : Math.round(Math.min(max, Math.max(min, width))) };
    remember(WIDTHS_KEY, widths);
    this.set({ widths });
  }

  toggleMenu(open?: boolean): void {
    this.set({ menu: open ?? !this.state.menu });
  }

  toggleSidebar(): void {
    this.set({ sidebar: this.state.sidebar === "expanded" ? "collapsed" : "expanded" });
  }

  notice(text: string, tone: "info" | "error" = "info"): void {
    this.set({ banner: { text, tone } });
  }

  dismissBanner(): void {
    this.set({ banner: null });
  }

  openModal(modal: Modal): void {
    this.set({ modal });
  }

  closeModal(): void {
    this.set({ modal: null });
  }

  setTheme(choice: ThemeChoice): void {
    try {
      if (choice === "system") localStorage.removeItem(THEME_KEY);
      else localStorage.setItem(THEME_KEY, choice);
    } catch {
      /* private mode: the choice does not survive a reload */
    }
    this.set({ theme: choice });
  }

  /** 一次调用；失败时说出来并把错误交回去。 */
  async call<T = unknown>(method: string, params?: unknown): Promise<T> {
    return (await this.client.call(method, params)) as T;
  }

  /** 上传一个文件：先要一张票，再把文件原样 POST 过去（`harnix.upload.begin`）。 */
  async upload(file: File): Promise<string> {
    const limit = this.state.hello?.limits?.max_upload_bytes;
    if (limit && file.size > limit) {
      throw new Error(t("composer.uploadTooLarge", { name: file.name, limit: (limit / 1048576).toFixed(0) }));
    }
    const grant = await this.call<{ url: string; path: string }>("harnix.upload.begin", { name: file.name, bytes: file.size });
    const response = await fetch(grant.url, { method: "POST", body: file });
    if (!response.ok) throw new Error(`${response.status} ${await response.text()}`);
    const answer = (await response.json().catch(() => ({}))) as { path?: string };
    return answer.path ?? grant.path;
  }
}
