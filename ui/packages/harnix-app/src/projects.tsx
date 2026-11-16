/**
 * The projects space's left column: the registered projects as a tree, one open at a time, its
 * conversations under it on a guide line. Up to five projects show, then "more"; up to eight
 * rows of the open one, then "all".
 *
 * A project is a workspace (`harnix-app.workspaces.*`): a directory sessions run in. Its conversations
 * are the session space's — the same sessions, listed by where they run — so opening one here
 * claims the session space, and a new one started here runs in the open project
 * (`newSessionDefaults`). Under a project, between its conversations, are the items other
 * packages register for it (`harnix-app.project.items`: the flows' tasks), ordered with them by
 * activity; the project's "+" offers a conversation and whatever those kinds make.
 *
 * A trusted project carries a shield. A conversation open in a project that is not trusted, and
 * whose own settings are therefore held back, has a banner over it offering to trust it — until it
 * is trusted, or the banner is closed for that project.
 */

import { useEffect, useRef, type MouseEvent } from "react";
import { Contributed, Glyph, t, useCurrentSession, useRoving, useT, type HostApi, type HostSessionRef, type Registry } from "@harnix/react";

import { addWorkspaceDialog, newSessionDialog, trustCategories, trustDialog, workspaceMenu } from "./dialogs.tsx";
import { PROJECT_ITEMS, activityOf, createEntries, useProjectItems, type ProjectItem, type ProjectItemContext } from "./items.ts";
import { SessionRow, useApp } from "./sessions.tsx";
import { SESSIONS as SESSION_SPACE, keyOf, type AppSession, type AppState, type AppStore, type Workspace } from "./store.ts";

const PROJECTS = 5;
const SESSIONS = 8;

/** A tree item's parent: a project's conversation hangs under the project. */
const parentOf = (item: HTMLElement) => (item.matches(".srow") ? item.closest(".project")?.querySelector<HTMLElement>(".prj") ?? null : null);
/** A project opens and closes as a click on it does. */
const toggle = (item: HTMLElement) => item.click();

/** Where a menu opened by a pointer goes: the pointer, else under the element (a key's click). */
const anchorOf = (event: MouseEvent<HTMLElement>): Element | { x: number; y: number } => (event.clientX || event.clientY ? { x: event.clientX, y: event.clientY } : event.currentTarget);

/** The project's "+" menu: a conversation, then whatever the registered item kinds make (a task). */
function newInProject(host: HostApi, r: Registry, store: AppStore, workspace: Workspace, anchor: Element | { x: number; y: number }): void {
  void host.dialogs.menu(anchor, [
    { id: "session", icon: "chat", label: t("projects.newSession"), run: () => newSessionDialog(host, store, workspace.path) },
    ...createEntries(r, { root: workspace.path, workspace, space: SESSION_SPACE }),
  ]);
}

export function ProjectsSidebar({ host, r, store }: { host: HostApi; r: Registry; store: AppStore }) {
  const t = useT();
  const s = useApp(store);
  // Read again whenever the space comes to the front: nothing pushes a change to the workspaces, so
  // a project made elsewhere (another client, a direct call) shows only after a read.
  useEffect(() => void store.refresh(), [store]);
  // Each project's trust, read once on this page: the shield, and what its menu offers.
  useEffect(() => {
    for (const workspace of s.workspaces) store.ensureTrust(workspace.path);
  }, [store, s.workspaces]);
  const all = Boolean(s.expanded["projects:more"]);
  const shown = all ? s.workspaces : s.workspaces.slice(0, PROJECTS);
  // A tree, one Tab stop: Up and Down go along the projects and the open one's conversations, Right
  // and Left open and close a project.
  const tree = useRef<HTMLDivElement>(null);
  const roving = useRoving(tree, { items: ".prj, .pgroup .srow", orientation: "vertical", parentOf, toggle });
  return (
    <div className="projects" data-testid="projects">
      <div className="sec">
        <span className="truncate">{t("space.projects")}</span>
        <button type="button" className="ib sm add" title={t("app.addProject")} data-testid="add-project" onClick={() => void addWorkspaceDialog(host, store)}>
          <Glyph name="plus" className="glyph" />
        </button>
      </div>
      {!s.workspaces.length ? <div className="empty">{t("projects.none")}</div> : null}
      <div className="ptree" role="tree" aria-label={t("space.projects")} ref={tree} onKeyDown={roving.onKeyDown} onFocus={roving.onFocus}>
        {shown.map((workspace) => (
          <Project key={workspace.id} host={host} r={r} store={store} s={s} workspace={workspace} />
        ))}
      </div>
      {s.workspaces.length > PROJECTS ? (
        <button type="button" className="more-rows" onClick={() => store.toggleExpanded("projects:more")}>
          {all ? t("sidebar.showLess") : t("projects.more", { count: s.workspaces.length - PROJECTS })}
        </button>
      ) : null}
    </div>
  );
}

/** A row of the open project: one of its conversations, or an item another package registered. */
type Entry = { kind: "session"; at: number | null; session: AppSession } | { kind: "item"; at: number | null; item: ProjectItem };

/** The project's conversations and items together, newest activity first; what says no time last. */
export function interleave(sessions: AppSession[], items: ProjectItem[]): Entry[] {
  const entries: Entry[] = [
    ...sessions.map((session) => ({ kind: "session" as const, at: activityOf(session.last_active), session })),
    ...items.map((item) => ({ kind: "item" as const, at: activityOf(item.row.at), item })),
  ];
  return entries.sort((a, b) => (b.at ?? -1) - (a.at ?? -1));
}

function Project({ host, r, store, s, workspace }: { host: HostApi; r: Registry; store: AppStore; s: AppState; workspace: Workspace }) {
  const t = useT();
  const open = s.project === workspace.id;
  const sessions = s.sessions.filter((row) => row.workspace_id === workspace.id && (!row.archived || s.showArchived));
  // The other kinds of item the project holds (a task): read only while it is open, where they show.
  const items = useProjectItems(r, open ? workspace.path : null);
  const entries = interleave(sessions, items);
  const all = Boolean(s.expanded[`project:${workspace.id}`]);
  const rows = all ? entries : entries.slice(0, SESSIONS);
  const running = sessions.some((row) => s.running[keyOf(row.instance, row.session_id)]) || items.some((item) => item.row.running);
  const trusted = s.trust[workspace.path]?.trust === "trusted";
  const context: ProjectItemContext = { root: workspace.path, workspace, space: SESSION_SPACE };
  const more = (anchor: Element | { x: number; y: number }) => workspaceMenu(host, store, workspace, anchor, createEntries(r, context));
  return (
    <div className="project" role="none" data-project={workspace.id} data-open={open}>
      <div className="prj-row" role="none">
        <button
          type="button"
          className={`prj${open ? " on" : ""}`}
          role="treeitem"
          aria-level={1}
          aria-selected={open}
          title={workspace.path}
          aria-expanded={open}
          onClick={() => store.openProject(open ? null : workspace.id)}
          onContextMenu={(e) => {
            e.preventDefault();
            more(anchorOf(e));
          }}
        >
          <Glyph name="chevronDown" className={`glyph chv${open ? " open" : ""}`} />
          <Glyph name="folder" className="glyph" />
          <span className="nm">{workspace.title ?? basename(workspace.path)}</span>
          {trusted ? (
            <span className="trusted" data-testid="project-trusted" title={t("projects.trust.badge")} aria-label={t("projects.trust.badge")}>
              <Glyph name="shield" className="glyph" />
            </span>
          ) : null}
          {running && !open ? <span className="live" /> : null}
          <span className="c">{sessions.length + (open ? items.length : 0)}</span>
        </button>
        <button
          type="button"
          className="more"
          data-testid="project-new"
          title={t("projects.new")}
          aria-label={t("projects.new")}
          aria-haspopup="menu"
          onClick={(e) => newInProject(host, r, store, workspace, e.currentTarget)}
        >
          <Glyph name="plus" className="glyph" />
        </button>
        <button type="button" className="more" data-testid="project-more" title={t("sidebar.projectActions")} aria-label={t("sidebar.projectActions")} aria-haspopup="menu" onClick={(e) => more(e.currentTarget)}>
          <Glyph name="more" className="glyph" />
        </button>
      </div>
      {open ? (
        <div className="pgroup sel" role="group">
          {rows.map((entry) =>
            entry.kind === "session" ? (
              <SessionRow key={`s:${entry.session.session_id}`} host={host} store={store} session={entry.session} nested />
            ) : (
              <ItemRow key={`i:${entry.item.kind.id}:${entry.item.row.id}`} host={host} r={r} item={entry.item} context={context} />
            ),
          )}
          {!entries.length ? <div className="empty nested">{t("projects.noSessions")}</div> : null}
          {entries.length > SESSIONS ? (
            <button type="button" className="more-rows" onClick={() => store.toggleExpanded(`project:${workspace.id}`)}>
              {all ? t("sidebar.showLess") : t("projects.all", { count: entries.length })}
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/**
 * An item another package registered, as a row of the tree: the package draws its body and opens
 * it; the tree gives it the row, its place among the conversations, and its menu button. Marked
 * current while the session it names is the open one.
 */
function ItemRow({ host, r, item, context }: { host: HostApi; r: Registry; item: ProjectItem; context: ProjectItemContext }) {
  const t = useT();
  const current = useCurrentSession();
  const { kind, row } = item;
  const on = Boolean(row.session && current && current.sessionId === row.session.sessionId && current.instance === row.session.instance);
  const menu = (anchor: Element | { x: number; y: number }) => {
    let items;
    try {
      items = kind.menu?.(row, context) ?? [];
    } catch (e) {
      r.fail(PROJECT_ITEMS, kind, e);
      return;
    }
    if (items.length) void host.dialogs.menu(anchor, items);
  };
  return (
    <div className={`srow-wrap${on ? " on" : ""} nested`} role="none">
      <button
        type="button"
        className={`srow nested item${on ? " on" : ""}`}
        role="treeitem"
        aria-level={2}
        aria-selected={on}
        data-testid="project-item"
        data-kind={kind.id}
        data-item={row.id}
        data-state={on ? "current" : undefined}
        title={row.title}
        onClick={() => void Promise.resolve(kind.open(row, context)).catch((e: unknown) => r.fail(PROJECT_ITEMS, kind, e))}
        onContextMenu={(e) => {
          if (!kind.menu) return;
          e.preventDefault();
          menu(anchorOf(e));
        }}
      >
        <Contributed registry={r} point={PROJECT_ITEMS} contribution={kind}>
          {kind.render(row, context)}
        </Contributed>
      </button>
      {kind.menu ? (
        <button type="button" className="more" data-testid="project-item-more" title={t("sidebar.sessionActions")} aria-label={t("sidebar.sessionActions")} aria-haspopup="menu" onClick={(e) => menu(e.currentTarget)}>
          <Glyph name="more" className="glyph" />
        </button>
      ) : null}
    </div>
  );
}

/**
 * Over a conversation in a project that is not trusted and has settings held back for it: what is
 * held back, by kind, and an offer to trust it. Closed, it stays closed for that project.
 */
export function TrustBanner({ host, store, session }: { host: HostApi; store: AppStore; session: HostSessionRef }) {
  const t = useT();
  const s = useApp(store);
  const listed = store.row(session.instance, session.sessionId)?.project_root;
  const context = host.session.context()?.session?.project_root;
  const root = listed ?? (typeof context === "string" ? context : null);
  useEffect(() => store.ensureTrust(root), [store, root]);
  const status = root ? s.trust[root] : null;
  if (!root || !status || status.trust !== "untrusted" || !status.withheld.length || s.trustDismissed[root]) return null;
  const name = s.workspaces.find((w) => w.path === root)?.title ?? basename(root);
  return (
    <div className="trust-banner" role="status" data-testid="trust-banner">
      <Glyph name="shield" className="glyph" />
      <span className="text">{t("projects.trust.banner", { categories: trustCategories(status) })}</span>
      <button type="button" className="btn sm primary" data-testid="trust-banner-trust" onClick={() => void trustDialog(host, store, root, name)}>
        {t("projects.trust.bannerAction")}
      </button>
      <button
        type="button"
        className="ib sm"
        data-testid="trust-banner-dismiss"
        title={t("projects.trust.bannerDismiss")}
        aria-label={t("projects.trust.bannerDismiss")}
        onClick={() => store.dismissTrust(root)}
      >
        <Glyph name="close" className="glyph" />
      </button>
    </div>
  );
}

function basename(path: string): string {
  return path.replace(/\/+$/, "").split("/").pop() || path;
}
