/**
 * The projects space's left column: the registered projects as a tree, one open at a time, its
 * conversations under it on a guide line. Up to five projects show, then "more"; up to eight
 * conversations of the open one, then "all".
 *
 * A project is a workspace (`harnix-app.workspaces.*`): a directory sessions run in. Its conversations
 * are the session space's — the same sessions, listed by where they run — so opening one here
 * claims the session space, and a new one started here runs in the open project
 * (`newSessionDefaults`).
 *
 * A trusted project carries a shield. A conversation open in a project that is not trusted, and
 * whose own settings are therefore held back, has a banner over it offering to trust it — until it
 * is trusted, or the banner is closed for that project.
 */

import { useEffect, useRef } from "react";
import { Glyph, useRoving, useT, type HostApi, type HostSessionRef } from "@harnix/react";

import { addWorkspaceDialog, trustCategories, trustDialog, workspaceMenu } from "./dialogs.tsx";
import { SessionRow, useApp } from "./sessions.tsx";
import { keyOf, type AppState, type AppStore, type Workspace } from "./store.ts";

const PROJECTS = 5;
const SESSIONS = 8;

/** A tree item's parent: a project's conversation hangs under the project. */
const parentOf = (item: HTMLElement) => (item.matches(".srow") ? item.closest(".project")?.querySelector<HTMLElement>(".prj") ?? null : null);
/** A project opens and closes as a click on it does. */
const toggle = (item: HTMLElement) => item.click();

export function ProjectsSidebar({ host, store }: { host: HostApi; store: AppStore }) {
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
          <Project key={workspace.id} host={host} store={store} s={s} workspace={workspace} />
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

function Project({ host, store, s, workspace }: { host: HostApi; store: AppStore; s: AppState; workspace: Workspace }) {
  const t = useT();
  const open = s.project === workspace.id;
  const sessions = s.sessions
    .filter((row) => row.workspace_id === workspace.id && (!row.archived || s.showArchived))
    .sort((a, b) => String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")));
  const all = Boolean(s.expanded[`project:${workspace.id}`]);
  const rows = all ? sessions : sessions.slice(0, SESSIONS);
  const running = sessions.some((row) => s.running[keyOf(row.instance, row.session_id)]);
  const trusted = s.trust[workspace.path]?.trust === "trusted";
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
            workspaceMenu(host, store, workspace, e.clientX || e.clientY ? { x: e.clientX, y: e.clientY } : e.currentTarget);
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
          <span className="c">{sessions.length}</span>
        </button>
        <button
          type="button"
          className="more"
          data-testid="project-more"
          title={t("sidebar.projectActions")}
          aria-label={t("sidebar.projectActions")}
          aria-haspopup="menu"
          onClick={(e) => workspaceMenu(host, store, workspace, e.currentTarget)}
        >
          <Glyph name="more" className="glyph" />
        </button>
      </div>
      {open ? (
        <div className="pgroup sel" role="group">
          {rows.map((row) => (
            <SessionRow key={row.session_id} host={host} store={store} session={row} nested />
          ))}
          {!sessions.length ? <div className="empty nested">{t("projects.noSessions")}</div> : null}
          {sessions.length > SESSIONS ? (
            <button type="button" className="more-rows" onClick={() => store.toggleExpanded(`project:${workspace.id}`)}>
              {all ? t("sidebar.showLess") : t("projects.all", { count: sessions.length })}
            </button>
          ) : null}
        </div>
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
