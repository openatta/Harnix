/**
 * The settings panel: a section list on the left, the chosen section on the right.
 *
 * The frame is the shell's, and so are four sections every product has: the appearance first;
 * this page's diagnostics and what the product is last. Between them, whatever is registered at
 * `settings.section`, in order — the engine's settings, the plugins, the devices are UI packages'
 * like any application's. A section that throws loses its page, not the panel: the nav stays usable.
 */

import { Fragment, useEffect, useState, useSyncExternalStore, type ReactNode } from "react";
import { errorText } from "@harnix/core";
import { Contributed, DISPLAY_DEFAULTS, FONT_SIZE_RANGE, FieldForm, Glyph, SettingsReload, SettingsRow, SettingsSection, SettingsValue, display, localFieldStore, localeChoice, setDisplay, setLocale, t, useRegistry, useT, type Display, type LocalField, type LocaleChoice, type SettingsNeed } from "@harnix/react";

import type { App, ThemeChoice } from "./app.ts";
import { APPS_LIST } from "./constants.ts";
import { Body, CloseFoot } from "./dialogs.tsx";
import { packagesOf } from "./inline/index.tsx";
import { PageFrames } from "./surfaces/status.tsx";

let active = "general";

export function openSettings(app: App, section?: string): void {
  if (section) active = section;
  // Opened again while it is on top (`host.settings.open` from a section's frame), it is drawn anew in place.
  app.openModal({ title: t("settings.title"), wide: true, key: "settings", content: <Settings app={app} /> });
}

/** The settings at the section that covers `need` (`settings.section`'s `covers`); at the panel's
 *  first when none does. */
export function openSettingsFor(app: App, need: SettingsNeed): void {
  openSettings(app, app.registry.at("settings.section").find((s) => s.covers?.includes(need))?.id ?? settingsSections(app)[0]);
}

/** The shell's own sections, around the registered ones. */
function ownSections(app: App) {
  const section = (id: string, label: string, render: () => ReactNode) => ({ id, label: () => t(label), render, owner: "shell" });
  return {
    first: [section("general", "settings.nav.general", () => <General app={app} />)],
    last: [section("diagnostics", "settings.nav.diagnostics", () => <Diagnostics app={app} />), section("about", "settings.nav.about", () => <About app={app} />)],
  };
}

/** Every section's id, in the panel's order: what `host.settings.open` may name. */
export function settingsSections(app: App): string[] {
  const own = ownSections(app);
  return [...own.first, ...app.registry.at("settings.section"), ...own.last].map((s) => s.id);
}

function Settings({ app }: { app: App }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const [current, setCurrent] = useState(active);
  const [version, setVersion] = useState(0);
  const own = ownSections(app);
  const sections = [...own.first, ...registry.at("settings.section"), ...own.last];
  const section = sections.find((s) => s.id === current) ?? sections[0];
  return (
    <SettingsReload version={version} reload={() => setVersion((v) => v + 1)}>
      <Body>
        <div className="settings-body">
          <div className="settings-nav" data-testid="settings-nav">
            {sections.map((s) => (
              <button
                type="button"
                key={s.id}
                data-section={s.id}
                data-state={s.id === section?.id ? "current" : undefined}
                className={`nav-item${s.id === section?.id ? " on" : ""}`}
                onClick={() => {
                  active = s.id;
                  setCurrent(s.id);
                }}
              >
                {s.label()}
              </button>
            ))}
          </div>
          <div className="settings-page" data-testid="settings-page" key={`${section?.id}-${version}`}>
            {section ? (
              <Contributed registry={registry} point="settings.section" contribution={section}>
                {section.render()}
              </Contributed>
            ) : (
              <div className="empty">{t("details.noPanel")}</div>
            )}
          </div>
        </div>
      </Body>
      <CloseFoot app={app} />
    </SettingsReload>
  );
}

/* ── the sections ───────────────────────────────────────────────────── */

/**
 * This browser's preferences, drawn from their declarations like any section's fields: the theme
 * the shell applies, the language. Kept here, not in the engine's settings; more come the same way
 * (a row each).
 */
function preferences(app: App) {
  return localFieldStore(
    [
      {
        key: "theme",
        type: "select",
        label: "settings.theme",
        hint: "settings.themeHint",
        options: ["auto", "light", "dark"],
        optionLabels: { auto: "settings.theme.auto", light: "settings.theme.light", dark: "settings.theme.dark" },
        default: "auto",
        get: () => app.snapshot.theme,
        set: (value) => app.setTheme(value as ThemeChoice),
      },
      {
        key: "language",
        type: "select",
        label: "settings.language",
        hint: "settings.languageHint",
        options: ["auto", "zh-CN", "en"],
        optionLabels: { auto: "settings.language.auto", "zh-CN": "settings.language.zh-CN", en: "settings.language.en" },
        default: "auto",
        get: () => localeChoice(),
        set: (value) => setLocale(value as LocaleChoice),
      },
    ],
    { prefix: "harnix.pref.shell." },
  );
}

/** A display preference: kept by the display module, which the chat components read. */
function shown<K extends keyof Display>(key: K, rest: Omit<LocalField, "key" | "get" | "set" | "default">): LocalField {
  return {
    key,
    default: DISPLAY_DEFAULTS[key],
    get: () => display()[key],
    set: (value: unknown) => setDisplay({ [key]: value } as Partial<Display>),
    ...rest,
  };
}

const options = (prefix: string, ids: string[]) => ({
  options: ids,
  optionLabels: Object.fromEntries(ids.map((id) => [id, `${prefix}.${id}`])),
});

/**
 * How this browser draws the conversation, as rows of their own like the preferences above.
 */
function displayPreferences() {
  return localFieldStore(
    [
      // general & display
      shown("fontSize", { type: "number", label: "settings.display.fontSize", hint: "settings.display.fontSizeHint", ...FONT_SIZE_RANGE }),
      shown("fontBody", { type: "text", label: "settings.display.fontBody", hint: "settings.display.fontHint" }),
      shown("fontCode", { type: "text", label: "settings.display.fontCode", hint: "settings.display.fontHint" }),
      shown("toolsOpen", { type: "toggle", label: "settings.display.toolsOpen", hint: "settings.display.toolsOpenHint" }),
      shown("thinkingFolded", { type: "toggle", label: "settings.display.thinkingFolded", hint: "settings.display.thinkingFoldedHint" }),
      shown("usage", { type: "select", label: "settings.display.usage", hint: "settings.display.usageHint", ...options("settings.display.usage", ["none", "compact", "full"]) }),
      shown("links", { type: "select", label: "settings.display.links", hint: "settings.display.linksHint", ...options("settings.display.links", ["same", "new", "ask"]) }),
      shown("enter", { type: "select", label: "settings.display.enter", hint: "settings.display.enterHint", ...options("settings.display.enter", ["queue", "steer"]) }),
    ],
    { prefix: "harnix.pref.display." },
  );
}

function General({ app }: { app: App }) {
  const t = useT();
  const [store] = useState(() => preferences(app));
  const [shownStore] = useState(displayPreferences);
  return (
    <>
      <FieldForm title={t("settings.appearance")} store={store} />
      <FieldForm title={t("settings.display")} store={shownStore} />
      <PackagesMark app={app} />
    </>
  );
}

/**
 * Whether the left column marks the spaces extension packages brought: the mark is the shell's,
 * drawn for what `packages.list` says is installed, so the choice is here, where this page hosts
 * packages at all.
 */
function PackagesMark({ app }: { app: App }) {
  const t = useT();
  const watch = packagesOf(app);
  useSyncExternalStore(watch.subscribe, watch.getVersion);
  if (!app.snapshot.reachable.includes(APPS_LIST)) return null;
  return (
    <SettingsRow label={t("packages.mark")} hint={t("packages.markHint")}>
      <input type="checkbox" data-testid="packages-mark-toggle" checked={watch.marked()} onChange={(e) => watch.setMarked(e.target.checked)} />
    </SettingsRow>
  );
}

/** Every extension and whether it is enabled, as hello and `harnix.ui` say: which UI packages draw;
 *  and under it, what its frames on this page are doing. */
function Extensions({ app }: { app: App }) {
  const t = useT();
  const extensions = useSyncExternalStore(app.subscribe, () => app.snapshot.extensions);
  return (
    <>
      <h3>{t("diag.extensions")}</h3>
      <div className="kv-table" data-testid="extensions">
        {extensions.map((e) => (
          <Fragment key={e.id}>
            <div className="kv-row" data-extension={e.id} data-enabled={e.enabled}>
              <code>{e.id}</code>
              <span className="muted">{[e.carrier ?? "", e.trust, e.version ?? "", e.enabled ? "" : t("diag.extensionOff")].filter(Boolean).join(" · ")}</span>
            </div>
            <PageFrames ext={e.id} />
          </Fragment>
        ))}
      </div>
    </>
  );
}

/** The `health` block of `engine.daemon.doctor`: a status per check, `ok`, `degraded` or `failing`. */
export type DoctorHealth = { status?: string; checks?: { name: string; status?: string; summary?: string }[] };

/** A degraded check is a warning, not a failure; a status this build does not know reads as a failure. */
export function healthTone(status: string | undefined): "ok" | "warn" | "err" {
  return status === "ok" ? "ok" : status === "degraded" ? "warn" : "err";
}

const OVERALL = { ok: "settings.doctorOk", warn: "settings.doctorDegraded", err: "settings.doctorFailed" } as const;
const EACH = { ok: "settings.checkPass", warn: "settings.checkWarn", err: "settings.checkFail" } as const;

export function Diagnostics({ app }: { app: App }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const [report, setReport] = useState<{ running: true } | { error: string } | { health?: DoctorHealth } | null>(null);
  const refused = registry.refusals();
  return (
    <>
      <SettingsSection title={t("settings.engine")} load={() => app.call<{ version?: string; uptime_secs?: number; sessions?: number }>("engine.daemon.status", {})}>
        {(data) => (
          <>
            <SettingsRow label={t("settings.version")}>
              <SettingsValue>{data.version || t("common.none")}</SettingsValue>
            </SettingsRow>
            <SettingsRow label={t("settings.uptime")}>
              <SettingsValue>{t("settings.uptimeValue", { minutes: Math.round((data.uptime_secs ?? 0) / 60) })}</SettingsValue>
            </SettingsRow>
            <SettingsRow label={t("settings.sessionCount")}>
              <SettingsValue>{String(data.sessions ?? "—")}</SettingsValue>
            </SettingsRow>
            <SettingsRow label={t("settings.activeScenes")}>
              <SettingsValue>{(app.snapshot.hello?.engine?.active_scenes ?? []).join(" · ") || t("common.none")}</SettingsValue>
            </SettingsRow>
          </>
        )}
      </SettingsSection>
      <RuntimeModules app={app} />
      <Extensions app={app} />
      <h3>{t("settings.contributions")}</h3>
      {!refused.length ? (
        <div className="hint">{t("contrib.noneRefused")}</div>
      ) : (
        refused.map((entry, i) => (
          <SettingsRow key={i} label={t("contrib.refusedAt", { point: entry.point })} hint={entry.id ? `${entry.id} — ${entry.reason}` : entry.reason}>
            <SettingsValue>{entry.owner}</SettingsValue>
          </SettingsRow>
        ))
      )}
      <h3>{t("settings.doctor")}</h3>
      <button
        type="button"
        className="btn sm outline"
        onClick={() => {
          setReport({ running: true });
          app
            .call<{ health?: DoctorHealth }>("engine.daemon.doctor", {})
            .then(setReport)
            .catch((e: unknown) => setReport({ error: errorText(e) }));
        }}
      >
        <Glyph name="refresh" className="glyph" />
        {t("settings.doctorRun")}
      </button>
      {report && "running" in report ? <div className="hint">{t("common.running")}</div> : null}
      {report && "error" in report ? <div className="hint err">{report.error}</div> : null}
      {report && "health" in report ? (
        <>
          <div className="hint">{t(OVERALL[healthTone(report.health?.status)])}</div>
          {(report.health?.checks ?? []).map((check) => (
            <SettingsRow key={check.name} label={check.name} hint={check.summary ?? ""}>
              <SettingsValue tone={healthTone(check.status)}>{t(EACH[healthTone(check.status)])}</SettingsValue>
            </SettingsRow>
          ))}
        </>
      ) : null}
    </>
  );
}

/**
 * How installed packages reach the interface (concept §5.8): the contribution API this shell
 * speaks, whether runtime modules load at all, the import map they resolve `react` and
 * `@harnix/react` through and the CSP line that admits it. What each plugin's entry became is the
 * plugins section's.
 */
function RuntimeModules({ app }: { app: App }) {
  const t = useT();
  const hello = app.snapshot.hello;
  const on = hello?.shell?.runtime_modules !== false;
  const map = document.querySelector('script[type="importmap"]')?.textContent ?? "";
  let imports: Record<string, string> = {};
  try {
    imports = (JSON.parse(map) as { imports?: Record<string, string> }).imports ?? {};
  } catch {
    /* no import map: a page built without one */
  }
  const [csp, setCsp] = useState<string | null>(null);
  useEffect(() => {
    void fetch(location.pathname, { credentials: "same-origin" })
      .then((r) => setCsp(r.headers.get("content-security-policy")))
      .catch(() => setCsp(null));
  }, []);
  return (
    <>
      <h3>{t("diag.runtimeModules")}</h3>
      <SettingsRow label={t("diag.contribApi")} hint={t("diag.contribApiHint")}>
        <SettingsValue>{`v${hello?.contrib_api_version ?? "?"}`}</SettingsValue>
      </SettingsRow>
      <SettingsRow label="[ui] runtime_modules" hint={t("diag.runtimeHint")}>
        <SettingsValue tone={on ? "ok" : undefined}>{on ? "true" : "false"}</SettingsValue>
      </SettingsRow>
      <SettingsRow label={t("diag.importMap")} hint={t("diag.importMapHint")} />
      <div className="kv-table" data-testid="import-map">
        {Object.entries(imports).map(([name, path]) => (
          <div key={name} className="kv-row">
            <code>{name}</code>
            <span className="muted">{path}</span>
          </div>
        ))}
      </div>
      <SettingsRow label="Content-Security-Policy" hint={t("diag.cspHint")} />
      {csp ? <pre className="hx-pre" data-testid="csp">{csp}</pre> : null}
    </>
  );
}

function About({ app }: { app: App }) {
  const t = useT();
  const shell = app.snapshot.hello?.shell;
  const hello = (app.snapshot.hello ?? {}) as Record<string, unknown> & { limits?: Record<string, number>; engine?: Record<string, unknown> };
  const limits = hello.limits ?? {};
  const mb = (n: number | undefined) => t("settings.megabytes", { size: Math.round((n ?? 0) / 1048576) });
  return (
    <>
      <h3>{t("settings.about")}</h3>
      <SettingsRow label={t("about.product")}>
        <SettingsValue>{shell?.brand?.name || t("app.title")}</SettingsValue>
      </SettingsRow>
      <SettingsRow label="product_id" hint={t("about.productIdHint")}>
        <SettingsValue>{shell?.brand?.product_id ?? "harnix"}</SettingsValue>
      </SettingsRow>
      <SettingsRow label={t("settings.protocolVersion")}>
        <SettingsValue>{String(hello.protocol_version ?? "—")}</SettingsValue>
      </SettingsRow>
      <SettingsRow label={t("settings.cwd")}>
        <SettingsValue>{String(hello.cwd ?? t("common.none"))}</SettingsValue>
      </SettingsRow>
      <SettingsRow label={t("settings.attaDir")} hint={t("settings.attaDirHint")}>
        <SettingsValue>{String(hello.engine?.data_root ?? t("common.none"))}</SettingsValue>
      </SettingsRow>
      <SettingsRow label={t("settings.projectsDir")} hint={t("settings.projectsDirHint")}>
        <SettingsValue>{String(shell?.projects_dir ?? t("common.none"))}</SettingsValue>
      </SettingsRow>
      <SettingsRow label={t("settings.stateDir")} hint={t("settings.stateDirHint")}>
        <SettingsValue>{String(shell?.state_dir ?? t("common.none"))}</SettingsValue>
      </SettingsRow>
      <SettingsRow label={t("settings.frameLimit")}>
        <SettingsValue>{mb(limits.max_frame_bytes)}</SettingsValue>
      </SettingsRow>
      <SettingsRow label={t("settings.uploadLimit")}>
        <SettingsValue>{mb(limits.max_upload_bytes)}</SettingsValue>
      </SettingsRow>
    </>
  );
}
