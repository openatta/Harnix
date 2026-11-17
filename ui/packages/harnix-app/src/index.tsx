/**
 * @harnix/harnix-app — the Harnix app's interface (the `harnix-app` extension's): the session
 * space and the projects space, which list the same conversations by time and by project; the
 * grouping by time; a session's archiving and export, offered on the shell's session menu; the
 * composer's `@` files of the open session's project; a project's trust (its menu, its shield,
 * the banner over a conversation in a project that is not trusted); and the right column's files,
 * preview and changes tabs, with the changes card a turn that wrote files leaves in the conversation;
 * and two first-run steps on the empty page (`home.card`): a project, and the scene and permission
 * mode a conversation starts with. It defines four points of its own — the kinds of item a project
 * tree lists besides its conversations, and the blocks of a project's overview (`items.ts`,
 * `overview.tsx`); the sources of what waits on a person, put together as one list (`inbox.tsx`);
 * and a badge on a session's row (`items.ts`).
 *
 * Only the host API: the sessions come from the hub (`harnix.sessions.list`, kept current by the
 * instance's pushed session list) and what this product notes about them from its own methods
 * (`harnix-app.*`). A session's own menu items (rename, fork, close, delete) are the shell's; its
 * rows open that menu.
 */

import { t, type ActionTarget, type CommandCandidate, type Dispose, type HostApi, type Registry } from "@harnix/react";

import { exportDialog } from "./dialogs.tsx";
import { INBOX, Inbox, InboxEntryButton, hubAsks, inboxDialog } from "./inbox.tsx";
import { PROJECT_ITEMS, PROJECT_PANELS, SESSION_BADGE } from "./items.ts";
import { OVERVIEW, OverviewTab, registerOverview } from "./overview.tsx";
import { ProjectsSidebar, TrustBanner } from "./projects.tsx";
import { SessionsSidebar, byTime } from "./sessions.tsx";
import { claimFor, registerSidebar } from "./sidebar.tsx";
import { PROJECT_STEP, ProjectStep, SCENE_STEP, SceneStep } from "./setup.tsx";
import { AppStore, SESSIONS } from "./store.ts";

export { AppStore, SESSIONS, type AppSession, type AppState, type TrustItem, type TrustStatus, type Workspace } from "./store.ts";
export { byTime } from "./sessions.tsx";
export { interleave } from "./projects.tsx";
export { CHANGES_KIND, FILES_KIND, FILE_PATTERN, PREVIEW_KIND, fileOfCall, lineOf } from "./sidebar.tsx";
export { PROJECT_STEP, SCENE_STEP } from "./setup.tsx";
export { INBOX, Inbox, askDetail, askItem, askKind, sameDir, type InboxEntry, type InboxItem, type InboxSource } from "./inbox.tsx";
export { OVERVIEW, orderedPanels, projectSessions, recentChanges, rulesByTier, skillsByTier, type RecentChange } from "./overview.tsx";
export {
  PROJECT_ITEMS,
  PROJECT_PANELS,
  SESSION_BADGE,
  activityOf,
  createEntries,
  useProjectItems,
  type ProjectItem,
  type ProjectItemContext,
  type ProjectItemKind,
  type ProjectItemRow,
  type ProjectPanel,
  type SessionBadge,
} from "./items.ts";

/** The backend extension this is the interface of. */
export const extension = "harnix-app";

/** Where the system space's metrics bar takes a count to the list it counts (`system.metrics.link`). */
const METRICS_LINK = "system.metrics.link";

export function register(r: Registry, host: HostApi): Dispose {
  // What else a project holds, and what its overview shows: other packages fill these (`items.ts`).
  r.definePoint(PROJECT_ITEMS, { kind: "register" });
  r.definePoint(PROJECT_PANELS, { kind: "render" });
  // A badge on a session's row, from whoever has something to say about the session.
  r.definePoint(SESSION_BADGE, { kind: "render" });
  // What waits on a person, from whoever says so: the hub's asks are this package's own source.
  r.definePoint(INBOX, { kind: "register" });
  const store = new AppStore(host);
  store.start();
  const followed = { subscribe: store.subscribe, getSnapshot: store.getSnapshot };
  const inbox = new Inbox(r);
  r.register(INBOX, hubAsks(host), extension);
  inbox.start();
  // The project's overview: the projects space's right column while a project is open and no
  // conversation is (the new conversation's page); a conversation's column is its own.
  const overview = { id: OVERVIEW, title: () => t("overview.title"), render: () => <OverviewTab r={r} store={store} /> };

  r.register(
    "space",
    {
      id: SESSIONS,
      icon: "chat",
      order: 10,
      title: () => t("space.sessions"),
      conversation: { subtitle: () => t("space.sessions.subtitle") },
      center: { mode: "chat" },
      sidebar: {
        render: () => (
          <>
            <InboxEntryButton host={host} r={r} inbox={inbox} />
            <SessionsSidebar host={host} r={r} store={store} />
          </>
        ),
        search: (q, submit) => store.search(q, submit),
      },
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
      sidebar: {
        render: () => (
          <>
            <InboxEntryButton host={host} r={r} inbox={inbox} />
            <ProjectsSidebar host={host} r={r} store={store} />
          </>
        ),
      },
      details: { tabs: () => (store.projectRoot() && !host.session.current() ? [overview] : []) },
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
  // First run, after the settings' model step: a project, then what a conversation starts with.
  r.register("home.card", { id: PROJECT_STEP, order: 20, render: () => <ProjectStep host={host} store={store} /> }, extension);
  r.register("home.card", { id: SCENE_STEP, order: 30, render: () => <SceneStep host={host} store={store} /> }, extension);
  // The overview's own blocks; other packages' go between them by order.
  registerOverview(r, host, store, inbox, extension);
  // The system space's "等你处理" count opens the list, where that space defines the link.
  r.register(METRICS_LINK, { id: "harnix-app.inbox", metric: "waiting", title: () => t("inbox.title"), open: () => void inboxDialog(host, r, inbox) }, extension, {
    orElse: () => undefined,
  });

  // Disabled: the stores stop following the hub with everything they fed.
  return () => {
    inbox.stop();
    store.stop();
  };
}
