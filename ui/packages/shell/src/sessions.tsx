/**
 * The session space's left column: every conversation, grouped by the `sidebar.group` point (by
 * time unless an application registers its own), with a name filter that searches the transcripts
 * on Enter, and the archived ones behind a toggle.
 *
 * `SessionRow` is shared with the projects space, which lists the same sessions by project.
 */

import { errorText } from "@harnix/core";
import { clip, currentLocale, useRegistry, useT, type SidebarGroup } from "@harnix/react";

import { keyOf, type App, type ShellSession, type ShellState } from "./app.ts";
import { sessionMenu } from "./dialogs.tsx";
import { Glyph } from "./glyph.tsx";
import { useShell } from "./hooks.ts";

const PAGE = 8;

export function SessionsSidebar({ app }: { app: App }) {
  const s = useShell(app);
  return (
    <>
      <SearchBox app={app} s={s} />
      <SessionList app={app} s={s} />
    </>
  );
}

function SearchBox({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const search = s.search;
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
              app.set({ search: { active: false, query: "", hits: null, truncated: false } });
              e.currentTarget.blur();
            } else if (e.key === "Enter" && search.query.trim()) void run(search.query.trim());
          }}
        />
      </label>
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
        <div className="grp title">
          <span className="label">{t("sidebar.searchHits", { count: hits.length })}</span>
          <button type="button" className="ib sm more" title={t("common.clear")} onClick={() => app.set({ search: { ...s.search, query: "", hits: null } })}>
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
      {groups.map((group) => {
        const all = Boolean(s.expanded[group.key]);
        const rows = all ? group.rows : group.rows.slice(0, PAGE * 2);
        return (
          <div key={group.key} className="group" data-group={group.key}>
            <div className="grp title">
              <span className="label truncate" title={group.hint}>
                {group.title}
              </span>
              <span className="count">{group.rows.length}</span>
            </div>
            {rows.map((row) => (
              <SessionRow key={keyOf((row as ShellSession).instance ?? "system", row.session_id)} app={app} s={s} session={row as ShellSession} />
            ))}
            {group.rows.length > rows.length || all ? (
              <button type="button" className="more-rows" onClick={() => app.set({ expanded: { ...s.expanded, [group.key]: !all } })}>
                {all ? t("sidebar.showLess") : t("sidebar.showMore", { count: group.rows.length - rows.length })}
              </button>
            ) : null}
          </div>
        );
      })}
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

export function SessionRow({ app, s, session, nested = false }: { app: App; s: ShellState; session: ShellSession; nested?: boolean }) {
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
      className={`srow${on ? " on" : ""}${nested ? " nested" : ""}`}
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

export function ago(iso: string | undefined, t: (k: string, v?: Record<string, unknown>) => string): string {
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
