/** The frames tab: the session's frames as they arrive. */

import { useEffect, useRef, useState } from "react";
import { clip, useT, since, type HostApi, type HostSessionRef as SessionKey } from "@harnix/react";

import { SYSTEM } from "../constants.ts";

interface FrameRow {
  at: number;
  seq?: number;
  kind: string;
  frame: Record<string, unknown>;
}

/**
 * Watching a session here is `harnix.session.attach` like any viewer's, claiming the system space: the
 * events that arrive are the ones every viewer gets, in order, and each turn's end. Leaving the tab
 * detaches, unless this page has the session open somewhere else.
 */
export function FramesTab({ host, session }: { host: HostApi; session: SessionKey }) {
  const tr = useT();
  const [rows, setRows] = useState<FrameRow[]>([]);
  const [paused, setPaused] = useState(false);
  const [kind, setKind] = useState("");
  const pausedRef = useRef(paused);
  pausedRef.current = paused;
  useEffect(() => {
    const target = { instance: session.instance, session_id: session.sessionId };
    let alive = true;
    const push = (kind: string, frame: Record<string, unknown>) => {
      if (!alive || pausedRef.current) return;
      setRows((now) => [...now.slice(-499), { at: Date.now(), kind, frame }]);
    };
    const offEvents = host.events.onSessionEvent(session, (e) => {
      const kind = (e.raw as { kind?: string } | null)?.kind ?? "harnix.event";
      push(kind, { method: "harnix.event", params: { ...target, event: e.raw, ...(e.trust === "extension" ? { trust: e.trust } : {}) } });
    });
    const offSettled = host.events.onTurnSettled((e) => {
      if (e.session_id !== session.sessionId || (e.instance !== undefined && e.instance !== session.instance)) return;
      push("harnix.turn_settled", { method: "harnix.turn_settled", params: e });
    });
    void host.call("harnix.session.attach", { ...target, space: SYSTEM }).catch(() => undefined);
    return () => {
      alive = false;
      offEvents();
      offSettled();
      if (!host.session.isOpen(session)) void host.call("harnix.session.detach", target).catch(() => undefined);
    };
  }, [host, session.instance, session.sessionId]);
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
