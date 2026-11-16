/**
 * The right column's usage tab: what this session has used in its current run — turns, calls,
 * tokens in and out (and how much the cache served), time in the model and in tools, the first
 * token's wait, the output rate, how long the run took, the sub-agents' totals — from
 * `observability.usage` and `observability.turns`, and the way into the trajectory.
 *
 * The arithmetic is `@harnix/core`'s `summarizeUsage`, the one the system space's overview uses too.
 */

import { useEffect, useState } from "react";
import { summarizeUsage, type SessionUsage, type TurnSummary } from "@harnix/core";
import { compact, millis, useT, useTabInfo, type HostSessionRef as SessionKey } from "@harnix/react";

import { Failure, RunNote, TRAJECTORY_VIEW, sessionParams, type Observed } from "./shared.tsx";

export function UsageTab({ observed }: { observed: Observed }) {
  const { tab } = useTabInfo();
  if (!tab.session) return null;
  return <SessionUsageView observed={observed} session={tab.session} />;
}

export function SessionUsageView({ observed, session }: { observed: Observed; session: SessionKey }) {
  const tr = useT();
  const host = observed.host;
  const [data, setData] = useState<{ usage: SessionUsage | null; turns: TurnSummary[] } | null>(null);
  const [error, setError] = useState<unknown>(null);
  useEffect(() => {
    let alive = true;
    const params = sessionParams(observed, session);
    const load = () =>
      Promise.all([
        host.call<{ sessions?: Record<string, SessionUsage> }>("observability.usage", { instance: session.instance, session_ids: [session.sessionId] }),
        host.call<{ turns?: TurnSummary[] }>("observability.turns", params),
      ])
        .then(([u, t]) => alive && (setData({ usage: u.sessions?.[session.sessionId] ?? null, turns: t.turns ?? [] }), setError(null)))
        .catch((e) => alive && setError(e ?? "error"));
    void load();
    const off = host.events.onTurnSettled((e) => {
      if (e.session_id === session.sessionId) void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [host, observed, session.instance, session.sessionId]);
  const open = () => {
    const current = host.session.current();
    if (current && current.instance === session.instance && current.sessionId === session.sessionId) host.conversation.show(TRAJECTORY_VIEW);
    else void host.session.open(session, { switchSpace: true, view: TRAJECTORY_VIEW });
  };
  if (error) return <Failure error={error} testId="usage-failed" />;
  if (!data) return null;
  const agents = data.usage?.agents ?? [];
  const own = agents.find((u) => u.session_id === session.sessionId);
  const subs = agents.filter((u) => u.session_id !== session.sessionId);
  const s = summarizeUsage(own, subs, data.turns);
  const pct = (x: number | null) => (x == null ? "—" : `${Math.round(x * 100)}%`);
  const cells: [string, string, string][] = [
    ["turns", tr("system.stat.turns"), String(s.turns)],
    ["calls", tr("system.stat.calls"), s.auxiliary ? `${s.calls} +${s.auxiliary}` : String(s.calls)],
    ["total", tr("system.stat.total"), compact(s.total)],
    ["hit", tr("system.stat.cacheHit"), pct(s.cacheHit)],
    ["rate", tr("system.stat.rate"), s.rate == null ? "—" : `${s.rate.toFixed(1)}/s`],
    ["ttft", tr("system.stat.ttft"), s.ttftMs == null ? "—" : millis(s.ttftMs)],
    ["model", tr("system.stat.time"), millis(s.modelMs)],
    ["tool", tr("system.stat.toolTime"), s.toolMs == null ? "—" : millis(s.toolMs)],
    ["span", tr("usage.span"), s.spanMs == null ? "—" : millis(s.spanMs)],
  ];
  return (
    <div className="system-pane obs-usage" data-testid="usage-tab">
      {!own && !data.turns.length ? (
        <div className="note" data-testid="usage-none">
          {tr("system.stat.noRecording")}
        </div>
      ) : (
        <>
          <div className="stat-grid" data-testid="usage-stats">
            {cells.map(([id, label, value]) => (
              <div key={id} className="cell" data-cell={id}>
                <div className="v">{value}</div>
                <div className="l">{label}</div>
              </div>
            ))}
          </div>
          {own ? (
            <div className="note" data-testid="usage-tokens">
              {tr("system.tokenTip", { input: own.input_tokens, read: own.cache_read_tokens, write: own.cache_write_tokens, output: own.output_tokens })}
              {s.errors ? ` · ${tr("system.stat.errors", { count: s.errors })}` : ""}
            </div>
          ) : null}
          {s.agents.count ? (
            <div className="note" data-testid="usage-agents">
              {tr("system.subTotals", { count: s.agents.count, tokens: compact(s.agents.total), time: millis(s.agents.modelMs) })}
            </div>
          ) : null}
          <RunNote run={data.usage?.run ?? null} testId="usage-run-note" />
          {data.usage?.stale ? <div className="note">{tr("usage.stale")}</div> : null}
        </>
      )}
      <button type="button" className="btn sm outline wide" data-testid="usage-open-trajectory" onClick={open}>
        {tr("usage.openTrajectory")}
      </button>
    </div>
  );
}
