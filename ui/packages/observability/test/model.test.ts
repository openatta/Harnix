/**
 * The trajectory's shape: turns grouped from the outline and the recording, the calls beside the
 * turns and the sub-agents' placed, the rows folded, searched, filtered and narrowed to a stretch of
 * time, the rows in view, the overview's bars and gaps, the zoom, and the feedback on a turn.
 */

import { describe, expect, it } from "vitest";
import type { CallSummary, OutlineTurn } from "@harnix/core";

import {
  UNFOLDED,
  feedbackByTurn,
  foldLevel,
  groupTurns,
  overviewOf,
  rowsOf,
  toggleFold,
  window as windowOf,
  zoomed,
} from "../src/trajectory/model";
import { call } from "./host";

const turn = (id: string, seq: number, prompt: string, marks?: OutlineTurn["marks"]): OutlineTurn => ({ turn_id: id, seq, at: "2026-01-01T00:00:00Z", prompt, reply: "", ...(marks ? { marks } : {}) });

const c = (index: number, over: Parameters<typeof call>[1] = {}) => call(index, over) as unknown as CallSummary;

describe("turns", () => {
  it("are the outline's, with the recording's calls that name them, the calls beside a turn under it, and unknown turns after", () => {
    const outline = [turn("t1", 0, "first"), turn("t2", 2, "second", [{ kind: "station", label: "plan" }]), turn("t3", 4, "third")];
    const calls = [
      c(0, { turn_id: "t1", turn: 0, tools: [{ id: "tu1", name: "Bash" }] }),
      c(1, { turn_id: "t1", turn: 0, step: 1 }),
      c(2, { turn_id: null, turn: 0, purpose: "title" }),
      c(3, { turn_id: "t2", turn: 1 }),
      c(4, { turn_id: "t9", turn: 7 }),
      c(5, { turn_id: null, turn: 1, purpose: "compaction" }),
      c(6, { turn_id: null, turn: 5, purpose: "memory" }),
    ];
    const groups = groupTurns(outline, calls);
    expect(groups.map((g) => [g.key, g.ordinal, g.number, g.calls.length, g.aux.length])).toEqual([
      // A call beside a turn names the turn's number alone: it goes with the turn whose calls have it.
      ["t1", 1, 0, 2, 1],
      ["t2", 2, 1, 1, 1],
      ["t3", 3, null, 0, 0],
      ["t9", 4, 7, 1, 0],
      // Naming a turn number no listed turn has: between the turns, last.
      ["between", 5, null, 0, 1],
    ]);
    expect(groups[1]!.marks).toEqual([{ kind: "station", label: "plan" }]);
    expect(groups[4]!.between).toBe(true);
    // The recording alone: its turns by number.
    const alone = groupTurns([], [c(0, { turn_id: null, turn: 3 }), c(1, { turn_id: null, turn: 1 }), c(2, { turn_id: null, turn: 1, purpose: "memory" })]);
    expect(alone.map((g) => [g.key, g.calls.length, g.aux.length])).toEqual([
      ["turn:1", 1, 1],
      ["turn:3", 1, 0],
    ]);
  });

  it("put a sub-agent's calls under the turn whose own calls were running when it started", () => {
    const outline = [turn("t1", 0, "a"), turn("t2", 2, "b")];
    const calls = [
      c(0, { turn_id: "t1", ts: 1000, timing: { sent_at: 1000, ttft_ms: 10, generate_ms: 10, tokens_per_s: 1 }, duration_ms: 5000 }),
      c(1, { turn_id: "t2", ts: 9000, timing: { sent_at: 9000, ttft_ms: 10, generate_ms: 10, tokens_per_s: 1 }, duration_ms: 500 }),
      c(2, { session_id: "sub", parent_session_id: "s1", agent_type: "general", turn_id: null, turn: 0, ts: 2000, timing: { sent_at: 2000, ttft_ms: 5, generate_ms: 5, tokens_per_s: 1 } }),
      c(3, { session_id: "sub", parent_session_id: "s1", agent_type: "general", turn_id: null, turn: 0, ts: 3000, timing: { sent_at: 3000, ttft_ms: 5, generate_ms: 5, tokens_per_s: 1 } }),
      c(4, { session_id: "sub2", parent_session_id: "s1", agent_type: "late", turn_id: null, turn: 0, ts: 9700, timing: { sent_at: 9700, ttft_ms: 5, generate_ms: 5, tokens_per_s: 1 } }),
    ];
    const groups = groupTurns(outline, calls);
    expect(groups[0]!.agents.map((a) => [a.agentType, a.calls.length])).toEqual([["general", 2]]);
    // Started after the second turn's call ended: the nearest turn before it.
    expect(groups[1]!.agents.map((a) => a.agentType)).toEqual(["late"]);
  });
});

describe("rows", () => {
  const outline = [turn("t1", 0, "first question"), turn("t2", 2, "second")];
  const calls = [
    c(0, { turn_id: "t1", tools: [{ id: "tu1", name: "Bash", input_preview: '{"command":"ls"}' }], output: { chars: 3, thinking_chars: 0, tool_uses: 1, text_preview: null } }),
    c(1, { turn_id: "t1", step: 1, status: "error", error: { message: "overloaded" } }),
    c(2, { turn_id: "t2", output: { chars: 9, thinking_chars: 0, tool_uses: 0, text_preview: "the answer" } }),
    c(3, { turn_id: "t1", turn: 0, purpose: "title" }),
    c(4, { session_id: "sub", parent_session_id: "s1", agent_type: "general", turn_id: null, ts: 1200, timing: { sent_at: 1200, ttft_ms: 5, generate_ms: 5, tokens_per_s: 1 }, tools: [{ id: "tu-sub", name: "Read" }] }),
  ];
  const groups = groupTurns(outline, calls);
  const none = { filter: "all" as const, query: "", range: null };
  const kinds = (rows: ReturnType<typeof rowsOf>) => rows.map((r) => `${r.kind}${"call" in r ? `:${r.call.index}` : ""}${r.kind === "tool" ? `:${r.tool.id}` : ""}`);

  it("unfold a turn into its words, calls with their tools, sub-agents and the calls beside it", () => {
    expect(kinds(rowsOf(groups, 0, UNFOLDED, none, new Map()))).toEqual(["turn", "user", "call:0", "tool:0:tu1", "call:1", "agent", "call:4", "tool:4:tu-sub", "aux:3", "turn", "user", "call:2"]);
  });

  it("fold by level and against the level, and start from a later turn", () => {
    expect(kinds(rowsOf(groups, 0, foldLevel(UNFOLDED, "turns", true), none, new Map()))).toEqual(["turn", "turn"]);
    const second = toggleFold(foldLevel(UNFOLDED, "turns", true), "turns", "t2");
    expect(kinds(rowsOf(groups, 0, second, none, new Map()))).toEqual(["turn", "turn", "user", "call:2"]);
    expect(kinds(rowsOf(groups, 0, foldLevel(UNFOLDED, "calls", true), none, new Map()))).toEqual(["turn", "user", "call:0", "call:1", "agent", "call:4", "aux:3", "turn", "user", "call:2"]);
    expect(kinds(rowsOf(groups, 0, foldLevel(UNFOLDED, "agents", true), none, new Map()))).toEqual(["turn", "user", "call:0", "tool:0:tu1", "call:1", "agent", "aux:3", "turn", "user", "call:2"]);
    expect(kinds(rowsOf(groups, 1, UNFOLDED, none, new Map()))).toEqual(["turn", "user", "call:2"]);
  });

  it("search what is folded away, and keep only the turns with a call that passes", () => {
    const everything = foldLevel(foldLevel(UNFOLDED, "turns", true), "calls", true);
    expect(kinds(rowsOf(groups, 0, everything, { ...none, query: "ls" }, new Map()))).toEqual(["turn", "call:0", "tool:0:tu1"]);
    expect(kinds(rowsOf(groups, 0, UNFOLDED, { ...none, query: "answer" }, new Map()))).toEqual(["turn", "call:2"]);
    expect(kinds(rowsOf(groups, 0, UNFOLDED, { ...none, filter: "failed" }, new Map()))).toEqual(["turn", "call:1"]);
    expect(kinds(rowsOf(groups, 0, UNFOLDED, { ...none, filter: "tools" }, new Map()))).toEqual(["turn", "call:0", "tool:0:tu1", "agent", "call:4", "tool:4:tu-sub"]);
    // A stretch of time: the calls active in it (the third call runs 3000–3500).
    expect(kinds(rowsOf(groups, 0, UNFOLDED, { ...none, range: [3200, 3300] }, new Map()))).toEqual(["turn", "call:2"]);
    expect(rowsOf(groups, 0, UNFOLDED, { ...none, query: "nothing says this" }, new Map())).toEqual([]);
  });

  it("mark the turn's last own call with the feedback that stands on the turn", () => {
    const feedback = feedbackByTurn([
      { turn_id: "t1", rating: "down", category: "slow", at: 1, by: "d" },
      { turn_id: "t2", rating: "up", at: 2, by: "d" },
      { turn_id: "t2", rating: null, at: 3, by: "d" },
      null,
    ]);
    const rows = rowsOf(groups, 0, UNFOLDED, none, feedback);
    const rated = rows.filter((r) => r.kind === "call" && r.feedback).map((r) => (r.kind === "call" ? [r.call.index, r.feedback!.rating] : null));
    expect(rated).toEqual([[1, "down"]]);
  });

  it("are drawn only where they are in view, with the room above and below", () => {
    const rows = rowsOf(groups, 0, UNFOLDED, none, new Map());
    const all = windowOf(rows, 0, 10_000, 0);
    expect([all.start, all.end, all.above, all.below]).toEqual([0, rows.length, 0, 0]);
    const some = windowOf(rows, 100, 60, 0);
    expect(some.start).toBeGreaterThan(0);
    expect(some.end).toBeLessThan(rows.length);
    expect(some.above + some.below).toBeLessThan(some.total);
    // Overscan reaches both ways, within the rows.
    const wide = windowOf(rows, 100, 60, 100);
    expect([wide.start, wide.end]).toEqual([0, rows.length]);
  });
});

describe("the overview", () => {
  it("has a bar per timed call, split at the first chunk, on a lane per side, and the tools' time between a turn's calls", () => {
    const groups = groupTurns([turn("t1", 0, "q")], [
      c(0, { turn_id: "t1", ts: 1000, timing: { sent_at: 1000, ttft_ms: 200, generate_ms: 300, tokens_per_s: 1 }, duration_ms: 500 }),
      c(1, { turn_id: "t1", step: 1, ts: 2500, timing: { sent_at: 2500, ttft_ms: null, generate_ms: null, tokens_per_s: null }, duration_ms: 500 }),
      c(2, { session_id: "sub", parent_session_id: "s1", turn_id: null, ts: 1600, timing: { sent_at: 1600, ttft_ms: 50, generate_ms: 50, tokens_per_s: 1 }, duration_ms: 100 }),
      c(3, { turn_id: "t1", turn: 0, purpose: "title", ts: 3100, timing: null, duration_ms: null }),
    ]);
    const overview = overviewOf(groups)!;
    expect([overview.start, overview.end]).toEqual([1000, 3100]);
    expect(overview.bars.map((b) => [b.call.index, b.lane, b.from, b.to, b.first])).toEqual([
      [0, 0, 1000, 1500, 1200],
      [1, 0, 2500, 3000, null],
      [3, 0, 3100, 3100, null],
      [2, 1, 1600, 1700, 1650],
    ]);
    expect(overview.gaps).toEqual([{ key: "0-1", from: 1500, to: 2500 }]);
    expect(overviewOf([])).toBeNull();
  });

  it("zooms around the pointer, within the whole, and no closer than a thousandth", () => {
    const whole: [number, number] = [0, 10_000];
    expect(zoomed(whole, whole, 0.5, 0.5)).toEqual([2500, 7500]);
    expect(zoomed([2500, 7500], whole, 0, 0.5)).toEqual([2500, 5000]);
    expect(zoomed([2500, 7500], whole, 0.5, 4)).toEqual([0, 10_000]);
    expect(zoomed([9000, 10_000], whole, 1, 2)).toEqual([8000, 10_000]);
    const [from, to] = zoomed([5000, 5020], whole, 0.5, 0.1);
    expect(to - from).toBe(10);
  });
});
