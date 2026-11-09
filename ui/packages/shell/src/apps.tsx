/**
 * Settings → Applications: the runtime applications installed here, and the store's catalog.
 *
 * Installed: each with its version, how far it is trusted (countersigned by the store, signed by a
 * publisher the profile trusts, or unsigned), whether it runs; its permissions — every capability
 * as a sentence, and what the shell draws for it from its manifest; what it keeps, with a button
 * that clears it; and its switches — enable or disable, update when the store has a newer version,
 * uninstall. Above them, whether the left column marks the spaces applications brought.
 *
 * The store: what it offers this Harnix, each with an install button. Installing and updating go
 * through the disclosure, a modal over the settings: the backend answers what the application would
 * be granted (for an update, only what it adds) and a digest, the modal shows it, and only
 * "Install" / "Update" in it sends the digest back. Nothing is installed by a click anywhere else.
 * An update that asks for nothing more is done without asking. Clearing and uninstalling ask in a
 * modal of their own too.
 *
 * Registered only when the backend hosts applications (`harnix.apps.list` is reachable).
 */

import { useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { errorText } from "@harnix/core";
import { useLayer, useT } from "@harnix/react";

import type { App } from "./app.ts";
import { fileSize } from "./format.ts";
import { appsOf, type Inline } from "./inline/index.tsx";
import { Row, Section, Value, useReload } from "./settings.tsx";

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
  app?: { publisher?: string; description?: string | null; lines?: Line[]; ui?: unknown[]; inline?: Inline | null } | null;
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
  method: "harnix.apps.install" | "harnix.apps.update";
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

/** A question asked before something that cannot be undone. */
interface Asking {
  testId: string;
  title: string;
  message: string;
  confirm: string;
  run: () => Promise<unknown>;
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
  if (line.capability === "hub" && raw.startsWith("topic:")) {
    key = "apps.cap.hubTopic";
    value = raw.slice("topic:".length);
  }
  if (line.capability === "fs") key = raw.startsWith("${data}") ? "apps.cap.fsData" : "apps.cap.fsProject";
  if (line.capability === "inline") {
    const [what, ...rest] = raw.split(" ");
    key = what === "command" ? "apps.cap.inlineCommand" : "apps.cap.inlineGroup";
    value = rest.join(" ");
  }
  const from = line.from !== undefined ? ` (${t("apps.from", { value: t("apps.megabytes", { size: String(line.from) }) })})` : "";
  const why = ["net", "net_local", "env", "hub", "ui", "ui_hub", "events", "frames", "agent"].includes(line.capability) ? t(`apps.cap.why.${line.capability}`) : "";
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

/**
 * A modal over the settings (which are the shell's modal already): a layer of its own over them
 * (`@harnix/react`'s `layers.ts`) — the focus in it, the settings under it inert, Escape closing only this one. Drawn
 * in the page's layers, not inside the settings, which go inert under it.
 */
function Dialog({ testId, label, onClose, children, actions }: { testId: string; label: string; onClose: () => void; children: ReactNode; actions: ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  // The focus on the box, not on a button: what is asked is read before an answer is a keypress away.
  const layer = useLayer(box, { close: onClose, initial: (el) => el });
  return createPortal(
    <div
      className="apps-veil"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div ref={box} className="apps-dialog" data-testid={testId} role="dialog" aria-modal="true" aria-label={label} tabIndex={-1} onKeyDown={layer.onKeyDown}>
        <h3>{label}</h3>
        <div className="apps-dialog-body">{children}</div>
        <div className="apps-actions">{actions}</div>
      </div>
    </div>,
    document.getElementById("layers") ?? document.body,
  );
}

export function AppsSettings({ app }: { app: App }) {
  const t = useT();
  const reload = useReload();
  const watch = appsOf(app);
  useSyncExternalStore(watch.subscribe, watch.getVersion);
  const [pending, setPending] = useState<Pending | null>(null);
  const [asking, setAsking] = useState<Asking | null>(null);
  const [busy, setBusy] = useState(false);
  const [latest, setLatest] = useState<Record<string, string>>({});

  const fail = (e: unknown) => app.notice(errorText(e), "error");
  /** Ask; a disclosure waits for the person, anything else is done. */
  const ask = async (method: Pending["method"], id: string, version?: string, downgrade?: boolean) => {
    setBusy(true);
    try {
      const asked = await app.call<Asked>(method, { id, version, ...(downgrade ? { allow_downgrade: true } : {}) });
      if (asked.consent === "required") setPending({ method, asked, version, downgrade });
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
      const done = await app.call<Asked>(pending.method, {
        id: pending.asked.id,
        version: pending.version,
        consent: pending.asked.digest,
        ...(pending.downgrade ? { allow_downgrade: true } : {}),
      });
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
  const confirmed = async () => {
    if (!asking) return;
    setBusy(true);
    try {
      await asking.run();
    } catch (e) {
      fail(e);
    } finally {
      setAsking(null);
      setBusy(false);
      reload();
    }
  };

  const trust = (value: string | undefined) => t(`apps.trust.${value ?? "unsigned"}`);

  return (
    <>
      {pending ? <Disclosure pending={pending} busy={busy} onConsent={() => void consent()} onCancel={() => setPending(null)} trust={trust} /> : null}
      {asking ? (
        <Dialog
          testId={asking.testId}
          label={asking.title}
          onClose={() => setAsking(null)}
          actions={
            <>
              <button type="button" className="btn sm outline" disabled={busy} onClick={() => setAsking(null)}>
                {t("common.cancel")}
              </button>
              <button type="button" className="btn sm danger" data-testid={`${asking.testId}-confirm`} disabled={busy} onClick={() => void confirmed()}>
                {asking.confirm}
              </button>
            </>
          }
        >
          <div>{asking.message}</div>
        </Dialog>
      ) : null}
      <Row label={t("apps.mark")} hint={t("apps.markHint")}>
        <input type="checkbox" data-testid="apps-mark-toggle" checked={watch.marked()} onChange={(e) => watch.setMarked(e.target.checked)} />
      </Row>
      <Section title={t("apps.installed")} load={() => app.call<{ apps?: Installed[] }>("harnix.apps.list", {})}>
        {(data) => {
          const apps = data.apps ?? [];
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
                const hint = [a.id, trust(a.trust), t(`apps.state.${state}`)];
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
                    <Row label={`${name} ${a.version}`} hint={hint.filter(Boolean).join(" · ")}>
                      <Value tone={state === "running" || state === "hibernating" ? "ok" : state === "failed" || state === "faulted" ? "err" : undefined}>
                        {a.enabled ? t("apps.enabled") : t("apps.disabled")}
                      </Value>
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
                        back ? (
                          <button
                            type="button"
                            className="btn sm outline"
                            data-testid="app-update"
                            data-direction="back"
                            disabled={busy}
                            onClick={() => void ask("harnix.apps.update", a.id, back, true)}
                          >
                            {t("apps.downgradeTo", { version: back })}
                          </button>
                        ) : (
                          <button type="button" className="btn sm outline" data-testid="app-update" disabled={busy} onClick={() => void ask("harnix.apps.update", a.id)}>
                            {newer ? t("apps.updateTo", { version: newer }) : t("apps.update")}
                          </button>
                        )
                      ) : null}
                      <button
                        type="button"
                        className="btn sm danger"
                        data-testid="app-uninstall"
                        disabled={busy}
                        onClick={() =>
                          setAsking({
                            testId: "app-uninstall-ask",
                            title: t("apps.uninstall"),
                            message: t("apps.uninstallConfirm", { name }),
                            confirm: t("apps.uninstall"),
                            run: () => app.call("harnix.apps.uninstall", { id: a.id }),
                          })
                        }
                      >
                        {t("apps.uninstall")}
                      </button>
                    </Row>
                    <div className="apps-permissions" data-testid="app-permissions">
                      <div className="apps-sub">{t("apps.permissions")}</div>
                      <Lines lines={a.app?.lines ?? []} empty={t("apps.asksNothing")} />
                      <InlinePoints inline={a.app?.inline} />
                    </div>
                    <Row label={t("apps.data", { size: fileSize(a.data_bytes) })}>
                      <Value>
                        <span data-testid="app-data" data-bytes={a.data_bytes ?? 0}>
                          {fileSize(a.data_bytes)}
                        </span>
                      </Value>
                      <button
                        type="button"
                        className="btn sm outline"
                        data-testid="app-clear"
                        disabled={busy || !a.data_bytes}
                        onClick={() =>
                          setAsking({
                            testId: "app-clear-ask",
                            title: t("apps.clearData"),
                            message: t("apps.clearDataConfirm", { name, size: fileSize(a.data_bytes) }),
                            confirm: t("apps.clearData"),
                            run: async () => {
                              const done = await app.call<{ cleared_bytes?: number }>("harnix.apps.clearData", { id: a.id });
                              app.notice(t("apps.cleared", { name, size: fileSize(done.cleared_bytes) }));
                            },
                          })
                        }
                      >
                        {t("apps.clearData")}
                      </button>
                    </Row>
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
  const title = update ? t("apps.askUpdate", { name, from: update.from, version: asked.version }) : t("apps.askInstall", { name, version: asked.version });
  return (
    <Dialog
      testId="app-disclosure"
      label={title}
      onClose={onCancel}
      actions={
        <>
          <button type="button" className="btn sm outline" disabled={busy} onClick={onCancel}>
            {t("common.cancel")}
          </button>
          <button type="button" className="btn sm primary" data-testid="app-consent" disabled={busy} onClick={onConsent}>
            {update ? t("apps.consentUpdate") : t("apps.consentInstall")}
          </button>
        </>
      }
    >
      {d.description ? <div className="hint">{d.description}</div> : null}
      <Row label={t("apps.publisher")} hint={asked.id}>
        <Value>{d.publisher ?? "—"}</Value>
      </Row>
      <Row label={t("apps.signature")}>
        <Value tone={d.trust === "unsigned" ? "err" : "ok"}>{trust(d.trust)}</Value>
      </Row>
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
    </Dialog>
  );
}
