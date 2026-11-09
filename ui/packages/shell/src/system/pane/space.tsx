/** The right column for a space: its sessions, every instance's. */

import { useEffect, useState } from "react";
import { errorText } from "@harnix/core";
import { useT } from "@harnix/react";

import type { App, SessionKey } from "../../app.ts";
import { SYSTEM } from "../constants.ts";
import type { SessionRowView } from "../types.ts";
import type { View } from "../view.ts";

/** One space's sessions, every instance's: the one a person came from first, then by when last active. */
export function SpaceSessions({ app, view, space, current }: { app: App; view: View; space: string; current: SessionKey | null }) {
  const tr = useT();
  const [rows, setRows] = useState<SessionRowView[] | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () =>
      app
        .call<{ sessions: SessionRowView[] }>("harnix.sessions", { space: SYSTEM })
        .then((r) => alive && setRows((r.sessions ?? []).filter((row) => (row.space ?? "sessions") === space)))
        .catch((e) => alive && setFailure(errorText(e)));
    void load();
    const off = app.client.onNotify((method) => {
      if (method === "harnix.turn_settled") void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [app, space]);
  if (failure) return <div className="empty">{failure}</div>;
  if (!rows) return null;
  const isCurrent = (row: SessionRowView) => current != null && row.session_id === current.sessionId && (row.instance ?? "system") === current.instance;
  const sorted = [...rows].sort((a, b) => Number(isCurrent(b)) - Number(isCurrent(a)) || String(b.last_active ?? "").localeCompare(String(a.last_active ?? "")));
  return (
    <div className="system-pane" data-testid="system-space" data-space={space}>
      <div className="label">{tr("system.spaceSessionsCount", { space, count: rows.length })}</div>
      {!sorted.length ? <div className="note">{tr("sidebar.noSessions")}</div> : null}
      <div className="inst-sessions">
        {sorted.map((row) => {
          const instance = row.instance ?? "system";
          return (
            <button
              type="button"
              key={`${instance}/${row.session_id}`}
              className={`srow sub${isCurrent(row) ? " on" : ""}`}
              data-sid={row.session_id}
              data-instance={instance}
              data-current={isCurrent(row) ? "true" : undefined}
              title={[row.name, row.scene, row.project_root].filter(Boolean).join(" · ")}
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
