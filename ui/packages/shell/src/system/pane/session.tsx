/** The right column for a main Agent's session: its facts, goal, context, figures and operations. */

import { useMemo, useState } from "react";
import { useT } from "@harnix/react";

import type { App, SessionKey } from "../../app.ts";
import { confirmDialog, exportDialog, renameDialog } from "../../dialogs.tsx";
import { Glyph } from "../../glyph.tsx";
import { compact, millis } from "../../format.ts";
import { useShell } from "../../hooks.ts";
import { SYSTEM } from "../constants.ts";
import { stateOf, stateSignature, useAgentFacts } from "../data.ts";
import { totalTokens } from "../format.ts";
import { openWhereItBelongs } from "../navigate.ts";
import type { AgentRow, AgentUsage, Context, SessionRowView, TurnRow } from "../types.ts";
import { act, useTopic, type View } from "../view.ts";
import { Facts } from "./facts.tsx";
import { GoalBlock } from "./goal.tsx";

/** The figures DSH puts by the composer, for one Agent: its turns and calls, its tokens, how fast. */
export function Stats({ own, turns }: { own: AgentUsage | undefined; turns: TurnRow[] }) {
  const tr = useT();
  if (!own && !turns.length) return <div className="note" data-testid="agent-stats-none">{tr("system.stat.noRecording")}</div>;
  const sum = (f: (t: TurnRow) => number) => turns.reduce((n, t) => n + f(t), 0);
  const ttfts = turns.map((t) => t.ttft_ms).filter((v): v is number => typeof v === "number");
  const output = own?.output_tokens ?? sum((t) => t.output_tokens);
  // Tokens a second, weighted by what each turn wrote.
  const rated = turns.filter((t) => t.tokens_per_s && t.output_tokens);
  const writing = rated.reduce((n, t) => n + t.output_tokens / t.tokens_per_s!, 0);
  const rate = writing > 0 ? rated.reduce((n, t) => n + t.output_tokens, 0) / writing : null;
  const input = own ? own.input_tokens + own.cache_read_tokens + own.cache_write_tokens : sum((t) => t.input_tokens);
  const hit = own && input > 0 ? own.cache_read_tokens / input : null;
  const cells: [string, string, string][] = [
    ["turns", tr("system.stat.turns"), String(own?.turns ?? turns.length)],
    ["calls", tr("system.stat.calls"), String(own?.calls ?? sum((t) => t.calls.length))],
    ["total", tr("system.stat.total"), compact(input + output)],
    ["hit", tr("system.stat.cacheHit"), hit === null ? "—" : `${Math.round(hit * 100)}%`],
    ["rate", tr("system.stat.rate"), rate === null ? "—" : `${rate.toFixed(1)}/s`],
    ["ttft", tr("system.stat.ttft"), ttfts.length ? millis(Math.round(ttfts.reduce((a, b) => a + b, 0) / ttfts.length)) : "—"],
    ["model", tr("system.stat.time"), millis(own?.model_ms ?? sum((t) => t.model_ms))],
    ["tool", tr("system.stat.toolTime"), turns.length ? millis(sum((t) => t.tool_ms)) : "—"],
  ];
  return (
    <>
      <div className="stat-grid" data-testid="agent-stats">
        {cells.map(([id, label, value]) => (
          <div key={id} className="cell" data-cell={id}>
            <div className="v">{value}</div>
            <div className="l">{label}</div>
          </div>
        ))}
      </div>
      {own ? (
        <div className="note" data-testid="agent-tokens">
          {tr("system.tokenTip", { input: own.input_tokens, read: own.cache_read_tokens, write: own.cache_write_tokens, output: own.output_tokens })}
          {own.errors ? ` · ${tr("system.stat.errors", { count: own.errors })}` : ""}
        </div>
      ) : null}
    </>
  );
}

/** How full the next call's context is, against the window: the provider's count when there is one. */
function ContextRing({ context }: { context: Context | null }) {
  const tr = useT();
  if (!context) return null;
  const used = context.measured?.tokens ?? context.total_tokens ?? 0;
  const window = context.measured?.window ?? 0;
  const pct = window ? Math.min(100, Math.round((used / window) * 100)) : null;
  return (
    <div className="ctx-ring" data-testid="agent-context" data-pct={pct ?? undefined}>
      <div className="ring" style={{ "--pct": `${pct ?? 0}%` } as React.CSSProperties}>
        <span>{pct === null ? "—" : `${pct}%`}</span>
      </div>
      <div className="txt">
        <div>{window ? tr("system.tokensOf", { count: used, of: window }) : tr("system.tokens", { count: `~${used}` })}</div>
        <div className="note">{context.measured ? tr("system.fact.measured") : tr("system.fact.estimated")}</div>
      </div>
    </div>
  );
}

export function SessionOverview({ app, view, session }: { app: App; view: View; session: SessionKey }) {
  const tr = useT();
  const s = useShell(app);
  const params = useMemo(() => ({ instance: session.instance }), [session.instance]);
  const listed = useTopic<{ sessions: SessionRowView[] }>(view, "harnix.instances.sessions", params);
  const row = listed?.sessions?.find((r) => r.session_id === session.sessionId);
  const space = row?.space ?? "sessions";
  const agent: AgentRow | undefined = row ? { ...row, instance: session.instance, key: `${session.instance}/${row.session_id}` } : undefined;
  const turns = new Set(Object.keys(s.running));
  const state = agent ? stateOf(agent, turns) : null;
  const busy = Boolean(agent && (turns.has(agent.key) || agent.running || agent.tasks));
  const [round, setRound] = useState(0);
  // Read again whenever the row's pushed state says something happened, or an action here changed it.
  const facts = useAgentFacts(app, session, busy, `${round}|${agent ? stateSignature(agent) : ""}|${JSON.stringify(row?.goal ?? null)}`);
  const own = facts.usage?.find((u) => u.session_id === session.sessionId);
  const subs = (facts.usage ?? []).filter((u) => u.session_id !== session.sessionId);
  const model = facts.context?.model ?? own?.models.join(", ") ?? "";
  return (
    <div className="system-pane" data-testid="system-session">
      <div className="agent-head">
        {state ? <span className={`dot ${state.dot}`} /> : null}
        <span className="nm">{row?.name || tr("sidebar.untitled")}</span>
        {row?.started_by === "harnix" ? <span className="kd own">Harnix</span> : null}
        {state ? <span className="kd" data-testid="agent-state">{state.label}</span> : null}
      </div>
      <Facts
        rows={[
          [tr("system.fact.instance"), session.instance],
          [tr("system.fact.session"), session.sessionId],
          [tr("system.fact.name"), row?.name ?? ""],
          [tr("system.fact.space"), space],
          [tr("system.fact.scene"), row?.scene ?? ""],
          [tr("system.fact.model"), model],
          [tr("system.fact.project"), row?.project_root ?? ""],
          [tr("system.fact.startedBy"), row?.started_by === "harnix" ? tr("system.startedBy.harnix") : tr("system.startedBy.user")],
        ]}
      />
      <GoalBlock app={app} session={session} goal={facts.goal} changed={() => setRound((n) => n + 1)} />
      <ContextRing context={facts.context} />
      <div className="label">{tr("system.stats")}</div>
      <Stats own={own} turns={facts.turns} />
      {subs.length ? (
        <div className="note" data-testid="agent-subs">
          {tr("system.subTotals", { count: subs.length, tokens: compact(subs.reduce((n, u) => n + totalTokens(u), 0)), time: millis(subs.reduce((n, u) => n + u.model_ms, 0)) })}
        </div>
      ) : null}
      {space !== SYSTEM ? (
        <button
          type="button"
          className="btn sm outline wide"
          data-testid="open-in-space"
          onClick={() => openWhereItBelongs(app, session, space)}
        >
          {tr("system.openInSpace", { space })}
        </button>
      ) : null}
      <button
        type="button"
        className="btn sm outline wide"
        data-testid="system-space-sessions"
        onClick={() => view.select({ kind: "space", space, current: session })}
      >
        {tr("system.spaceSessions", { space })}
      </button>
      <div className="label">{tr("system.ops")}</div>
      <Operations app={app} view={view} session={session} name={row?.name ?? null} />
    </div>
  );
}

function Operations({ app, view, session, name }: { app: App; view: View; session: SessionKey; name: string | null }) {
  const tr = useT();
  const target = { instance: session.instance, session_id: session.sessionId };
  const system = (method: string, extra: Record<string, unknown> = {}) => app.call(`harnix.system.${method}`, { ...target, ...extra });
  const claimed = (method: string, extra: Record<string, unknown> = {}) => app.call(method, { ...target, ...extra, space: SYSTEM });
  const settled = () => void app.refreshSessions();
  const ops: { id: string; icon: string; label: string; run: () => void; danger?: boolean }[] = [
    { id: "compact", icon: "archive", label: tr("system.op.compact"), run: () => void act(app, () => system("compact")).then(settled) },
    {
      id: "clear",
      icon: "refresh",
      label: tr("system.op.clear"),
      run: () =>
        confirmDialog(app, tr("system.confirmClear"), async () => {
          await system("clear");
          app.closeModal();
          settled();
        }),
    },
    {
      id: "fork",
      icon: "fork",
      label: tr("system.op.fork"),
      run: () =>
        void act(app, async () => {
          const forked = await app.call<{ session_id: string; instance?: string }>("session.fork", { ...target, space: SYSTEM });
          view.select({ kind: "session", instance: forked.instance ?? session.instance, sessionId: forked.session_id });
          settled();
        }),
    },
    {
      id: "model",
      icon: "agent",
      label: tr("system.op.model"),
      run: () => renameDialog(app, tr("system.op.model"), app.snapshot.hello?.engine?.model ?? "", (model) => system("model", { model: model.trim() })),
    },
    { id: "interrupt", icon: "stop", label: tr("system.op.interrupt"), run: () => void act(app, () => claimed("session.interrupt")) },
    { id: "export", icon: "download", label: tr("system.op.export"), run: () => exportDialog(app, { ...target, space: SYSTEM }) },
    { id: "rename", icon: "edit", label: tr("system.op.rename"), run: () => renameDialog(app, tr("system.op.rename"), name, (value) => system("rename", { name: value.trim() })) },
    { id: "close", icon: "close", label: tr("system.op.close"), run: () => void act(app, () => claimed("session.close")).then(settled) },
    {
      id: "delete",
      icon: "trash",
      label: tr("system.op.delete"),
      danger: true,
      run: () =>
        confirmDialog(app, tr("system.confirmDelete"), async () => {
          await claimed("session.delete");
          app.closeModal();
          app.forget(session);
          view.select({ kind: "instance", instance: session.instance });
          settled();
        }),
    },
  ];
  return (
    <div className="ops">
      {ops.map((op) => (
        <button type="button" key={op.id} className={`btn sm outline op${op.danger ? " danger" : ""}`} data-testid={`op-${op.id}`} onClick={op.run}>
          <Glyph name={op.icon} className="glyph" />
          {op.label}
        </button>
      ))}
    </div>
  );
}
