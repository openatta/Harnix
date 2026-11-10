/**
 * What the Harnix app keeps about its sessions on this page: the session space's list joined with
 * the workspaces and the archive (`harnix-app.workspaces.list`), which of them are running, the
 * left column's name filter and transcript search, the groups opened past their length, the
 * project open in the projects space, and whether the archived ones show.
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

export interface SearchHit {
  session_id: string;
  name?: string | null;
  snippet?: string;
}

export interface AppState {
  sessions: AppSession[];
  workspaces: Workspace[];
  showArchived: boolean;
  /** Running sessions, `instance/session` → true (`harnix.sessions.status`). */
  running: Record<string, boolean>;
  /** The left column's box: its text, and the transcript search's answer once asked. */
  search: { query: string; hits: SearchHit[] | null | "pending"; truncated: boolean };
  /** Groups shown past their first rows, by key. */
  expanded: Record<string, boolean>;
  /** The project open in the projects space: one at a time. */
  project: string | null;
}

export const keyOf = (instance: string, sessionId: string) => `${instance}/${sessionId}`;

/**
 * A session list with each session once — a defence, not the fix. Agix's `engine.session.list` lists a
 * session it has let go of (past `session_cap`) once per project it has served, and counts a folder
 * reached under two spellings (macOS's `/var/…` and `/private/var/…`) as two projects, so such a
 * session comes back twice (openatta/Agix#23). Two rows with one key would draw two entries for one
 * session.
 */
export function uniqueSessions<T extends { instance?: unknown; session_id: string }>(rows: T[], fallback: string): T[] {
  const seen = new Set<string>();
  return rows.filter((row) => {
    const key = keyOf(typeof row.instance === "string" ? row.instance : fallback, row.session_id);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function storedProject(): string | null {
  try {
    const raw = localStorage.getItem(PROJECT_KEY);
    return raw == null ? null : (JSON.parse(raw) as string | null);
  } catch {
    return null;
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
    showArchived: false,
    running: {},
    search: { query: "", hits: null, truncated: false },
    expanded: {},
    project: storedProject(),
  };
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
      const sessions: AppSession[] = uniqueSessions(listed.sessions ?? [], fallback).map((row) => {
        const workspace = workspaces.find((w) => row.project_root && w.path === row.project_root);
        return {
          ...row,
          instance: typeof row.instance === "string" ? row.instance : fallback,
          archived: Boolean(notes[row.session_id]?.archived),
          workspace_id: workspace?.id ?? null,
        };
      });
      this.set({ sessions, workspaces, showArchived: Boolean(spaces.prefs?.showArchived ?? this.state.showArchived) });
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
