/**
 * @harnix/harnix-app — the Harnix app's interface (the `harnix-app` extension's): the session
 * space and the projects space, which list the same conversations by time and by project; the
 * grouping by time; a session's archiving and export, offered on the shell's session menu; the
 * composer's `@` files of the open session's project; a project's trust (its menu, its shield,
 * the banner over a conversation in a project that is not trusted); and the right column's files,
 * preview and changes tabs, with the changes card a turn that wrote files leaves in the conversation.
 *
 * Only the host API: the sessions come from the hub (`harnix.sessions.list`, kept current by the
 * instance's pushed session list) and what this product notes about them from its own methods
 * (`harnix-app.*`). A session's own menu items (rename, fork, close, delete) are the shell's; its
 * rows open that menu.
 */

import { t, type ActionTarget, type CommandCandidate, type Dispose, type HostApi, type Registry } from "@harnix/react";

import { exportDialog } from "./dialogs.tsx";
import { ProjectsSidebar, TrustBanner } from "./projects.tsx";
import { SessionsSidebar, byTime } from "./sessions.tsx";
import { claimFor, registerSidebar } from "./sidebar.tsx";
import { AppStore, SESSIONS } from "./store.ts";

export { AppStore, SESSIONS, type AppSession, type AppState, type TrustItem, type TrustStatus, type Workspace } from "./store.ts";
export { byTime } from "./sessions.tsx";
export { CHANGES_KIND, FILES_KIND, FILE_PATTERN, PREVIEW_KIND, fileOfCall, lineOf } from "./sidebar.tsx";

/** The backend extension this is the interface of. */
export const extension = "harnix-app";

export function register(r: Registry, host: HostApi): Dispose {
  const store = new AppStore(host);
  store.start();
  const followed = { subscribe: store.subscribe, getSnapshot: store.getSnapshot };

  r.register(
    "space",
    {
      id: SESSIONS,
      icon: "chat",
      order: 10,
      title: () => t("space.sessions"),
      conversation: { subtitle: () => t("space.sessions.subtitle") },
      center: { mode: "chat" },
      sidebar: { render: () => <SessionsSidebar host={host} r={r} store={store} />, search: (q, submit) => store.search(q, submit) },
      store: followed,
    },
    extension,
  );
  r.register(
    "space",
    {
      id: "projects",
      icon: "folder",
      order: 20,
      title: () => t("space.projects"),
      conversation: {
        subtitle: () => t("space.projects.subtitle"),
        // A conversation started here runs in the open project.
        newSessionDefaults: () => {
          const root = store.projectRoot();
          return root ? { project_root: root } : undefined;
        },
        // A project that is not trusted, with its own settings held back: an offer to trust it.
        pinned: (session) => <TrustBanner host={host} store={store} session={session} />,
      },
      center: { mode: "chat" },
      sessions: SESSIONS,
      sidebar: { render: () => <ProjectsSidebar host={host} store={store} /> },
      store: followed,
    },
    extension,
  );

  r.register("sidebar.group", { id: "harnix-app.time", label: () => t("sidebar.byTime"), group: (sessions) => byTime(sessions) }, extension);

  // The backend space a target's interface space lists: what a call about the session claims.
  const claimOf = (target: ActionTarget) => r.at("space").find((c) => c.id === target.space)?.sessions ?? target.space;
  const archived = (target: ActionTarget) => store.row(target.session.instance, target.session.sessionId)?.archived === true;

  r.register(
    "session.action",
    {
      id: "harnix-app.archive",
      at: ["menu"],
      icon: "archive",
      label: (target) => (archived(target) ? t("dialog.unarchive") : t("dialog.archive")),
      // Archiving is a note this product keeps about its own sessions: only theirs.
      match: (target) => target.kind === "session" && claimOf(target) === SESSIONS,
      async run(target) {
        await host.call("harnix-app.sessions.archive", {
          instance: target.session.instance,
          session_id: target.session.sessionId,
          space: claimOf(target),
          archived: !archived(target),
        });
        await store.refresh();
      },
    },
    extension,
  );
  r.register(
    "session.action",
    {
      id: "harnix-app.export",
      at: ["menu"],
      icon: "download",
      label: () => t("dialog.export"),
      match: (target) => target.kind === "session",
      run: (target) => exportDialog(host, { instance: target.session.instance, session_id: target.session.sessionId, space: claimOf(target) }),
    },
    extension,
  );

  // `@` offers the open session's project files: the backend walks the session's project, as it
  // has it, not a directory this page names.
  r.register(
    "command",
    {
      id: "harnix-app.files",
      trigger: "@",
      async complete(query, ctx): Promise<CommandCandidate[]> {
        if (query.startsWith("session:")) return [];
        const session = ctx?.session ?? host.session.current();
        if (!session) return [];
        const listed = store.row(session.instance, session.sessionId)?.project_root;
        const context = host.session.context()?.session?.project_root;
        // Known to have no project: nothing to ask for.
        if (!(listed ?? (typeof context === "string" ? context : null))) return [];
        const { files } = await host.call<{ files: { path: string; size?: number }[] }>("harnix-app.files", {
          instance: session.instance,
          session_id: session.sessionId,
          space: claimFor(r, host),
          query,
        });
        // As the menu draws a candidate: the path, its size beside it, `@path ` inserted.
        return files.map((f) => ({ label: f.path, description: f.size != null ? `${(f.size / 1024).toFixed(1)} KB` : undefined, replacement: `@${f.path} ` }));
      },
    },
    extension,
  );
  // The right column's files, preview and changes, the changes card, and a file tool's path opening the file.
  registerSidebar(r, host, store, extension);
  // Disabled: the store stops following the hub with everything it fed.
  return () => store.stop();
}
