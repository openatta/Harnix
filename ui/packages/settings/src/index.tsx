/**
 * @harnix/settings — the settings sections for the engine (the `settings` extension's interface):
 * its defaults, the scenes, the model providers and the models by task, the MCP servers, the
 * permissions, the sandbox and the network, privacy, and the advanced values it only shows.
 *
 * Engine settings are drawn from the fields `settings.describe` declares and written to a tier's
 * settings.json through `settings.write`, against the revision they were read at; the value a row
 * shows is the engine's own for the open (project, scene), and it says when the project tier wrote
 * one the engine does not use (an untrusted project can only tighten). A field's `section` is
 * which of the sections below draws it. Providers go through `settings.setProvider`, the only path
 * credentials take from the interface; a stored key is only ever shown as its last four characters.
 * MCP servers go through `settings.mcp.*` (`./mcp.tsx`), with the same rule for their values.
 *
 * And the first run's model step on the empty page (`home.card`): drawn while no model is
 * configured, with the way to these sections.
 */

import { useEffect, useState, type ReactNode } from "react";
import { errorText, modelConfigured, type Hello } from "@harnix/core";
import { FieldForm, Glyph, SettingsRow, SettingsSection, SettingsValue, SetupStep, onLocaleChoice, setupStepDismissed, t, useHello, useSetupStep, useSettingsReload, useT, type FieldDoc, type FieldStore, type HostApi, type Registry, type LocaleChoice, type SettingsNeed } from "@harnix/react";

import { Mcp } from "./mcp.tsx";

export { givenOf, mcpState, signIn } from "./mcp.tsx";

/** The backend extension this is the interface of. */
export const extension = "settings";

/** The first-run step this package owns. */
export const MODEL_STEP = "settings.setup.model";

/** The sections, at `settings.section`, in this order. */
export function register(r: Registry, host: HostApi): void {
  const section = (id: string, label: string, render: () => ReactNode, covers?: SettingsNeed[]) =>
    r.register("settings.section", { id, label: () => t(label), render, ...(covers ? { covers } : {}) }, extension);
  section("engine", "settings.engineDefaults", () => <EngineSettings host={host} section="engine" />);
  section("scenes", "settings.nav.scenes", () => <Scenes host={host} />);
  // Where the shell sends a reader when a turn has no model to go to.
  section("models", "settings.nav.models", () => <Models host={host} />, ["models"]);
  section("task-models", "settings.nav.taskModels", () => <EngineSettings host={host} section="task-models" title="settings.taskModels" />);
  section("mcp", "settings.nav.mcp", () => <McpSection host={host} />);
  // permissions, sandbox, network
  section("permissions", "settings.nav.permissions", () => <EngineSettings host={host} section="permissions" title="settings.nav.permissions" />);
  section("sandbox", "settings.nav.sandbox", () => <EngineSettings host={host} section="sandbox" title="settings.nav.sandbox" />);
  section("network", "settings.nav.network", () => <Network host={host} />);
  // privacy & advanced
  section("privacy", "settings.nav.privacy", () => <Privacy host={host} />);
  section("advanced", "settings.nav.advanced", () => <Advanced host={host} />);
  followLanguage(host);
  // First of the first-run steps: nothing runs without a model.
  r.register("home.card", { id: MODEL_STEP, order: 10, render: () => <ModelStep host={host} /> }, extension);
}

let unfollow: (() => void) | null = null;

/**
 * The language chosen in the shell's general section is the engine's too (`language`, the global
 * tier): what it answers in follows what the interface speaks. `auto` gives the setting back.
 */
function followLanguage(host: HostApi): void {
  unfollow?.();
  unfollow = onLocaleChoice((choice: LocaleChoice) => {
    host
      .call("settings.write", { tier: "global", changes: { language: choice === "auto" ? null : choice } })
      .catch((e: unknown) => host.shell.notice(t("settings.writeFailed", { error: errorText(e) }), "error"));
  });
}

/* ── first run ────────────────────────────────────────────────────────── */

/** The model a new session runs on, as hello says: the default provider's, else the engine's. */
function modelOf(engine: Hello["engine"]): string | null {
  const providers = engine.providers;
  const chosen = providers?.default_provider ? providers.providers?.[providers.default_provider] : undefined;
  return chosen?.default_model || (typeof engine.model === "string" ? engine.model : null);
}

/**
 * Whether a turn has a model to go to, read from hello when the card is drawn and again each time a
 * dialog closes (the settings panel is where one is set up): done once it has.
 */
export function ModelStep({ host }: { host: HostApi }) {
  const t = useT();
  const [engine, setEngine] = useState<Hello["engine"] | null>(null);
  useEffect(() => {
    if (setupStepDismissed(MODEL_STEP)) return;
    let alive = true;
    const read = () =>
      host
        .call<Hello>("harnix.hello", {})
        .then((hello) => {
          if (alive && hello?.engine) setEngine(hello.engine);
        })
        .catch(() => undefined);
    void read();
    const off = host.modal.onChange((open) => {
      if (!open) void read();
    });
    return () => {
      alive = false;
      off();
    };
  }, [host]);
  const state = useSetupStep(MODEL_STEP, engine ? modelConfigured(engine) : null);
  const model = engine ? modelOf(engine) : null;
  return (
    <SetupStep
      id={MODEL_STEP}
      state={state}
      icon="key"
      title={t("setup.model.title")}
      actions={
        state === "todo" ? (
          <button type="button" className="btn sm primary" data-testid="setup-model-open" onClick={() => host.settings.open("models")}>
            {t("setup.model.action")}
          </button>
        ) : null
      }
    >
      {state === "todo" ? t("setup.model.todo") : model ? t("setup.model.done", { model }) : t("setup.model.doneBare")}
    </SetupStep>
  );
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

interface Described extends FieldDoc {
  paths?: Record<string, string>;
  providers?: {
    providers?: Record<
      string,
      {
        api_type?: string | null;
        base_url?: string | null;
        api_key?: unknown;
        api_key_env?: string | null;
        default_model?: string | null;
        models?: ModelEntry[];
        thinking_toggle?: string | null;
        reasoning_passback?: boolean;
      }
    >;
    default_provider?: string | null;
  };
}

let tier = "global";

/** A provider's `models` entry: a model's id, or an object with its id and what the engine knows of it. */
type ModelEntry = string | ({ id: string } & Record<string, unknown>);

const modelId = (m: ModelEntry): string => (typeof m === "string" ? m : m.id);

/** What a write is about: the open session's scene and project, else the engine's first active
 *  scene and no project. */
function where(host: HostApi, hello: Hello | null): { scene?: string; project_root: string | null } {
  const session = host.session.current() ? host.session.context()?.session : null;
  const scene = typeof session?.scene === "string" ? session.scene : undefined;
  const root = typeof session?.project_root === "string" ? session.project_root : null;
  return { scene: scene ?? hello?.engine?.active_scenes?.[0], project_root: root };
}

/** The engine's settings as a field store: read for (scene, project), written into one tier. */
export function engineStore(host: HostApi, request: { scene?: string; project_root: string | null }, tier: string): FieldStore<Described> {
  return {
    load: () => host.call<Described>("settings.describe", request),
    save: (changes, revision) => host.call("settings.write", { ...request, tier, changes, ...(revision ? { revision } : {}) }),
  };
}

/** The settings file of each tier, and a way to it: the path to copy, to open where the person's
 *  own editor and file manager are. Nothing here reveals a file itself. */
function SettingsFiles({ paths, name }: { paths: Record<string, string | null | undefined>; name: (tier: string) => string }) {
  const t = useT();
  const [copied, setCopied] = useState<string | null>(null);
  const copy = (id: string, path: string) => {
    const clip = typeof navigator !== "undefined" ? navigator.clipboard : undefined;
    if (!clip) return;
    clip.writeText(path).then(
      () => {
        setCopied(id);
        setTimeout(() => setCopied((now) => (now === id ? null : now)), 1200);
      },
      () => undefined,
    );
  };
  return (
    <>
      {(["global", "scene", "project"] as const)
        .filter((id) => paths[id])
        .map((id) => (
          <SettingsRow key={id} setting={`file-${id}`} label={t("settings.file", { tier: name(id) })} hint={<code data-testid={`settings-file-${id}`}>{paths[id]}</code>}>
            <button type="button" className="btn sm outline" data-testid={`settings-file-copy-${id}`} onClick={() => copy(id, paths[id]!)}>
              {copied === id ? t("settings.fileCopied") : t("settings.fileCopy")}
            </button>
          </SettingsRow>
        ))}
    </>
  );
}

/** The engine's fields of one section, with the tier they are written to; headed `title` (a text
 *  key). `only` fixes the tier: what the section's fields are written in, for fields only one tier
 *  may have. */
function EngineSettings({ host, section, title = "settings.engineDefaults", only }: { host: HostApi; section: string; title?: string; only?: string }) {
  const t = useT();
  const reload = useSettingsReload();
  const hello = useHello();
  const request = where(host, hello);
  const [chosen, setSelected] = useState(tier);
  const selected = only ?? chosen;
  const name = (id: string) => t(`settings.tier.${id}`, { scene: request.scene });
  return (
    <FieldForm<Described>
      title={t(title)}
      store={engineStore(host, request, selected)}
      section={section}
      tier={selected}
      tierName={name}
      header={(data) => {
        const paths = data.paths ?? {};
        const tiers = (["global", "scene", "project"] as const).filter((id) => paths[id]).map((id) => [id, name(id)] as [string, string]);
        return (
          <>
            {only ? null : (
              <SettingsRow label={t("settings.tier")} hint={paths[selected] ?? ""}>
                <Select
                  testId="settings-tier"
                  options={tiers}
                  value={selected}
                  onChange={(value) => {
                    tier = value;
                    setSelected(value);
                  }}
                />
              </SettingsRow>
            )}
            {section === "engine" || only ? <SettingsFiles paths={only ? { [only]: paths[only] } : paths} name={name} /> : null}
          </>
        );
      }}
      onSaved={(_, data) => {
        host.shell.notice(t("settings.written", { path: data?.paths?.[selected] }));
        reload();
      }}
      onFailed={(e) => host.shell.notice(t("settings.writeFailed", { error: errorText(e) }), "error")}
    />
  );
}

/* ── network and web ──────────────────────────────────────────────────── */

/** The engine's own proxy and the domains WebFetch may reach; which models search and read the web
 *  are routes, set with the others in "models by task". */
function Network({ host }: { host: HostApi }) {
  const t = useT();
  return (
    <>
      <EngineSettings host={host} section="network" title="settings.nav.network" />
      <SettingsRow setting="web-routes" label={t("settings.webRoutes")}>
        <button type="button" className="btn sm outline" data-testid="settings-web-routes" onClick={() => host.settings.open("task-models")}>
          {t("settings.webRoutesOpen")}
        </button>
      </SettingsRow>
    </>
  );
}

/* ── privacy & advanced ───────────────────────────────────────────────── */

/** Telemetry: what is recorded and where, said plainly, then the switch (global tier only). */
function Privacy({ host }: { host: HostApi }) {
  const t = useT();
  return (
    <>
      <h3>{t("settings.privacy")}</h3>
      <div className="callout" data-testid="privacy-explain">
        <Glyph name="key" className="glyph" />
        <span>{t("settings.privacyExplain")}</span>
      </div>
      <EngineSettings host={host} section="privacy" only="global" />
    </>
  );
}

/** What the engine runs with for compaction, execution, hooks, teams and checkpoints, and which
 *  tier says so; the file to edit is named, and nothing here writes it. */
function Advanced({ host }: { host: HostApi }) {
  const t = useT();
  const hello = useHello();
  const request = where(host, hello);
  const name = (id: string) => t(`settings.tier.${id}`, { scene: request.scene });
  return (
    <SettingsSection title={t("settings.nav.advanced")} load={() => host.call<Described>("settings.describe", request)}>
      {(data) => {
        const paths = data.paths ?? {};
        return (
          <div data-testid="advanced">
            <div className="hint">{t("settings.advancedHint")}</div>
            {(data.fields ?? [])
              .filter((field) => field.section === "advanced")
              .map((field) => {
                const value = field.effective;
                const empty = value === null || value === undefined || (typeof value === "object" && !Object.keys(value as object).length);
                return (
                  <SettingsRow
                    key={field.key}
                    setting={field.key}
                    label={t(field.label ?? `settings.field.${field.key}`)}
                    hint={field.source ? t("settings.sourceTier", { tier: name(field.source) }) : t("settings.advancedDefault")}
                  >
                    <code data-testid={`settings-${field.key}`} data-source={field.source ?? ""}>
                      {empty ? t("settings.advancedUnset") : JSON.stringify(value)}
                    </code>
                  </SettingsRow>
                );
              })}
            <SettingsFiles paths={paths} name={name} />
          </div>
        );
      }}
    </SettingsSection>
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
                  <span>{hello?.engine?.has_credentials === true ? t("settings.providersEnv") : t("settings.providersNone")}</span>
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
      <EngineSettings host={host} section="models" title="settings.modelDefaults" />
    </>
  );
}

type ProviderEntry = NonNullable<NonNullable<Described["providers"]>["providers"]>[string];

/**
 * A stored key as the page shows it: `***` and its last four characters at most. The engine and the
 * `settings` extension both redact it before it gets here; this holds even if neither did. Null when
 * no key is stored.
 */
export function maskedKey(key: unknown): string | null {
  if (key === null || key === undefined || key === "") return null;
  const text = String(key);
  if (/^\*{3}[^*]{0,4}$/.test(text)) return text;
  return text.length >= 12 ? `***${text.slice(-4)}` : "***";
}

/** The fields an OpenAI-compatible endpoint switches thinking by (Agix's `thinking_toggle`). */
const THINKING_TOGGLES = ["deepseek", "enable_thinking", "chat_template_kwargs"];

const API_TYPES = [
  { id: "anthropic", label: "settings.apiTypeAnthropic", url: "https://api.anthropic.com", model: "claude-sonnet-4-6" },
  { id: "openai_compatible", label: "settings.apiTypeOpenai", url: "https://api.openai.com/v1", model: "gpt-4.1" },
  { id: "openai_responses", label: "settings.apiTypeOpenaiResponses", url: "https://api.openai.com/v1", model: "gpt-4.1" },
];

interface ApiType {
  id: string;
  label: string;
  url: string;
  model: string;
}

/** A type the table does not know (written by hand): kept as it is, never turned into another one. */
function unknownType(id: string): ApiType {
  return { id, label: "", url: "", model: "" };
}

export function apiTypeOf(provider: ProviderEntry): ApiType {
  if (!provider.api_type) return API_TYPES[0]!;
  return API_TYPES.find((x) => x.id === provider.api_type) ?? unknownType(provider.api_type);
}

function typeLabel(t: ReturnType<typeof useT>, type: ApiType): string {
  return type.label ? t(type.label) : t("settings.apiTypeUnknown", { type: type.id });
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
  const models = [provider.default_model, ...(provider.models ?? []).map(modelId)].filter((m, i, all): m is string => Boolean(m) && all.indexOf(m) === i);
  return (
    <div className={`provider-card${isDefault ? " default" : ""}`} data-provider={id} data-state={isDefault ? "default" : undefined}>
      <div className="pc-head">
        <span className="pc-name">{id}</span>
        <span className="pill">{typeLabel(t, apiTypeOf(provider))}</span>
        {isDefault ? <span className="pill acc">{t("settings.providerDefault")}</span> : null}
        <span className="sp" />
        <ProviderKey provider={provider} />
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

/** Where a provider's key comes from: the one stored, as its last four; else the environment
 *  variable it names (whether the engine's environment has it, the engine does not say). */
function ProviderKey({ provider }: { provider: ProviderEntry }) {
  const t = useT();
  const stored = maskedKey(provider.api_key);
  if (stored) {
    return (
      <span data-testid="provider-key" data-state="stored">
        <SettingsValue tone="ok">{t("settings.keyMasked", { value: stored })}</SettingsValue>
      </span>
    );
  }
  if (provider.api_key_env) {
    return (
      <span data-testid="provider-key" data-state="env" title={t("settings.keyFromEnvHint", { name: provider.api_key_env })}>
        <SettingsValue tone="ok">{t("settings.keyFromEnv", { name: provider.api_key_env })}</SettingsValue>
      </span>
    );
  }
  return (
    <span data-testid="provider-key" data-state="missing">
      <SettingsValue tone="err">{t("settings.keyMissing")}</SettingsValue>
    </span>
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
    api_type: apiTypeOf(provider).id,
    base_url: provider.base_url ?? "",
    api_key: "",
    api_key_env: provider.api_key_env ?? "",
    default_model: provider.default_model ?? "",
    models: (provider.models ?? []).map(modelId).join(", "),
    thinking_toggle: provider.thinking_toggle ?? "",
    reasoning_passback: provider.reasoning_passback === true,
    make_default: isDefault,
  });
  const [saving, setSaving] = useState(false);
  const type = API_TYPES.find((x) => x.id === fields.api_type) ?? unknownType(fields.api_type);
  // A type the table lacks stays selectable as it is, so the other fields can be saved without turning it into another.
  const types = API_TYPES.some((x) => x.id === fields.api_type) ? API_TYPES : [...API_TYPES, type];
  const set = (key: keyof typeof fields, value: string | boolean) => setFields({ ...fields, [key]: value });
  const text = (key: "provider_id" | "base_url" | "api_key" | "api_key_env" | "default_model" | "models", label: string, placeholder?: string, hint?: string) => (
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
    // An id still listed keeps the object it was written as, with what that says of the model.
    const written = new Map((provider.models ?? []).map((m) => [modelId(m), m] as const));
    const models = fields.models.split(/[,，\s]+/).map((m) => m.trim()).filter(Boolean).map((m) => written.get(m) ?? m);
    const config: Record<string, unknown> = {
      api_type: fields.api_type,
      base_url: fields.base_url.trim(),
      default_model: fields.default_model.trim(),
      models: models.length ? models : null,
      api_key_env: fields.api_key_env.trim(),
    };
    if (fields.api_key) config.api_key = fields.api_key;
    // Only an OpenAI-compatible endpoint reads them; another type keeps what it has.
    if (fields.api_type === "openai_compatible") {
      config.thinking_toggle = fields.thinking_toggle || null;
      config.reasoning_passback = fields.reasoning_passback;
    }
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
          {types.map((x) => (
            <button
              type="button"
              role="radio"
              key={x.id}
              data-testid={`provider-type-${x.id}`}
              aria-checked={fields.api_type === x.id}
              className={fields.api_type === x.id ? "on" : ""}
              onClick={() => set("api_type", x.id)}
            >
              {typeLabel(t, x)}
            </button>
          ))}
        </div>
      </div>
      {text("base_url", t("settings.baseUrl"), type.url)}
      {text(
        "api_key",
        t("settings.apiKey"),
        id && provider.api_key ? `${maskedKey(provider.api_key)} · ${t("settings.apiKeyKeep")}` : "sk-…",
        t("settings.apiKeyHint", { path: path ?? "~/.harnix/engine/settings.json" }),
      )}
      {text("api_key_env", t("settings.apiKeyEnv"), "OPENAI_API_KEY", t("settings.apiKeyEnvHint"))}
      {text("default_model", t("settings.defaultModel"), type.model)}
      {text("models", t("settings.models"), undefined, t("settings.modelsHint"))}
      {fields.api_type === "openai_compatible" ? (
        <>
          <div className="field">
            <label>{t("settings.thinkingToggle")}</label>
            <Select
              testId="provider-thinking_toggle"
              options={[["", t("settings.thinkingToggleNone")], ...THINKING_TOGGLES.map((x) => [x, x] as [string, string])]}
              value={fields.thinking_toggle}
              onChange={(value) => set("thinking_toggle", value)}
            />
            <div className="hint">{t("settings.thinkingToggleHint")}</div>
          </div>
          <label className="check">
            <input type="checkbox" data-testid="provider-reasoning_passback" checked={fields.reasoning_passback} onChange={(e) => set("reasoning_passback", e.target.checked)} />
            {t("settings.reasoningPassback")}
          </label>
        </>
      ) : null}
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

/** The MCP servers of the open (project, scene). */
function McpSection({ host }: { host: HostApi }) {
  const hello = useHello();
  return <Mcp host={host} request={where(host, hello)} />;
}
