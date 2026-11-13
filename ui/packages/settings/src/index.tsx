/**
 * @harnix/settings — the settings sections for the engine (the `settings` extension's interface):
 * its defaults, the scenes, the model providers and the MCP servers.
 *
 * Engine settings are written to a tier's settings.json through `settings.set`; the engine keeps a
 * per-(project, scene) cache that a reload does not clear, so a row says when the running engine
 * still has the old value rather than pretending. Providers go through `settings.setProvider`, the
 * only path credentials take from the interface.
 */

import { useState, type ReactNode } from "react";
import { errorText, type Hello } from "@harnix/core";
import { Glyph, SettingsRow, SettingsSection, SettingsValue, t, useHello, useSettingsReload, useT, type HostApi, type Registry, type SettingsNeed } from "@harnix/react";

/** The backend extension this is the interface of. */
export const extension = "settings";

/** The sections, at `settings.section`, in this order. */
export function register(r: Registry, host: HostApi): void {
  const section = (id: string, label: string, render: () => ReactNode, covers?: SettingsNeed[]) =>
    r.register("settings.section", { id, label: () => t(label), render, ...(covers ? { covers } : {}) }, extension);
  section("engine", "settings.engineDefaults", () => <EngineSettings host={host} only={["permission_mode", "memory_enabled", "allow_client_permission_override"]} />);
  section("scenes", "settings.nav.scenes", () => <Scenes host={host} />);
  // Where the shell sends a reader when a turn has no model to go to.
  section("models", "settings.nav.models", () => <Models host={host} />, ["models"]);
  section("mcp", "settings.nav.mcp", () => <Mcp host={host} />);
}

/* ── pieces ───────────────────────────────────────────────────────────── */

function Select({ options, value, onChange, testId }: { options: [string, string][]; value: string; onChange: (v: string) => void; testId?: string }) {
  return (
    <select value={value} data-testid={testId} onChange={(e) => onChange(e.target.value)}>
      {options.map(([key, text]) => (
        <option key={key} value={key}>
          {text}
        </option>
      ))}
    </select>
  );
}

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
  providers?: {
    providers?: Record<string, { api_type?: string | null; base_url?: string | null; api_key?: unknown; default_model?: string | null; models?: string[] }>;
    default_provider?: string | null;
  };
}

let tier = "global";

/** What a write is about: the open session's scene and project, else the engine's first active
 *  scene and no project. */
function where(host: HostApi, hello: Hello | null): { scene?: string; project_root: string | null } {
  const session = host.session.current() ? host.session.context()?.session : null;
  const scene = typeof session?.scene === "string" ? session.scene : undefined;
  const root = typeof session?.project_root === "string" ? session.project_root : null;
  return { scene: scene ?? hello?.engine?.active_scenes?.[0], project_root: root };
}

function EngineSettings({ host, only }: { host: HostApi; only: string[] }) {
  const t = useT();
  const reload = useSettingsReload();
  const hello = useHello();
  const request = where(host, hello);
  const [selected, setSelected] = useState(tier);
  return (
    <SettingsSection title={t("settings.engineDefaults")} load={() => host.call<Described>("settings.describe", request)}>
      {(data) => {
        const paths = data.paths ?? {};
        const write = (key: string, value: unknown) =>
          host
            .call("settings.set", { ...request, tier: selected, key, value })
            .then(() => {
              host.shell.notice(t("settings.written", { path: paths[selected] }));
              reload();
            })
            .catch((e: unknown) => host.shell.notice(t("settings.writeFailed", { error: errorText(e) }), "error"));
        const tiers = (
          [
            ["global", t("settings.tier.global")],
            ["scene", t("settings.tier.scene", { scene: request.scene })],
            ["project", t("settings.tier.project")],
          ] as [string, string][]
        ).filter(([id]) => paths[id]);
        return (
          <>
            <SettingsRow label={t("settings.tier")} hint={paths[selected] ?? ""}>
              <Select
                options={tiers}
                value={selected}
                onChange={(value) => {
                  tier = value;
                  setSelected(value);
                }}
              />
            </SettingsRow>
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
                  <SettingsRow key={field.key} label={t(`settings.field.${field.key}`)} hint={notes.join(" · ")}>
                    <FieldControl field={field} write={(value) => void write(field.key, value)} />
                    {!inherited ? (
                      <button type="button" className="btn sm outline" title={t("settings.clearHint")} onClick={() => void write(field.key, null)}>
                        {t("settings.clear")}
                      </button>
                    ) : null}
                  </SettingsRow>
                );
              })}
          </>
        );
      }}
    </SettingsSection>
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

function Scenes({ host }: { host: HostApi }) {
  const t = useT();
  return (
    <SettingsSection
      title={t("settings.scenes")}
      load={() =>
        host.call<{ scenes?: { scene: string; name?: string; active?: boolean; sessions?: number; capabilities?: { requires_project?: boolean; supports_team?: boolean } }[] }>(
          "engine.scene.list",
          {},
        )
      }
    >
      {(data) =>
        (data.scenes ?? []).map((scene) => {
          const caps = scene.capabilities ?? {};
          return (
            <SettingsRow
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
                <SettingsValue tone="ok">{t("settings.sceneActive")}</SettingsValue>
              ) : (
                <button
                  type="button"
                  className="btn sm outline"
                  onClick={() =>
                    host
                      .call("engine.scene.activate", { scene: scene.scene })
                      .then(() => {
                        host.shell.notice(t("settings.sceneActivated", { scene: scene.scene }));
                      })
                      .catch((e: unknown) => host.shell.notice(errorText(e), "error"))
                  }
                >
                  {t("settings.sceneActivate")}
                </button>
              )}
            </SettingsRow>
          );
        })
      }
    </SettingsSection>
  );
}

/**
 * The model providers: as many as wanted, each an Anthropic or OpenAI-compatible API, one of them the
 * default. Added and edited in place, so the panel stays the panel; written to the global tier.
 */
function Models({ host }: { host: HostApi }) {
  const t = useT();
  const reload = useSettingsReload();
  const hello = useHello();
  const [editing, setEditing] = useState<string | null>(null);
  const request = where(host, hello);
  // The shell reads the providers again when the panel closes: the composer's model menu follows.
  const saved = () => {
    setEditing(null);
    reload();
  };
  return (
    <>
      <SettingsSection title={t("settings.providers")} load={() => host.call<Described>("settings.describe", request)}>
        {(data) => {
          const providers = data.providers?.providers ?? {};
          const fallback = data.providers?.default_provider ?? null;
          const names = Object.keys(providers);
          return (
            <div className="providers" data-testid="providers">
              {!names.length ? (
                <div className={`callout${hello?.engine?.has_credentials === true ? "" : " warn"}`} data-testid="providers-none">
                  <Glyph name="key" className="glyph" />
                  <span>{t("settings.providersNone")}</span>
                </div>
              ) : null}
              {names.map((id) =>
                editing === id ? (
                  <ProviderForm key={id} host={host} id={id} provider={providers[id]!} isDefault={fallback === id} path={data.paths?.global} onDone={saved} onCancel={() => setEditing(null)} />
                ) : (
                  <ProviderCard key={id} host={host} id={id} provider={providers[id]!} isDefault={fallback === id} onEdit={() => setEditing(id)} onChanged={saved} />
                ),
              )}
              {editing === "" ? (
                <ProviderForm host={host} id="" provider={{}} isDefault={!names.length} path={data.paths?.global} onDone={saved} onCancel={() => setEditing(null)} />
              ) : (
                <button type="button" className="btn sm outline add-provider" data-testid="provider-add" onClick={() => setEditing("")}>
                  <Glyph name="plus" className="glyph" />
                  {t("settings.providerAdd")}
                </button>
              )}
              <div className="hint">{t("settings.providerHint")}</div>
            </div>
          );
        }}
      </SettingsSection>
      <EngineSettings host={host} only={["model.model_name", "model.max_tokens"]} />
    </>
  );
}

type ProviderEntry = NonNullable<NonNullable<Described["providers"]>["providers"]>[string];

const API_TYPES = [
  { id: "anthropic", label: "settings.apiTypeAnthropic", url: "https://api.anthropic.com", model: "claude-sonnet-4-6" },
  { id: "openai_compatible", label: "settings.apiTypeOpenai", url: "https://api.openai.com/v1", model: "gpt-4.1" },
] as const;

function apiTypeOf(provider: ProviderEntry) {
  return API_TYPES.find((x) => x.id === provider.api_type) ?? API_TYPES[0];
}

function ProviderCard({
  host,
  id,
  provider,
  isDefault,
  onEdit,
  onChanged,
}: {
  host: HostApi;
  id: string;
  provider: ProviderEntry;
  isDefault: boolean;
  onEdit: () => void;
  onChanged: () => void;
}) {
  const t = useT();
  const [confirming, setConfirming] = useState(false);
  const change = (params: Record<string, unknown>) =>
    host
      .call("settings.setProvider", { provider_id: id, ...params })
      .then(onChanged)
      .catch((e: unknown) => host.shell.notice(t("settings.writeFailed", { error: errorText(e) }), "error"));
  const models = [provider.default_model, ...(provider.models ?? [])].filter((m, i, all): m is string => Boolean(m) && all.indexOf(m) === i);
  return (
    <div className={`provider-card${isDefault ? " default" : ""}`} data-provider={id} data-state={isDefault ? "default" : undefined}>
      <div className="pc-head">
        <span className="pc-name">{id}</span>
        <span className="pill">{t(apiTypeOf(provider).label)}</span>
        {isDefault ? <span className="pill acc">{t("settings.providerDefault")}</span> : null}
        <span className="sp" />
        <SettingsValue tone={provider.api_key ? "ok" : "err"}>{provider.api_key ? t("settings.keySet") : t("settings.keyMissing")}</SettingsValue>
      </div>
      <div className="pc-meta">
        <span className="mono truncate">{provider.base_url || apiTypeOf(provider).url}</span>
        {models.length ? <span className="truncate">{models.join(" · ")}</span> : null}
      </div>
      <div className="pc-actions">
        {!isDefault ? (
          <button type="button" className="btn sm ghost" data-testid="provider-default" onClick={() => void change({ default_provider: id })}>
            {t("settings.providerMakeDefault")}
          </button>
        ) : null}
        <button type="button" className="btn sm ghost" data-testid="provider-edit" onClick={onEdit}>
          {t("common.edit")}
        </button>
        <button
          type="button"
          className={`btn sm ${confirming ? "outline danger" : "ghost"}`}
          data-testid="provider-delete"
          title={confirming ? t("settings.providerDeleteConfirm", { id }) : undefined}
          onBlur={() => setConfirming(false)}
          onClick={() => (confirming ? void change({ delete: true }) : setConfirming(true))}
        >
          {confirming ? t("settings.providerDeleteConfirm", { id }) : t("common.delete")}
        </button>
      </div>
    </div>
  );
}

/** Add (no id yet) or edit one provider — the only path credentials take from the interface. */
function ProviderForm({
  host,
  id,
  provider,
  isDefault,
  path,
  onDone,
  onCancel,
}: {
  host: HostApi;
  id: string;
  provider: ProviderEntry;
  isDefault: boolean;
  path?: string;
  onDone: () => void;
  onCancel: () => void;
}) {
  const t = useT();
  const [fields, setFields] = useState({
    provider_id: "",
    api_type: apiTypeOf(provider).id as string,
    base_url: provider.base_url ?? "",
    api_key: "",
    default_model: provider.default_model ?? "",
    models: (provider.models ?? []).join(", "),
    make_default: isDefault,
  });
  const [saving, setSaving] = useState(false);
  const type = API_TYPES.find((x) => x.id === fields.api_type) ?? API_TYPES[0];
  const set = (key: keyof typeof fields, value: string | boolean) => setFields({ ...fields, [key]: value });
  const text = (key: "provider_id" | "base_url" | "api_key" | "default_model" | "models", label: string, placeholder?: string, hint?: string) => (
    <div className="field" key={key}>
      <label>{label}</label>
      <input
        type={key === "api_key" ? "password" : "text"}
        data-testid={`provider-${key}`}
        value={fields[key]}
        placeholder={placeholder}
        autoComplete="off"
        spellCheck={false}
        onChange={(e) => set(key, e.target.value)}
      />
      {hint ? <div className="hint">{hint}</div> : null}
    </div>
  );
  const save = async () => {
    const providerId = id || fields.provider_id.trim();
    if (!providerId) {
      host.shell.notice(t("settings.providerIdRequired"));
      return;
    }
    const models = fields.models.split(/[,，\s]+/).map((m) => m.trim()).filter(Boolean);
    const config: Record<string, unknown> = {
      api_type: fields.api_type,
      base_url: fields.base_url.trim(),
      default_model: fields.default_model.trim(),
      models: models.length ? models : null,
    };
    if (fields.api_key) config.api_key = fields.api_key;
    setSaving(true);
    try {
      await host.call("settings.setProvider", {
        provider_id: providerId,
        config,
        ...(fields.make_default ? { default_provider: providerId } : {}),
      });
      onDone();
    } catch (e) {
      host.shell.notice(t("settings.writeFailed", { error: errorText(e) }), "error");
      setSaving(false);
    }
  };
  return (
    <form
      className="provider-form"
      data-testid="provider-form"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <div className="pf-title">{id ? t("settings.providerEdit", { id }) : t("settings.providerAdd")}</div>
      {!id ? text("provider_id", t("settings.providerId"), type.id === "anthropic" ? "anthropic" : "openai") : null}
      <div className="field">
        <label>{t("settings.apiType")}</label>
        <div className="seg" role="radiogroup">
          {API_TYPES.map((x) => (
            <button
              type="button"
              role="radio"
              key={x.id}
              data-testid={`provider-type-${x.id}`}
              aria-checked={fields.api_type === x.id}
              className={fields.api_type === x.id ? "on" : ""}
              onClick={() => set("api_type", x.id)}
            >
              {t(x.label)}
            </button>
          ))}
        </div>
      </div>
      {text("base_url", t("settings.baseUrl"), type.url)}
      {text("api_key", t("settings.apiKey"), id && provider.api_key ? t("settings.apiKeyKeep") : "sk-…", t("settings.apiKeyHint", { path: path ?? "~/.agix/settings.json" }))}
      {text("default_model", t("settings.defaultModel"), type.model)}
      {text("models", t("settings.models"), undefined, t("settings.modelsHint"))}
      <label className="check">
        <input type="checkbox" data-testid="provider-make-default" checked={fields.make_default} onChange={(e) => set("make_default", e.target.checked)} />
        {t("settings.providerAsDefault")}
      </label>
      <div className="pf-actions">
        <button type="button" className="btn sm outline" onClick={onCancel}>
          {t("common.cancel")}
        </button>
        <button type="submit" className="btn sm primary" data-testid="provider-save" disabled={saving}>
          {t("common.save")}
        </button>
      </div>
    </form>
  );
}

function Mcp({ host }: { host: HostApi }) {
  const t = useT();
  return (
    <SettingsSection title={t("settings.mcpServers")} load={() => host.call<{ servers?: { name: string; connected?: boolean; tools?: number; error?: string }[] }>("engine.mcp.status", {})}>
      {(data) => (
        <>
          {!(data.servers ?? []).length ? <div className="hint">{t("settings.mcpNone")}</div> : null}
          {(data.servers ?? []).map((server) => (
            <SettingsRow key={server.name} label={server.name} hint={`${t("settings.mcpTools", { count: server.tools ?? 0 })}${server.error ? ` · ${server.error}` : ""}`}>
              <SettingsValue tone={server.connected ? "ok" : "err"}>{server.connected ? t("settings.mcpConnected") : t("settings.mcpDisconnected")}</SettingsValue>
            </SettingsRow>
          ))}
          <div className="hint">{t("settings.mcpAddHint")}</div>
        </>
      )}
    </SettingsSection>
  );
}
