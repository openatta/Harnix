/**
 * The system space's Apps tab: the runtime applications this process hosts, and how each one's VM
 * is doing — its backend up or down, how often it was recycled or rebuilt, its faults, its memory
 * and storage against their limits, the calls into its methods and their errors, and the calls it
 * made as itself that were refused.
 *
 * Fed by the `apps` module's topic `harnix.apps` (the same value as `harnix.apps.list`, pushed
 * when it changes), so it stays current without asking. Offered only where that module runs:
 * `[apps] enabled`.
 */

import { Fragment, useEffect, useState } from "react";
import { useT } from "@harnix/react";

import type { App } from "../app.ts";
import { APPS_LIST, APPS_TOPIC } from "../constants.ts";
import { megabytes } from "../format.ts";
import { SurfaceCount, SurfaceDetail } from "../surfaces/status.tsx";
import { topicsOf } from "../topics.ts";

interface Health {
  world?: "tools" | "backend";
  state?: "running" | "stopped" | "disabled";
  reason?: string;
  calls?: number;
  faults?: number;
  timeouts?: number;
  rebuilds?: number;
  recycles?: number;
  last_error?: string | null;
  memory_bytes?: number;
  memory_limit_bytes?: number;
  storage_bytes?: number;
  storage_limit_bytes?: number;
  subscriptions?: number;
  timers?: number;
  /** An on-demand backend stopped while idle; the next call starts it again. */
  hibernating?: boolean;
  wakes?: number;
}

interface Refusal {
  method: string;
  reason: string;
  at_ms: number;
}

interface Runtime {
  state: "running" | "hibernating" | "faulted" | "stopped" | "failed";
  reason?: string | null;
  world?: string | null;
  methods?: string[];
  topics?: string[];
  calls?: { calls: number; errors: number; hub_calls: number; refused: Refusal[]; recent_errors: { method: string; message: string }[] };
  health?: Health | null;
}

interface AppRow {
  id: string;
  version: string;
  title?: string | null;
  enabled: boolean;
  registered: boolean;
  runtime?: Runtime;
}

interface AppsList {
  apps: AppRow[];
  unreadable?: { dir: string; reason: string }[];
}

/** Whether this process hosts applications: then the tab is there. */
export function appsOffered(app: App): boolean {
  return app.snapshot.reachable.includes(APPS_LIST);
}

/** What the row's dot says: running, idle with its backend stopped until the next call, set aside
 *  after faulting, switched off, or did not start. */
function appState(row: AppRow): Runtime["state"] {
  if (!row.enabled) return "stopped";
  return row.runtime?.state ?? "stopped";
}

export function AppsTab({ app }: { app: App }) {
  const tr = useT();
  const [list, setList] = useState<AppsList | undefined>(undefined);
  const [open, setOpen] = useState<string | null>(null);
  useEffect(() => topicsOf(app.client).subscribe(APPS_TOPIC, {}, (value) => setList((value as AppsList | null) ?? undefined)), [app]);
  if (!list) return <div className="system-pane" data-testid="system-apps">{tr("system.apps.loading")}</div>;
  if (!list.apps.length) {
    return (
      <div className="system-pane" data-testid="system-apps">
        <div className="muted" data-testid="system-apps-empty">{tr("system.apps.none")}</div>
      </div>
    );
  }
  const dot: Record<string, string> = { running: "running", hibernating: "cold", faulted: "waiting", stopped: "cold", failed: "waiting" };
  return (
    <div className="system-pane" data-testid="system-apps">
      <table className="frames">
        <thead>
          <tr>
            <th>{tr("system.apps.app")}</th>
            <th>{tr("system.apps.state")}</th>
            <th>{tr("system.apps.backend")}</th>
            <th>{tr("system.apps.recycles")}</th>
            <th>{tr("system.apps.faults")}</th>
            <th>{tr("system.apps.memory")}</th>
            <th>{tr("system.apps.storage")}</th>
            <th>{tr("system.apps.calls")}</th>
            <th>{tr("system.apps.refused")}</th>
            <th>{tr("system.apps.interface")}</th>
          </tr>
        </thead>
        <tbody>
          {list.apps.map((row) => {
            const state = appState(row);
            const rt = row.runtime;
            const h = rt?.health ?? undefined;
            const backend = h?.world === "backend" ? (h.hibernating ? "asleep" : h.state === "running" ? "up" : "down") : null;
            const refused = rt?.calls?.refused ?? [];
            return (
              <Fragment key={row.id}>
                <tr className="turn-row" data-testid="system-app" data-app={row.id} data-state={state} onClick={() => setOpen(open === row.id ? null : row.id)}>
                  <td>
                    <span className={`dot ${dot[state]}`} /> <span className="tt">{row.title ?? row.id}</span>{" "}
                    <span className="muted">{row.id} {row.version}</span>
                  </td>
                  <td data-field="state">{tr(`system.apps.state.${state}`)}</td>
                  <td data-field="backend">{backend ? tr(`system.apps.backend.${backend}`) : "—"}</td>
                  <td data-field="recycles">{h ? `${h.recycles ?? 0} / ${h.rebuilds ?? 0}` : "—"}</td>
                  <td data-field="faults">{h ? `${h.faults ?? 0} · ${h.timeouts ?? 0}` : "—"}</td>
                  <td data-field="memory">{h ? `${megabytes(h.memory_bytes)} / ${megabytes(h.memory_limit_bytes)}` : "—"}</td>
                  <td data-field="storage">{h ? `${megabytes(h.storage_bytes)} / ${megabytes(h.storage_limit_bytes)}` : "—"}</td>
                  <td data-field="calls">{rt?.calls ? `${rt.calls.calls} · ${rt.calls.errors}` : "—"}</td>
                  <td data-field="refused">{refused.length}</td>
                  <td data-field="surfaces">
                    <SurfaceCount app={row.id} />
                  </td>
                </tr>
                {open === row.id ? (
                  <tr data-testid="system-app-detail" data-app={row.id}>
                    <td colSpan={10}>
                      {rt?.reason ? <div role="alert" data-testid="system-app-reason">{rt.reason}</div> : null}
                      {h?.last_error ? <div className="muted">{tr("system.apps.lastError", { error: h.last_error })}</div> : null}
                      {rt?.methods?.length ? <div className="muted">{tr("system.apps.methods", { methods: rt.methods.join(", ") })}</div> : null}
                      {rt?.topics?.length ? <div className="muted">{tr("system.apps.topics", { topics: rt.topics.join(", ") })}</div> : null}
                      <div className="turnh">{tr("system.apps.refusedCalls")}</div>
                      {refused.length ? (
                        <ul>
                          {refused.map((r, i) => (
                            <li key={i} data-testid="system-app-refusal">
                              <code>{r.method}</code> — {r.reason}
                            </li>
                          ))}
                        </ul>
                      ) : (
                        <div className="muted">{tr("system.apps.noRefusals")}</div>
                      )}
                      <SurfaceDetail app={row.id} />
                    </td>
                  </tr>
                ) : null}
              </Fragment>
            );
          })}
        </tbody>
      </table>
      {list.unreadable?.length ? (
        <div className="muted">{tr("system.apps.unreadable", { dirs: list.unreadable.map((u) => u.dir).join(", ") })}</div>
      ) : null}
    </div>
  );
}
