/**
 * The left column, in Helm's shape: brand; new conversation and search; the spaces; the front
 * space's sessions with a search box and a sort; and at the bottom settings, the theme, and what
 * this page is connected to.
 *
 * In the session space the list is grouped by the `sidebar.group` point (workspaces by default):
 * the shell keeps the chrome — collapse, drag, show more, archived — and the contribution decides
 * which sessions belong together. One that throws loses only the grouping. Another space shows its
 * own sidebar if it registered one, else its sessions.
 */

import { useEffect, useRef, useState, type DragEvent, type ReactNode } from "react";
import { errorText } from "@harnix/core";
import { Contributed, clip, currentLocale, useRegistry, useT, type SidebarGroup } from "@harnix/react";

import { keyOf, type App, type ShellSession, type ShellState, type Workspace } from "./app.ts";
import { addWorkspaceDialog, newSessionDialog, sessionMenu, workspaceMenu } from "./dialogs.tsx";
import { Glyph } from "./glyph.tsx";
import { openSettings } from "./settings.tsx";

const PAGE = 5;

/** The spaces, in order. */
export function spacesOf(app: App) {
  return [...app.registry.at("space")].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
}

export function Sidebar({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const space = spacesOf(app).find((c) => c.id === s.space);
  const [phase, setPhase] = useState<"" | "fading" | "rail-in">("");

  /* Collapse in three phases: the expanded content fades in place, the rail lands, then the
   * controls slide in. Reduced motion skips to the end. */
  const toggle = () => {
    const still = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (still) {
      app.toggleSidebar();
      return;
    }
    setPhase("fading");
    setTimeout(() => {
      app.toggleSidebar();
      setPhase("rail-in");
      setTimeout(() => setPhase(""), 150);
    }, 150);
  };

  // New conversation: in the session space a dialog (scene, project); elsewhere a fresh, empty
  // session in that space, started by its first message.
  const startNew = () => {
    if (s.space === "sessions") void newSessionDialog(app);
    else app.open(null);
  };

  return (
    <aside id="side" className={phase}>
      <div className="l-top">
        <div className="brand">
          <Glyph name="brand" className="logo" />
          <span className="lbl">{t("app.title")}</span>
        </div>
        <button type="button" className="ib collapse" title={t("app.collapseSidebar")} onClick={toggle}>
          <Glyph name="panel" className="glyph" />
        </button>
      </div>
      <button type="button" id="new" className="nav new" title={t("sidebar.newSessionHint")} onClick={startNew}>
        <Glyph name="edit" className="glyph" />
        <span className="lbl">{t("sidebar.newSession")}</span>
        <kbd className="lbl">⌘K</kbd>
      </button>
      <div className="sep" />
      <SpaceList app={app} s={s} />
      <SectionHead app={app} s={s} />
      <div id="sessions" className="chats" data-space={s.space}>
        {s.space === "sessions" ? (
          <SessionList app={app} s={s} />
        ) : space?.sidebar ? (
          <Contributed registry={registry} point="space" contribution={space}>
            {space.sidebar()}
          </Contributed>
        ) : (
          <SpaceSessions app={app} s={s} />
        )}
      </div>
      <Footer app={app} s={s} />
    </aside>
  );
}

function SpaceList({ app, s }: { app: App; s: ShellState }) {
  useRegistry(app.registry);
  const spaces = spacesOf(app);
  if (spaces.length < 2) return null;
  const running = Object.keys(s.running).length;
  return (
    <nav className="spaces" aria-label="spaces">
      {spaces.map((space) => (
        <button
          type="button"
          key={space.id}
          className={`nav space${space.id === s.space ? " on" : ""}`}
          data-space={space.id}
          title={space.title()}
          aria-pressed={space.id === s.space}
          onClick={() => void app.switchSpace(space.id)}
        >
          <Glyph name={space.icon} className="glyph" />
          <span className="lbl truncate">{space.title()}</span>
          {space.id === "sessions" && running ? (
            <span className="end">
              <span className="bn">{running}</span>
            </span>
          ) : null}
        </button>
      ))}
    </nav>
  );
}

/** "<space> · sessions N", the search box, the sort — and, in the session space, add a project. */
function SectionHead({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const space = spacesOf(app).find((c) => c.id === s.space);
  const search = s.search;
  const count = s.space === "sessions" ? s.sessions.filter((r) => !r.archived || s.showArchived).length : null;

  const clear = () => app.set({ search: { active: false, query: "", hits: null, truncated: false } });
  const run = async (query: string) => {
    app.set({ search: { ...app.snapshot.search, hits: "pending" } });
    try {
      const result = await app.call<{ hits?: { session_id: string; name?: string | null; snippet?: string }[]; truncated?: boolean }>(
        "harnix.search",
        { query },
      );
      app.set({ search: { ...app.snapshot.search, hits: result.hits ?? [], truncated: Boolean(result.truncated) } });
    } catch (e) {
      app.set({ search: { ...app.snapshot.search, hits: [] } });
      app.notice(t("banner.searchFailed", { error: errorText(e) }), "error");
    }
  };

  return (
    <>
      <div className="sec">
        <span className="truncate">
          {space?.title() ?? s.space} · {t("sidebar.sessions")}
        </span>
        {count !== null ? <span className="cnt">{count}</span> : null}
        {s.space === "sessions" ? (
          <button type="button" className="ib sm add" title={t("app.addProject")} onClick={() => addWorkspaceDialog(app)}>
            <Glyph name="plus" className="glyph" />
          </button>
        ) : null}
      </div>
      {s.space === "sessions" ? (
        <div className={`s-tools section-head${search.active || search.query ? " searching" : ""}`}>
          <label className="s-search">
            <Glyph name="search" className="glyph" />
            <input
              id="search"
              type="text"
              spellCheck={false}
              placeholder={t("sidebar.search")}
              value={search.query}
              onFocus={() => app.set({ search: { ...app.snapshot.search, active: true } })}
              onChange={(e) => app.set({ search: { ...search, active: true, query: e.target.value, hits: null } })}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  clear();
                  e.currentTarget.blur();
                } else if (e.key === "Enter" && search.query.trim()) void run(search.query.trim());
              }}
            />
          </label>
          <span className="sortbtn" title={t("sidebar.sortHint")}>
            <Glyph name="refresh" className="glyph" />
            {t("sidebar.sortRecent")}
          </span>
        </div>
      ) : null}
    </>
  );
}

function SessionList({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const hits = s.search.hits;

  if (hits === "pending") return <div className="empty">{t("sidebar.searching")}</div>;
  if (hits) {
    return (
      <div className="group">
        <div className="grp title">
          <span className="label">{t("sidebar.searchHits", { count: hits.length })}</span>
          <button
            type="button"
            className="ib sm more"
            title={t("common.clear")}
            onClick={() => app.set({ search: { ...s.search, query: "", hits: null } })}
          >
            <Glyph name="close" className="glyph" />
          </button>
        </div>
        {hits.map((hit) => (
          <button
            type="button"
            key={hit.session_id}
            className="srow hit"
            data-sid={hit.session_id}
            onClick={() => app.open({ instance: "system", sessionId: hit.session_id })}
          >
            <span className="tt">
              <span className="name">{hit.name || t("sidebar.untitled")}</span>
              <span className="snippet">{clip(hit.snippet ?? "", 120)}</span>
            </span>
          </button>
        ))}
        {!hits.length ? <div className="empty">{t("sidebar.searchNoHits")}</div> : null}
        {s.search.truncated ? <div className="empty">{t("sidebar.searchPartial")}</div> : null}
      </div>
    );
  }

  const query = s.search.query.trim().toLowerCase();
  const visible = s.sessions.filter((session) => {
    if (session.archived && !s.showArchived) return false;
    return !query || (session.name ?? "").toLowerCase().includes(query);
  });
  let groups: SidebarGroup[] = [{ key: "all", title: t("sidebar.byRecent"), rows: visible }];
  const grouping = registry.grouping();
  if (grouping) {
    try {
      groups = grouping.group(visible);
    } catch (e) {
      registry.fail("sidebar.group", grouping, e);
    }
  }
  const archived = s.sessions.filter((session) => session.archived).length;

  return (
    <>
      {!groups.length ? <div className="empty">{query ? t("sidebar.noMatches") : t("sidebar.noSessions")}</div> : null}
      {groups.map((group) => (
        <Group key={group.key} app={app} s={s} group={group} />
      ))}
      {archived ? (
        <button
          type="button"
          className="archive-toggle"
          onClick={() => {
            const showArchived = !s.showArchived;
            app.set({ showArchived });
            app.call("harnix.prefs.set", { key: "showArchived", value: showArchived }).catch(() => undefined);
          }}
        >
          <Glyph name="archive" className="glyph" />
          <span>{s.showArchived ? t("sidebar.archivedHide") : t("sidebar.archivedShow", { count: archived })}</span>
        </button>
      ) : null}
    </>
  );
}

function Group({ app, s, group }: { app: App; s: ShellState; group: SidebarGroup }) {
  const t = useT();
  const workspace = group.workspace as Workspace | undefined;
  const collapsed = Boolean(workspace?.collapsed);
  const [dragging, setDragging] = useState(false);
  const [drop, setDrop] = useState(false);

  const setCollapsed = async (value: boolean) => {
    if (!workspace) return;
    // Optimistic: the list redraws at once.
    app.set({ workspaces: app.snapshot.workspaces.map((w) => (w.id === workspace.id ? { ...w, collapsed: value } : w)) });
    try {
      await app.call("harnix.workspaces.update", { id: workspace.id, collapsed: value });
    } catch (e) {
      app.notice(errorText(e), "error");
    }
  };

  const onDrop = async (event: DragEvent) => {
    event.preventDefault();
    setDrop(false);
    const moved = event.dataTransfer.getData("text/plain");
    if (!workspace || !moved || moved === workspace.id) return;
    try {
      const result = await app.call<{ workspaces?: Workspace[] }>("harnix.workspaces.reorder", { id: moved, before_id: workspace.id });
      if (result.workspaces) app.set({ workspaces: result.workspaces });
    } catch (e) {
      app.notice(errorText(e), "error");
    }
  };

  const showAll = Boolean(s.expanded[group.key]);
  const rows = showAll ? group.rows : group.rows.slice(0, PAGE);

  return (
    <div
      className={["group", dragging ? "dragging" : "", drop ? "drop" : ""].filter(Boolean).join(" ")}
      onDragOver={
        workspace
          ? (e) => {
              e.preventDefault();
              setDrop(true);
            }
          : undefined
      }
      onDragLeave={workspace ? () => setDrop(false) : undefined}
      onDrop={workspace ? onDrop : undefined}
    >
      <div
        className="grp title"
        draggable={Boolean(workspace)}
        onClick={workspace ? () => void setCollapsed(!collapsed) : undefined}
        onDragStart={(e) => {
          if (!workspace) return;
          e.dataTransfer.setData("text/plain", workspace.id);
          setDragging(true);
        }}
        onDragEnd={() => setDragging(false)}
      >
        <button
          type="button"
          className="chev"
          title={collapsed ? t("sidebar.expand") : t("sidebar.collapse")}
          onClick={(e) => {
            e.stopPropagation();
            void setCollapsed(!collapsed);
          }}
        >
          <Glyph name={collapsed ? "chevronRight" : "chevronDown"} className="glyph arrow" />
        </button>
        <span className="label truncate" title={group.hint}>
          {group.title}
        </span>
        <span className="count">{group.rows.length}</span>
        {workspace ? (
          <button
            type="button"
            className="ib sm more"
            title={t("sidebar.projectActions")}
            onClick={(e) => {
              e.stopPropagation();
              workspaceMenu(app, workspace);
            }}
          >
            <Glyph name="more" className="glyph" />
          </button>
        ) : null}
      </div>
      {collapsed ? null : (
        <>
          {rows.map((row) => (
            <SessionRow key={keyOf((row as ShellSession).instance ?? "system", row.session_id)} app={app} s={s} session={row as ShellSession} />
          ))}
          {group.rows.length > PAGE ? (
            <button type="button" className="more-rows" onClick={() => app.set({ expanded: { ...s.expanded, [group.key]: !showAll } })}>
              {showAll ? t("sidebar.showLess") : t("sidebar.showMore", { count: group.rows.length - PAGE })}
            </button>
          ) : null}
        </>
      )}
    </div>
  );
}

/** A space with no sidebar of its own: its sessions. */
export function SpaceSessions({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const [rows, setRows] = useState<ShellSession[] | null>(null);
  const space = s.space;
  useEffect(() => {
    let alive = true;
    const load = () =>
      app
        .call<{ sessions: ShellSession[] }>("harnix.sessions", { space })
        .then((r) => alive && setRows([...r.sessions].sort((a, b) => String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")))))
        .catch(() => alive && setRows([]));
    void load();
    const off = app.client.onNotify((method) => {
      if (method === "harnix.turn_settled") void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [app, space, s.current]);
  if (rows === null) return null;
  if (!rows.length) return <div className="empty">{t("sidebar.noSessions")}</div>;
  return (
    <div className="group">
      {rows.map((row) => (
        <SessionRow key={keyOf(row.instance ?? "system", row.session_id)} app={app} s={s} session={row} />
      ))}
    </div>
  );
}

function SessionRow({ app, s, session }: { app: App; s: ShellState; session: ShellSession }) {
  const t = useT();
  const instance = session.instance ?? "system";
  const on = s.current?.sessionId === session.session_id && s.current.instance === instance;
  const running = s.running[keyOf(instance, session.session_id)];
  const tooltip = [
    session.name || t("sidebar.untitled"),
    session.scene || session.status || "",
    session.message_count ? t("sidebar.messageCount", { count: session.message_count }) : "",
    session.archived ? t("sidebar.archivedTag") : "",
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <button
      type="button"
      className={`srow${on ? " on" : ""}`}
      data-sid={session.session_id}
      data-instance={instance}
      title={tooltip}
      onClick={() => app.open({ instance, sessionId: session.session_id })}
      onContextMenu={(e) => {
        e.preventDefault();
        sessionMenu(app, session);
      }}
    >
      {running ? <span className="live" /> : null}
      <span className="tt name">{session.name || t("sidebar.untitled")}</span>
      <span className="meta time">{ago(session.last_active, t)}</span>
      <span
        role="button"
        tabIndex={-1}
        className="more"
        title={t("sidebar.sessionActions")}
        onClick={(e) => {
          e.stopPropagation();
          sessionMenu(app, session);
        }}
      >
        <Glyph name="more" className="glyph" />
      </span>
    </button>
  );
}

function ago(iso: string | undefined, t: (k: string, v?: Record<string, unknown>) => string): string {
  if (!iso) return "";
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const secs = Math.max(0, (Date.now() - then) / 1000);
  if (secs < 60) return t("time.justNow");
  if (secs < 3600) return t("time.minutes", { count: Math.floor(secs / 60) });
  if (secs < 86400) return t("time.hours", { count: Math.floor(secs / 3600) });
  if (secs < 86400 * 30) return t("time.days", { count: Math.floor(secs / 86400) });
  return new Date(then).toLocaleDateString(currentLocale());
}

function Footer({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const conn = s.connection === "open" ? "on" : s.connection === "connecting" || s.connection === "degraded" ? "" : "off";
  const label =
    s.connection === "open"
      ? t("conn.open")
      : s.connection === "connecting" || s.connection === "degraded"
        ? t("conn.connecting")
        : s.connection === "incompatible"
          ? t("conn.incompatible")
          : t("conn.closed");
  const current = s.current?.instance ?? "system";
  return (
    <div className="l-bottom foot">
      <FootButton icon="settings" label={t("app.settings")} testId="open-settings" onClick={() => openSettings(app)} />
      <div id="conn" className={`conn ${conn}`} data-state={s.connection}>
        <span className="dot" />
        <span className="label lbl truncate">
          {label} · {t("sidebar.instance", { id: current })}
          {s.hello?.engine?.model ? ` · ${s.hello.engine.model}` : ""}
        </span>
      </div>
    </div>
  );
}

function FootButton({ icon, label, onClick, testId }: { icon: string; label: string; onClick: () => void; testId?: string }): ReactNode {
  return (
    <button type="button" className="nav" title={label} data-testid={testId} onClick={onClick}>
      <Glyph name={icon} className="glyph" />
      <span className="lbl">{label}</span>
    </button>
  );
}
