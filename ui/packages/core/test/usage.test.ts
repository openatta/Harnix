import { describe, expect, it } from "vitest";

import { agentTokens, breakdownUsage, summarizeUsage, windowDays, type AgentUsage, type DayUsage, type TurnSummary } from "../src/index.ts";

const agent = (over: Partial<AgentUsage> = {}): AgentUsage => ({
  session_id: "s1",
  parent_session_id: null,
  calls: 2,
  auxiliary: 1,
  turns: 2,
  models: ["m"],
  input_tokens: 10,
  cache_read_tokens: 90,
  cache_write_tokens: 0,
  output_tokens: 20,
  model_ms: 3000,
  errors: 0,
  first_at: 1000,
  last_at: 6000,
  ...over,
});

const day = (d: string, model: string, over: Partial<DayUsage> = {}): DayUsage => ({
  day: d,
  model,
  calls: 1,
  auxiliary: 0,
  input_tokens: 10,
  cache_read_tokens: 5,
  cache_write_tokens: 1,
  output_tokens: 4,
  model_ms: 100,
  errors: 0,
  ...over,
});

describe("summarizeUsage", () => {
  it("takes the recording's figures, the turns' timing and the sub-agents' totals", () => {
    const turns = [
      { ttft_ms: 400, output_tokens: 10, tokens_per_s: 10, tool_ms: 50, input_tokens: 0, model_ms: 0, calls: [], auxiliary: 0 },
      { ttft_ms: 600, output_tokens: 10, tokens_per_s: 20, tool_ms: 150, input_tokens: 0, model_ms: 0, calls: [], auxiliary: 0 },
    ] as unknown as TurnSummary[];
    const s = summarizeUsage(agent(), [agent({ session_id: "a", input_tokens: 1, cache_read_tokens: 0, output_tokens: 1, model_ms: 7 })], turns);
    expect(s).toMatchObject({ turns: 2, calls: 2, auxiliary: 1, input: 10, cacheRead: 90, output: 20, total: 120, cacheHit: 0.9, ttftMs: 500, toolMs: 200, spanMs: 5000 });
    expect(s.rate).toBeCloseTo(20 / 1.5);
    expect(s.agents).toEqual({ count: 1, total: 2, modelMs: 7 });
  });

  it("without a recording, sums what the turns say and leaves the rest open", () => {
    const s = summarizeUsage(undefined, [], []);
    expect(s).toMatchObject({ turns: 0, calls: 0, total: 0, cacheHit: null, toolMs: null, ttftMs: null, rate: null, spanMs: null });
  });

  it("an Agent's tokens are the four figures", () => {
    expect(agentTokens(agent())).toBe(120);
  });
});

describe("breakdownUsage", () => {
  const days = windowDays(Date.UTC(2026, 10, 14), 3, 0);

  it("counts the days of the window, a day at midnight where the caller is", () => {
    expect(days).toEqual(["2026-11-14", "2026-11-15", "2026-11-16"]);
    // 2026-11-14T23:30Z, eight hours east, is the 15th already.
    expect(windowDays(Date.UTC(2026, 10, 14, 23, 30), 1, 480)).toEqual(["2026-11-15"]);
    expect(windowDays(Date.UTC(2026, 10, 15, 3), 1, -300)).toEqual(["2026-11-14"]);
    expect(windowDays(0, 0, 0)).toEqual([]);
  });

  it("sums by project, day and model, a session once per row, nothing outside the window", () => {
    const b = breakdownUsage(
      [
        { instance: "main", sessionId: "a", project: "/p", days: [day("2026-11-14", "big"), day("2026-11-14", "small"), day("2026-11-13", "big", { calls: 9 })] },
        { instance: "main", sessionId: "b", project: "/p", days: [day("2026-11-16", "big", { input_tokens: 100, errors: 1 })] },
        { instance: "side", sessionId: "c", project: null, days: [day("2026-11-15", "small", { auxiliary: 2, calls: 0 })] },
      ],
      days,
    );
    expect(b.totals).toMatchObject({ sessions: 3, calls: 3, auxiliary: 2, input: 130, cacheRead: 20, cacheWrite: 4, output: 16, errors: 1, modelMs: 400 });
    expect(b.totals.total).toBe(130 + 20 + 4 + 16);
    expect(b.byProject.map((r) => [r.key, r.sessions, r.calls])).toEqual([
      ["/p", 2, 3],
      ["", 1, 0],
    ]);
    expect(b.byDay.map((r) => [r.key, r.sessions, r.calls])).toEqual([
      ["2026-11-14", 1, 2],
      ["2026-11-15", 1, 0],
      ["2026-11-16", 1, 1],
    ]);
    expect(b.byModel.map((r) => [r.key, r.sessions])).toEqual([
      ["big", 2],
      ["small", 2],
    ]);
  });

  it("with nothing to sum, every day of the window is there at zero", () => {
    const b = breakdownUsage([{ instance: "main", sessionId: "a", project: null, days: [] }], days);
    expect(b.totals.sessions).toBe(0);
    expect(b.byDay.map((r) => r.total)).toEqual([0, 0, 0]);
    expect(b.byProject).toEqual([]);
    expect(b.byModel).toEqual([]);
  });

  it("many sessions and days stay as many rows as keys", () => {
    const window = windowDays(Date.UTC(2026, 9, 18), 30, 0);
    const sessions = Array.from({ length: 2000 }, (_, i) => ({
      instance: "main",
      sessionId: `s${i}`,
      project: `/p${i % 7}`,
      days: window.map((d) => day(d, `m${i % 3}`)),
    }));
    const b = breakdownUsage(sessions, window);
    expect(b.byDay).toHaveLength(30);
    expect(b.byProject).toHaveLength(7);
    expect(b.byModel).toHaveLength(3);
    expect(b.totals.calls).toBe(2000 * 30);
    expect(b.byDay[0]!.sessions).toBe(2000);
  });
});
