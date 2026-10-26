/**
 * The settings panel: a section list on the left, the chosen section on the right.
 *
 * The sections are whatever is registered at `settings.section`, in order; the built-in ones below
 * arrive through the same point a package's would. A section that throws loses its page, not the
 * panel: the nav stays usable.
 *
 * Engine settings are written to a tier's settings.json through `harnix.settings.set`; the engine
 * keeps a per-(project, scene) cache that a reload does not clear, so a row says when the running
 * engine still has the old value rather than pretending.
 */

import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { errorText } from "@harnix/core";
import { Contributed, localeChoice, setLocale, t, useRegistry, useT, type LocaleChoice } from "@harnix/react";

import type { App, ThemeChoice } from "./app.ts";
import { Body, CloseFoot, Foot } from "./dialogs.tsx";
import { Glyph } from "./glyph.tsx";

/** Bumped after a write, so every section reads again. */
const Reload = createContext<{ version: number; reload: () => void }>({ version: 0, reload: () => {} });

let active = "general";

export function openSettings(app: App, section?: string): void {
  if (section) active = section;
  app.openModal({ title: t("settings.title"), wide: true, content: <Settings app={app} /> });
}

function Settings({ app }: { app: App }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const [current, setCurrent] = useState(active);
  const [version, setVersion] = useState(0);
  const sections = registry.at("settings.section");
  const section = sections.find((s) => s.id === current) ?? sections[0];
  return (
    <Reload.Provider value={{ version, reload: () => setVersion((v) => v + 1) }}>
      <Body>
        <div className="settings-body">
          <div className="settings-nav">
            {sections.map((s) => (
              <button
                type="button"
                key={s.id}
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
          <div className="settings-page" key={`${section?.id}-${version}`}>
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
    </Reload.Provider>
  );
}

/* ── pieces ───────────────────────────────────────────────────────────── */

export function Row({ label, hint, children }: { label: ReactNode; hint?: ReactNode; children?: ReactNode }) {
  return (
    <div className="srow-setting">
      <div className="text">
        <div className="label">{label}</div>
        {hint ? <div className="hint">{hint}</div> : null}
      </div>
      <div className="control">{children}</div>
    </div>
  );
}

function Select({ options, value, onChange }: { options: [string, string][]; value: string; onChange: (v: string) => void }) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value)}>
      {options.map(([key, text]) => (
        <option key={key} value={key}>
          {text}
        </option>
      ))}
    </select>
  );
}

/** A titled block whose content is loaded; a late answer for a page no longer shown is dropped. */
function Section<T>({ title, load, children }: { title: string; load: () => Promise<T>; children: (data: T) => ReactNode }) {
  const t = useT();
  const { version } = useContext(Reload);
  const [state, setState] = useState<{ data?: T; error?: string } | null>(null);
  useEffect(() => {
    let alive = true;
    setState(null);
    load().then(
      (data) => alive && setState({ data }),
      (e: unknown) => alive && setState({ error: errorText(e) }),
    );
    return () => {
      alive = false;
    };
  }, [version]);
  return (
    <>
      <h3>{title}</h3>
      <div>
        {!state ? (
          <div className="hint">{t("common.loading")}</div>
        ) : state.error !== undefined ? (
          <div className="hint err">{state.error}</div>
        ) : (
          children(state.data as T)
        )}
      </div>
    </>
  );
}

function Value({ tone, children }: { tone?: "ok" | "err"; children: ReactNode }) {
  return <span className={`value${tone ? ` ${tone}` : ""}`}>{children}</span>;
}

/* ── engine settings ──────────────────────────────────────────────────── */

interface Field {
  key: string;
  kind: "choice" | "flag" | "count" | "text";
  options?: string[];
  effective?: unknown;
  tiers: Record<string, unknown>;
  source?: string | null;
  engine?: unknown;
}

interface Described {
  paths?: Record<string, string>;
  fields?: Field[];
  providers?: { providers?: Record<string, { api_type?: string; base_url?: string; api_key?: unknown; default_model?: string }> };
}

let tier = "global";

function where(app: App): { scene?: string; project_root: string | null } {
  const current = app.snapshot.current;
  const row = current ? app.snapshot.sessions.find((r) => r.session_id === current.sessionId) : null;
  return {
    scene: row?.scene ?? app.snapshot.hello?.engine?.active_scenes?.[0],
    project_root: row?.project_root ?? null,
  };
}

function EngineSettings({ app, only }: { app: App; only: string[] }) {
  const t = useT();
  const { reload } = useContext(Reload);
  const request = where(app);
  const [selected, setSelected] = useState(tier);
  return (
    <Section title={t("settings.engineDefaults")} load={() => app.call<Described>("harnix.settings.describe", request)}>
      {(data) => {
        const paths = data.paths ?? {};
        const write = (key: string, value: unknown) =>
          app
            .call("harnix.settings.set", { ...request, tier: selected, key, value })
            .then(() => {
              app.notice(t("settings.written", { path: paths[selected] }));
              setTimeout(() => app.dismissBanner(), 2500);
              reload();
            })
            .catch((e: unknown) => app.notice(t("settings.writeFailed", { error: errorText(e) }), "error"));
        const tiers = (
          [
            ["global", t("settings.tier.global")],
            ["scene", t("settings.tier.scene", { scene: request.scene })],
            ["project", t("settings.tier.project")],
          ] as [string, string][]
        ).filter(([id]) => paths[id]);
        return (
          <>
            <Row label={t("settings.tier")} hint={paths[selected] ?? ""}>
              <Select
                options={tiers}
                value={selected}
                onChange={(value) => {
                  tier = value;
                  setSelected(value);
                }}
              />
            </Row>
            {(data.fields ?? [])
              .filter((field) => only.includes(field.key))
              .map((field) => {
                const own = field.tiers?.[selected];
                const inherited = own === null || own === undefined;
                const notes = [inherited ? t("settings.inherited") : t("settings.setHere")];
                if (field.source) notes.push(t("settings.sourceTier", { tier: t(`settings.tier.${field.source}`, { scene: request.scene }) }));
                if (field.engine !== null && field.engine !== undefined && JSON.stringify(field.engine) !== JSON.stringify(field.effective)) {
                  notes.push(t("settings.engineLags", { value: JSON.stringify(field.engine) }));
                }
                return (
                  <Row key={field.key} label={t(`settings.field.${field.key}`)} hint={notes.join(" · ")}>
                    <FieldControl field={field} write={(value) => void write(field.key, value)} />
                    {!inherited ? (
                      <button type="button" className="btn sm outline" title={t("settings.clearHint")} onClick={() => void write(field.key, null)}>
                        {t("settings.clear")}
                      </button>
                    ) : null}
                  </Row>
                );
              })}
          </>
        );
      }}
    </Section>
  );
}

function FieldControl({ field, write }: { field: Field; write: (value: unknown) => void }) {
  const [text, setText] = useState(String(field.effective ?? ""));
  if (field.kind === "choice") {
    return <Select options={(field.options ?? []).map((o) => [o, o])} value={String(field.effective ?? "")} onChange={write} />;
  }
  if (field.kind === "flag") {
    return <input type="checkbox" checked={Boolean(field.effective)} onChange={(e) => write(e.target.checked)} />;
  }
  return (
    <input
      type={field.kind === "count" ? "number" : "text"}
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => write(field.kind === "count" ? Number(text) : text)}
    />
  );
}

/* ── the sections ─────────────────────────────────────────────────────── */

function General({ app }: { app: App }) {
  const t = useT();
  const [theme, setTheme] = useState<ThemeChoice>(app.snapshot.theme);
  const [locale, setLocaleChoice] = useState<LocaleChoice>(localeChoice());
  return (
    <>
      <h3>{t("settings.appearance")}</h3>
      <Row label={t("settings.theme")} hint={t("settings.themeHint")}>
        <Select
          options={[
            ["system", t("settings.theme.system")],
            ["light", t("settings.theme.light")],
            ["dark", t("settings.theme.dark")],
          ]}
          value={theme}
          onChange={(value) => {
            setTheme(value as ThemeChoice);
            app.setTheme(value as ThemeChoice);
          }}
        />
      </Row>
      <Row label={t("settings.language")} hint={t("settings.languageHint")}>
        <Select
          options={[
            ["system", t("settings.language.system")],
            ["zh-CN", "简体中文"],
            ["en", "English"],
          ]}
          value={locale}
          onChange={(value) => {
            setLocaleChoice(value as LocaleChoice);
            setLocale(value as LocaleChoice);
          }}
        />
      </Row>
      <EngineSettings app={app} only={["permission_mode", "memory_enabled", "allow_client_permission_override"]} />
    </>
  );
}

function Scenes({ app }: { app: App }) {
  const t = useT();
  return (
    <Section
      title={t("settings.scenes")}
      load={() =>
        app.call<{ scenes?: { scene: string; name?: string; active?: boolean; sessions?: number; capabilities?: { requires_project?: boolean; supports_team?: boolean } }[] }>(
          "scene.list",
          {},
        )
      }
    >
      {(data) =>
        (data.scenes ?? []).map((scene) => {
          const caps = scene.capabilities ?? {};
          return (
            <Row
              key={scene.scene}
              label={`${scene.name || scene.scene} · ${scene.scene}`}
              hint={[
                caps.requires_project ? t("settings.sceneNeedsProject") : t("settings.sceneNoProject"),
                caps.supports_team ? t("settings.sceneSupportsTeam") : null,
                t("settings.sceneSessions", { count: scene.sessions ?? 0 }),
              ]
                .filter(Boolean)
                .join(" · ")}
            >
              {scene.active ? (
                <Value tone="ok">{t("settings.sceneActive")}</Value>
              ) : (
                <button
                  type="button"
                  className="btn sm outline"
                  onClick={() =>
                    app
                      .call("scene.activate", { scene: scene.scene })
                      .then(() => {
                        app.notice(t("settings.sceneActivated", { scene: scene.scene }));
                        setTimeout(() => app.dismissBanner(), 2500);
                      })
                      .catch((e: unknown) => app.notice(errorText(e), "error"))
                  }
                >
                  {t("settings.sceneActivate")}
                </button>
              )}
            </Row>
          );
        })
      }
    </Section>
  );
}

function Models({ app }: { app: App }) {
  const t = useT();
  const { reload } = useContext(Reload);
  const request = where(app);
  return (
    <>
      <EngineSettings app={app} only={["model.model_name", "model.max_tokens"]} />
      <Section title={t("settings.providers")} load={() => app.call<Described>("harnix.settings.describe", request)}>
        {(data) => {
          const providers = data.providers?.providers ?? {};
          const names = Object.keys(providers);
          return (
            <>
              {!names.length ? <div className="hint">{t("settings.providersNone")}</div> : null}
              {names.map((id) => {
                const provider = providers[id]!;
                return (
                  <Row key={id} label={id} hint={[provider.api_type, provider.base_url, provider.default_model].filter(Boolean).join(" · ")}>
                    <Value tone={provider.api_key ? "ok" : "err"}>{provider.api_key ? t("settings.keySet") : t("settings.keyMissing")}</Value>
                    <button type="button" className="btn sm outline" onClick={() => providerDialog(app, id, provider, reload)}>
                      {t("common.edit")}
                    </button>
                  </Row>
                );
              })}
              <button type="button" className="btn sm outline" onClick={() => providerDialog(app, "", {}, reload)}>
                <Glyph name="key" className="glyph" />
                {t("settings.providerAdd")}
              </button>
              <div className="hint">{t("settings.providerHint")}</div>
            </>
          );
        }}
      </Section>
    </>
  );
}

/** Add or edit one provider — the only path credentials take from the interface. It opens over the
 *  settings panel and goes back to it. */
function providerDialog(app: App, id: string, provider: { api_type?: string; base_url?: string; default_model?: string }, _reload: () => void) {
  app.openModal({
    title: id ? t("settings.providerEdit", { id }) : t("settings.providerAdd"),
    content: <ProviderForm app={app} id={id} provider={provider} />,
  });
}

function ProviderForm({ app, id, provider }: { app: App; id: string; provider: { api_type?: string; base_url?: string; default_model?: string } }) {
  const t = useT();
  const [fields, setFields] = useState({
    provider_id: "",
    api_type: provider.api_type ?? "anthropic",
    base_url: provider.base_url ?? "",
    api_key: "",
    default_model: provider.default_model ?? "",
  });
  const back = () => openSettings(app);
  const field = (key: keyof typeof fields, label: string, type = "text") => (
    <div className="field" key={key}>
      <label>{label}</label>
      <input type={type} value={fields[key]} onChange={(e) => setFields({ ...fields, [key]: e.target.value })} />
    </div>
  );
  const save = async () => {
    const providerId = id || fields.provider_id.trim();
    if (!providerId) {
      app.notice(t("settings.providerIdRequired"));
      return;
    }
    const config: Record<string, string> = {
      api_type: fields.api_type.trim(),
      base_url: fields.base_url.trim(),
      default_model: fields.default_model.trim(),
    };
    if (fields.api_key) config.api_key = fields.api_key;
    try {
      await app.call("harnix.settings.setProvider", { provider_id: providerId, config });
      back();
    } catch (e) {
      app.notice(t("settings.writeFailed", { error: errorText(e) }), "error");
    }
  };
  return (
    <>
      <Body>
        {!id ? field("provider_id", t("settings.providerId")) : null}
        {field("api_type", t("settings.apiType"))}
        {field("base_url", t("settings.baseUrl"))}
        {field("api_key", t("settings.apiKey"), "password")}
        {field("default_model", t("settings.defaultModel"))}
        <div className="hint">{t("settings.apiKeyHint")}</div>
      </Body>
      <Foot>
        <button type="button" className="btn outline" onClick={back}>
          {t("common.cancel")}
        </button>
        {id ? (
          <button
            type="button"
            className="btn outline danger"
            onClick={() =>
              app
                .call("harnix.settings.setProvider", { provider_id: id, delete: true })
                .then(back)
                .catch((e: unknown) => app.notice(errorText(e), "error"))
            }
          >
            {t("common.delete")}
          </button>
        ) : null}
        <button type="button" className="btn primary" onClick={() => void save()}>
          {t("common.save")}
        </button>
      </Foot>
    </>
  );
}

function Mcp({ app }: { app: App }) {
  const t = useT();
  return (
    <Section title={t("settings.mcpServers")} load={() => app.call<{ servers?: { name: string; connected?: boolean; tools?: number; error?: string }[] }>("mcp.status", {})}>
      {(data) => (
        <>
          {!(data.servers ?? []).length ? <div className="hint">{t("settings.mcpNone")}</div> : null}
          {(data.servers ?? []).map((server) => (
            <Row key={server.name} label={server.name} hint={`${t("settings.mcpTools", { count: server.tools ?? 0 })}${server.error ? ` · ${server.error}` : ""}`}>
              <Value tone={server.connected ? "ok" : "err"}>{server.connected ? t("settings.mcpConnected") : t("settings.mcpDisconnected")}</Value>
            </Row>
          ))}
          <div className="hint">{t("settings.mcpAddHint")}</div>
        </>
      )}
    </Section>
  );
}

interface PluginList {
  available?: boolean;
  reason?: string;
  plugins?: { name: string; version?: string; enabled?: boolean; description?: string; script_faults?: string[]; component_faults?: string[] }[];
  contributes?: { plugin: string; ui?: { point: string; module: string }[]; inert?: string[]; notes?: string[] }[];
}

/* Plugins are Agix's. `PLUGINS_DISABLED` means this build carries no plugin subsystem at all, which
 * is a different fact from "nothing is installed", and is said as one. */
function Plugins({ app }: { app: App }) {
  const t = useT();
  const { reload } = useContext(Reload);
  const act = (method: string, params: unknown) =>
    app
      .call(method, params)
      .then(reload)
      .catch((e: unknown) => app.notice(errorText(e), "error"));
  return (
    <Section title={t("settings.plugins")} load={() => app.call<PluginList>("harnix.plugins.list", {})}>
      {(data) => {
        if (data.available === false) return <div className="hint">{data.reason || t("settings.pluginsUnavailable")}</div>;
        const plugins = data.plugins ?? [];
        return (
          <>
            <Installer app={app} />
            {!plugins.length ? <div className="hint">{t("settings.pluginsNone")}</div> : null}
            {plugins.map((plugin) => {
              const contributes = (data.contributes ?? []).find((c) => c.plugin === plugin.name);
              return (
                <div key={plugin.name}>
                  <Row label={`${plugin.name} ${plugin.version ?? ""}`} hint={plugin.description ?? ""}>
                    <button
                      type="button"
                      className="btn sm outline"
                      onClick={() => void act(plugin.enabled ? "harnix.plugins.disable" : "harnix.plugins.enable", { name: plugin.name })}
                    >
                      {plugin.enabled ? t("settings.pluginDisable") : t("settings.pluginEnable")}
                    </button>
                    <button
                      type="button"
                      className="btn sm danger"
                      onClick={() => {
                        if (confirm(t("settings.pluginRemoveConfirm", { name: plugin.name }))) void act("harnix.plugins.uninstall", { name: plugin.name });
                      }}
                    >
                      {t("common.remove")}
                    </button>
                  </Row>
                  {(contributes?.ui ?? []).map((entry) => (
                    <Row key={entry.module} label={t("settings.pluginContributes")} hint={entry.module}>
                      <Value>{entry.point}</Value>
                    </Row>
                  ))}
                  {(contributes?.inert ?? []).map((reason) => (
                    <Row key={reason} label={t("settings.pluginInert")} hint={reason} />
                  ))}
                  {(contributes?.notes ?? []).map((note) => (
                    <Row key={note} label={t("settings.pluginNote")} hint={note} />
                  ))}
                  {[...(plugin.script_faults ?? []), ...(plugin.component_faults ?? [])].map((fault) => (
                    <Row key={fault} label={t("settings.pluginFault")} hint={fault} />
                  ))}
                </div>
              );
            })}
          </>
        );
      }}
    </Section>
  );
}

/* Agix fetches packages itself and has no upload channel; this sends the .zip up the bulk channel
 * and asks the engine to install it from where it landed. The file is not opened on this side. */
function Installer({ app }: { app: App }) {
  const t = useT();
  const pick = useRef<HTMLInputElement>(null);
  const [status, setStatus] = useState<{ text: string; disclosure?: string } | null>(null);
  const install = async (file: File) => {
    setStatus({ text: t("settings.pluginUploading", { name: file.name }) });
    try {
      const grant = await app.call<{ url: string; path: string }>("harnix.plugins.upload", { name: file.name });
      const response = await fetch(grant.url, { method: "POST", body: file });
      if (!response.ok) throw new Error(await response.text());
      setStatus({ text: t("settings.pluginInstalling") });
      const result = await app.call<{ message?: string; disclosure?: unknown }>("harnix.plugins.install", { path: grant.path, name: file.name });
      setStatus({
        text: result.message || t("settings.pluginInstalled"),
        disclosure:
          result.disclosure == null ? undefined : typeof result.disclosure === "string" ? result.disclosure : JSON.stringify(result.disclosure, null, 2),
      });
    } catch (e) {
      setStatus({ text: errorText(e) });
    }
  };
  return (
    <div>
      <input
        ref={pick}
        type="file"
        accept=".zip"
        hidden
        onChange={(e) => {
          const file = e.target.files?.[0];
          e.target.value = "";
          if (file) void install(file);
        }}
      />
      <button type="button" className="btn sm outline" onClick={() => pick.current?.click()}>
        {t("settings.pluginInstall")}
      </button>
      <div className="hint">{t("settings.pluginInstallHint")}</div>
      {status ? (
        <div className="hint">
          {status.text}
          {status.disclosure ? <pre>{status.disclosure}</pre> : null}
        </div>
      ) : null}
    </div>
  );
}

/* The pairing code exists in the clear only here, once; it is read off and typed into the other
 * machine, hence large, monospaced and counting down. */
function Devices({ app }: { app: App }) {
  const t = useT();
  const { reload } = useContext(Reload);
  const [grant, setGrant] = useState<{ code: string; left: number } | { error: string } | null>(null);
  useEffect(() => {
    if (!grant || !("code" in grant) || grant.left <= 0) return;
    const tick = setTimeout(() => setGrant({ ...grant, left: grant.left - 1 }), 1000);
    return () => clearTimeout(tick);
  }, [grant]);
  return (
    <Section title={t("settings.devices")} load={() => app.call<{ devices?: { id: string; label?: string }[] }>("harnix.devices.list", {})}>
      {(data) => (
        <>
          <div className="hint">{t("settings.devicesHint")}</div>
          {!(data.devices ?? []).length ? <div className="hint">{t("settings.devicesNone")}</div> : null}
          {(data.devices ?? []).map((device) => (
            <Row key={device.id} label={device.label || device.id} hint={device.id}>
              <button
                type="button"
                className="btn sm danger"
                onClick={() => {
                  if (!confirm(t("settings.deviceRevokeConfirm", { name: device.label || device.id }))) return;
                  app
                    .call("harnix.devices.revoke", { device_id: device.id })
                    .then(reload)
                    .catch((e: unknown) => app.notice(errorText(e), "error"));
                }}
              >
                {t("settings.deviceRevoke")}
              </button>
            </Row>
          ))}
          <button
            type="button"
            className="btn sm outline"
            onClick={() =>
              app
                .call<{ code: string; expires_in_secs: number }>("harnix.devices.pair.begin", { label: t("settings.deviceNew") })
                .then((g) => setGrant({ code: g.code, left: g.expires_in_secs }))
                .catch((e: unknown) => setGrant({ error: errorText(e) }))
            }
          >
            {t("settings.devicePair")}
          </button>
          {grant && "error" in grant ? <div className="hint">{grant.error}</div> : null}
          {grant && "code" in grant ? (
            grant.left > 0 ? (
              <div>
                <div className="pairing-code">{grant.code}</div>
                <div className="hint">{t("settings.devicePairHint")}</div>
                <span className="hint">
                  {t("settings.devicePairLeft", { minutes: Math.floor(grant.left / 60), seconds: String(grant.left % 60).padStart(2, "0") })}
                </span>
              </div>
            ) : (
              <div className="hint">{t("settings.devicePairExpired")}</div>
            )
          ) : null}
        </>
      )}
    </Section>
  );
}

export function Diagnostics({ app }: { app: App }) {
  const t = useT();
  const registry = useRegistry(app.registry);
  const [report, setReport] = useState<{ running: true } | { error: string } | { ok?: boolean; checks?: { scope: string; name: string; ok: boolean; detail?: string }[] } | null>(null);
  const refused = registry.refusals();
  return (
    <>
      <Section title={t("settings.engine")} load={() => app.call<{ version?: string; uptime_secs?: number; sessions?: number }>("daemon.status", {})}>
        {(data) => (
          <>
            <Row label={t("settings.version")}>
              <Value>{data.version || t("common.none")}</Value>
            </Row>
            <Row label={t("settings.uptime")}>
              <Value>{t("settings.uptimeValue", { minutes: Math.round((data.uptime_secs ?? 0) / 60) })}</Value>
            </Row>
            <Row label={t("settings.sessionCount")}>
              <Value>{String(data.sessions ?? "—")}</Value>
            </Row>
            <Row label={t("settings.activeScenes")}>
              <Value>{(app.snapshot.hello?.engine?.active_scenes ?? []).join(" · ") || t("common.none")}</Value>
            </Row>
          </>
        )}
      </Section>
      <RuntimeModules app={app} />
      <h3>{t("settings.contributions")}</h3>
      {!refused.length ? (
        <div className="hint">{t("contrib.noneRefused")}</div>
      ) : (
        refused.map((entry, i) => (
          <Row key={i} label={t("contrib.refusedAt", { point: entry.point })} hint={entry.id ? `${entry.id} — ${entry.reason}` : entry.reason}>
            <Value>{entry.owner}</Value>
          </Row>
        ))
      )}
      <h3>{t("settings.doctor")}</h3>
      <button
        type="button"
        className="btn sm outline"
        onClick={() => {
          setReport({ running: true });
          app
            .call<{ ok?: boolean; checks?: { scope: string; name: string; ok: boolean; detail?: string }[] }>("daemon.doctor", {})
            .then(setReport)
            .catch((e: unknown) => setReport({ error: errorText(e) }));
        }}
      >
        <Glyph name="refresh" className="glyph" />
        {t("settings.doctorRun")}
      </button>
      {report && "running" in report ? <div className="hint">{t("common.running")}</div> : null}
      {report && "error" in report ? <div className="hint err">{report.error}</div> : null}
      {report && "checks" in report ? (
        <>
          <div className="hint">{report.ok ? t("settings.doctorOk") : t("settings.doctorFailed")}</div>
          {(report.checks ?? []).map((check) => (
            <Row key={`${check.scope}-${check.name}`} label={`${check.scope} · ${check.name}`} hint={check.detail ?? ""}>
              <Value tone={check.ok ? "ok" : "err"}>{check.ok ? t("settings.checkPass") : t("settings.checkFail")}</Value>
            </Row>
          ))}
        </>
      ) : null}
    </>
  );
}

/**
 * How installed packages reach the interface (design §10.5): the contribution API this shell
 * speaks, whether runtime modules load at all, the import map they resolve `react` and
 * `@harnix/react` through and the CSP line that admits it, and what each package entry became.
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
      <Row label={t("diag.contribApi")} hint={t("diag.contribApiHint")}>
        <Value>{`v${hello?.contrib_api_version ?? "?"}`}</Value>
      </Row>
      <Row label="[ui] runtime_modules" hint={t("diag.runtimeHint")}>
        <Value tone={on ? "ok" : undefined}>{on ? "true" : "false"}</Value>
      </Row>
      <Row label={t("diag.importMap")} hint={t("diag.importMapHint")} />
      <div className="kv-table" data-testid="import-map">
        {Object.entries(imports).map(([name, path]) => (
          <div key={name} className="kv-row">
            <code>{name}</code>
            <span className="muted">{path}</span>
          </div>
        ))}
      </div>
      <Row label="Content-Security-Policy" hint={t("diag.cspHint")} />
      {csp ? <pre className="hx-pre" data-testid="csp">{csp}</pre> : null}
      <Section title={t("diag.packageEntries")} load={() => app.call<PluginList>("harnix.plugins.list", {})}>
        {(data) => {
          const rows = (data.contributes ?? []).flatMap((c) => [
            ...(c.ui ?? []).map((entry) => ({ plugin: c.plugin, what: `${entry.point} → ${entry.module}`, state: on ? "on" : "off" })),
            ...(c.inert ?? []).map((reason) => ({ plugin: c.plugin, what: reason, state: "inert" })),
          ]);
          if (!rows.length) return <div className="hint">{t("diag.noEntries")}</div>;
          return (
            <>
              {rows.map((row, i) => (
                <Row key={i} label={row.plugin} hint={row.what}>
                  <Value tone={row.state === "on" ? "ok" : row.state === "inert" ? "err" : undefined}>{t(`diag.entry.${row.state}`)}</Value>
                </Row>
              ))}
            </>
          );
        }}
      </Section>
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
      <Row label={t("about.product")}>
        <Value>{shell?.brand?.name || t("app.title")}</Value>
      </Row>
      <Row label="product_id" hint={t("about.productIdHint")}>
        <Value>{shell?.brand?.product_id ?? "harnix"}</Value>
      </Row>
      <Row label={t("settings.protocolVersion")}>
        <Value>{String(hello.protocol_version ?? "—")}</Value>
      </Row>
      <Row label={t("settings.cwd")}>
        <Value>{String(hello.cwd ?? t("common.none"))}</Value>
      </Row>
      <Row label={t("settings.attaDir")} hint={t("settings.attaDirHint")}>
        <Value>{String(hello.engine?.data_root ?? t("common.none"))}</Value>
      </Row>
      <Row label={t("settings.projectsDir")} hint={t("settings.projectsDirHint")}>
        <Value>{String(shell?.projects_dir ?? t("common.none"))}</Value>
      </Row>
      <Row label={t("settings.stateDir")} hint={t("settings.stateDirHint")}>
        <Value>{String(shell?.state_dir ?? t("common.none"))}</Value>
      </Row>
      <Row label={t("settings.frameLimit")}>
        <Value>{mb(limits.max_frame_bytes)}</Value>
      </Row>
      <Row label={t("settings.uploadLimit")}>
        <Value>{mb(limits.max_upload_bytes)}</Value>
      </Row>
    </>
  );
}

/** The built-in sections, registered at `settings.section` in this order. */
export function builtinSections(app: App) {
  const section = (id: string, label: string, render: () => ReactNode) => ({ id, label: () => t(label), render });
  return [
    section("general", "settings.nav.general", () => <General app={app} />),
    section("scenes", "settings.nav.scenes", () => <Scenes app={app} />),
    section("models", "settings.nav.models", () => <Models app={app} />),
    section("mcp", "settings.nav.mcp", () => <Mcp app={app} />),
    section("plugins", "settings.nav.plugins", () => <Plugins app={app} />),
    section("devices", "settings.nav.devices", () => <Devices app={app} />),
    section("diagnostics", "settings.nav.diagnostics", () => <Diagnostics app={app} />),
    section("about", "settings.nav.about", () => <About app={app} />),
  ];
}
