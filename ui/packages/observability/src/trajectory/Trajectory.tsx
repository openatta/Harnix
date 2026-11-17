/**
 * The trajectory: a session's recorded model calls as one table grouped by turn — the person's
 * words, each call with the tool calls it asked for, the sub-agents' calls, the calls beside the turn
 * (compaction, memory, a title) — with a time overview above it and an inspector beside it.
 *
 * Turns, calls and sub-agents fold; a search finds what is folded away; the filters keep the failed
 * calls or the ones that called a tool; a stretch dragged on the overview keeps the calls active in
 * it. Long histories show their last fifty turns and the earlier ones on asking; only the rows in
 * view are drawn; while the session streams, the list follows its tail unless it was scrolled up.
 * A flow's station boundaries come from the outline's marks; the feedback given on a turn marks the
 * turn's last call.
 *
 * Drawn in the centre as a `conversation.view`; `compact` has the inspector take the list's place
 * while a call is open, for a narrow column.
 */

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { FEEDBACK_NS, parseFeedback, type Feedback, type OutlineTurn, type SessionEntry, type TurnSummary } from "@harnix/core";
import { Glyph, clip, compact, rowKeys, useT, type HostSessionRef as SessionKey } from "@harnix/react";

import { Failure, RunNote, seconds, sessionParams, usageText, type Observed, type Run, type TraceCall } from "../shared.tsx";
import { CallDetail } from "./Inspector.tsx";
import {
  UNFOLDED,
  callKey,
  failed,
  feedbackByTurn,
  foldLevel,
  folded,
  groupTurns,
  narrowed,
  overviewOf,
  rowsOf,
  toggleFold,
  window as windowOf,
  type Filter,
  type Folding,
  type Narrowing,
  type Row,
} from "./model.ts";
import { Timeline } from "./Timeline.tsx";

/** How many turns a long history shows at first, and how many more each "earlier" adds. */
export const PAGE_TURNS = 50;
/** Within this many pixels of the end, the list is at its tail and follows it. */
const TAIL_PX = 48;
/** A session event is followed by a read of the trace this soon; several in a row by one read. */
const SETTLE_MS = 600;

type Level = "turns" | "calls" | "agents";
/** The context column on a turn's row: how much it grew since the turn before, or the running total. */
type ContextMode = "delta" | "total";

export function Trajectory({ observed, session, tool, compact: compactMode = false }: { observed: Observed; session: SessionKey; tool?: string; compact?: boolean }) {
  const tr = useT();
  const host = observed.host;
  const [calls, setCalls] = useState<TraceCall[] | null>(null);
  const [run, setRun] = useState<Run | null>(null);
  const [outline, setOutline] = useState<OutlineTurn[]>([]);
  const [turns, setTurns] = useState<TurnSummary[]>([]);
  const [contextMode, setContextMode] = useState<ContextMode>("delta");
  const [feedback, setFeedback] = useState<Map<string, Feedback>>(() => new Map());
  const [error, setError] = useState<unknown>(null);
  const [folding, setFolding] = useState<Folding>(UNFOLDED);
  const [narrowing, setNarrowing] = useState<Narrowing>({ filter: "all", query: "", range: null });
  const [shown, setShown] = useState(PAGE_TURNS);
  const [selected, setSelected] = useState<number | null>(null);
  // Back from the inspector: it closes, and the call stays marked in the list, where the person was.
  const [inspecting, setInspecting] = useState(false);
  const pick = (index: number) => {
    setSelected(index);
    setInspecting(true);
  };
  // A jump from a tool row names a tool call: the call that asked for it is picked out once the calls are in.
  const [focus, setFocus] = useState(() => tool ?? null);

  useEffect(() => {
    let alive = true;
    const params = sessionParams(observed, session);
    const readTrace = () =>
      Promise.all([
        host.call<{ calls?: TraceCall[]; run?: Run | null }>("observability.trace", params),
        // The turns' sums: a run without a recording has none, which is no failure of the trace.
        host.call<{ turns?: TurnSummary[] }>("observability.turns", params).catch(() => ({ turns: [] })),
      ])
        .then(([r, t]) => alive && (setCalls(r.calls ?? []), setRun(r.run ?? null), setTurns(t.turns ?? []), setError(null)))
        .catch((e) => alive && setError(e ?? "error"));
    // A session with no transcript (a fresh one) is no failure of the trajectory: the recording says.
    const readOutline = () =>
      host
        .call<{ turns?: OutlineTurn[] }>("harnix.session.outline", params)
        .then((r) => alive && setOutline(Array.isArray(r.turns) ? r.turns : []))
        .catch(() => undefined);
    const readFeedback = () =>
      host
        .call<{ entries?: SessionEntry[] }>("engine.session.entries", { ...params, namespaces: [FEEDBACK_NS] })
        .then((r) => alive && setFeedback(feedbackByTurn((r.entries ?? []).map(parseFeedback))))
        .catch(() => undefined);
    const readAll = () => void Promise.all([readTrace(), readOutline(), readFeedback()]);
    readAll();
    const offs = [
      host.events.onTurnSettled((e) => {
        if (e.session_id === session.sessionId) readAll();
      }),
      // The calls land in the recording one by one as the turn runs: read again a moment after each step.
      (() => {
        let timer: ReturnType<typeof setTimeout> | null = null;
        const off = host.events.onSessionEvent(session, (e) => {
          const kind = e.parsed.kind;
          if (kind !== "tool_called" && kind !== "tool_result" && kind !== "turn_complete" && kind !== "user_message") return;
          if (timer) clearTimeout(timer);
          timer = setTimeout(() => void readTrace(), SETTLE_MS);
        });
        return () => {
          off();
          if (timer) clearTimeout(timer);
        };
      })(),
      host.subscribe("harnix.feedback", undefined, () => void readFeedback()),
    ];
    return () => {
      alive = false;
      for (const off of offs) off();
    };
  }, [host, observed, session.instance, session.sessionId]);

  const groups = useMemo(() => groupTurns(outline, calls ?? []), [outline, calls]);
  const sums = useMemo(() => new Map(turns.map((t) => [t.turn, t])), [turns]);
  const overview = useMemo(() => overviewOf(groups), [groups]);
  const from = Math.max(0, groups.length - shown);
  const rows = useMemo(() => rowsOf(groups, from, folding, narrowing, feedback), [groups, from, folding, narrowing, feedback]);
  const firstFailure = calls?.find(failed) ?? null;
  const all = calls ?? [];

  // The call a tool row sent us to: picked out, opened, scrolled to — once.
  useEffect(() => {
    if (!focus || !calls) return;
    const call = calls.find((c) => c.tools.some((t) => t.id === focus));
    if (call) {
      setSelected(call.index);
      setInspecting(true);
      const group = groups.find((g) => g.calls.includes(call) || g.agents.some((a) => a.calls.includes(call)));
      if (group) setShown((n) => Math.max(n, groups.length - groups.indexOf(group)));
    }
    setFocus(null);
  }, [focus, calls, groups]);

  if (error) return <Failure error={error} testId="system-calls-failed" />;
  if (!calls) return null;
  const opened = selected != null && inspecting ? (all.find((c) => c.index === selected) ?? null) : null;
  const previous = opened ? (all.filter((c) => c.index < opened.index && !c.purpose && c.session_id === opened.session_id).pop() ?? null) : null;
  const inspector = opened ? <CallDetail observed={observed} session={session} call={opened} ordinal={all.indexOf(opened) + 1} previous={previous} onBack={() => setInspecting(false)} /> : null;
  const fold = (level: Level) => setFolding((f) => foldLevel(f, level, !f[level]));
  const jump = (index: number) => {
    pick(index);
    const group = groups.find((g) => g.calls.some((c) => c.index === index) || g.aux.some((c) => c.index === index) || g.agents.some((a) => a.calls.some((c) => c.index === index)));
    if (group) setShown((n) => Math.max(n, groups.length - groups.indexOf(group)));
  };
  return (
    <div className={`hx-traj${compactMode ? " compact" : ""}`} data-testid="trajectory" data-session={session.sessionId}>
      {compactMode && inspector ? (
        inspector
      ) : (
        <>
          <div className="traj-tools">
            <div className="tools-row">
              {(["all", "failed", "tools"] as Filter[]).map((f) => (
                <button type="button" key={f} className={`btn sm outline${narrowing.filter === f ? " on" : ""}`} data-filter={f} aria-pressed={narrowing.filter === f} onClick={() => setNarrowing({ ...narrowing, filter: f })}>
                  {tr(`obs.filter.${f}`)}
                </button>
              ))}
              {firstFailure ? (
                <button type="button" className="btn sm outline" data-testid="jump-to-failure" onClick={() => jump(firstFailure.index)}>
                  {tr("obs.jumpToFailure")}
                </button>
              ) : null}
              <span className="sp" />
              {(["turns", "calls", "agents"] as Level[]).map((level) => (
                <button
                  type="button"
                  key={level}
                  className={`btn sm outline${folding[level] ? " on" : ""}`}
                  data-testid={`traj-fold-${level}`}
                  aria-pressed={folding[level]}
                  title={tr(`trajectory.fold.${level}Hint`)}
                  onClick={() => fold(level)}
                >
                  {tr(`trajectory.fold.${level}`)}
                </button>
              ))}
            </div>
            <div className="tools-row">
              <input className="search" type="search" data-testid="calls-search" placeholder={tr("obs.searchCalls")} value={narrowing.query} onChange={(e) => setNarrowing({ ...narrowing, query: e.target.value })} />
              {turns.length ? (
                <span className="seg" role="group" aria-label={tr("obs.turns.context")}>
                  {(["delta", "total"] as const).map((m) => (
                    <button type="button" key={m} className={`btn sm outline${contextMode === m ? " on" : ""}`} data-testid={`turns-${m}`} aria-pressed={contextMode === m} onClick={() => setContextMode(m)}>
                      {tr(`obs.turns.${m}`)}
                    </button>
                  ))}
                </span>
              ) : null}
              <span className="muted" data-testid="traj-count">
                {tr("trajectory.count", { turns: groups.filter((g) => !g.between).length, calls: all.length })}
              </span>
            </div>
            <RunNote run={run} />
            {overview ? <Timeline overview={overview} range={narrowing.range} onRange={(range) => setNarrowing({ ...narrowing, range })} selected={selected} onPick={jump} /> : null}
          </div>
          <div className="traj-body">
            {!all.length && !groups.length ? (
              <div className="empty" data-testid="traj-empty">
                {tr("obs.noCalls")}
              </div>
            ) : (
              <Rows
                rows={rows}
                sums={sums}
                contextMode={contextMode}
                earlier={from > 0 ? from : 0}
                onEarlier={() => setShown((n) => n + PAGE_TURNS)}
                folding={folding}
                onFold={(level, key) => setFolding((f) => toggleFold(f, level, key))}
                selected={selected}
                onSelect={pick}
                narrowedNow={narrowed(narrowing)}
                located={tool ?? null}
                follow={!narrowed(narrowing)}
              />
            )}
            {!compactMode ? inspector : null}
          </div>
        </>
      )}
    </div>
  );
}

/**
 * The rows, only those in view drawn (a spacer above and below stands for the rest), following the
 * tail while the list is at it. "Earlier" at the top brings in the turns before the ones shown.
 */
function Rows({
  rows,
  sums,
  contextMode,
  earlier,
  onEarlier,
  folding,
  onFold,
  selected,
  onSelect,
  narrowedNow,
  located,
  follow,
}: {
  rows: Row[];
  sums: ReadonlyMap<number, TurnSummary>;
  contextMode: ContextMode;
  earlier: number;
  onEarlier: () => void;
  folding: Folding;
  onFold: (level: Level, key: string) => void;
  selected: number | null;
  onSelect: (index: number) => void;
  narrowedNow: boolean;
  located: string | null;
  follow: boolean;
}) {
  const tr = useT();
  const list = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(0);
  const atTail = useRef(true);
  useEffect(() => {
    const node = list.current;
    if (!node || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => setHeight(node.clientHeight));
    ro.observe(node);
    setHeight(node.clientHeight);
    return () => ro.disconnect();
  }, []);
  // New rows while the list is at its tail: stay at the tail. Scrolled up, the reader is left where they are.
  const total = rows.length;
  useLayoutEffect(() => {
    const node = list.current;
    if (!node || !follow || !atTail.current) return;
    node.scrollTop = node.scrollHeight;
  }, [total, follow]);
  // Without a measured height (not laid out yet, or no layout engine) every row is drawn.
  const win = height > 0 ? windowOf(rows, scrollTop, height) : { start: 0, end: rows.length, above: 0, below: 0, total: 0 };
  const onScroll = () => {
    const node = list.current!;
    setScrollTop(node.scrollTop);
    atTail.current = node.scrollTop + node.clientHeight >= node.scrollHeight - TAIL_PX;
  };
  return (
    <div className="traj-rows" ref={list} data-testid="traj-rows" data-rows={rows.length} onScroll={onScroll}>
      {earlier > 0 ? (
        <button type="button" className="btn sm outline wide traj-earlier" data-testid="traj-earlier" onClick={onEarlier}>
          {tr("trajectory.earlier", { count: earlier })}
        </button>
      ) : null}
      {!rows.length ? (
        <div className="note" data-testid="calls-none">
          {tr("obs.noMatchingCalls")}
        </div>
      ) : null}
      <div style={{ height: win.above }} />
      {rows.slice(win.start, win.end).map((row) => (
        <TrajectoryRow key={row.key} row={row} sum={row.kind === "turn" && row.group.number != null ? (sums.get(row.group.number) ?? null) : null} contextMode={contextMode} folding={folding} onFold={onFold} selected={selected} onSelect={onSelect} narrowedNow={narrowedNow} located={located} />
      ))}
      <div style={{ height: win.below }} />
    </div>
  );
}

function TrajectoryRow({
  row,
  sum,
  contextMode,
  folding,
  onFold,
  selected,
  onSelect,
  narrowedNow,
  located,
}: {
  row: Row;
  sum: TurnSummary | null;
  contextMode: ContextMode;
  folding: Folding;
  onFold: (level: Level, key: string) => void;
  selected: number | null;
  onSelect: (index: number) => void;
  narrowedNow: boolean;
  located: string | null;
}) {
  const tr = useT();
  const indent = { "--depth": row.depth } as React.CSSProperties;
  switch (row.kind) {
    case "turn": {
      const g = row.group;
      const open = narrowedNow || !folded(folding, "turns", g.key);
      const tokens = g.calls.reduce((n, c) => n + (c.usage?.output_tokens ?? 0) + (c.usage?.input_tokens ?? 0) + (c.usage?.cache_read_input_tokens ?? 0) + (c.usage?.cache_creation_input_tokens ?? 0), 0);
      const first = g.calls[0];
      const last = g.calls[g.calls.length - 1];
      const duration = first && last ? (last.timing?.sent_at ?? last.ts) + (last.duration_ms ?? 0) - (first.timing?.sent_at ?? first.ts) : null;
      const when = g.at ? new Date(g.at) : first ? new Date(first.timing?.sent_at ?? first.ts) : null;
      const pctOf = (x?: number | null) => (x != null ? `${Math.round(x * 100)}%` : "—");
      const signed = (n?: number | null) => (n == null ? "—" : n > 0 ? `+${compact(n)}` : n < 0 ? `−${compact(-n)}` : "0");
      return (
        <div
          className={`traj-row turn${g.between ? " between" : ""}`}
          data-testid="turn-row"
          data-turn={g.ordinal}
          data-turn-id={g.turnId ?? undefined}
          role="button"
          tabIndex={0}
          aria-expanded={open}
          style={indent}
          onClick={() => onFold("turns", g.key)}
          onKeyDown={rowKeys(() => onFold("turns", g.key), { open, set: () => onFold("turns", g.key) })}
        >
          <Glyph name="chevronRight" className={`glyph chev${open ? " open" : ""}`} />
          <b>{g.between ? tr("trajectory.between") : tr("obs.turn", { turn: g.ordinal })}</b>
          {g.marks.map((mark, i) => (
            <span key={i} className="kd mark" data-testid="turn-mark" data-kind={mark.kind} title={mark.kind}>
              {mark.label}
            </span>
          ))}
          <span className="r">
            {when && !Number.isNaN(when.getTime()) ? when.toLocaleTimeString() : ""}
            {g.calls.length ? ` · ${tr("trajectory.turnCalls", { count: g.calls.length })}` : ""}
            {g.aux.length ? ` +${g.aux.length}` : ""}
            {g.agents.length ? ` · ${tr("trajectory.turnAgents", { count: g.agents.length })}` : ""}
            {sum ? (
              <>
                {` · ${tr("obs.turns.ttft")} ${seconds(sum.ttft_ms)} · ${tr("obs.turns.model")} ${seconds(sum.model_ms)} · ${tr("obs.turns.tools")} ${seconds(sum.tool_ms)} · ${compact(sum.input_tokens)} / ${compact(sum.output_tokens)} · `}
                <span data-testid="turn-cache">{pctOf(sum.cache_hit)}</span>
                {sum.tokens_per_s != null ? ` · ${sum.tokens_per_s} tok/s` : ""}
                {` · ${contextMode === "delta" ? tr("obs.turns.contextDelta") : tr("obs.turns.context")} `}
                <span data-testid="turn-context">{contextMode === "delta" ? signed(sum.context_delta) : sum.context_tokens != null ? compact(sum.context_tokens) : "—"}</span>
              </>
            ) : (
              <>
                {tokens ? ` · ${compact(tokens)} tok` : ""}
                {duration != null ? ` · ${seconds(duration)}` : ""}
              </>
            )}
          </span>
        </div>
      );
    }
    case "user":
      return (
        <div className="traj-row user" data-testid="traj-user" style={indent} title={row.group.prompt}>
          <span className="kd2 k-user">{tr("trajectory.kind.user")}</span>
          <span className="txt">{clip(row.group.prompt, 160)}</span>
        </div>
      );
    case "agent": {
      const open = narrowedNow || !folded(folding, "agents", row.agent.key);
      return (
        <div
          className="traj-row agent"
          data-testid="traj-agent"
          data-agent={row.agent.sessionId}
          role="button"
          tabIndex={0}
          aria-expanded={open}
          style={indent}
          onClick={() => onFold("agents", row.agent.key)}
          onKeyDown={rowKeys(() => onFold("agents", row.agent.key), { open, set: () => onFold("agents", row.agent.key) })}
        >
          <Glyph name="chevronRight" className={`glyph chev${open ? " open" : ""}`} />
          <span className="kd2 k-agent">{tr("trajectory.kind.agent")}</span>
          <b>{row.agent.agentType ?? row.agent.sessionId}</b>
          <span className="r">{tr("trajectory.turnCalls", { count: row.agent.calls.length })}</span>
        </div>
      );
    }
    case "aux": {
      const c = row.call;
      return (
        <button type="button" className={`traj-row aux${failed(c) ? " bad" : ""}${selected === c.index ? " on" : ""}`} data-testid="traj-aux" data-call={c.index} data-purpose={c.purpose ?? undefined} style={indent} onClick={() => onSelect(c.index)}>
          <span className="kd2 k-aux">{c.purpose}</span>
          <span className="r">
            {c.model} · {usageText(c.usage)} · {seconds(c.duration_ms)}
          </span>
        </button>
      );
    }
    case "call": {
      const c = row.call;
      const open = narrowedNow || !folded(folding, "calls", callKey(c));
      const hasTools = c.tools.length > 0;
      return (
        <button
          type="button"
          className={`traj-row call${failed(c) ? " bad" : ""}${selected === c.index ? " on" : ""}${located && c.tools.some((t) => t.id === located) ? " located" : ""}`}
          data-testid="system-call"
          data-call={c.index}
          data-step={c.step}
          data-state={failed(c) ? "failed" : c.status === "in_flight" ? "running" : undefined}
          data-located={located && c.tools.some((t) => t.id === located) ? "true" : undefined}
          data-feedback={row.feedback?.rating ?? undefined}
          aria-expanded={hasTools ? open : undefined}
          style={indent}
          onClick={() => onSelect(c.index)}
        >
          <div className="l1">
            {hasTools ? (
              <span
                role="button"
                tabIndex={-1}
                className={`chev-btn${open ? " open" : ""}`}
                data-testid="call-fold"
                aria-label={tr(open ? "trajectory.fold.calls" : "trajectory.unfold")}
                onClick={(e) => {
                  e.stopPropagation();
                  onFold("calls", callKey(c));
                }}
              >
                <Glyph name="chevronRight" className="glyph chev" />
              </span>
            ) : (
              <span className="chev-btn none" />
            )}
            <span className="kd2 k-model">{row.agent ? tr("trajectory.kind.agentCall") : tr("obs.step.model")}</span>
            <b>#{c.step + 1}</b>
            {row.feedback ? (
              <span className={`kd fb ${row.feedback.rating}`} data-testid="traj-feedback" data-rating={row.feedback.rating ?? undefined} title={[row.feedback.category, row.feedback.note].filter(Boolean).join(" · ")}>
                {tr(`trajectory.feedback.${row.feedback.rating}`)}
              </span>
            ) : null}
            <span className="r">
              {c.model} · {usageText(c.usage)} · {seconds(c.duration_ms)}
              {c.timing?.ttft_ms != null ? <span data-testid="call-ttft"> · {tr("obs.ttft", { ms: c.timing.ttft_ms })}</span> : null}
              {c.timing?.tokens_per_s != null ? ` · ${tr("obs.rate", { rate: c.timing.tokens_per_s })}` : ""} · {c.stop_reason ?? c.status}
            </span>
          </div>
          <div className="l2">
            <span className="in">{tr("obs.callIn", { blocks: c.input.system.length, tools: c.input.tools, messages: c.input.messages })}</span>
            <span className="arrow">→</span>
            <span className="out">{callOut(c, tr)}</span>
          </div>
        </button>
      );
    }
    case "tool": {
      const t = row.tool;
      return (
        <button
          type="button"
          className={`traj-row tool${t.result_is_error ? " bad" : ""}${located === t.id ? " located" : ""}`}
          data-testid="system-call-tool"
          data-tool={t.id}
          data-located={located === t.id ? "true" : undefined}
          data-state={t.result_is_error ? "failed" : undefined}
          style={indent}
          onClick={() => onSelect(row.call.index)}
        >
          <span className="kd2 k-tool">{tr("obs.step.tool")}</span>
          <b>{t.name}</b>
          <span className="r" title={t.input_preview ?? undefined}>
            {t.input_preview ?? tr("obs.toolDetailHint")}
            {t.result_is_error ? ` · ${tr("obs.error")}` : ""}
          </span>
        </button>
      );
    }
  }
}

function callOut(call: TraceCall, tr: (k: string, v?: Record<string, unknown>) => string): string {
  if (call.status === "error") return `${tr("obs.error")}: ${call.error?.message || call.error?.kind || ""}`;
  if (call.output.text_preview) return call.output.text_preview;
  const tools = call.tools.map((tool) => `tool_use: ${tool.name}`).join(", ");
  const text = call.output.chars ? tr("obs.callOutChars", { chars: call.output.chars }) : "";
  return [text, tools].filter(Boolean).join(" · ") || "—";
}
