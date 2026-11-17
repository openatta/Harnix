/**
 * The extension packages as they run (`PackagesTab`, a block of the settings' diagnostics): the ones
 * this process hosts, and how each one's VM is doing — its backend up or
 * down, how often it was recycled or rebuilt, its faults, its memory and storage against their
 * limits, the calls into its methods and their errors, and the calls it made as itself that were
 * refused. What this page's frames of it do is the shell's, above it in the same section.
 *
 * Extension processes that start unsandboxed, or not at all for want of a sandbox, are said in red
 * above the table; a process's last lines of stderr are in its row opened.
 *
 * Fed by the topic `packages.list` (the same value as `packages.list`, pushed when it changes), so
 * it stays current without asking.
 */

import { Fragment, useEffect, useState } from "react";
import type { PackageRow, PackagesList } from "@harnix/core";
import { megabytes, rowKeys, useT, type HostApi } from "@harnix/react";

import { SandboxAlert } from "./sandbox.tsx";

/** What the row's dot says: running, idle with its backend stopped until the next call, set aside
 *  after faulting, switched off, or did not start. */
function appState(row: PackageRow): string {
  if (!row.enabled) return "stopped";
  return row.runtime?.state ?? "stopped";
}

export function PackagesTab({ host }: { host: HostApi }) {
  const tr = useT();
  const [list, setList] = useState<PackagesList | undefined>(undefined);
  const [open, setOpen] = useState<string | null>(null);
  useEffect(() => host.subscribe("packages.list", {}, (value) => setList((value as PackagesList | null) ?? undefined)), [host]);
  if (!list) return <div className="packages-runtime" data-testid="packages-runtime">{tr("packages.runtime.loading")}</div>;
  if (!list.packages.length) {
    return (
      <div className="packages-runtime" data-testid="packages-runtime">
        <SandboxAlert sandbox={list.runtime?.sandbox} />
        <div className="muted" data-testid="packages-runtime-empty">{tr("packages.runtime.none")}</div>
      </div>
    );
  }
  const dot: Record<string, string> = { running: "running", hibernating: "cold", faulted: "waiting", stopped: "cold", failed: "waiting" };
  // Five columns, each a word or two: what a package is and how it answers. Its VM's figures
  // (recycles, faults, memory, storage) are in the row opened, which has the room for them.
  return (
    <div className="packages-runtime" data-testid="packages-runtime">
      <SandboxAlert sandbox={list.runtime?.sandbox} />
      <div className="table-wrap">
        <table className="packages-table">
          <thead>
            <tr>
              <th>{tr("packages.runtime.app")}</th>
              <th>{tr("packages.runtime.state")}</th>
              <th>{tr("packages.runtime.backend")}</th>
              <th>{tr("packages.runtime.calls")}</th>
              <th>{tr("packages.runtime.refused")}</th>
            </tr>
          </thead>
          <tbody data-rows="">
            {list.packages.map((row) => {
              const state = appState(row);
              const rt = row.runtime;
              const h = rt?.health ?? undefined;
              const backend = h?.world === "backend" ? (h.hibernating ? "asleep" : h.state === "running" ? "up" : "down") : null;
              const refused = rt?.calls?.refused ?? [];
              const stderr = rt?.process?.stderr ?? [];
              const shown = open === row.id;
              const flip = () => setOpen(shown ? null : row.id);
              return (
                <Fragment key={row.id}>
                  <tr
                    className="package-row"
                    data-testid="packages-runtime-row"
                    data-package={row.id}
                    data-state={state}
                    data-sandbox={row.sandbox}
                    data-row=""
                    tabIndex={0}
                    aria-expanded={shown}
                    onClick={flip}
                    onKeyDown={rowKeys(flip, { open: shown, set: (to) => setOpen(to ? row.id : null) })}
                  >
                    <td className="package-name">
                      <span className={`packages-dot ${dot[state]}`} /> <span className="tt">{row.title ?? row.id}</span>
                      {row.sandbox && row.sandbox !== "on" ? <span className="packages-sandbox-tag">{tr("packages.sandbox.tag")}</span> : null}
                      <div className="muted">{row.id} {row.version}</div>
                    </td>
                    <td data-field="state">{tr(`packages.runtime.state.${state}`)}</td>
                    <td data-field="backend">{backend ? tr(`packages.runtime.backend.${backend}`) : "—"}</td>
                    <td data-field="calls">{rt?.calls ? `${rt.calls.calls} · ${rt.calls.errors}` : "—"}</td>
                    <td data-field="refused">{refused.length}</td>
                  </tr>
                  {shown ? (
                    <tr className="package-detail" data-testid="packages-runtime-detail" data-package={row.id}>
                      <td colSpan={5}>
                        {h ? (
                          <dl className="facts" data-testid="packages-runtime-vm">
                            <div className="fact" data-field="recycles"><dt>{tr("packages.runtime.recycles")}</dt><dd>{`${h.recycles ?? 0} / ${h.rebuilds ?? 0}`}</dd></div>
                            <div className="fact" data-field="faults"><dt>{tr("packages.runtime.faults")}</dt><dd>{`${h.faults ?? 0} · ${h.timeouts ?? 0}`}</dd></div>
                            <div className="fact" data-field="memory"><dt>{tr("packages.runtime.memory")}</dt><dd>{`${megabytes(h.memory_bytes)} / ${megabytes(h.memory_limit_bytes)}`}</dd></div>
                            <div className="fact" data-field="storage"><dt>{tr("packages.runtime.storage")}</dt><dd>{`${megabytes(h.storage_bytes)} / ${megabytes(h.storage_limit_bytes)}`}</dd></div>
                          </dl>
                        ) : null}
                        {rt?.reason ? <div role="alert" data-testid="packages-runtime-reason">{rt.reason}</div> : null}
                        {h?.last_error ? <div className="muted">{tr("packages.runtime.lastError", { error: h.last_error })}</div> : null}
                        {rt?.methods?.length ? <div className="muted">{tr("packages.runtime.methods", { methods: rt.methods.join(", ") })}</div> : null}
                        {rt?.topics?.length ? <div className="muted">{tr("packages.runtime.topics", { topics: rt.topics.join(", ") })}</div> : null}
                        {stderr.length ? (
                          <>
                            <div className="packages-runtime-h">{tr("packages.runtime.stderr")}</div>
                            <pre className="packages-stderr" data-testid="packages-runtime-stderr">{stderr.join("\n")}</pre>
                          </>
                        ) : null}
                        <div className="packages-runtime-h">{tr("packages.runtime.refusedCalls")}</div>
                        {refused.length ? (
                          <ul>
                            {refused.map((r, i) => (
                              <li key={i} data-testid="packages-runtime-refusal">
                                <code>{r.method}</code> — {r.reason}
                              </li>
                            ))}
                          </ul>
                        ) : (
                          <div className="muted">{tr("packages.runtime.noRefusals")}</div>
                        )}
                      </td>
                    </tr>
                  ) : null}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </div>
      {list.unreadable?.length ? (
        <div className="muted">{tr("packages.runtime.unreadable", { dirs: list.unreadable.map((u) => u.dir).join(", ") })}</div>
      ) : null}
    </div>
  );
}
