/**
 * The trajectory's shape, apart from React: a session's recorded calls grouped by turn — the
 * person's words, each model call with the tool calls it asked for, the sub-agents' calls, the
 * calls beside the turns (compaction, memory, a title) — flattened into the rows a list draws, with
 * what is collapsed, searched for, filtered and narrowed to a stretch of time; the time overview's
 * arithmetic; which call a turn's feedback marks.
 *
 * The turns come from `harnix.session.outline` (every turn of the transcript, with a flow's station
 * marks), the calls from `observability.trace` (the current run's recording). A turn the recording
 * has no calls for is still listed, with its words alone; a call whose turn the outline does not
 * name is listed under its recorded turn number.
 */

import type { CallSummary, Feedback, OutlineMark, OutlineTurn, ToolUseSummary } from "@harnix/core";

/* ── groups ───────────────────────────────────────────────────────────── */

/** The calls of one sub-agent in a turn: one group per sub-agent session. */
export interface AgentGroup {
  key: string;
  sessionId: string;
  agentType: string | null;
  calls: CallSummary[];
}

/** One turn as the trajectory lists it. */
export interface TurnGroup {
  key: string;
  /** The outline's turn id, or the recording's when the outline has none for it. */
  turnId: string | null;
  /** Its place among the turns listed, from 1. */
  ordinal: number;
  seq: number | null;
  at: string | null;
  prompt: string;
  reply: string;
  marks: OutlineMark[];
  /** The session's own model calls, in order. */
  calls: CallSummary[];
  /** The calls beside the turn: compaction, memory, a title. */
  aux: CallSummary[];
  agents: AgentGroup[];
  /** The calls beside every turn that name no turn the recording has: listed last, by time. */
  between: boolean;
}

/** The recorded turn a call is of: its id when the recording says, else its number. */
function turnKeyOf(call: CallSummary): string {
  return call.turn_id ?? `turn:${call.turn}`;
}

/** From when a call was sent to its end, in ms since the epoch; null without timing. */
export function spanOf(call: CallSummary): [number, number] | null {
  const sent = call.timing?.sent_at ?? call.ts;
  if (sent == null || !Number.isFinite(sent)) return null;
  return [sent, sent + Math.max(call.duration_ms ?? 0, 0)];
}

/**
 * The turns, grouped: the outline's turns in their order, each with the recording's calls that name
 * it; then the recorded turns the outline does not know, by number; then, when any, the calls beside
 * the turns that name no listed turn. A sub-agent's calls go under the turn whose own calls were
 * running when the sub-agent first called the model, else the nearest turn before it.
 */
export function groupTurns(outline: readonly OutlineTurn[], calls: readonly CallSummary[]): TurnGroup[] {
  const groups: TurnGroup[] = [];
  const byKey = new Map<string, TurnGroup>();
  const add = (group: TurnGroup) => {
    groups.push(group);
    byKey.set(group.key, group);
    return group;
  };
  for (const turn of outline) {
    const key = turn.turn_id ?? `seq:${turn.seq}`;
    if (byKey.has(key)) continue;
    add({ key, turnId: turn.turn_id, ordinal: 0, seq: turn.seq, at: turn.at, prompt: turn.prompt, reply: turn.reply, marks: turn.marks ?? [], calls: [], aux: [], agents: [], between: false });
  }
  const own = calls.filter((c) => !c.parent_session_id);
  const recorded = (call: CallSummary): TurnGroup => {
    const key = turnKeyOf(call);
    return byKey.get(key) ?? add({ key, turnId: call.turn_id, ordinal: 0, seq: null, at: null, prompt: "", reply: "", marks: [], calls: [], aux: [], agents: [], between: false });
  };
  // The turns' own calls first; a turn the outline lacks is listed after the outline's, by its number.
  const listed = new Set(byKey.keys());
  for (const call of own.filter((c) => !c.purpose)) recorded(call).calls.push(call);
  const unknown = groups.filter((g) => !listed.has(g.key)).sort((a, b) => (a.calls[0]?.turn ?? 0) - (b.calls[0]?.turn ?? 0));
  groups.splice(0, groups.length, ...groups.filter((g) => listed.has(g.key)), ...unknown);
  // Beside the turns: with their turn when it is listed, else between the turns.
  let between: TurnGroup | null = null;
  for (const call of own.filter((c) => c.purpose)) {
    const group = byKey.get(turnKeyOf(call));
    if (group) group.aux.push(call);
    else {
      between ??= { key: "between", turnId: null, ordinal: 0, seq: null, at: null, prompt: "", reply: "", marks: [], calls: [], aux: [], agents: [], between: true };
      between.aux.push(call);
    }
  }
  // Sub-agents: by session, under the turn running when they started.
  const agents = new Map<string, AgentGroup>();
  for (const call of calls.filter((c) => c.parent_session_id)) {
    let group = agents.get(call.session_id);
    if (!group) agents.set(call.session_id, (group = { key: `agent:${call.session_id}`, sessionId: call.session_id, agentType: call.agent_type, calls: [] }));
    group.calls.push(call);
  }
  const withCalls = groups.filter((g) => g.calls.length);
  for (const agent of agents.values()) {
    const first = spanOf(agent.calls[0]!)?.[0] ?? agent.calls[0]!.ts;
    let home: TurnGroup | undefined;
    for (const group of withCalls) {
      const from = spanOf(group.calls[0]!)?.[0] ?? group.calls[0]!.ts;
      const last = group.calls[group.calls.length - 1]!;
      const to = spanOf(last)?.[1] ?? last.ts;
      if (first >= from && first <= to) {
        home = group;
        break;
      }
      if (first >= from) home = group;
    }
    (home ?? withCalls[withCalls.length - 1] ?? recorded(agent.calls[0]!)).agents.push(agent);
  }
  for (const group of groups) group.agents.sort((a, b) => (spanOf(a.calls[0]!)?.[0] ?? 0) - (spanOf(b.calls[0]!)?.[0] ?? 0));
  if (between) {
    between.aux.sort((a, b) => a.ts - b.ts);
    groups.push(between);
  }
  groups.forEach((group, i) => (group.ordinal = i + 1));
  return groups;
}

/* ── rows ─────────────────────────────────────────────────────────────── */

export type Row =
  | { kind: "turn"; key: string; group: TurnGroup; depth: 0 }
  | { kind: "user"; key: string; group: TurnGroup; depth: 1 }
  | { kind: "call"; key: string; group: TurnGroup; call: CallSummary; agent: AgentGroup | null; depth: 1 | 2; feedback: Feedback | null }
  | { kind: "tool"; key: string; group: TurnGroup; call: CallSummary; tool: ToolUseSummary; depth: 2 | 3 }
  | { kind: "agent"; key: string; group: TurnGroup; agent: AgentGroup; depth: 1 }
  | { kind: "aux"; key: string; group: TurnGroup; call: CallSummary; depth: 1 };

export type Filter = "all" | "failed" | "tools";

/** What narrows the rows: the filter, the words searched for, the stretch of time dragged. */
export interface Narrowing {
  filter: Filter;
  query: string;
  range: [number, number] | null;
}

/** Which levels are folded, and the keys folded or unfolded against their level. */
export interface Folding {
  turns: boolean;
  calls: boolean;
  agents: boolean;
  /** Keys whose state is the opposite of their level's. */
  against: ReadonlySet<string>;
}

export const UNFOLDED: Folding = { turns: false, calls: false, agents: false, against: new Set() };

export function folded(folding: Folding, level: "turns" | "calls" | "agents", key: string): boolean {
  return folding.against.has(key) ? !folding[level] : folding[level];
}

/** Fold or unfold one key against its level. */
export function toggleFold(folding: Folding, level: "turns" | "calls" | "agents", key: string): Folding {
  const against = new Set(folding.against);
  if (against.has(key)) against.delete(key);
  else against.add(key);
  return { ...folding, against };
}

/** Fold or unfold a whole level: what was set against it is forgotten. */
export function foldLevel(folding: Folding, level: "turns" | "calls" | "agents", fold: boolean): Folding {
  return { ...folding, [level]: fold, against: new Set() };
}

export function callKey(call: CallSummary): string {
  return `call:${call.index}`;
}

export function failed(call: CallSummary): boolean {
  return call.status === "error";
}

/** What a search over a call looks in: the model, its purpose, its words, each tool's name and input. */
export function searchable(call: CallSummary): string {
  return [call.model, call.purpose ?? "", call.output.text_preview ?? "", ...call.tools.flatMap((tool) => [tool.name, tool.input_preview ?? ""])].join("\n").toLowerCase();
}

export function overlaps(call: CallSummary, [from, to]: [number, number]): boolean {
  const span = spanOf(call);
  return span != null && span[0] <= to && span[1] >= from;
}

/** Whether a call passes the narrowing. */
export function passes(call: CallSummary, narrowing: Narrowing): boolean {
  const needle = narrowing.query.trim().toLowerCase();
  return (
    (narrowing.filter === "failed" ? failed(call) : narrowing.filter === "tools" ? call.tools.length > 0 : true) &&
    (!needle || searchable(call).includes(needle)) &&
    (!narrowing.range || overlaps(call, narrowing.range))
  );
}

export function narrowed(narrowing: Narrowing): boolean {
  return narrowing.filter !== "all" || narrowing.query.trim() !== "" || narrowing.range !== null;
}

/** The feedback that stands on each turn: the last given, taken back ones gone; a rating of one call keeps its id. */
export function feedbackByTurn(entries: Iterable<Feedback | null>): Map<string, Feedback> {
  const standing = new Map<string, Feedback>();
  for (const feedback of entries) {
    if (!feedback) continue;
    if (feedback.rating) standing.set(feedback.turn_id, feedback);
    else standing.delete(feedback.turn_id);
  }
  return standing;
}

/**
 * The rows to draw: the turns from `from` on (the earlier ones wait for "earlier"), each unfolded
 * into its words, calls, tools, sub-agents and calls beside it, as the folding says — except that
 * with a narrowing in force a call that passes is shown with its turn whatever is folded, so a search
 * finds what is folded away, and a turn none of whose calls pass is left out.
 */
export function rowsOf(groups: readonly TurnGroup[], from: number, folding: Folding, narrowing: Narrowing, feedback: ReadonlyMap<string, Feedback>): Row[] {
  const rows: Row[] = [];
  const searching = narrowed(narrowing);
  for (const group of groups.slice(Math.max(0, from))) {
    const calls = group.calls.filter((c) => passes(c, narrowing));
    const aux = group.aux.filter((c) => passes(c, narrowing));
    const agents = group.agents.map((agent) => ({ agent, calls: agent.calls.filter((c) => passes(c, narrowing)) })).filter((a) => a.calls.length);
    if (searching && !calls.length && !aux.length && !agents.length) continue;
    rows.push({ kind: "turn", key: group.key, group, depth: 0 });
    if (!searching && folded(folding, "turns", group.key)) continue;
    if (group.prompt && !searching) rows.push({ kind: "user", key: `${group.key}:user`, group, depth: 1 });
    const last = group.calls[group.calls.length - 1] ?? null;
    const rated = group.turnId ? (feedback.get(group.turnId) ?? null) : null;
    const pushCall = (call: CallSummary, agent: AgentGroup | null, depth: 1 | 2) => {
      rows.push({ kind: "call", key: callKey(call), group, call, agent, depth, feedback: !agent && call === last ? rated : null });
      if (!searching && folded(folding, "calls", callKey(call))) return;
      for (const tool of call.tools) rows.push({ kind: "tool", key: `${callKey(call)}:${tool.id}`, group, call, tool, depth: depth === 1 ? 2 : 3 });
    };
    for (const call of calls) pushCall(call, null, 1);
    for (const { agent, calls: own } of agents) {
      rows.push({ kind: "agent", key: agent.key, group, agent, depth: 1 });
      if (!searching && folded(folding, "agents", agent.key)) continue;
      for (const call of own) pushCall(call, agent, 2);
    }
    for (const call of aux) rows.push({ kind: "aux", key: callKey(call), group, call, depth: 1 });
  }
  return rows;
}

/** How tall each row is drawn, for the list to draw only what is in view. */
export const ROW_HEIGHT: Record<Row["kind"], number> = { turn: 36, user: 30, call: 46, tool: 28, agent: 30, aux: 30 };

/** The rows in view, and the room above and below them, for a scroll position and a viewport. */
export function window(rows: readonly Row[], scrollTop: number, height: number, overscan = 6): { start: number; end: number; above: number; below: number; total: number } {
  let total = 0;
  const tops: number[] = [];
  for (const row of rows) {
    tops.push(total);
    total += ROW_HEIGHT[row.kind];
  }
  let start = 0;
  while (start < rows.length - 1 && tops[start + 1]! <= scrollTop) start += 1;
  let end = start;
  while (end < rows.length && tops[end]! < scrollTop + height) end += 1;
  start = Math.max(0, start - overscan);
  end = Math.min(rows.length, end + overscan);
  const above = tops[start] ?? 0;
  const below = total - (end < rows.length ? tops[end]! : total);
  return { start, end, above, below, total };
}

/* ── the time overview ────────────────────────────────────────────────── */

export interface Bar {
  call: CallSummary;
  /** 0: the session's own calls; 1: its sub-agents'. */
  lane: 0 | 1;
  from: number;
  to: number;
  /** When the first chunk came, if it did: the bar is split there. */
  first: number | null;
}

/** When the tools ran: between two of a turn's own calls. */
export interface Gap {
  key: string;
  from: number;
  to: number;
}

export interface Overview {
  start: number;
  end: number;
  bars: Bar[];
  gaps: Gap[];
}

/** Every timed call as a bar, the tools' time between a turn's calls as gaps, and the whole stretch. */
export function overviewOf(groups: readonly TurnGroup[]): Overview | null {
  const bars: Bar[] = [];
  const gaps: Gap[] = [];
  for (const group of groups) {
    const own = group.calls.map((call) => ({ call, span: spanOf(call) })).filter((x): x is { call: CallSummary; span: [number, number] } => x.span != null);
    own.forEach(({ call, span }, i) => {
      bars.push({ call, lane: 0, from: span[0], to: span[1], first: call.timing?.ttft_ms != null ? span[0] + call.timing.ttft_ms : null });
      const next = own[i + 1];
      if (next && next.span[0] > span[1]) gaps.push({ key: `${call.index}-${next.call.index}`, from: span[1], to: next.span[0] });
    });
    for (const call of group.aux) {
      const span = spanOf(call);
      if (span) bars.push({ call, lane: 0, from: span[0], to: span[1], first: call.timing?.ttft_ms != null ? span[0] + call.timing.ttft_ms : null });
    }
    for (const agent of group.agents) {
      for (const call of agent.calls) {
        const span = spanOf(call);
        if (span) bars.push({ call, lane: 1, from: span[0], to: span[1], first: call.timing?.ttft_ms != null ? span[0] + call.timing.ttft_ms : null });
      }
    }
  }
  if (!bars.length) return null;
  const start = Math.min(...bars.map((b) => b.from));
  const end = Math.max(start + 1, ...bars.map((b) => b.to));
  return { start, end, bars, gaps };
}

/** The stretch shown after a wheel at `at` (a 0..1 place in it), zoomed by `factor` (above 1: out), within the whole. */
export function zoomed(shown: [number, number], whole: [number, number], at: number, factor: number): [number, number] {
  const length = shown[1] - shown[0];
  const least = Math.max((whole[1] - whole[0]) / 1000, 1);
  const next = Math.min(Math.max(length * factor, least), whole[1] - whole[0]);
  const pivot = shown[0] + Math.min(Math.max(at, 0), 1) * length;
  let from = pivot - Math.min(Math.max(at, 0), 1) * next;
  from = Math.max(whole[0], Math.min(from, whole[1] - next));
  return [from, from + next];
}

/** Where a time falls in a stretch, 0..1. */
export function placeOf(t: number, [from, to]: [number, number]): number {
  return Math.min(Math.max((t - from) / Math.max(to - from, 1), 0), 1);
}
