/** The right column for a space: its sessions, every instance's. */

import { useEffect, useState } from "react";
import { useT, type HostSessionRef as SessionKey } from "@harnix/react";

import { SYSTEM } from "../constants.ts";
import { Failure } from "../failure.tsx";
import type { SessionRowView } from "../types.ts";
import type { View } from "../view.ts";

/** One space's sessions, every instance's: the one a person came from first, then by when last active. */
export function SpaceSessions({ view, space, current }: { view: View; space: string; current: SessionKey | null }) {
  const host = view.host;
  const tr = useT();
  const [rows, setRows] = useState<SessionRowView[] | null>(null);
  const [failure, setFailure] = useState<unknown>(null);
  useEffect(() => {
    let alive = true;
    const load = () =>
      host
        .call<{ sessions: SessionRowView[] }>("harnix.sessions.list", { space: SYSTEM })
        .then((r) => alive && setRows((r.sessions ?? []).filter((row) => (row.space ?? host.defaults.space) === space)))
        .catch((e) => alive && setFailure(e ?? "error"));
    void load();
    const off = host.events.onTurnSettled(() => void load());
    return () => {
      alive = false;
      off();
    };
  }, [host, space]);
  if (failure) return <Failure error={failure} testId="system-space-failed" />;
  if (!rows) return null;
  const isCurrent = (row: SessionRowView) => current != null && row.session_id === current.sessionId && (row.instance ?? host.defaults.instance) === current.instance;
  const sorted = [...rows].sort((a, b) => Number(isCurrent(b)) - Number(isCurrent(a)) || String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")));
  return (
    <div className="system-pane" data-testid="system-space" data-space={space}>
      <div className="label">{tr("system.spaceSessionsCount", { space, count: rows.length })}</div>
      {!sorted.length ? <div className="note">{tr("sidebar.noSessions")}</div> : null}
      <div className="inst-sessions">
        {sorted.map((row) => {
          const instance = row.instance ?? host.defaults.instance;
          return (
            <button
              type="button"
              key={`${instance}/${row.session_id}`}
              className={`srow sub${isCurrent(row) ? " on" : ""}`}
              data-testid="space-session"
              data-sid={row.session_id}
              data-instance={instance}
              data-current={isCurrent(row) ? "true" : undefined}
              title={[row.name, row.scene_layer ?? row.scene, row.project_root].filter(Boolean).join(" · ")}
              onClick={() => view.select({ kind: "session", instance, sessionId: row.session_id })}
            >
              <span className="tt name">{row.name || tr("sidebar.untitled")}</span>
              {isCurrent(row) ? <span className="space-tag">{tr("system.currentSession")}</span> : null}
              <span className="space-tag">{instance}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
