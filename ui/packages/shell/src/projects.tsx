/**
 * The projects space's left column: the registered projects as a tree, one open at a time, its
 * conversations under it on a guide line. Up to five projects show, then "more"; up to eight
 * conversations of the open one, then "all".
 *
 * A project is a workspace (`harnix.workspaces.*`): a directory sessions run in. Its conversations
 * are the session space's — the same sessions, listed by where they run — so opening one here
 * claims the session space, and a new one started here runs in the open project.
 */

import { useT } from "@harnix/react";

import { type App, type ShellSession, type ShellState, type Workspace } from "./app.ts";
import { addWorkspaceDialog, workspaceMenu } from "./dialogs.tsx";
import { Glyph } from "./glyph.tsx";
import { useShell } from "./hooks.ts";
import { menuBelow, menuPoint } from "./Menu.tsx";
import { SessionRow } from "./sessions.tsx";

const PROJECTS = 5;
const SESSIONS = 8;

export function ProjectsSidebar({ app }: { app: App }) {
  const t = useT();
  const s = useShell(app);
  const all = Boolean(s.expanded["projects:more"]);
  const shown = all ? s.workspaces : s.workspaces.slice(0, PROJECTS);
  return (
    <div className="projects" data-testid="projects">
      <div className="sec">
        <span className="truncate">{t("space.projects")}</span>
        <button type="button" className="ib sm add" title={t("app.addProject")} data-testid="add-project" onClick={() => addWorkspaceDialog(app)}>
          <Glyph name="plus" className="glyph" />
        </button>
      </div>
      {!s.workspaces.length ? <div className="empty">{t("projects.none")}</div> : null}
      {shown.map((workspace) => (
        <Project key={workspace.id} app={app} s={s} workspace={workspace} />
      ))}
      {s.workspaces.length > PROJECTS ? (
        <button type="button" className="more-rows" onClick={() => app.set({ expanded: { ...s.expanded, "projects:more": !all } })}>
          {all ? t("sidebar.showLess") : t("projects.more", { count: s.workspaces.length - PROJECTS })}
        </button>
      ) : null}
    </div>
  );
}

function Project({ app, s, workspace }: { app: App; s: ShellState; workspace: Workspace }) {
  const t = useT();
  const open = s.project === workspace.id;
  const sessions = s.sessions
    .filter((row) => row.workspace_id === workspace.id && (!row.archived || s.showArchived))
    .sort((a, b) => String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")));
  const all = Boolean(s.expanded[`project:${workspace.id}`]);
  const rows = all ? sessions : sessions.slice(0, SESSIONS);
  const running = sessions.some((row) => s.running[`${row.instance ?? "system"}/${row.session_id}`]);
  const menu = (event: { clientX?: number; clientY?: number; currentTarget: EventTarget | null }) =>
    workspaceMenu(app, workspace, { origin: event.currentTarget instanceof HTMLElement ? event.currentTarget : null, at: menuPoint(event) });
  return (
    <div className="project" data-project={workspace.id} data-open={open}>
      <div className="prj-row">
        <button
          type="button"
          className={`prj${open ? " on" : ""}`}
          title={workspace.path}
          aria-expanded={open}
          onClick={() => {
            app.openProject(open ? null : workspace.id);
            // A project opened afresh starts on its own empty conversation.
            if (!open) app.open(null);
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            menu(e);
          }}
        >
          <Glyph name="chevronDown" className={`glyph chv${open ? " open" : ""}`} />
          <Glyph name="folder" className="glyph" />
          <span className="nm">{workspace.title ?? basename(workspace.path)}</span>
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
          onClick={(e) => workspaceMenu(app, workspace, { origin: e.currentTarget, at: menuBelow(e.currentTarget) })}
        >
          <Glyph name="more" className="glyph" />
        </button>
      </div>
      {open ? (
        <div className="pgroup sel">
          {rows.map((row) => (
            <SessionRow key={row.session_id} app={app} s={s} session={row as ShellSession} nested />
          ))}
          {!sessions.length ? <div className="empty nested">{t("projects.noSessions")}</div> : null}
          {sessions.length > SESSIONS ? (
            <button type="button" className="more-rows" onClick={() => app.set({ expanded: { ...s.expanded, [`project:${workspace.id}`]: !all } })}>
              {all ? t("sidebar.showLess") : t("projects.all", { count: sessions.length })}
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function basename(path: string): string {
  return path.replace(/\/+$/, "").split("/").pop() || path;
}
