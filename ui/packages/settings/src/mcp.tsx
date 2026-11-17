/**
 * Settings · MCP servers: every server of the tiers there are for the open (project, scene), with
 * what the engine says of it for that pair; added and edited in place, switched off and on,
 * reconnected, removed after asking, and signed in to.
 *
 * All through the `settings` extension (`settings.mcp.*`), which holds `engine.mcp` for the page:
 * a device has no capability of its own. The engine writes the tier chosen and says what each
 * server is doing; a pair whose servers have not connected yet says only what its settings make of
 * them, and connects them at its first session. A value given for an environment variable or a header is
 * written and never read back — the page gets `***` and its last four characters, and an edit that
 * leaves it says `keep`.
 *
 * Signing in opens the authorization page in a window of its own. The browser comes back to the
 * extension's route on this origin, which finishes the sign-in when the window could be admitted;
 * this page reads where the window landed (same origin) and hands the address over
 * (`settings.mcp.authComplete`) when it could not. Either way the second hand-over is told how the
 * first ended.
 */

import { useState } from "react";
import { errorText, type McpAuthDone, type McpAuthStarted, type McpList, type McpServer, type McpValue } from "@harnix/core";
import { Glyph, SettingsSection, SettingsValue, useSettingsReload, useT, type HostApi } from "@harnix/react";

import { writeFailed } from "./errors.ts";

/** Where a write goes: the open session's scene and project, else the engine's first active scene. */
export type McpRequest = { scene?: string; project_root: string | null };

type Tone = "ok" | "warn" | "err" | undefined;

/** What a server's row says of it, and in what tone. */
export function mcpState(server: McpServer): { key: string; tone: Tone } {
  if (!server.enabled) return { key: "off", tone: undefined };
  const status = server.status;
  if (!status) return { key: "unknown", tone: undefined };
  if (status.auth.state === "failed") return { key: "signInFailed", tone: "err" };
  if (status.state === "auth_required" || status.auth.state === "required") return { key: "auth_required", tone: "warn" };
  if (status.state === "connected") return { key: status.auth.state === "authorized" ? "authorized" : "connected", tone: "ok" };
  if (status.state === "disabled") return { key: "off", tone: undefined };
  if (status.state === "not_started") return { key: "not_started", tone: undefined };
  if (status.state === "connecting" || status.state === "disconnected") return { key: status.state, tone: "warn" };
  return { key: status.state, tone: "err" };
}

/** A remote server: one with something to sign in to. */
const remote = (server: McpServer) => server.transport === "http" || server.transport === "sse";

export function Mcp({ host, request }: { host: HostApi; request: McpRequest }) {
  const t = useT();
  const reload = useSettingsReload();
  const [editing, setEditing] = useState<{ tier: string; name: string } | "new" | null>(null);
  const done = () => {
    setEditing(null);
    reload();
  };
  const failed = (e: unknown) => host.shell.notice(writeFailed(e), "error");
  const tierName = (id: string) => t(`settings.tier.${id}`, { scene: request.scene });
  return (
    <SettingsSection title={t("settings.mcpServers")} load={() => host.call<McpList>("settings.mcp.list", request)}>
      {(data) => {
        const paths = data.paths ?? {};
        const listed = data.servers ?? [];
        const tiers = (["global", "scene", "project"] as const).filter((id) => paths[id]);
        const live = data.loaded !== false;
        return (
          <div className="providers" data-testid="mcp-servers">
            <div className="hint" data-testid="mcp-pair" data-loaded={data.loaded === undefined ? "" : String(data.loaded)}>
              {[
                data.project_root ? t("settings.mcpPair", { scene: data.scene ?? "", project: data.project_root }) : t("settings.mcpPairNoProject", { scene: data.scene ?? "" }),
                live ? null : t("settings.mcpNotLoaded"),
              ]
                .filter(Boolean)
                .join(" · ")}
            </div>
            {!listed.length ? <div className="hint">{t("settings.mcpNone")}</div> : null}
            {listed.map((server) =>
              editing !== null && editing !== "new" && editing.tier === server.tier && editing.name === server.name ? (
                <McpForm key={`${server.tier}/${server.name}`} host={host} request={request} tiers={tiers} tierName={tierName} server={server} onDone={done} onCancel={() => setEditing(null)} />
              ) : (
                <McpCard
                  key={`${server.tier}/${server.name}`}
                  host={host}
                  request={request}
                  server={server}
                  live={live}
                  tierName={tierName}
                  onEdit={() => setEditing({ tier: server.tier, name: server.name })}
                  onChanged={reload}
                  onFailed={failed}
                />
              ),
            )}
            {editing === "new" ? (
              <McpForm host={host} request={request} tiers={tiers} tierName={tierName} onDone={done} onCancel={() => setEditing(null)} />
            ) : (
              <button type="button" className="btn sm outline add-provider" data-testid="mcp-add" onClick={() => setEditing("new")}>
                <Glyph name="plus" className="glyph" />
                {t("settings.mcpAdd")}
              </button>
            )}
            <div className="hint">{t("settings.mcpAddHint")}</div>
          </div>
        );
      }}
    </SettingsSection>
  );
}

function McpCard({
  host,
  request,
  server,
  live,
  tierName,
  onEdit,
  onChanged,
  onFailed,
}: {
  host: HostApi;
  request: McpRequest;
  server: McpServer;
  /** The pair's servers are connected: one can be connected again. */
  live: boolean;
  tierName: (id: string) => string;
  onEdit: () => void;
  onChanged: () => void;
  onFailed: (e: unknown) => void;
}) {
  const t = useT();
  const state = mcpState(server);
  const at = { ...request, tier: server.tier, name: server.name };
  const act = (method: string, params: Record<string, unknown> = {}) =>
    host
      .call(method, { ...at, ...params })
      .then(onChanged)
      .catch(onFailed);
  const remove = async () => {
    const yes = await host.dialogs.confirm(t("settings.mcpRemoveConfirm", { name: server.name, tier: tierName(server.tier) }), { danger: true, confirm: t("common.remove") });
    if (yes) await act("settings.mcp.remove");
  };
  const auth = server.status?.auth.state;
  const where = server.transport === "stdio" ? [server.command, ...server.args].filter(Boolean).join(" ") : (server.url ?? "");
  return (
    <div className="provider-card" data-mcp={server.name} data-tier={server.tier} data-state={state.key}>
      <div className="pc-head">
        <span className="pc-name">{server.name}</span>
        <span className="pill">{server.transport}</span>
        <span className="pill">{tierName(server.tier)}</span>
        <span className="sp" />
        <span data-testid="mcp-state">
          <SettingsValue tone={state.tone}>{t(`settings.mcpState.${state.key}`)}</SettingsValue>
        </span>
      </div>
      <div className="pc-meta">
        <span className="mono truncate">{where}</span>
        {server.status && server.enabled ? <span>{t("settings.mcpTools", { count: server.status.tool_count })}</span> : null}
        {server.status?.error ?? server.status?.auth.error ? (
          <span className="err" data-testid="mcp-error">
            {server.status?.error ?? server.status?.auth.error}
          </span>
        ) : null}
        {server.overridden_by ? <span>{t("settings.mcpOverridden", { tier: tierName(server.overridden_by) })}</span> : null}
        {server.unsupported.includes("private_key_jwt") ? (
          <span className="err" data-testid="mcp-jwt">
            {t("settings.mcpJwt")}
          </span>
        ) : null}
      </div>
      <div className="pc-actions">
        {remote(server) && server.enabled && !server.overridden_by ? (
          auth === "authorized" ? (
            <button type="button" className="btn sm ghost" data-testid="mcp-signout" onClick={() => void act("settings.mcp.logout")}>
              {t("settings.mcpSignOut")}
            </button>
          ) : (
            <button type="button" className="btn sm ghost" data-testid="mcp-signin" onClick={() => void signIn(host, server.name, request, onChanged)}>
              {t("settings.mcpSignIn")}
            </button>
          )
        ) : null}
        {server.enabled && live && !server.overridden_by ? (
          <button type="button" className="btn sm ghost" data-testid="mcp-reconnect" onClick={() => void act("settings.mcp.reconnect")}>
            {t("settings.mcpReconnect")}
          </button>
        ) : null}
        <button type="button" className="btn sm ghost" data-testid="mcp-toggle" onClick={() => void act("settings.mcp.enable", { enabled: !server.enabled })}>
          {server.enabled ? t("settings.mcpDisable") : t("settings.mcpEnable")}
        </button>
        <button type="button" className="btn sm ghost" data-testid="mcp-edit" onClick={onEdit}>
          {t("common.edit")}
        </button>
        <button type="button" className="btn sm ghost" data-testid="mcp-remove" onClick={() => void remove()}>
          {t("common.remove")}
        </button>
      </div>
    </div>
  );
}

/** One environment variable or header in the form: given anew, read from a variable, or kept. */
type Row = { name: string; mode: "secret" | "var" | "keep"; value: string; masked?: string };

function rowsOf(values: Record<string, McpValue | undefined> | undefined): Row[] {
  return Object.entries(values ?? {}).flatMap(([name, value]) =>
    value ? [value.kind === "env" ? { name, mode: "var" as const, value: value.var ?? "" } : { name, mode: "keep" as const, value: "", masked: value.masked }] : [],
  );
}

/** The form's rows as `settings.mcp.*` takes them. */
export function givenOf(rows: Row[]): Record<string, { var: string } | { secret: string } | { keep: true }> {
  const out: Record<string, { var: string } | { secret: string } | { keep: true }> = {};
  for (const row of rows) {
    const name = row.name.trim();
    if (!name) continue;
    out[name] = row.mode === "keep" ? { keep: true } : row.mode === "var" ? { var: row.value.trim() } : { secret: row.value };
  }
  return out;
}

const TRANSPORTS = ["stdio", "http", "sse"] as const;

/** Add (no server) or edit one server: the only path its values take from the interface. */
function McpForm({
  host,
  request,
  tiers,
  tierName,
  server,
  onDone,
  onCancel,
}: {
  host: HostApi;
  request: McpRequest;
  tiers: readonly string[];
  tierName: (id: string) => string;
  server?: McpServer;
  onDone: () => void;
  onCancel: () => void;
}) {
  const t = useT();
  const [tier, setTier] = useState(server?.tier ?? tiers[0] ?? "global");
  const [name, setName] = useState(server?.name ?? "");
  const [transport, setTransport] = useState<string>(server && (TRANSPORTS as readonly string[]).includes(server.transport) ? server.transport : "stdio");
  const [command, setCommand] = useState(server?.command ?? "");
  const [args, setArgs] = useState((server?.args ?? []).join("\n"));
  const [url, setUrl] = useState(server?.url ?? "");
  const [env, setEnv] = useState<Row[]>(rowsOf(server?.env));
  const [headers, setHeaders] = useState<Row[]>(rowsOf(server?.headers));
  const [saving, setSaving] = useState(false);
  const stdio = transport === "stdio";
  const save = async () => {
    const id = server?.name ?? name.trim();
    if (!id) {
      host.shell.notice(t("settings.mcpNameRequired"));
      return;
    }
    const config = stdio
      ? { transport, command: command.trim(), args: args.split("\n").map((a) => a.trim()).filter(Boolean), env: givenOf(env) }
      : { transport, url: url.trim(), headers: givenOf(headers) };
    setSaving(true);
    try {
      await host.call(server ? "settings.mcp.update" : "settings.mcp.add", { ...request, tier, name: id, config });
      onDone();
    } catch (e) {
      host.shell.notice(writeFailed(e), "error");
      setSaving(false);
    }
  };
  return (
    <form
      className="provider-form"
      data-testid="mcp-form"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <div className="pf-title">{server ? t("settings.mcpEdit", { name: server.name }) : t("settings.mcpAdd")}</div>
      {!server ? (
        <>
          <div className="field">
            <label>{t("settings.tier")}</label>
            <select data-testid="mcp-tier" value={tier} onChange={(e) => setTier(e.target.value)}>
              {tiers.map((id) => (
                <option key={id} value={id}>
                  {tierName(id)}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label>{t("settings.mcpName")}</label>
            <input type="text" data-testid="mcp-name" value={name} autoComplete="off" spellCheck={false} onChange={(e) => setName(e.target.value)} />
            <div className="hint">{t("settings.mcpNameHint", { name: name.trim() || "…" })}</div>
          </div>
        </>
      ) : null}
      <div className="field">
        <label>{t("settings.mcpTransport")}</label>
        <div className="seg" role="radiogroup">
          {TRANSPORTS.map((x) => (
            <button type="button" role="radio" key={x} data-testid={`mcp-transport-${x}`} aria-checked={transport === x} className={transport === x ? "on" : ""} onClick={() => setTransport(x)}>
              {x}
            </button>
          ))}
        </div>
      </div>
      {stdio ? (
        <>
          <div className="field">
            <label>{t("settings.mcpCommand")}</label>
            <input type="text" data-testid="mcp-command" value={command} placeholder="npx" autoComplete="off" spellCheck={false} onChange={(e) => setCommand(e.target.value)} />
          </div>
          <div className="field">
            <label>{t("settings.mcpArgs")}</label>
            <textarea data-testid="mcp-args" rows={3} value={args} spellCheck={false} onChange={(e) => setArgs(e.target.value)} />
          </div>
          <Values label={t("settings.mcpEnv")} kind="env" rows={env} onChange={setEnv} project={tier === "project"} />
        </>
      ) : (
        <>
          <div className="field">
            <label>{t("settings.mcpUrl")}</label>
            <input type="text" data-testid="mcp-url" value={url} placeholder="https://…/mcp" autoComplete="off" spellCheck={false} onChange={(e) => setUrl(e.target.value)} />
          </div>
          <Values label={t("settings.mcpHeaders")} kind="headers" rows={headers} onChange={setHeaders} project={tier === "project"} />
        </>
      )}
      <div className="hint">{tier === "project" ? t("settings.mcpProjectEnvHint") : t("settings.mcpSecretHint")}</div>
      <div className="pf-actions">
        <button type="button" className="btn sm outline" onClick={onCancel}>
          {t("common.cancel")}
        </button>
        <button type="submit" className="btn sm primary" data-testid="mcp-save" disabled={saving}>
          {t("common.save")}
        </button>
      </div>
    </form>
  );
}

/** The environment's or the headers' rows. A value given is a password field, never filled back. */
function Values({ label, kind, rows, onChange, project }: { label: string; kind: "env" | "headers"; rows: Row[]; onChange: (rows: Row[]) => void; project: boolean }) {
  const t = useT();
  const set = (i: number, row: Partial<Row>) => onChange(rows.map((r, at) => (at === i ? { ...r, ...row } : r)));
  return (
    <div className="field" data-testid={`mcp-${kind}`}>
      <label>{label}</label>
      {rows.map((row, i) => (
        <div className="kv-row" key={i} data-testid={`mcp-${kind}-row`}>
          <input type="text" data-testid={`mcp-${kind}-name`} value={row.name} placeholder={t("settings.mcpValueName")} autoComplete="off" spellCheck={false} onChange={(e) => set(i, { name: e.target.value })} />
          <select data-testid={`mcp-${kind}-mode`} value={row.mode} onChange={(e) => set(i, { mode: e.target.value as Row["mode"], value: "" })}>
            {row.masked !== undefined ? <option value="keep">{t("settings.mcpValueKeep", { masked: row.masked })}</option> : null}
            <option value="secret">{t("settings.mcpValueSecret")}</option>
            {!project ? <option value="var">{t("settings.mcpValueVar")}</option> : null}
          </select>
          {row.mode === "keep" ? null : (
            <input
              type={row.mode === "secret" ? "password" : "text"}
              data-testid={`mcp-${kind}-value`}
              value={row.value}
              placeholder={row.mode === "var" ? "GITHUB_TOKEN" : ""}
              autoComplete="off"
              spellCheck={false}
              onChange={(e) => set(i, { value: e.target.value })}
            />
          )}
          <button type="button" className="btn sm ghost" aria-label={t("common.remove")} onClick={() => onChange(rows.filter((_, at) => at !== i))}>
            <Glyph name="close" className="glyph" />
          </button>
        </div>
      ))}
      <button type="button" className="btn sm ghost" data-testid={`mcp-${kind}-add`} onClick={() => onChange([...rows, { name: "", mode: "secret", value: "" }])}>
        <Glyph name="plus" className="glyph" />
        {t("settings.mcpValueAdd")}
      </button>
    </div>
  );
}

/** How often the sign-in window is looked at. */
const WATCH_MS = 500;

/**
 * Sign in to a remote server: a window opened at once (a browser lets a page open one only on a
 * click), sent to the authorization page, then watched until it comes back here or is closed.
 */
export async function signIn(host: HostApi, name: string, request: McpRequest, onSettled: () => void, win: Window = window): Promise<void> {
  const t = host.t;
  const popup = win.open("", "_blank");
  if (!popup) {
    host.shell.notice(t("settings.mcpSignInBlocked"), "error");
    return;
  }
  // The authorization page is someone else's: it gets no hold on this one.
  popup.opener = null;
  let started: McpAuthStarted;
  try {
    started = await host.call<McpAuthStarted>("settings.mcp.authorize", { ...request, name, origin: win.location.origin });
  } catch (e) {
    popup.close();
    host.shell.notice(t("settings.mcpSignInFailed", { name, error: errorText(e) }), "error");
    return;
  }
  popup.location.href = started.authorize_url;
  host.shell.notice(t(started.redirect === "loopback" ? "settings.mcpSignInLoopback" : "settings.mcpSignInWaiting", { name }));
  const ended = (done: McpAuthDone) => {
    host.shell.notice(done.ok ? t("settings.mcpSignedIn", { name: done.server }) : t("settings.mcpSignInFailed", { name: done.server, error: done.error ?? "" }), done.ok ? "info" : "error");
  };
  const until = Date.now() + started.expires_in * 1000;
  await new Promise<void>((resolve) => {
    const timer = win.setInterval(() => {
      const finish = () => {
        win.clearInterval(timer);
        onSettled();
        resolve();
      };
      if (popup.closed || Date.now() > until) return finish();
      let href: string;
      try {
        href = popup.location.href;
      } catch {
        return; // still on the authorization server's origin
      }
      if (!started.callback || !href.startsWith(`${win.location.origin}${started.callback}`)) return;
      win.clearInterval(timer);
      // Back here. The route finished the sign-in if it could admit the window, and this is told
      // how it ended; if it could not, this hands the address over, with the page's credential.
      void host
        .call<McpAuthDone>("settings.mcp.authComplete", { callback_url: href })
        .then(ended, (e: unknown) => host.shell.notice(t("settings.mcpSignInFailed", { name, error: errorText(e) }), "error"))
        .finally(() => {
          popup.close();
          onSettled();
          resolve();
        });
    }, WATCH_MS);
  });
}
