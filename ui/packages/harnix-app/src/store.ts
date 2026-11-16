/**
 * What the Harnix app keeps about its sessions on this page: the session space's list joined with
 * the workspaces and the archive (`harnix-app.workspaces.list`), which of them are running, the
 * left column's name filter and transcript search, the groups opened past their length, the
 * project open in the projects space, whether the archived ones show, and each project's trust
 * (`harnix-app.projects.status`, kept current by the `harnix-app.trust` topic).
 *
 * One immutable snapshot, `subscribe` / `getSnapshot`, which is also the spaces' `store`: when it
 * changes the shell draws them again. It follows the hub rather than being told: the instance's
 * session list is pushed whenever a session is made, renamed, closed or deleted, from this page's
 * menus or anyone else's, and the list is read again then.
 */

import { errorText, type SessionRow } from "@harnix/core";
import { t, type HostApi } from "@harnix/react";

/** The backend space the Harnix app declares: its sessions are the ones listed here. */
export const SESSIONS = "sessions";

const PROJECT_KEY = "harnix.project";
/** The projects whose "not trusted" banner was closed, on this page: closed once, it stays closed. */
const TRUST_DISMISSED_KEY = "harnix.trust.dismissed";

export interface Workspace {
  id: string;
  path: string;
  title?: string | null;
  collapsed?: boolean;
  session_order?: string[];
}

/** A row of `harnix.sessions.list`, with what this product notes about it. */
export interface AppSession extends SessionRow {
  instance: string;
  archived?: boolean;
  workspace_id?: string | null;
}

/** One key of a project's settings that waits on trust, as `harnix-app.projects.status` says it. */
export interface TrustItem {
  key: string;
  /** hooks · mcp · permissions · sandbox · models · telemetry · plugins · agents · skills · other */
  category: string;
  /** The one thing the key names (an MCP server, an agent type, a rule), when it names one. */
  name?: string | null;
}

/** A project's trust (`harnix-app.projects.status` / `.trust`, the `harnix-app.trust` topic). */
export interface TrustStatus {
  project_root: string | null;
  /** `none`: the directory has no project settings of its own. */
  trust: "trusted" | "untrusted" | "none";
  /** Who decided: `host` (trusted from here), `user` (an entry in the global settings), `default`. */
  trust_source?: string | null;
  /** The global settings' entry that trusts it, when one does. */
  matched?: string | null;
  /** Every key that waits on trust, whether or not it is withheld now. */
  items: TrustItem[];
  /** The kinds among `items`, in the order a page lists them. */
  categories: string[];
  /** What is not in force now: `items` while untrusted, nothing otherwise. */
  withheld: TrustItem[];
}

export interface SearchHit {
  session_id: string;
  name?: string | null;
  snippet?: string;
}

export interface AppState {
  sessions: AppSession[];
  workspaces: Workspace[];
  /** The sessions and workspaces have been read once: until then, none listed means not known yet. */
  loaded: boolean;
  showArchived: boolean;
  /** Running sessions, `instance/session` → true (`harnix.sessions.status`). */
  running: Record<string, boolean>;
  /** The left column's box: its text, and the transcript search's answer once asked. */
  search: { query: string; hits: SearchHit[] | null | "pending"; truncated: boolean };
  /** Groups shown past their first rows, by key. */
  expanded: Record<string, boolean>;
  /** The project open in the projects space: one at a time. */
  project: string | null;
  /** Each project's trust by its directory; null when it could not be read (nothing is offered). */
  trust: Record<string, TrustStatus | null>;
  /** The projects whose "not trusted" banner was closed, by directory. */
  trustDismissed: Record<string, true>;
}

export const keyOf = (instance: string, sessionId: string) => `${instance}/${sessionId}`;

function storedProject(): string | null {
  try {
    const raw = localStorage.getItem(PROJECT_KEY);
    return raw == null ? null : (JSON.parse(raw) as string | null);
  } catch {
    return null;
  }
}

function storedDismissed(): Record<string, true> {
  try {
    const raw = localStorage.getItem(TRUST_DISMISSED_KEY);
    const roots = raw == null ? [] : (JSON.parse(raw) as unknown);
    return Object.fromEntries((Array.isArray(roots) ? roots : []).filter((r) => typeof r === "string").map((r) => [r, true as const]));
  } catch {
    return {};
  }
}

function rememberProject(id: string | null): void {
  try {
    localStorage.setItem(PROJECT_KEY, JSON.stringify(id));
  } catch {
    /* private mode: it does not survive a reload */
  }
}

export class AppStore {
  private state: AppState = {
    sessions: [],
    workspaces: [],
    loaded: false,
    showArchived: false,
    running: {},
    search: { query: "", hits: null, truncated: false },
    expanded: {},
    project: storedProject(),
    trust: {},
    trustDismissed: storedDismissed(),
  };
  /** The projects whose trust was asked for on this page: each is read once, then followed. */
  private readonly trustAsked = new Set<string>();
  private readonly listeners = new Set<() => void>();
  private offs: (() => void)[] = [];
  /** A read in flight, and whether another was asked for meanwhile: pushes come in bursts. */
  private reading: Promise<void> | null = null;
  private again = false;

  constructor(private readonly host: HostApi) {}

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly getSnapshot = (): AppState => this.state;

  set(change: Partial<AppState>): void {
    this.state = { ...this.state, ...change };
    for (const fn of [...this.listeners]) fn();
  }

  /** Follow the hub: the instance's session list as it is pushed, which sessions are running, and
   *  each turn's end (a session's last activity moves). Reads the list once now. */
  start(): void {
    if (this.offs.length) return;
    const instance = this.host.defaults.instance;
    this.offs.push(
      this.host.subscribe("harnix.instances.sessions", { instance }, () => void this.refresh()),
      this.host.subscribe("harnix.sessions.status", {}, (value) => this.applyStatus(value)),
      this.host.subscribe("harnix-app.trust", {}, (value) => this.applyTrust(value)),
      this.host.events.onTurnSettled(() => void this.refresh()),
    );
    void this.refresh();
  }

  stop(): void {
    for (const off of this.offs.splice(0)) off();
  }

  private applyStatus(value: unknown): void {
    const rows = (value as { sessions?: { instance: string; session_id: string }[] } | null)?.sessions ?? [];
    const running: Record<string, boolean> = {};
    for (const row of rows) running[keyOf(row.instance, row.session_id)] = true;
    this.set({ running });
  }

  /** What another page (or this one) last read or changed of each project's trust. The topic says
   *  a project by its resolved directory; one this page asked about by another spelling (a link to
   *  it) is the entry whose status says that directory. */
  private applyTrust(value: unknown): void {
    const projects = (value as { projects?: Record<string, TrustStatus> } | null)?.projects;
    if (!projects || typeof projects !== "object") return;
    const trust = { ...this.state.trust, ...projects };
    for (const [asked, known] of Object.entries(this.state.trust)) {
      const resolved = known?.project_root;
      if (resolved && resolved !== asked && projects[resolved]) trust[asked] = projects[resolved]!;
    }
    this.set({ trust });
  }

  /** Read a project's trust once on this page; the topic keeps it current after that. */
  ensureTrust(root: string | null | undefined): void {
    if (!root || this.trustAsked.has(root)) return;
    this.trustAsked.add(root);
    void this.loadTrust(root).catch(() => undefined);
  }

  /** Read a project's trust now. One that cannot be read (the backend was not granted it, the
   *  directory is gone) is kept as null: nothing about trust is offered for it. */
  async loadTrust(root: string): Promise<TrustStatus | null> {
    try {
      const status = await this.host.call<TrustStatus>("harnix-app.projects.status", { project_root: root });
      this.set({ trust: { ...this.state.trust, [root]: status } });
      return status;
    } catch (e) {
      this.set({ trust: { ...this.state.trust, [root]: null } });
      throw e;
    }
  }

  /** Trust a project, or no longer; the status as written. */
  async setTrust(root: string, trusted: boolean): Promise<TrustStatus> {
    const status = await this.host.call<TrustStatus>("harnix-app.projects.trust", { project_root: root, trusted });
    this.set({ trust: { ...this.state.trust, [root]: status } });
    return status;
  }

  /** Close a project's "not trusted" banner for good, on this page. */
  dismissTrust(root: string): void {
    const trustDismissed = { ...this.state.trustDismissed, [root]: true as const };
    this.set({ trustDismissed });
    try {
      localStorage.setItem(TRUST_DISMISSED_KEY, JSON.stringify(Object.keys(trustDismissed)));
    } catch {
      /* private mode: it does not survive a reload */
    }
  }

  /** Read the list again; one read at a time, and one more after it when asked meanwhile. */
  refresh(): Promise<void> {
    if (this.reading) {
      this.again = true;
      return this.reading;
    }
    this.reading = this.read().finally(() => {
      this.reading = null;
      if (this.again) {
        this.again = false;
        void this.refresh();
      }
    });
    return this.reading;
  }

  private async read(): Promise<void> {
    const fallback = this.host.defaults.instance;
    try {
      const [listed, spaces] = await Promise.all([
        this.host.call<{ sessions: SessionRow[] }>("harnix.sessions.list", { space: SESSIONS }),
        this.host
          .call<{ workspaces: Workspace[]; prefs?: Record<string, unknown>; sessions?: Record<string, { archived?: boolean }> }>("harnix-app.workspaces.list", {})
          .catch(() => ({ workspaces: [], prefs: {}, sessions: {} }) as { workspaces: Workspace[]; prefs?: Record<string, unknown>; sessions?: Record<string, { archived?: boolean }> }),
      ]);
      const notes = spaces.sessions ?? {};
      const workspaces = spaces.workspaces ?? [];
      // Which workspace a session belongs to is its project root matched against a workspace's
      // path: the backend knows the paths, the hub the roots, and the join is made here.
      const sessions: AppSession[] = (listed.sessions ?? []).map((row) => {
        const workspace = workspaces.find((w) => row.project_root && w.path === row.project_root);
        return {
          ...row,
          instance: typeof row.instance === "string" ? row.instance : fallback,
          archived: Boolean(notes[row.session_id]?.archived),
          workspace_id: workspace?.id ?? null,
        };
      });
      this.set({ sessions, workspaces, loaded: true, showArchived: Boolean(spaces.prefs?.showArchived ?? this.state.showArchived) });
    } catch (e) {
      this.host.shell.notice(errorText(e), "error");
    }
  }

  /** A session as listed here, if it is. */
  row(instance: string, sessionId: string): AppSession | undefined {
    return this.state.sessions.find((s) => s.session_id === sessionId && s.instance === instance);
  }

  /** The left column's box: typing filters by name; Enter searches the transcripts; "" clears. */
  search(query: string, submit = false): void {
    if (!query) {
      this.set({ search: { query: "", hits: null, truncated: false } });
      return;
    }
    if (!submit || !query.trim()) {
      this.set({ search: { query, hits: null, truncated: false } });
      return;
    }
    void this.searchTranscripts(query);
  }

  private async searchTranscripts(query: string): Promise<void> {
    this.set({ search: { query, hits: "pending", truncated: false } });
    try {
      const result = await this.host.call<{ hits?: SearchHit[]; truncated?: boolean }>("harnix-app.search", { query: query.trim() });
      if (this.state.search.query !== query) return;
      this.set({ search: { query, hits: result.hits ?? [], truncated: Boolean(result.truncated) } });
    } catch (e) {
      this.set({ search: { ...this.state.search, hits: [] } });
      this.host.shell.notice(t("banner.searchFailed", { error: errorText(e) }), "error");
    }
  }

  toggleExpanded(key: string): void {
    this.set({ expanded: { ...this.state.expanded, [key]: !this.state.expanded[key] } });
  }

  /** Show or hide the archived ones; kept as this product's preference. */
  toggleArchived(): void {
    const showArchived = !this.state.showArchived;
    this.set({ showArchived });
    this.host.call("harnix-app.prefs.set", { key: "showArchived", value: showArchived }).catch(() => undefined);
  }

  /** Open one project in the projects space; the one open before closes. */
  openProject(id: string | null): void {
    rememberProject(id);
    this.set({ project: id });
  }

  /** The open project's directory: where a conversation started in the projects space runs. */
  projectRoot(): string | null {
    const id = this.state.project;
    return id ? (this.state.workspaces.find((w) => w.id === id)?.path ?? null) : null;
  }
}
