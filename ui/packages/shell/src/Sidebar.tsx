/**
 * The left column, in Helm's shape. Fixed in every space: the brand and a new conversation at
 * the top; the system entry and the avatar at the bottom. Between them, the spaces this product
 * keeps and — below them — the front space's own column (`sidebar()`, where the sessions space
 * has its search), or its sessions when it has none.
 *
 * The avatar is the settings: its ring says how this page is connected, hovering says to what,
 * and a click opens the menu. The system space, when kept, is entered from the bottom rather than
 * listed with the others: it is about the process, not one of the product's surfaces.
 */

import { useEffect, useRef, useState } from "react";
import { Contributed, setLocale, currentLocale, useRegistry, useT } from "@harnix/react";

import { keyOf, type App, type ShellSession, type ShellState } from "./app.ts";
import { connectionDialog } from "./dialogs.tsx";
import { Glyph } from "./glyph.tsx";
import { SessionRow } from "./sessions.tsx";
import { openSettings } from "./settings.tsx";

export const SYSTEM = "system";

/** The spaces, in order. */
export function spacesOf(app: App) {
  return [...app.registry.at("space")].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
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

  // A new conversation needs a space with a conversation: the front one if it has one, else the
  // first that does.
  const startNew = async () => {
    if (space?.center === "hidden" || space?.id === SYSTEM) {
      const talking = spacesOf(app).find((c) => c.id !== SYSTEM && c.center !== "hidden");
      if (!talking) return;
      await app.switchSpace(talking.id);
    }
    app.open(null);
  };
  const icon = s.hello?.shell?.brand?.icon;

  return (
    <aside id="side" className={phase}>
      <div className="l-top">
        <div className="brand" data-testid="brand">
          {icon ? <img className="logo brand-icon" src={icon} alt="" /> : <Glyph name="brand" className="logo" />}
          <span className="lbl" data-testid="brand-name">
            {brandName(app, t)}
          </span>
        </div>
        <button type="button" className="ib collapse" title={t("app.collapseSidebar")} onClick={toggle}>
          <Glyph name="panel" className="glyph" />
        </button>
      </div>
      <button type="button" id="new" className="nav new" title={t("sidebar.newSessionHint")} onClick={() => void startNew()}>
        <Glyph name="edit" className="glyph" />
        <span className="lbl">{t("sidebar.newSession")}</span>
        <kbd className="lbl">⌘K</kbd>
      </button>
      <div className="sep" />
      <SpaceList app={app} s={s} />
      <div id="sessions" className="chats" data-space={s.space}>
        {space?.sidebar ? (
          <Contributed registry={registry} point="space" contribution={space}>
            {space.sidebar()}
          </Contributed>
        ) : space ? (
          <SpaceSessions app={app} s={s} />
        ) : null}
      </div>
      <Bottom app={app} s={s} />
    </aside>
  );
}

function SpaceList({ app, s }: { app: App; s: ShellState }) {
  useRegistry(app.registry);
  const spaces = spacesOf(app).filter((c) => c.id !== SYSTEM);
  if (!spaces.length) return null;
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
        </button>
      ))}
    </nav>
  );
}

/** A space with no column of its own: its sessions, newest first. */
function SpaceSessions({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  const [rows, setRows] = useState<ShellSession[] | null>(null);
  const claim = app.claim(s.space);
  useEffect(() => {
    let alive = true;
    const load = () =>
      app
        .call<{ sessions: ShellSession[] }>("harnix.sessions", { space: claim })
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
  }, [app, claim, s.current]);
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

/* ── the bottom: the system entry and the avatar ──────────────────────── */

function Bottom({ app, s }: { app: App; s: ShellState }) {
  const t = useT();
  useRegistry(app.registry);
  const system = spacesOf(app).find((c) => c.id === SYSTEM);
  const running = Object.keys(s.running).length;
  return (
    <div className="l-bottom foot">
      {system ? (
        <button
          type="button"
          className={`nav space${s.space === SYSTEM ? " on" : ""}`}
          data-space={SYSTEM}
          title={system.title()}
          aria-pressed={s.space === SYSTEM}
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
  const menu = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!s.menu) return;
    const away = (event: MouseEvent) => {
      if (!menu.current?.contains(event.target as Node)) app.toggleMenu(false);
    };
    window.addEventListener("mousedown", away);
    return () => window.removeEventListener("mousedown", away);
  }, [app, s.menu]);
  const pick = (run: () => void) => () => {
    app.toggleMenu(false);
    run();
  };
  return (
    <div className="me-wrap" ref={menu}>
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
      {s.menu ? (
        <div className="menu avatar-menu" role="menu" data-testid="avatar-menu">
          <button type="button" role="menuitem" data-testid="open-settings" onClick={pick(() => openSettings(app))}>
            <Glyph name="settings" className="glyph" />
            {t("app.settings")}
          </button>
          <button type="button" role="menuitem" data-testid="menu-theme" onClick={pick(() => app.setTheme(nextTheme(s.theme)))}>
            <Glyph name="moon" className="glyph" />
            {t("app.theme")}
            <span className="mval">{t(`settings.theme.${s.theme}`)}</span>
          </button>
          <button type="button" role="menuitem" data-testid="menu-language" onClick={pick(() => setLocale(currentLocale() === "en" ? "zh-CN" : "en"))}>
            <Glyph name="globe" className="glyph" />
            {t("settings.language")}
            <span className="mval">{currentLocale() === "en" ? "English" : "中文"}</span>
          </button>
          <button type="button" role="menuitem" data-testid="menu-about" onClick={pick(() => openSettings(app, "about"))}>
            <Glyph name="info" className="glyph" />
            {t("settings.nav.about")}
          </button>
          <button type="button" role="menuitem" data-testid="menu-connection" onClick={pick(() => connectionDialog(app))}>
            <Glyph name="link" className="glyph" />
            {t("avatar.connection")}
            <span className={`mval dot ${ring}`} />
          </button>
        </div>
      ) : null}
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
