/**
 * The left column: brand, new session, search, the session list grouped by the `sidebar.group`
 * point, the archived toggle, and the footer (settings, theme, model).
 *
 * The shell keeps the chrome — collapse, drag, show more, archived — and the contribution decides
 * which sessions belong together and in what order. One that throws loses only the grouping:
 * everything falls into one list rather than the column going blank.
 */

import { useEffect, useRef, useState, type DragEvent } from "react";
import { errorText } from "@harnix/core";
import { Contributed, clip, currentLocale, useRegistry, useT, type SidebarGroup } from "@harnix/react";

import { keyOf, type App, type ShellSession, type ShellState, type Workspace } from "./app.ts";
import { addWorkspaceDialog, newSessionDialog, sessionMenu, workspaceMenu } from "./dialogs.tsx";
import { Glyph } from "./glyph.tsx";
import { openSettings } from "./settings.tsx";
import { isDark } from "./Shell.tsx";

const PAGE = 5;

export function Sidebar({ app, s }: { app: App; s: ShellState }) {
  const registry = useRegistry(app.registry);
  const space = spacesOf(app).find((c) => c.id === s.space);
  const t = useT();
  const side = useRef<HTMLElement>(null);
  const [phase, setPhase] = useState<"" | "fading" | "rail-in">("");
  const [quiet, setQuiet] = useState(true);
  const quietTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

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

  return (
    <aside
      id="side"
      ref={side}
      className={[phase, quiet ? "quiet" : ""].filter(Boolean).join(" ")}
      onPointerEnter={() => {
        if (quietTimer.current) clearTimeout(quietTimer.current);
        setQuiet(false);
      }}
      onPointerLeave={() => {
        if (quietTimer.current) clearTimeout(quietTimer.current);
        quietTimer.current = setTimeout(() => setQuiet(true), 2000);
      }}
    >
      <div className="brand">
        <Glyph name="brand" className="mark" />
        <span className="name">{t("app.title")}</span>
        <button type="button" className="icon-btn collapse" title={t("app.collapseSidebar")} onClick={toggle}>
          <Glyph name="panel" className="glyph" />
        </button>
      </div>
      <SpaceSwitcher app={app} s={s} />
      {s.space === "sessions" ? (
        <>
          <button type="button" id="new" title={t("sidebar.newSessionHint")} onClick={() => newSessionDialog(app)}>
            <Glyph name="plus" className="glyph" />
            <span className="label">{t("sidebar.newSession")}</span>
          </button>
          <SectionHead app={app} s={s} />
          <div id="sessions">
            <SessionList app={app} s={s} />
          </div>
        </>
      ) : space?.sidebar ? (
        <div id="sessions" data-space={s.space}>
          <Contributed registry={registry} point="space" contribution={space}>
            {space.sidebar()}
          </Contributed>
        </div>
      ) : (
        <SpaceSessions app={app} s={s} />
      )}
      <Footer app={app} s={s} />
    </aside>
  );
}

/** The spaces, in order: the registered ones the backend also knows (or that need no backend). */
export function spacesOf(app: App) {
  return [...app.registry.at("space")].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
}

function SpaceSwitcher({ app, s }: { app: App; s: ShellState }) {
  useRegistry(app.registry);
  const spaces = spacesOf(app);
  if (spaces.length < 2) return null;
  return (
    <nav className="spaces" aria-label="spaces">
      {spaces.map((space) => (
        <button
          type="button"
          key={space.id}
          className={`space${space.id === s.space ? " on" : ""}`}
          data-space={space.id}
          title={space.title()}
          aria-pressed={space.id === s.space}
          onClick={() => void app.switchSpace(space.id)}
        >
          <Glyph name={space.icon} className="glyph" />
          <span className="label truncate">{space.title()}</span>
        </button>
      ))}
    </nav>
  );
}

/** A space with no sidebar of its own: its sessions, and a way to start one. */
export function SpaceSessions({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const [rows, setRows] = useState<ShellSession[] | null>(null);
  const space = s.space;
  useEffect(() => {
    let alive = true;
    const load = () =>
      app
        .call<{ sessions: ShellSession[] }>("harnix.sessions", { space })
        .then((r) => alive && setRows(r.sessions))
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
  return (
    <>
      <button
        type="button"
        id="new"
        data-testid="space-new"
        title={t("sidebar.newSessionHint")}
        onClick={() => void app.create({}).catch((e: unknown) => app.notice(errorText(e), "error"))}
      >
        <Glyph name="plus" className="glyph" />
        <span className="label">{t("sidebar.newSession")}</span>
      </button>
      <div id="sessions" data-space={space}>
        {rows === null ? null : rows.length ? (
          <div className="group">
            {rows.map((row) => (
              <SessionRow key={keyOf(row.instance ?? "system", row.session_id)} app={app} s={s} session={row} />
            ))}
          </div>
        ) : (
          <div className="empty">{t("sidebar.noSessions")}</div>
        )}
      </div>
    </>
  );
}

function SectionHead({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const input = useRef<HTMLInputElement>(null);
  const search = s.search;

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
    <div className={`section-head${search.active ? " searching" : ""}`}>
      <span className="section-label truncate">{t("sidebar.sessions")}</span>
      <div className="search-slot">
        <div className="search-box">
          <button
            type="button"
            className="icon-btn"
            title={t("sidebar.search")}
            onClick={() => {
              if (search.active) clear();
              else {
                app.set({ search: { ...search, active: true } });
                setTimeout(() => input.current?.focus(), 60);
              }
            }}
          >
            <Glyph name="search" className="glyph" />
          </button>
          <input
            id="search"
            ref={input}
            type="text"
            spellCheck={false}
            placeholder={t("sidebar.search")}
            value={search.query}
            onChange={(e) => app.set({ search: { ...search, query: e.target.value, hits: null } })}
            onKeyDown={(e) => {
              if (e.key === "Escape") clear();
              else if (e.key === "Enter" && search.query.trim()) void run(search.query.trim());
            }}
          />
        </div>
      </div>
      <button type="button" className="icon-btn" title={t("app.addProject")} onClick={() => addWorkspaceDialog(app)}>
        <Glyph name="plus" className="glyph" />
      </button>
    </div>
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
        <div className="title">
          <span className="label">{t("sidebar.searchHits", { count: hits.length })}</span>
          <button
            type="button"
            className="icon-btn more"
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
            <span className="name">{hit.name || t("sidebar.untitled")}</span>
            <span className="snippet">{clip(hit.snippet ?? "", 120)}</span>
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
        className="title"
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
          <Glyph name="folder" className="glyph folder" />
          <Glyph name={collapsed ? "chevronRight" : "chevronDown"} className="glyph arrow" />
        </button>
        <span className="label" title={group.hint}>
          {group.title}
        </span>
        <span className="count">{group.rows.length}</span>
        {workspace ? (
          <button
            type="button"
            className="icon-btn more"
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
            <button
              type="button"
              className="more-rows"
              onClick={() => app.set({ expanded: { ...s.expanded, [group.key]: !showAll } })}
            >
              {showAll ? t("sidebar.showLess") : t("sidebar.showMore", { count: group.rows.length - PAGE })}
            </button>
          ) : null}
        </>
      )}
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
      <span className="name">{session.name || t("sidebar.untitled")}</span>
      <span className="time">{ago(session.last_active, t)}</span>
      <span
        role="button"
        tabIndex={-1}
        className="icon-btn more"
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
  const [, redraw] = useState(0);
  // The theme icon follows what is applied, which the system can change under it.
  useEffect(() => {
    const observer = new MutationObserver(() => redraw((n) => n + 1));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => observer.disconnect();
  }, []);
  return (
    <div className="foot">
      <button type="button" className="icon-btn" title={t("app.settings")} data-testid="open-settings" onClick={() => openSettings(app)}>
        <Glyph name="settings" className="glyph" />
      </button>
      <button type="button" className="icon-btn" title={t("app.theme")} onClick={() => app.setTheme(isDark() ? "light" : "dark")}>
        <Glyph name={isDark() ? "sun" : "moon"} className="glyph" />
      </button>
      <span className="text truncate">{s.hello?.engine?.model ?? ""}</span>
    </div>
  );
}
