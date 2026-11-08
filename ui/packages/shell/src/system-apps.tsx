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

import type { App } from "./app.ts";
import { topicsOf } from "./topics.ts";

/** The topic, and the method whose presence says the process hosts applications at all. */
export const APPS_TOPIC = "harnix.apps";
const APPS_LIST = "harnix.apps.list";

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
}

interface Refusal {
  method: string;
  reason: string;
  at_ms: number;
}

interface Runtime {
  state: "running" | "faulted" | "stopped" | "failed";
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

const mb = (bytes?: number) => (bytes == null ? "—" : `${(bytes / 1048576).toFixed(bytes < 1048576 ? 2 : 0)} MB`);

/** What the row's dot says: running, set aside after faulting, switched off, or did not start. */
function stateOf(row: AppRow): "running" | "faulted" | "stopped" | "failed" {
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
  const dot: Record<string, string> = { running: "running", faulted: "waiting", stopped: "cold", failed: "waiting" };
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
          </tr>
        </thead>
        <tbody>
          {list.apps.map((row) => {
            const state = stateOf(row);
            const rt = row.runtime;
            const h = rt?.health ?? undefined;
            const backend = h?.world === "backend" ? (h.state === "running" ? "up" : "down") : null;
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
                  <td data-field="memory">{h ? `${mb(h.memory_bytes)} / ${mb(h.memory_limit_bytes)}` : "—"}</td>
                  <td data-field="storage">{h ? `${mb(h.storage_bytes)} / ${mb(h.storage_limit_bytes)}` : "—"}</td>
                  <td data-field="calls">{rt?.calls ? `${rt.calls.calls} · ${rt.calls.errors}` : "—"}</td>
                  <td data-field="refused">{refused.length}</td>
                </tr>
                {open === row.id ? (
                  <tr data-testid="system-app-detail" data-app={row.id}>
                    <td colSpan={9}>
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
