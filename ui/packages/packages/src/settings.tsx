/**
 * Settings → the extension packages installed here, and the store's catalog.
 *
 * Installed: each with its version, how far it is trusted (countersigned by the store, signed by a
 * publisher the profile trusts, or unsigned), whether it runs; its permissions — every capability
 * as a sentence, and what the shell draws for it from its manifest; what it keeps, with a button
 * that clears it; and its switches — enable or disable, update when the store has a newer version,
 * uninstall.
 *
 * The store: what it offers this Harnix, each with an install button. Installing and updating go
 * through the disclosure, the shell's dialog over the settings: the backend answers what the
 * package would be granted (for an update, only what it adds) and a digest, the dialog shows it,
 * and only "Install" / "Update" in it sends the digest back. Nothing is installed by a click
 * anywhere else. An update that asks for nothing more is done without asking. Clearing and
 * uninstalling are confirmed in the shell's dialog too.
 */

import { useState } from "react";
import { errorText } from "@harnix/core";
import { DialogBody, DialogFoot, SettingsRow, SettingsSection, SettingsValue, fileSize, t, useSettingsReload, useT, type HostApi } from "@harnix/react";

/** What the shell draws for a package from its manifest: the names, a line each. */
export interface Inline {
  tool_row?: { tool: string }[];
  flow_block?: { kind: string }[];
  command?: { trigger: string; name: string }[];
  sidebar_group?: { space?: string }[];
}

interface Line {
  capability: string;
  value: string | number | boolean;
  from?: number;
}

interface Installed {
  id: string;
  version: string;
  title?: string | null;
  enabled: boolean;
  trust?: string;
  source?: string;
  data_bytes?: number;
  runtime?: { state?: string; reason?: string };
  ext?: { publisher?: string; description?: string | null; lines?: Line[]; ui?: unknown[]; inline?: Inline | null } | null;
}

interface CatalogApp {
  id: string;
  title?: string | null;
  publisher: string;
  description?: string | null;
  latest?: string | null;
  installable?: boolean;
  installed?: string;
}

interface Asked {
  id: string;
  version: string;
  consent?: "required";
  digest?: string;
  up_to_date?: boolean;
  disclosure?: {
    title?: string | null;
    publisher?: string;
    description?: string | null;
    trust?: string;
    lines?: Line[];
    ui_in_page?: boolean;
    inline?: Inline | null;
    update?: { from: string; added: Line[]; downgrade?: boolean };
  };
}

/** What a person is asked to agree to, and how to go ahead. */
interface Pending {
  method: "packages.install" | "packages.update";
  asked: Asked;
  version?: string;
  /** Back to an older version, which the person chose by name: asked and consented to as such. */
  downgrade?: boolean;
}

/** `a` is an older version than `b`: compared part by part as numbers, a pre-release ignored. */
export function older(a: string, b: string): boolean {
  const parts = (v: string) => v.split("-")[0]!.split(".").map((n) => Number(n) || 0);
  const [x, y] = [parts(a), parts(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i += 1) {
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) < (y[i] ?? 0);
  }
  return false;
}

/** What a line lets it do, in a sentence: the words, the value, and why it matters. */
function LineText({ line }: { line: Line }) {
  const t = useT();
  const raw = String(line.value);
  let key = `apps.cap.${line.capability}`;
  let value = raw;
  if (line.capability === "storage_mb") value = t("apps.megabytes", { size: raw });
  // A line that is the whole of what it says: nothing to show beside it.
  if (line.value === true || line.capability === "events") value = "";
  if (line.capability === "calls" && raw.startsWith("topic:")) {
    key = "apps.cap.callsTopic";
    value = raw.slice("topic:".length);
  }
  if (line.capability === "fs") key = raw.startsWith("${data}") ? "apps.cap.fsData" : "apps.cap.fsProject";
  if (line.capability === "inline") {
    const [what, ...rest] = raw.split(" ");
    key = what === "command" ? "apps.cap.inlineCommand" : "apps.cap.inlineGroup";
    value = rest.join(" ");
  }
  const from = line.from !== undefined ? ` (${t("apps.from", { value: t("apps.megabytes", { size: String(line.from) }) })})` : "";
  const why = ["net", "net_local", "env", "calls", "ui", "ui_calls", "events", "frames", "agent"].includes(line.capability) ? t(`apps.cap.why.${line.capability}`) : "";
  return (
    <li data-capability={line.capability}>
      <span className="apps-cap">{t(key)}</span>
      {value ? <> <code>{value}</code></> : null}
      {from}
      {why ? <span className="apps-why"> — {why}</span> : null}
    </li>
  );
}

function Lines({ lines, empty }: { lines: Line[]; empty: string }) {
  if (!lines.length) return <div className="hint">{empty}</div>;
  return (
    <ul className="apps-lines">
      {lines.map((line, i) => (
        <LineText key={i} line={line} />
      ))}
    </ul>
  );
}

/** What the shell draws for it from its manifest, a line each. */
function InlinePoints({ inline }: { inline: Inline | null | undefined }) {
  const t = useT();
  const items: [string, string][] = [
    ...(inline?.tool_row ?? []).map((r): [string, string] => ["tool_row", r.tool]),
    ...(inline?.flow_block ?? []).map((b): [string, string] => ["flow_block", b.kind]),
    ...(inline?.command ?? []).map((c): [string, string] => ["command", `${c.trigger}${c.name}`]),
    ...(inline?.sidebar_group ?? []).map((g): [string, string] => ["sidebar_group", g.space ?? ""]),
  ];
  if (!items.length) return null;
  return (
    <>
      <div className="hint">{t("apps.inlinePoints")}</div>
      <ul className="apps-lines">
        {items.map(([point, name], i) => (
          <li key={i} data-inline={point}>
            <span className="apps-cap">{t(`apps.inline.${point}`)}</span> <code>{name}</code>
          </li>
        ))}
      </ul>
    </>
  );
}

export function PackagesSettings({ host }: { host: HostApi }) {
  const t = useT();
  const reload = useSettingsReload();
  const [busy, setBusy] = useState(false);
  const [latest, setLatest] = useState<Record<string, string>>({});

  const fail = (e: unknown) => host.shell.notice(errorText(e), "error");
  /** Ask; a disclosure waits for the person, anything else is done. */
  const ask = async (method: Pending["method"], id: string, version?: string, downgrade?: boolean) => {
    setBusy(true);
    let asked: Asked;
    try {
      asked = await host.call<Asked>(method, { id, version, ...(downgrade ? { allow_downgrade: true } : {}) });
    } catch (e) {
      fail(e);
      return;
    } finally {
      setBusy(false);
    }
    if (asked.consent !== "required") {
      if (asked.up_to_date) host.shell.notice(t("apps.upToDate", { name: id }));
      reload();
      return;
    }
    const pending = { method, asked, version, downgrade };
    const done = await host.dialogs.open<true>((close) => <Disclosure host={host} first={pending} close={close} />, { title: disclosureTitle(pending) });
    if (done) reload();
  };
  const act = (method: string, params: unknown) =>
    host
      .call(method, params)
      .then(reload)
      .catch(fail);
  /** Something that cannot be undone, confirmed first in the shell's dialog. */
  const confirmed = async (title: string, message: string, run: () => Promise<unknown>) => {
    if (!(await host.dialogs.confirm(message, { title, confirm: title, danger: true }))) return;
    setBusy(true);
    try {
      await run();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
      reload();
    }
  };

  return (
    <>
      <SettingsSection title={t("apps.installed")} load={() => host.call<{ packages?: Installed[] }>("packages.list", {})}>
        {(data) => {
          const apps = data.packages ?? [];
          if (!apps.length) return <div className="hint">{t("apps.none")}</div>;
          return (
            <>
              {apps.map((a) => {
                const listed = latest[a.id] && latest[a.id] !== a.version ? latest[a.id]! : null;
                // The store's newest is older than this one (it took the newer back): going there is
                // a choice of its own, named as one, and disclosed as a downgrade.
                const back = listed && older(listed, a.version) ? listed : null;
                const newer = back ? null : listed;
                const name = a.title || a.id;
                const state = a.enabled ? (a.runtime?.state ?? "stopped") : "stopped";
                const hint = [a.id, trustText(a.trust), t(`apps.state.${state}`)];
                if (state === "failed" || state === "faulted") hint.push(a.runtime?.reason ?? "");
                return (
                  <div
                    key={a.id}
                    className="apps-app"
                    data-testid="app-installed"
                    data-app={a.id}
                    data-version={a.version}
                    data-trust={a.trust ?? "unsigned"}
                    data-state={state}
                  >
                    <SettingsRow label={`${name} ${a.version}`} hint={hint.filter(Boolean).join(" · ")}>
                      <SettingsValue tone={state === "running" || state === "hibernating" ? "ok" : state === "failed" || state === "faulted" ? "err" : undefined}>
                        {a.enabled ? t("apps.enabled") : t("apps.disabled")}
                      </SettingsValue>
                      <button
                        type="button"
                        className="btn sm outline"
                        data-testid="app-toggle"
                        disabled={busy}
                        onClick={() => void act(a.enabled ? "packages.disable" : "packages.enable", { id: a.id })}
                      >
                        {a.enabled ? t("apps.disable") : t("apps.enable")}
                      </button>
                      {a.source === "store" ? (
                        back ? (
                          <button
                            type="button"
                            className="btn sm outline"
                            data-testid="app-update"
                            data-direction="back"
                            disabled={busy}
                            onClick={() => void ask("packages.update", a.id, back, true)}
                          >
                            {t("apps.downgradeTo", { version: back })}
                          </button>
                        ) : (
                          <button type="button" className="btn sm outline" data-testid="app-update" disabled={busy} onClick={() => void ask("packages.update", a.id)}>
                            {newer ? t("apps.updateTo", { version: newer }) : t("apps.update")}
                          </button>
                        )
                      ) : null}
                      <button
                        type="button"
                        className="btn sm danger"
                        data-testid="app-uninstall"
                        disabled={busy}
                        onClick={() => void confirmed(t("apps.uninstall"), t("apps.uninstallConfirm", { name }), () => host.call("packages.uninstall", { id: a.id }))}
                      >
                        {t("apps.uninstall")}
                      </button>
                    </SettingsRow>
                    <div className="apps-permissions" data-testid="app-permissions">
                      <div className="apps-sub">{t("apps.permissions")}</div>
                      <Lines lines={a.ext?.lines ?? []} empty={t("apps.asksNothing")} />
                      <InlinePoints inline={a.ext?.inline} />
                    </div>
                    <SettingsRow label={t("apps.data", { size: fileSize(a.data_bytes) })}>
                      <SettingsValue>
                        <span data-testid="app-data" data-bytes={a.data_bytes ?? 0}>
                          {fileSize(a.data_bytes)}
                        </span>
                      </SettingsValue>
                      <button
                        type="button"
                        className="btn sm outline"
                        data-testid="app-clear"
                        disabled={busy || !a.data_bytes}
                        onClick={() =>
                          void confirmed(t("apps.clearData"), t("apps.clearDataConfirm", { name, size: fileSize(a.data_bytes) }), async () => {
                            const done = await host.call<{ cleared_bytes?: number }>("packages.clearData", { id: a.id });
                            host.shell.notice(t("apps.cleared", { name, size: fileSize(done.cleared_bytes) }));
                          })
                        }
                      >
                        {t("apps.clearData")}
                      </button>
                    </SettingsRow>
                  </div>
                );
              })}
            </>
          );
        }}
      </SettingsSection>
      <SettingsSection
        title={t("apps.store")}
        load={() =>
          host.call<{ name?: string | null; store?: string; apps?: CatalogApp[] }>("packages.catalog", {}).then((catalog) => {
            setLatest(Object.fromEntries((catalog.apps ?? []).filter((c) => c.latest).map((c) => [c.id, c.latest as string])));
            return catalog;
          })
        }
      >
        {(catalog) => (
          <>
            <div className="hint">{catalog.name ? `${catalog.name} · ${catalog.store ?? ""}` : catalog.store}</div>
            {!(catalog.apps ?? []).length ? <div className="hint">{t("apps.storeEmpty")}</div> : null}
            {(catalog.apps ?? []).map((c) => (
              <div key={c.id} data-testid="app-offered" data-app={c.id}>
                <SettingsRow label={`${c.title || c.id} ${c.latest ?? ""}`} hint={[c.id, c.publisher, c.description].filter(Boolean).join(" · ")}>
                  {c.installed ? (
                    <SettingsValue tone="ok">{t("apps.installedVersion", { version: c.installed })}</SettingsValue>
                  ) : c.installable ? (
                    <button type="button" className="btn sm outline" data-testid="app-install" disabled={busy} onClick={() => void ask("packages.install", c.id)}>
                      {t("apps.install")}
                    </button>
                  ) : (
                    <SettingsValue>{t("apps.noBuild")}</SettingsValue>
                  )}
                </SettingsRow>
              </div>
            ))}
          </>
        )}
      </SettingsSection>
    </>
  );
}

const trustText = (value: string | undefined) => t(`apps.trust.${value ?? "unsigned"}`);

/** What the dialog is titled: installing a version, or updating from one to another. */
function disclosureTitle({ method, asked }: Pending): string {
  const d = asked.disclosure ?? {};
  const name = d.title || asked.id;
  const update = method === "packages.update" ? d.update : undefined;
  return update ? t("apps.askUpdate", { name, from: update.from, version: asked.version }) : t("apps.askInstall", { name, version: asked.version });
}

/**
 * What the package would be granted, in the shell's dialog: only "Install" / "Update" here sends
 * the digest back. Asked again when what would be installed changed since it was shown.
 */
function Disclosure({ host, first, close }: { host: HostApi; first: Pending; close: (done: true | null) => void }) {
  const t = useT();
  const [pending, setPending] = useState(first);
  const [busy, setBusy] = useState(false);
  const consent = async () => {
    setBusy(true);
    try {
      const done = await host.call<Asked>(pending.method, {
        id: pending.asked.id,
        version: pending.version,
        consent: pending.asked.digest,
        ...(pending.downgrade ? { allow_downgrade: true } : {}),
      });
      // Something changed since it was shown: show what it is now.
      if (done.consent === "required") setPending({ ...pending, asked: done });
      else close(true);
    } catch (e) {
      host.shell.notice(errorText(e), "error");
      close(null);
    } finally {
      setBusy(false);
    }
  };
  const { asked } = pending;
  const d = asked.disclosure ?? {};
  const update = pending.method === "packages.update" ? d.update : undefined;
  return (
    <>
      <DialogBody>
        <div data-testid="app-disclosure">
          {d.description ? <div className="hint">{d.description}</div> : null}
          <SettingsRow label={t("apps.publisher")} hint={asked.id}>
            <SettingsValue>{d.publisher ?? "—"}</SettingsValue>
          </SettingsRow>
          <SettingsRow label={t("apps.signature")}>
            <SettingsValue tone={d.trust === "unsigned" ? "err" : "ok"}>{trustText(d.trust)}</SettingsValue>
          </SettingsRow>
          {update?.downgrade ? (
            <div className="hint err" data-testid="app-downgrade">
              {t("apps.downgrade", { from: update.from, version: asked.version })}
            </div>
          ) : null}
          <div className="hint">{update ? t("apps.asksMore") : t("apps.asks")}</div>
          <Lines lines={update ? update.added : d.lines ?? []} empty={t("apps.asksNothing")} />
          {update ? (
            <div className="hint">{t("apps.updateKeeps")}</div>
          ) : (
            <>
              <div className="hint">{d.ui_in_page ? t("apps.interfaceInPage") : t("apps.interfaceNone")}</div>
              <InlinePoints inline={d.inline} />
            </>
          )}
        </div>
      </DialogBody>
      <DialogFoot>
        <button type="button" className="btn outline" disabled={busy} onClick={() => close(null)}>
          {t("common.cancel")}
        </button>
        <button type="button" className="btn primary" data-testid="app-consent" disabled={busy} onClick={() => void consent()}>
          {update ? t("apps.consentUpdate") : t("apps.consentInstall")}
        </button>
      </DialogFoot>
    </>
  );
}
