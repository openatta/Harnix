/**
 * The home page: the home space, where the interface lands. No conversation of its own — a page of
 * blocks, the blocks of `harnix-app.home.panels` by `order`, each from the package that owns what it
 * shows: the process at a glance and the usage of the last days (the observability's), what waits on
 * a person and the recent sessions (this package's), the recent tasks (the flows'). This package
 * knows none of the others: it draws whatever is registered, and hands each block what only it
 * keeps — how many things wait on a person, and the way to the whole list.
 *
 * A block is a column of the page, or the page's width (`wide`): a row of figures, a table.
 */

import { useSyncExternalStore, type ReactNode } from "react";
import { Contributed, ago, t, useContributions, useT, type HostApi, type Registry } from "@harnix/react";

import { InboxList, inboxDialog, type Inbox } from "./inbox.tsx";
import { orderedPanels } from "./overview.tsx";
import { useApp } from "./sessions.tsx";
import { SESSIONS, keyOf, type AppSession, type AppStore } from "./store.ts";
import "./home.css";

/** The interface space, and the backend space it claims: declared by the `harnix-app` extension. */
export const HOME = "home";
/** The point this package defines: a block of the home page. */
export const HOME_PANELS = "harnix-app.home.panels";

/** How many rows a list of the home page shows. */
const ROWS_SHOWN = 8;

/** What a block is handed: what only the home page's owner keeps. */
export interface HomeContext {
  /** How many things wait on a person now, from every source of 待我处理 (`harnix-app.inbox`). */
  waiting: number;
  /** Open the whole list of what waits. */
  openWaiting(): void;
}

/** A block of the home page (`harnix-app.home.panels`, a `render` point). */
export interface HomePanel {
  id: string;
  title(): string;
  /** Smaller first; 100 when absent. */
  order?: number;
  /** Across the page's width rather than one column of it. */
  wide?: boolean;
  /** Read at every draw of the page: a block that keeps something follows it itself. */
  render(context: HomeContext): ReactNode;
}

/** "Good morning" by the clock. */
function greeting(): string {
  const hour = new Date().getHours();
  return t(hour < 12 ? "greet.morning" : hour < 18 ? "greet.afternoon" : "greet.evening");
}

/** The page: a greeting and the date, then the blocks in force. */
export function HomePage({ r, host, inbox }: { r: Registry; host: HostApi; inbox: Inbox }) {
  const tt = useT();
  const panels = orderedPanels(useContributions<HomePanel>(HOME_PANELS, r));
  // Every block is told the count, so the page is drawn again when it changes.
  const waiting = useSyncExternalStore(inbox.subscribe, inbox.getSnapshot).length;
  const context: HomeContext = { waiting, openWaiting: () => void inboxDialog(host, r, inbox) };
  return (
    <div className="hx-home" data-testid="home">
      <div className="hx-home-head">
        <h2>{greeting()}</h2>
        <span className="date">{new Date().toLocaleDateString(undefined, { weekday: "long", year: "numeric", month: "long", day: "numeric" })}</span>
      </div>
      {panels.length ? (
        <div className="hx-home-grid">
          {panels.map((panel) => (
            <section key={`${panel.owner}/${panel.id}`} className={`hx-home-block${panel.wide ? " wide" : ""}`} data-testid="home-block" data-block={panel.id}>
              <Contributed registry={r} point={HOME_PANELS} contribution={panel}>
                <BlockTitle title={() => panel.title()} />
                <PanelBody panel={panel} context={context} />
              </Contributed>
            </section>
          ))}
        </div>
      ) : (
        <div className="hx-home-none" data-testid="home-none">
          {tt("home.none")}
        </div>
      )}
    </div>
  );
}

/** A block's title and body are called inside its guard: one that throws costs that block, not the page. */
function BlockTitle({ title }: { title: () => string }) {
  return <h3>{title()}</h3>;
}

function PanelBody({ panel, context }: { panel: HomePanel; context: HomeContext }) {
  return <>{panel.render(context)}</>;
}

/* ── this package's blocks ────────────────────────────────────────────── */

/** Register this package's blocks of the home page; the others' go between them by order. */
export function registerHome(r: Registry, host: HostApi, store: AppStore, inbox: Inbox, owner: string): void {
  r.register(HOME_PANELS, { id: "harnix-app.inbox", order: 20, title: () => t("inbox.title"), render: () => <InboxList r={r} inbox={inbox} limit={ROWS_SHOWN} /> }, owner);
  r.register(HOME_PANELS, { id: "harnix-app.sessions", order: 30, title: () => t("home.sessions.title"), render: () => <RecentSessions host={host} store={store} /> }, owner);
}

/** The latest conversations of the session space, newest activity first; the archived left out. */
export function recentSessions(sessions: AppSession[], limit = ROWS_SHOWN): AppSession[] {
  return sessions
    .filter((s) => !s.archived)
    .sort((a, b) => String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")))
    .slice(0, limit);
}

function basename(path: string): string {
  return path.replace(/\/+$/, "").split("/").pop() || path;
}

/** The latest conversations, each opening in its space; the way to them all. */
function RecentSessions({ host, store }: { host: HostApi; store: AppStore }) {
  const tt = useT();
  const s = useApp(store);
  const rows = recentSessions(s.sessions);
  return (
    <div className="hx-home-rows" data-testid="home-sessions">
      {rows.length ? (
        rows.map((session) => (
          <button
            type="button"
            key={keyOf(session.instance, session.session_id)}
            className="hx-home-row"
            data-testid="home-session"
            data-sid={session.session_id}
            title={session.project_root ?? undefined}
            onClick={() => void host.session.open({ instance: session.instance, sessionId: session.session_id }, { switchSpace: true })}
          >
            {s.running[keyOf(session.instance, session.session_id)] ? <span className="live" /> : null}
            <span className="name">{session.name || tt("sidebar.untitled")}</span>
            {session.project_root ? <span className="tag">{basename(session.project_root)}</span> : null}
            <span className="meta">{ago(session.last_active, tt)}</span>
          </button>
        ))
      ) : (
        <div className="empty">{tt("home.sessions.none")}</div>
      )}
      <button type="button" className="btn sm ghost" data-testid="home-sessions-all" onClick={() => void host.spaces.switch(SESSIONS)}>
        {tt("home.sessions.all")}
      </button>
    </div>
  );
}
