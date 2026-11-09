/** The turns tab: a row per turn, opened into its calls. */

import { Fragment, useEffect, useState } from "react";
import { errorText } from "@harnix/core";
import { useT } from "@harnix/react";

import type { App, SessionKey } from "../../app.ts";
import { compact } from "../../format.ts";
import { usageText } from "../format.ts";
import type { TraceCall, TurnRow } from "../types.ts";

/**
 * Per turn (DSH's statistics, for the system space): a row per turn — its calls, time to first
 * token, model and tool time, tokens, the cache's share by tokens, output rate, the context at its
 * end — opened into its calls. The context column shows the change from the turn before, or the
 * running total.
 */
export function TurnsTab({ app, session }: { app: App; session: SessionKey }) {
  const tr = useT();
  const [data, setData] = useState<{ turns: TurnRow[]; calls: TraceCall[] } | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [mode, setMode] = useState<"delta" | "total">("delta");
  const [open, setOpen] = useState<Set<number>>(new Set());
  useEffect(() => {
    let alive = true;
    const target = { instance: session.instance, session_id: session.sessionId };
    const load = () =>
      Promise.all([app.call<{ turns?: TurnRow[] }>("harnix.system.turns", target), app.call<{ calls?: TraceCall[] }>("harnix.system.trace", target)])
        .then(([t, c]) => alive && (setData({ turns: t.turns ?? [], calls: c.calls ?? [] }), setFailure(null)))
        .catch((e) => alive && setFailure(errorText(e)));
    void load();
    const off = app.client.onNotify((method, params) => {
      if (method === "harnix.turn_settled" && params.session_id === session.sessionId) void load();
    });
    return () => {
      alive = false;
      off();
    };
  }, [app, session.instance, session.sessionId]);
  if (failure) return <div className="empty">{failure}</div>;
  if (!data) return null;
  if (!data.turns.length) return <div className="empty">{tr("system.noCalls")}</div>;
  const sec = (ms?: number | null) => (ms != null ? `${(ms / 1000).toFixed(1)}s` : "—");
  const pctOf = (x?: number | null) => (x != null ? `${Math.round(x * 100)}%` : "—");
  const signed = (n?: number | null) => (n == null ? "—" : n > 0 ? `+${compact(n)}` : n < 0 ? `−${compact(-n)}` : "0");
  const toggle = (turn: number) => setOpen((was) => {
    const next = new Set(was);
    if (next.has(turn)) next.delete(turn);
    else next.add(turn);
    return next;
  });
  return (
    <div className="system-pane turns" data-testid="system-turns">
      <div className="tools-row">
        {(["delta", "total"] as const).map((m) => (
          <button type="button" key={m} className={`btn sm outline${mode === m ? " on" : ""}`} data-testid={`turns-${m}`} onClick={() => setMode(m)}>
            {tr(`system.turns.${m}`)}
          </button>
        ))}
      </div>
      <table className="frames">
        <thead>
          <tr>
            <th>{tr("system.turns.turn")}</th>
            <th>{tr("system.turns.calls")}</th>
            <th>TTFT</th>
            <th>{tr("system.turns.model")}</th>
            <th>{tr("system.turns.tools")}</th>
            <th>{tr("system.turns.inOut")}</th>
            <th>{tr("system.turns.cache")}</th>
            <th>{tr("system.turns.rate")}</th>
            <th>{mode === "delta" ? tr("system.turns.contextDelta") : tr("system.turns.context")}</th>
          </tr>
        </thead>
        <tbody>
          {data.turns.map((row) => (
            <Fragment key={row.turn}>
              <tr className="turn-row" data-testid="turn-row" data-turn={row.turn} onClick={() => toggle(row.turn)}>
                <td>{row.turn}</td>
                <td>
                  {row.calls.length}
                  {row.auxiliary ? <span className="muted"> +{row.auxiliary}</span> : null}
                </td>
                <td>{sec(row.ttft_ms)}</td>
                <td>{sec(row.model_ms)}</td>
                <td>{sec(row.tool_ms)}</td>
                <td>
                  {compact(row.input_tokens)} / {compact(row.output_tokens)}
                </td>
                <td data-testid="turn-cache">{pctOf(row.cache_hit)}</td>
                <td>{row.tokens_per_s != null ? row.tokens_per_s : "—"}</td>
                <td data-testid="turn-context">{mode === "delta" ? signed(row.context_delta) : row.context_tokens != null ? compact(row.context_tokens) : "—"}</td>
              </tr>
              {open.has(row.turn)
                ? row.calls.map((index) => {
                    const call = data.calls.find((c) => c.index === index);
                    if (!call) return null;
                    return (
                      <tr key={index} className="step-row" data-testid="turn-step" data-call={index}>
                        <td />
                        <td>#{index + 1}</td>
                        <td>{sec(call.timing?.ttft_ms)}</td>
                        <td>{sec(call.duration_ms)}</td>
                        <td>{call.tools.map((tool) => tool.name).join(", ") || "—"}</td>
                        <td colSpan={2}>{usageText(call.usage)}</td>
                        <td>{call.timing?.tokens_per_s ?? "—"}</td>
                        <td />
                      </tr>
                    );
                  })
                : null}
            </Fragment>
          ))}
        </tbody>
      </table>
    </div>
  );
}
