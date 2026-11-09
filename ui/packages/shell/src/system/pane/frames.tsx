/** The frames tab: the session's frames as they arrive. */

import { useEffect, useRef, useState } from "react";
import { clip, useT } from "@harnix/react";

import { keyOf, type App, type SessionKey } from "../../app.ts";
import { since } from "../../format.ts";
import { SYSTEM } from "../constants.ts";

interface FrameRow {
  at: number;
  seq?: number;
  kind: string;
  frame: Record<string, unknown>;
}

/**
 * Watching a session here is `harnix.attach` like any viewer's, claiming the system space: the
 * frames that arrive are the ones every viewer gets, in order, with their `seq`. Leaving the tab
 * detaches, unless this page has the session open somewhere else.
 */
export function FramesTab({ app, session }: { app: App; session: SessionKey }) {
  const tr = useT();
  const [rows, setRows] = useState<FrameRow[]>([]);
  const [paused, setPaused] = useState(false);
  const [kind, setKind] = useState("");
  const pausedRef = useRef(paused);
  pausedRef.current = paused;
  useEffect(() => {
    const target = { instance: session.instance, session_id: session.sessionId };
    let alive = true;
    const off = app.client.onNotify((method, params) => {
      if (!alive || pausedRef.current || params.session_id !== session.sessionId) return;
      if (params.instance !== undefined && params.instance !== session.instance) return;
      const event = (params.event ?? {}) as { kind?: string };
      const row: FrameRow = { at: Date.now(), seq: params.seq as number | undefined, kind: event.kind ?? method, frame: { method, params } };
      setRows((now) => [...now.slice(-499), row]);
    });
    void app.call("harnix.attach", { ...target, space: SYSTEM }).catch(() => undefined);
    return () => {
      alive = false;
      off();
      const open = Object.values(app.snapshot.currentBySpace).some((k) => k && keyOf(k.instance, k.sessionId) === keyOf(session.instance, session.sessionId));
      if (!open) void app.call("harnix.detach", target).catch(() => undefined);
    };
  }, [app, session.instance, session.sessionId]);
  const kinds = [...new Set(rows.map((r) => r.kind))];
  const shown = kind ? rows.filter((r) => r.kind === kind) : rows;
  return (
    <div className="system-pane" data-testid="system-frames">
      <div className="tools-row">
        <select value={kind} onChange={(e) => setKind(e.target.value)} data-testid="frames-kind">
          <option value="">{tr("system.frames.all")}</option>
          {kinds.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
        <button type="button" className={`btn sm outline${paused ? " on" : ""}`} onClick={() => setPaused(!paused)}>
          {paused ? tr("system.frames.resume") : tr("system.frames.pause")}
        </button>
        <button type="button" className="btn sm outline" onClick={() => void navigator.clipboard?.writeText(JSON.stringify(shown.map((r) => r.frame), null, 2)).catch(() => undefined)}>
          {tr("system.copyJson")}
        </button>
      </div>
      {!shown.length ? <div className="note">{tr("system.frames.waiting")}</div> : null}
      <table className="frames">
        <tbody>
          {shown
            .slice()
            .reverse()
            .map((row, i) => (
              <tr key={`${row.at}-${i}`} data-kind={row.kind}>
                <td>{row.seq ?? ""}</td>
                <td>{row.kind}</td>
                <td>{since(row.at)}</td>
                <td className="truncate">{clip(JSON.stringify(row.frame.params), 140)}</td>
              </tr>
            ))}
        </tbody>
      </table>
    </div>
  );
}
