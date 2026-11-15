/** The right column for a main Agent's session: its facts, goal, context, figures and operations. */

import { useMemo, useState } from "react";
import { useRegistry, useT, Glyph, compact, millis, type ActionTarget, type HostSessionRef as SessionKey, type SessionAction } from "@harnix/react";

import { SYSTEM } from "../constants.ts";
import { stateOf, stateSignature, useAgentFacts } from "../data.ts";
import { totalTokens } from "../format.ts";
import { openWhereItBelongs } from "../navigate.ts";
import type { AgentRow, AgentUsage, Context, SessionRowView, TurnRow } from "../types.ts";
import { act, useRunning, useTopic, type View } from "../view.ts";
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

export function SessionOverview({ view, session }: { view: View; session: SessionKey }) {
  const tr = useT();
  const host = view.host;
  const params = useMemo(() => ({ instance: session.instance }), [session.instance]);
  const listed = useTopic<{ sessions: SessionRowView[] }>(view, "harnix.instances.sessions", params);
  const row = listed?.sessions?.find((r) => r.session_id === session.sessionId);
  const space = row?.space ?? host.defaults.space ?? "";
  const agent: AgentRow | undefined = row ? { ...row, instance: session.instance, key: `${session.instance}/${row.session_id}` } : undefined;
  const turns = useRunning(view);
  const state = agent ? stateOf(agent, turns) : null;
  const busy = Boolean(agent && (turns.has(agent.key) || agent.running || agent.tasks));
  const [round, setRound] = useState(0);
  // Read again whenever the row's pushed state says something happened, or an action here changed it.
  const facts = useAgentFacts(host, session, busy, `${round}|${agent ? stateSignature(agent) : ""}|${JSON.stringify(row?.goal ?? null)}`);
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
          [tr("system.fact.scene"), row?.scene_layer ?? row?.scene ?? ""],
          [tr("system.fact.model"), model],
          [tr("system.fact.project"), row?.project_root ?? ""],
          [tr("system.fact.startedBy"), row?.started_by === "harnix" ? tr("system.startedBy.harnix") : tr("system.startedBy.user")],
        ]}
      />
      <GoalBlock host={host} session={session} goal={facts.goal} changed={() => setRound((n) => n + 1)} />
      <ContextRing context={facts.context} />
      <div className="label">{tr("system.stats")}</div>
      <Stats own={own} turns={facts.turns} />
      {subs.length ? (
        <div className="note" data-testid="agent-subs">
          {tr("system.subTotals", { count: subs.length, tokens: compact(subs.reduce((n, u) => n + totalTokens(u), 0)), time: millis(subs.reduce((n, u) => n + u.model_ms, 0)) })}
        </div>
      ) : null}
      {space && space !== SYSTEM ? (
        <button
          type="button"
          className="btn sm outline wide"
          data-testid="open-in-space"
          onClick={() => openWhereItBelongs(view, session, space)}
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
      <Operations view={view} session={session} name={row?.name ?? null} space={space} model={model} changed={() => setRound((n) => n + 1)} />
    </div>
  );
}

function Operations({
  view,
  session,
  name,
  space,
  model,
  changed,
}: {
  view: View;
  session: SessionKey;
  name: string | null;
  space: string;
  model: string;
  changed: () => void;
}) {
  const tr = useT();
  const host = view.host;
  const registry = useRegistry(view.registry);
  const target = { instance: session.instance, session_id: session.sessionId };
  const system = (method: string, extra: Record<string, unknown> = {}) => host.call(`system.${method}`, { ...target, ...extra });
  const claimed = (method: string, extra: Record<string, unknown> = {}) => host.call(method, { ...target, ...extra, space: SYSTEM });
  // Done, the facts above are read again: a pushed row does not say the model changed.
  const apply = (run: () => Promise<unknown>) =>
    void act(host, run).then((ok) => {
      if (ok) changed();
    });
  const confirmed = (text: string, run: () => Promise<unknown>) =>
    void host.dialogs.confirm(text, { danger: true }).then((yes) => {
      if (yes) apply(run);
    });
  const prompted = (title: string, value: string, run: (value: string) => Promise<unknown>) =>
    void host.dialogs.prompt({ title, label: title, value }).then((text) => {
      if (text !== null) apply(() => run(text));
    });
  const ops: { id: string; icon: string; label: string; run: () => void; danger?: boolean }[] = [
    { id: "compact", icon: "archive", label: tr("system.op.compact"), run: () => apply(() => system("compact")) },
    { id: "clear", icon: "refresh", label: tr("system.op.clear"), run: () => confirmed(tr("system.confirmClear"), () => system("clear")) },
    {
      id: "fork",
      icon: "fork",
      label: tr("system.op.fork"),
      run: () =>
        apply(async () => {
          const forked = await host.call<{ session_id: string; instance?: string }>("harnix.session.fork", { ...target, space: SYSTEM });
          view.select({ kind: "session", instance: forked.instance ?? session.instance, sessionId: forked.session_id });
        }),
    },
    { id: "model", icon: "agent", label: tr("system.op.model"), run: () => prompted(tr("system.op.model"), model, (value) => system("model", { model: value.trim() })) },
    { id: "interrupt", icon: "stop", label: tr("system.op.interrupt"), run: () => apply(() => claimed("harnix.session.interrupt")) },
    { id: "rename", icon: "edit", label: tr("system.op.rename"), run: () => prompted(tr("system.op.rename"), name ?? "", (value) => system("rename", { name: value.trim() })) },
    { id: "close", icon: "close", label: tr("system.op.close"), run: () => apply(() => claimed("harnix.session.close")) },
    {
      id: "delete",
      icon: "trash",
      label: tr("system.op.delete"),
      danger: true,
      run: () =>
        confirmed(tr("system.confirmDelete"), async () => {
          await claimed("harnix.session.delete");
          view.select({ kind: "instance", instance: session.instance });
        }),
    },
  ];
  // What other extensions offer on a session (exporting it, say): the session menu's actions, asked
  // about the session in its own space.
  const actionTarget: ActionTarget = { kind: "session", session, space };
  const offered = registry.actions("menu", actionTarget).flatMap((action) => {
    try {
      return [{ action, label: action.label(actionTarget) }];
    } catch {
      return [];
    }
  });
  const run = async (action: SessionAction, label: string) => {
    if (action.danger && !(await host.dialogs.confirm(label, { danger: true, confirm: label }))) return;
    await act(host, async () => action.run(actionTarget));
  };
  return (
    <div className="ops">
      {ops.map((op) => (
        <button type="button" key={op.id} className={`btn sm outline op${op.danger ? " danger" : ""}`} data-testid={`op-${op.id}`} onClick={op.run}>
          <Glyph name={op.icon} className="glyph" />
          {op.label}
        </button>
      ))}
      {offered.map(({ action, label }) => (
        <button
          type="button"
          key={`action:${action.id}`}
          className={`btn sm outline op${action.danger ? " danger" : ""}`}
          data-testid={`op-action-${action.id}`}
          onClick={() => void run(action, label)}
        >
          {action.icon ? <Glyph name={action.icon} className="glyph" /> : null}
          {label}
        </button>
      ))}
    </div>
  );
}
