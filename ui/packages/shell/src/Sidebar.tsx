/**
 * The left column, in Helm's shape. Fixed in every space: the brand and a new conversation at
 * the top; the system entry and the avatar at the bottom. Between them, the spaces this product
 * keeps and — below them — the front space's own column (`sidebar()`, where the sessions space
 * has its search), or its sessions when it has none.
 *
 * The avatar is the settings: its ring says how this page is connected, hovering says to what,
 * and a click opens the menu. The system space, when kept, is entered from the bottom rather than
 * listed with the others: it is about the process, not one of the product's surfaces. A space
 * that says `place: "bottom"` (something global, not a surface) sits there too, above it. One shown
 * inside another (`within`, the flows inside the system space) is not listed: the other's entry is
 * lit while it is in front.
 */

import { Fragment, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { Contributed, setLocale, currentLocale, useRegistry, useT, type SpaceContribution } from "@harnix/react";

import { ask, disabledOf, keyOf, uniqueSessions, type App, type ShellSession, type ShellState } from "./app.ts";
import { SYSTEM } from "./constants.ts";
import { connectionDialog } from "./dialogs.tsx";
import { Glyph } from "./glyph.tsx";
import { AppMark, GroupedSessions, appsOf, groupingOf } from "./inline/index.tsx";
import { Menu, type MenuEntry } from "./Menu.tsx";
import { SessionRow } from "./sessions.tsx";
import { openSettings } from "./settings.tsx";

/** The spaces, in order. */
export function spacesOf(app: App) {
  return [...app.registry.at("space")].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
}

/** Whether `space`'s entry is lit: it is in front, or the one in front is shown inside it (`within`). */
function lit(app: App, s: ShellState, space: string): boolean {
  return s.space === space || app.registry.at("space").find((c) => c.id === s.space)?.within === space;
}

/** The product's name: `[brand] name`, else Harnix's own. */
export function brandName(app: App, t: (k: string) => string): string {
  return app.snapshot.hello?.shell?.brand?.name || t("app.title");
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
    // Narrow, the toggle opens a drawer over the page: nothing to fade, nothing lands.
    if (still || app.snapshot.narrow) {
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

  // A new conversation needs a space with a conversation: the front one if it has one, else the
  // first that does.
  const startNew = async () => {
    if (space?.center === "hidden" || space?.id === SYSTEM || space?.within) {
      const talking = spacesOf(app).find((c) => c.id !== SYSTEM && !c.within && c.center !== "hidden" && disabledOf(app.registry, c) === null);
      if (!talking) return;
      await app.switchSpace(talking.id);
    }
    app.open(null);
  };
  const icon = s.hello?.shell?.brand?.icon;
  const open = app.sidebarShown(s) === "expanded";
  const collapse = (
    <button
      type="button"
      className="ib collapse"
      data-testid="sidebar-toggle"
      title={open ? t("app.collapseSidebar") : t("app.expandSidebar")}
      aria-label={open ? t("app.collapseSidebar") : t("app.expandSidebar")}
      aria-expanded={open}
      onClick={toggle}
    >
      <Glyph name="panel" className="glyph" />
    </button>
  );
  // Without a title bar (the desktop shell) the toggle lives beside the traffic lights, outside the
  // column: it stays put and visible while the column fades and lands as a rail.
  const overlay = document.documentElement.dataset.chrome === "overlay";

  return (
    <aside id="side" className={phase}>
      <div className="l-top" data-tauri-drag-region="deep">
        <div className="brand" data-testid="brand">
          {icon ? <img className="logo brand-icon" src={icon} alt="" /> : <Glyph name="brand" className="logo" />}
          <span className="lbl" data-testid="brand-name">
            {brandName(app, t)}
          </span>
        </div>
        {overlay ? createPortal(<div id="tl-tools">{collapse}</div>, document.body) : collapse}
      </div>
      <ContextLine s={s} />
      {space?.newSession === false || !app.startsSessions() ? null : (
        <>
          <button type="button" id="new" className="nav new" title={t("sidebar.newSessionHint")} onClick={() => void startNew()}>
            <Glyph name="edit" className="glyph" />
            <span className="lbl">{t("sidebar.newSession")}</span>
            <kbd className="lbl">⌘K</kbd>
          </button>
          <div className="sep" />
        </>
      )}
      <SpaceList app={app} s={s} />
      {space ? <div className="sep chats-sep" role="separator" data-testid="sessions-sep" /> : null}
      <div id="sessions" className="chats" data-space={s.space}>
        {space?.sidebar ? (
          <Contributed registry={registry} point="space" contribution={space}>
            {space.sidebar()}
          </Contributed>
        ) : space ? (
          <SpaceSessions app={app} s={s} listed={space.listSession} />
        ) : null}
      </div>
      {/* On the rail, where the column folded away: it opens it again (narrow, as a drawer). */}
      {space ? (
        <button type="button" className="nav rail-entry" data-testid="rail-sessions" title={t("sidebar.showSessions")} aria-label={t("sidebar.showSessions")} onClick={toggle}>
          <Glyph name="list" className="glyph" />
        </button>
      ) : null}
      <Bottom app={app} s={s} />
    </aside>
  );
}

/** The application's line under the brand (`host.shell.context`, ipdrun-49): a button when it can
 *  be clicked, plain text otherwise. */
function ContextLine({ s }: { s: ShellState }) {
  const c = s.shellContext;
  if (!c) return null;
  const body = (
    <>
      {c.tone === "active" ? <span className="dot" aria-hidden="true" /> : null}
      <span className="lbl truncate">{c.label}</span>
    </>
  );
  const props = { className: "context", "data-testid": "shell-context", "data-tone": c.tone, title: c.title };
  return c.onClick ? (
    <button type="button" {...props} onClick={() => c.onClick?.()}>
      {body}
    </button>
  ) : (
    <div {...props}>{body}</div>
  );
}

function SpaceList({ app, s }: { app: App; s: ShellState }) {
  useRegistry(app.registry);
  const spaces = spacesOf(app).filter((c) => c.id !== SYSTEM && c.place !== "bottom" && !c.within);
  if (!spaces.length) return null;
  return (
    <nav className="spaces" aria-label="spaces">
      {spaces.map((space, i) => (
        <Fragment key={space.id}>
          {/* Two neighbours of different groups (`group`, unset counts as ""): a line between. */}
          {i > 0 && (spaces[i - 1]!.group ?? "") !== (space.group ?? "") ? <div className="sep" role="separator" data-testid="space-sep" /> : null}
          <SpaceEntry app={app} s={s} space={space} />
        </Fragment>
      ))}
    </nav>
  );
}

/** A space's entry: its icon, its title, and its badge if it has one. */
function SpaceEntry({ app, s, space }: { app: App; s: ShellState; space: SpaceContribution & { owner: string } }) {
  const t = useT();
  const badge = badgeOf(app, space);
  // `disabled()` (ipdrun-48): greyed, says why on hover, and a click does nothing. Not the `disabled`
  // attribute: a disabled button shows no tooltip in every browser.
  const why = disabledOf(app.registry, space);
  const on = lit(app, s, space.id);
  return (
    <button
      type="button"
      className={`nav space${on ? " on" : ""}${why !== null ? " disabled" : ""}`}
      data-space={space.id}
      data-disabled={why !== null ? "" : undefined}
      aria-disabled={why !== null ? true : undefined}
      title={why ?? space.title()}
      aria-pressed={on}
      onClick={() => {
        if (disabledOf(app.registry, space) !== null) return;
        if (space.id === s.space && space.reenter) {
          try {
            space.reenter();
          } catch {
            // the space's own business; the click stays harmless
          }
          return;
        }
        void app.switchSpace(space.id);
      }}
    >
      <Glyph name={space.icon} className="glyph" />
      <span className="lbl truncate">{space.title()}</span>
      {/* A space an installed application brought says so (unless the reader turned it off). */}
      <AppMark app={app} owner={space.owner} />
      {badge ? (
        <span className="end">
          <span className="bn" data-testid="space-badge" aria-label={t("space.badge", { space: space.title(), value: badge })}>
            {badge}
          </span>
        </span>
      ) : null}
    </button>
  );
}

/** What a space's `badge()` says, as drawn: nothing for null, undefined, 0 and "". */
function badgeOf(app: App, space: SpaceContribution & { owner: string }): string | null {
  if (!space.badge) return null;
  const value = ask(app.registry, space, () => space.badge!(), null);
  return value === null || value === undefined || value === 0 || value === "" ? null : String(value);
}

/** A space with no column of its own: its sessions, newest first. */
function SpaceSessions({ app, s, listed }: { app: App; s: ShellState; listed?: SpaceContribution["listSession"] }) {
  const t = useT();
  const [rows, setRows] = useState<ShellSession[] | null>(null);
  const claim = app.claim(s.space);
  // Drawn again when an application starts or stops: its grouping comes and goes with it.
  const watch = appsOf(app);
  useSyncExternalStore(watch.subscribe, watch.getVersion);
  // Read again when a turn settles, and when the shell's own list was read again (`refreshSessions`:
  // a session made, renamed, archived or deleted) — not each time another listed one is opened.
  useEffect(() => {
    let alive = true;
    const load = () =>
      app
        .call<{ sessions: ShellSession[] }>("harnix.sessions", { space: claim })
        .then((r) => alive && setRows(uniqueSessions(r.sessions).sort((a, b) => String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")))))
        .catch(() => alive && setRows([]));
    void load();
    const off = app.client.onNotify((method) => {
      if (method === "harnix.turn_settled") void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [app, claim, s.sessions]);
  if (rows === null) return null;
  if (!rows.length) return <div className="empty">{t("sidebar.noSessions")}</div>;
  // the space leaves some out (its main conversation, which its entry opens): an empty result draws nothing
  const shown = listed ? rows.filter((row) => shownBy(listed, row)) : rows;
  if (!shown.length) return null;
  // An application's space grouped as its manifest declares.
  const rule = groupingOf(app, s.space);
  if (rule) return <GroupedSessions app={app} s={s} rule={rule} rows={shown} />;
  return (
    <div className="group">
      {shown.map((row) => (
        <SessionRow key={keyOf(row.instance ?? "system", row.session_id)} app={app} s={s} session={row} />
      ))}
    </div>
  );
}

function shownBy(listed: NonNullable<SpaceContribution["listSession"]>, row: ShellSession): boolean {
  try {
    return listed({ instance: row.instance ?? "system", sessionId: row.session_id }) !== false;
  } catch {
    return true;
  }
}

/* ── the bottom: the system entry and the avatar ──────────────────────── */

function Bottom({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  useRegistry(app.registry);
  const system = spacesOf(app).find((c) => c.id === SYSTEM);
  const below = spacesOf(app).filter((c) => c.id !== SYSTEM && c.place === "bottom" && !c.within);
  const running = Object.keys(s.running).length;
  return (
    <div className="l-bottom foot">
      {below.map((space) => (
        <SpaceEntry key={space.id} app={app} s={s} space={space} />
      ))}
      {system ? (
        <button
          type="button"
          className={`nav space${lit(app, s, SYSTEM) ? " on" : ""}`}
          data-space={SYSTEM}
          title={system.title()}
          aria-pressed={lit(app, s, SYSTEM)}
          onClick={() => void app.switchSpace(SYSTEM)}
        >
          <Glyph name={system.icon} className="glyph" />
          <span className="lbl truncate">{system.title()}</span>
          {running ? (
            <span className="end">
              <span className="bn" data-testid="running-count">
                {running}
              </span>
            </span>
          ) : null}
        </button>
      ) : null}
      <Avatar app={app} s={s} />
    </div>
  );
}

/** The connection, in the avatar's ring: green open, amber on the way, red gone. */
export function ringOf(connection: ShellState["connection"]): "ok" | "retry" | "down" {
  if (connection === "open") return "ok";
  if (connection === "connecting" || connection === "degraded") return "retry";
  return "down";
}

function Avatar({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const ring = ringOf(s.connection);
  const user = s.hello?.shell?.user || t("avatar.you");
  const label =
    s.connection === "open"
      ? t("conn.open")
      : s.connection === "connecting" || s.connection === "degraded"
        ? t("conn.connecting")
        : s.connection === "incompatible"
          ? t("conn.incompatible")
          : t("conn.closed");
  const tooltip = [label, app.client.topology ?? "", location.host, s.hello?.engine?.model ?? ""].filter(Boolean).join(" · ");
  const wrap = useRef<HTMLDivElement>(null);
  const items: MenuEntry[] = [
    { id: "settings", icon: "settings", label: t("app.settings"), testId: "open-settings", run: () => openSettings(app) },
    {
      id: "theme",
      icon: "moon",
      label: t("app.theme"),
      testId: "menu-theme",
      value: <span className="mval">{t(`settings.theme.${s.theme}`)}</span>,
      run: () => app.setTheme(nextTheme(s.theme)),
    },
    {
      id: "language",
      icon: "globe",
      label: t("settings.language"),
      testId: "menu-language",
      value: <span className="mval">{currentLocale() === "en" ? "English" : "中文"}</span>,
      run: () => setLocale(currentLocale() === "en" ? "zh-CN" : "en"),
    },
    { id: "about", icon: "info", label: t("settings.nav.about"), testId: "menu-about", run: () => openSettings(app, "about") },
    {
      id: "connection",
      icon: "link",
      label: t("avatar.connection"),
      testId: "menu-connection",
      value: <span className={`mval dot ${ring}`} />,
      run: () => connectionDialog(app),
    },
  ];
  return (
    <div className="me-wrap" ref={wrap}>
      <button
        type="button"
        id="conn"
        className={`me${ring === "ok" ? " on" : ""}${s.menu ? " open" : ""}`}
        data-conn={ring}
        data-state={s.connection}
        data-testid="avatar"
        title={tooltip}
        aria-haspopup="menu"
        aria-expanded={s.menu}
        onClick={() => app.toggleMenu()}
      >
        <span className="avatar">{initials(user)}</span>
        <span className="lbl truncate">{user}</span>
      </button>
      {s.menu ? <Menu label={user} items={items} className="menu avatar-menu" testId="avatar-menu" within={wrap} onClose={() => app.toggleMenu(false)} /> : null}
    </div>
  );
}

function nextTheme(theme: ShellState["theme"]): ShellState["theme"] {
  return theme === "system" ? "light" : theme === "light" ? "dark" : "system";
}

function initials(name: string): string {
  const parts = name.trim().split(/[\s._-]+/).filter(Boolean);
  const letters = parts.length > 1 ? parts[0]![0]! + parts[1]![0]! : name.slice(0, 2);
  return letters.toUpperCase();
}
