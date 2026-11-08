/**
 * Settings → Applications: the runtime applications installed here, and the store's catalog.
 *
 * Installed: each with its version, how far it is trusted (countersigned by the store, signed by a
 * publisher the profile trusts, or unsigned), what it keeps, what it asks for, and its switches —
 * enable or disable, update when the store has a newer version, uninstall.
 *
 * The store: what it offers this Harnix, each with an install button. Installing and updating go
 * through the disclosure: the backend answers what the application would be granted (for an
 * update, only what it adds) and a digest, the panel shows it, and only "Install" / "Update" in it
 * sends the digest back. Nothing is installed by a click anywhere else. An update that asks for
 * nothing more is done without asking.
 *
 * Registered only when the backend hosts applications (`harnix.apps.list` is reachable).
 */

import { useState } from "react";
import { errorText } from "@harnix/core";
import { useT } from "@harnix/react";

import type { App } from "./app.ts";
import { Row, Section, Value, useReload } from "./settings.tsx";

interface Line {
  capability: string;
  value: string | number;
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
  app?: { publisher?: string; description?: string | null; lines?: Line[] } | null;
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
    trust?: string;
    lines?: Line[];
    update?: { from: string; added: Line[] };
  };
}

/** What a person is asked to agree to, and how to go ahead. */
interface Pending {
  method: "harnix.apps.install" | "harnix.apps.update";
  asked: Asked;
  version?: string;
}

function size(bytes: number | undefined): string {
  const n = bytes ?? 0;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
}

function LineText({ line }: { line: Line }) {
  const t = useT();
  const value = line.capability === "storage_mb" ? t("apps.megabytes", { size: String(line.value) }) : String(line.value);
  const from = line.from !== undefined ? ` (${t("apps.from", { value: t("apps.megabytes", { size: String(line.from) }) })})` : "";
  return (
    <li data-capability={line.capability}>
      <span className="apps-cap">{t(`apps.cap.${line.capability}`)}</span> <code>{value}</code>
      {from}
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

export function AppsSettings({ app }: { app: App }) {
  const t = useT();
  const reload = useReload();
  const [pending, setPending] = useState<Pending | null>(null);
  const [busy, setBusy] = useState(false);
  const [latest, setLatest] = useState<Record<string, string>>({});

  const fail = (e: unknown) => app.notice(errorText(e), "error");
  /** Ask; a disclosure waits for the person, anything else is done. */
  const ask = async (method: Pending["method"], id: string, version?: string) => {
    setBusy(true);
    try {
      const asked = await app.call<Asked>(method, { id, version });
      if (asked.consent === "required") setPending({ method, asked, version });
      else {
        if (asked.up_to_date) app.notice(t("apps.upToDate", { name: id }));
        reload();
      }
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  const consent = async () => {
    if (!pending) return;
    setBusy(true);
    try {
      const done = await app.call<Asked>(pending.method, { id: pending.asked.id, version: pending.version, consent: pending.asked.digest });
      // Something changed since it was shown: show what it is now.
      if (done.consent === "required") setPending({ ...pending, asked: done });
      else {
        setPending(null);
        reload();
      }
    } catch (e) {
      setPending(null);
      fail(e);
    } finally {
      setBusy(false);
    }
  };
  const act = (method: string, params: unknown) =>
    app
      .call(method, params)
      .then(reload)
      .catch(fail);

  const trust = (value: string | undefined) => t(`apps.trust.${value ?? "unsigned"}`);

  return (
    <>
      {pending ? <Disclosure pending={pending} busy={busy} onConsent={() => void consent()} onCancel={() => setPending(null)} trust={trust} /> : null}
      <Section title={t("apps.installed")} load={() => app.call<{ apps?: Installed[] }>("harnix.apps.list", {})}>
        {(data) => {
          const apps = data.apps ?? [];
          if (!apps.length) return <div className="hint">{t("apps.none")}</div>;
          return (
            <>
              {apps.map((a) => {
                const newer = latest[a.id] && latest[a.id] !== a.version ? latest[a.id] : null;
                const state = a.runtime?.state;
                const hint = [a.id, trust(a.trust), t("apps.data", { size: size(a.data_bytes) })];
                if (state === "failed") hint.push(t("apps.failed", { reason: a.runtime?.reason ?? "" }));
                return (
                  <div key={a.id} className="apps-app" data-testid="app-installed" data-app={a.id} data-version={a.version} data-trust={a.trust ?? "unsigned"}>
                    <Row label={`${a.title || a.id} ${a.version}`} hint={hint.join(" · ")}>
                      <Value tone={state === "running" ? "ok" : undefined}>{a.enabled ? t("apps.enabled") : t("apps.disabled")}</Value>
                      <button
                        type="button"
                        className="btn sm outline"
                        data-testid="app-toggle"
                        disabled={busy}
                        onClick={() => void act(a.enabled ? "harnix.apps.disable" : "harnix.apps.enable", { id: a.id })}
                      >
                        {a.enabled ? t("apps.disable") : t("apps.enable")}
                      </button>
                      {a.source === "store" ? (
                        <button type="button" className="btn sm outline" data-testid="app-update" disabled={busy} onClick={() => void ask("harnix.apps.update", a.id)}>
                          {newer ? t("apps.updateTo", { version: newer }) : t("apps.update")}
                        </button>
                      ) : null}
                      <button
                        type="button"
                        className="btn sm danger"
                        data-testid="app-uninstall"
                        disabled={busy}
                        onClick={() => {
                          if (confirm(t("apps.uninstallConfirm", { name: a.title || a.id }))) void act("harnix.apps.uninstall", { id: a.id });
                        }}
                      >
                        {t("apps.uninstall")}
                      </button>
                    </Row>
                    <Lines lines={a.app?.lines ?? []} empty={t("apps.asksNothing")} />
                  </div>
                );
              })}
            </>
          );
        }}
      </Section>
      <Section
        title={t("apps.store")}
        load={() =>
          app.call<{ name?: string | null; store?: string; apps?: CatalogApp[] }>("harnix.apps.catalog", {}).then((catalog) => {
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
                <Row label={`${c.title || c.id} ${c.latest ?? ""}`} hint={[c.id, c.publisher, c.description].filter(Boolean).join(" · ")}>
                  {c.installed ? (
                    <Value tone="ok">{t("apps.installedVersion", { version: c.installed })}</Value>
                  ) : c.installable ? (
                    <button type="button" className="btn sm outline" data-testid="app-install" disabled={busy} onClick={() => void ask("harnix.apps.install", c.id)}>
                      {t("apps.install")}
                    </button>
                  ) : (
                    <Value>{t("apps.noBuild")}</Value>
                  )}
                </Row>
              </div>
            ))}
          </>
        )}
      </Section>
    </>
  );
}

function Disclosure({
  pending,
  busy,
  onConsent,
  onCancel,
  trust,
}: {
  pending: Pending;
  busy: boolean;
  onConsent: () => void;
  onCancel: () => void;
  trust: (value: string | undefined) => string;
}) {
  const t = useT();
  const { asked } = pending;
  const d = asked.disclosure ?? {};
  const name = d.title || asked.id;
  const update = pending.method === "harnix.apps.update" ? d.update : undefined;
  return (
    <div className="apps-disclosure" data-testid="app-disclosure" role="dialog" aria-label={name}>
      <h3>
        {update
          ? t("apps.askUpdate", { name, from: update.from, version: asked.version })
          : t("apps.askInstall", { name, version: asked.version })}
      </h3>
      <Row label={t("apps.publisher")} hint={asked.id}>
        <Value>{d.publisher ?? "—"}</Value>
      </Row>
      <Row label={t("apps.signature")}>
        <Value tone={d.trust === "unsigned" ? "err" : "ok"}>{trust(d.trust)}</Value>
      </Row>
      <div className="hint">{update ? t("apps.asksMore") : t("apps.asks")}</div>
      <Lines lines={update ? update.added : d.lines ?? []} empty={t("apps.asksNothing")} />
      <div className="apps-actions">
        <button type="button" className="btn sm outline" disabled={busy} onClick={onCancel}>
          {t("common.cancel")}
        </button>
        <button type="button" className="btn sm primary" data-testid="app-consent" disabled={busy} onClick={onConsent}>
          {update ? t("apps.consentUpdate") : t("apps.consentInstall")}
        </button>
      </div>
    </div>
  );
}
