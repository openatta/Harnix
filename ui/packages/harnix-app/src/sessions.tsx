/**
 * The session space's left column: every conversation, grouped by the `sidebar.group` point (by
 * time unless an application registers its own), filtered by the shell's search box as it is typed
 * and searched in the transcripts on Enter, and the archived ones behind a toggle.
 *
 * `SessionRow` is shared with the projects space, which lists the same sessions by project. Its
 * menu is the shell's (`host.session.menu`): the title bar opens the same one.
 */

import { useSyncExternalStore, type MouseEvent } from "react";
import { Contributed, Glyph, ago, clip, t, useContributions, useCurrentSession, useRegistry, useT, type HostApi, type Registry, type SidebarGroup } from "@harnix/react";

import { importDialog } from "./dialogs.tsx";
import { SESSION_BADGE, type SessionBadge } from "./items.ts";
import { keyOf, type AppSession, type AppStore } from "./store.ts";

const PAGE = 8;

export function useApp(store: AppStore) {
  return useSyncExternalStore(store.subscribe, store.getSnapshot);
}

export function SessionsSidebar({ host, r, store }: { host: HostApi; r: Registry; store: AppStore }) {
  const t = useT();
  const registry = useRegistry(r);
  const s = useApp(store);
  const hits = s.search.hits;

  if (hits === "pending") return <div className="empty">{t("sidebar.searching")}</div>;
  if (hits) {
    return (
      <div className="group">
        <div className="grp title">
          <span className="label">{t("sidebar.searchHits", { count: hits.length })}</span>
          <button type="button" className="ib sm more" title={t("common.clear")} onClick={() => store.set({ search: { ...s.search, hits: null } })}>
            <Glyph name="close" className="glyph" />
          </button>
        </div>
        {hits.map((hit) => (
          <button
            type="button"
            key={hit.session_id}
            className="srow hit"
            data-testid="session-hit"
            data-sid={hit.session_id}
            onClick={() => void host.session.open({ instance: host.defaults.instance, sessionId: hit.session_id })}
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
              <SessionRow key={keyOf((row as AppSession).instance ?? host.defaults.instance, row.session_id)} host={host} r={r} store={store} session={row as AppSession} />
            ))}
            {group.rows.length > rows.length || all ? (
              <button type="button" className="more-rows" onClick={() => store.toggleExpanded(group.key)}>
                {all ? t("sidebar.showLess") : t("sidebar.showMore", { count: group.rows.length - rows.length })}
              </button>
            ) : null}
          </div>
        );
      })}
      {archived ? (
        <button type="button" className="archive-toggle" data-testid="archive-toggle" onClick={() => store.toggleArchived()}>
          <Glyph name="archive" className="glyph" />
          <span>{s.showArchived ? t("sidebar.archivedHide") : t("sidebar.archivedShow", { count: archived })}</span>
        </button>
      ) : null}
      <button type="button" className="archive-toggle" data-testid="session-import" onClick={() => void importDialog(host, store)}>
        <Glyph name="upload" className="glyph" />
        <span>{t("sidebar.importSession")}</span>
      </button>
    </>
  );
}

/** Where a menu opened by a pointer goes: the pointer, else under the element (a key's click). */
function anchorOf(event: MouseEvent<HTMLElement>): Element | { x: number; y: number } {
  return event.clientX || event.clientY ? { x: event.clientX, y: event.clientY } : event.currentTarget;
}

export function SessionRow({ host, r, store, session, nested = false }: { host: HostApi; r: Registry; store: AppStore; session: AppSession; nested?: boolean }) {
  const t = useT();
  const s = useApp(store);
  const current = useCurrentSession();
  const instance = session.instance ?? host.defaults.instance;
  const on = current?.sessionId === session.session_id && current.instance === instance;
  const running = s.running[keyOf(instance, session.session_id)];
  const tooltip = [
    session.name || t("sidebar.untitled"),
    session.scene_layer || session.scene || session.status || "",
    session.message_count ? t("sidebar.messageCount", { count: session.message_count }) : "",
    session.archived ? t("sidebar.archivedTag") : "",
  ]
    .filter(Boolean)
    .join(" · ");
  const ref = { instance, sessionId: session.session_id, name: session.name ?? null };
  // `⋯` is the row's sibling, not inside it: a button in a button is neither HTML nor reachable by Tab.
  // Nested under a project it is an item of the projects' tree.
  return (
    <div className={`srow-wrap${on ? " on" : ""}${nested ? " nested" : ""}`} role={nested ? "none" : undefined}>
      <button
        type="button"
        className={`srow${on ? " on" : ""}${nested ? " nested" : ""}`}
        {...(nested ? { role: "treeitem", "aria-level": 2, "aria-selected": on } : {})}
        data-testid="session-row"
        data-state={on ? "current" : undefined}
        data-sid={session.session_id}
        data-instance={instance}
        title={tooltip}
        onClick={() => void host.session.open({ instance, sessionId: session.session_id })}
        onContextMenu={(e) => {
          e.preventDefault();
          host.session.menu(ref, anchorOf(e));
        }}
      >
        {running ? <span className="live" /> : null}
        <span className="tt name">{session.name || t("sidebar.untitled")}</span>
        <SessionBadges r={r} instance={instance} sessionId={session.session_id} />
        <span className="meta time">{ago(session.last_active, t)}</span>
      </button>
      <button
        type="button"
        className="more"
        data-testid="session-more"
        title={t("sidebar.sessionActions")}
        aria-label={t("sidebar.sessionActions")}
        aria-haspopup="menu"
        onClick={(e) => host.session.menu(ref, e.currentTarget)}
      >
        <Glyph name="more" className="glyph" />
      </button>
    </div>
  );
}

/** The other packages' badges on a session's row (`harnix-app.session.badge`), each in its own guard. */
function SessionBadges({ r, instance, sessionId }: { r: Registry; instance: string; sessionId: string }) {
  const badges = useContributions<SessionBadge>(SESSION_BADGE, r);
  if (!badges.length) return null;
  return (
    <span className="badges" data-testid="session-badges">
      {badges.map((badge) => (
        <Contributed key={`${badge.owner}/${badge.id}`} registry={r} point={SESSION_BADGE} contribution={badge}>
          <BadgeBody badge={badge} instance={instance} sessionId={sessionId} />
        </Contributed>
      ))}
    </span>
  );
}

/** Called inside the guard: a `render` that throws costs that badge, not the row. */
function BadgeBody({ badge, instance, sessionId }: { badge: SessionBadge; instance: string; sessionId: string }) {
  return <>{badge.render({ instance, sessionId })}</>;
}

/* The session space lists everything by when it was last active: today, the last seven days,
 * earlier. Projects have a space of their own; an application that wants another grouping
 * registers its own `sidebar.group`, and the last registration wins. */
export function byTime(sessions: SessionRowLike[]): SidebarGroup[] {
  const now = Date.now();
  const startOfToday = new Date(new Date().toDateString()).getTime();
  const groups = [
    { key: "today", title: t("time.today"), rows: [] as SessionRowLike[] },
    { key: "week", title: t("time.week"), rows: [] as SessionRowLike[] },
    { key: "earlier", title: t("time.earlier"), rows: [] as SessionRowLike[] },
  ];
  const sorted = [...sessions].sort((a, b) => String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")));
  for (const session of sorted) {
    const at = Date.parse(session.last_active ?? "");
    const group = Number.isNaN(at) ? groups[2]! : at >= startOfToday ? groups[0]! : now - at < 7 * 86_400_000 ? groups[1]! : groups[2]!;
    group.rows.push(session);
  }
  return groups.filter((g) => g.rows.length);
}

type SessionRowLike = SidebarGroup["rows"][number];
