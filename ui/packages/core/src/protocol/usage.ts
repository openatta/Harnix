/**
 * 录像读出来的用量，算成界面上的数：一个会话这次运行的概况（`observability.usage` 加 `observability.turns`），
 * 以及许多会话按项目、按天、按模型的汇总（`observability.usage.daily`）。不依赖框架，右栏的用量页签、
 * 用量汇总用的是同一份算法。
 *
 * 用词照 `AgentUsage`：`input` 是没走缓存的输入，`cacheRead`、`cacheWrite` 是缓存读、写，四项不重叠，合计是四项之和。
 */

import type { AgentUsage, DayUsage, TurnSummary } from "./generated/readers.ts";

/** 一个 Agent 这次运行用掉的全部 token：输入（走没走缓存）与输出。 */
export function agentTokens(u: AgentUsage): number {
  return u.input_tokens + u.cache_read_tokens + u.cache_write_tokens + u.output_tokens;
}

export interface UsageSummary {
  turns: number;
  calls: number;
  auxiliary: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  /** 进出的全部 token。 */
  total: number;
  /** 输入里缓存占的份额（按 token）；没有输入时为 null。 */
  cacheHit: number | null;
  modelMs: number;
  toolMs: number | null;
  /** 有首字时间的那些轮的平均等待。 */
  ttftMs: number | null;
  /** 每秒 token，按每轮写出的量加权。 */
  rate: number | null;
  /** 从第一次调用到最后一次结束。 */
  spanMs: number | null;
  errors: number;
  agents: { count: number; total: number; modelMs: number };
}

/**
 * 一个会话这次运行的数：它自己那个 Agent 的（有录像的合计时用合计，没有时按轮加起来），轮上的时间，
 * 子 Agent 的合计。`own` 缺省、`turns` 为空时各项是 0 或 null。
 */
export function summarizeUsage(own: AgentUsage | undefined, subs: readonly AgentUsage[], turns: readonly TurnSummary[]): UsageSummary {
  const sum = (f: (t: TurnSummary) => number) => turns.reduce((n, t) => n + f(t), 0);
  const ttfts = turns.map((t) => t.ttft_ms).filter((v): v is number => typeof v === "number");
  const rated = turns.filter((t) => t.tokens_per_s && t.output_tokens);
  const writing = rated.reduce((n, t) => n + t.output_tokens / t.tokens_per_s!, 0);
  const input = own ? own.input_tokens + own.cache_read_tokens + own.cache_write_tokens : sum((t) => t.input_tokens);
  const output = own?.output_tokens ?? sum((t) => t.output_tokens);
  return {
    turns: own?.turns ?? turns.length,
    calls: own?.calls ?? sum((t) => t.calls.length),
    auxiliary: own?.auxiliary ?? sum((t) => t.auxiliary),
    input: own?.input_tokens ?? input,
    output,
    cacheRead: own?.cache_read_tokens ?? 0,
    cacheWrite: own?.cache_write_tokens ?? 0,
    total: input + output,
    cacheHit: own && input > 0 ? own.cache_read_tokens / input : null,
    modelMs: own?.model_ms ?? sum((t) => t.model_ms),
    toolMs: turns.length ? sum((t) => t.tool_ms) : null,
    ttftMs: ttfts.length ? Math.round(ttfts.reduce((a, b) => a + b, 0) / ttfts.length) : null,
    rate: writing > 0 ? rated.reduce((n, t) => n + t.output_tokens, 0) / writing : null,
    spanMs: own?.first_at != null && own.last_at != null ? Math.max(0, own.last_at - own.first_at) : null,
    errors: own?.errors ?? 0,
    agents: {
      count: subs.length,
      total: subs.reduce((n, u) => n + agentTokens(u), 0),
      modelMs: subs.reduce((n, u) => n + u.model_ms, 0),
    },
  };
}

/* ── 许多会话的汇总 ─────────────────────────────────────────────────── */

/** 一组调用的合计：汇总表里的一行。 */
export interface UsageTotals {
  /** 有调用落在这一行的会话数。 */
  sessions: number;
  calls: number;
  auxiliary: number;
  input: number;
  cacheRead: number;
  cacheWrite: number;
  output: number;
  /** 四项 token 之和。 */
  total: number;
  modelMs: number;
  errors: number;
}

/** 汇总表的一行：`key` 是项目根、日期（`YYYY-MM-DD`）或模型名；没有项目的会话的 `key` 是空串。 */
export interface UsageRow extends UsageTotals {
  key: string;
}

/** 汇总的一个会话：它在哪台实例、属于哪个项目，`observability.usage.daily` 给它的按天按模型的数。 */
export interface UsageSession {
  instance: string;
  sessionId: string;
  project: string | null;
  days: readonly DayUsage[];
}

export interface UsageBreakdown {
  totals: UsageTotals;
  /** 按合计 token 从多到少。 */
  byProject: UsageRow[];
  /** `days` 里的每一天，从早到晚，没有调用的那天是 0。 */
  byDay: UsageRow[];
  /** 按合计 token 从多到少；录像没写模型的调用归在空串下。 */
  byModel: UsageRow[];
}

export function emptyTotals(): UsageTotals {
  return { sessions: 0, calls: 0, auxiliary: 0, input: 0, cacheRead: 0, cacheWrite: 0, output: 0, total: 0, modelMs: 0, errors: 0 };
}

function add(t: UsageTotals, d: DayUsage): void {
  t.calls += d.calls;
  t.auxiliary += d.auxiliary;
  t.input += d.input_tokens;
  t.cacheRead += d.cache_read_tokens;
  t.cacheWrite += d.cache_write_tokens;
  t.output += d.output_tokens;
  t.total += d.input_tokens + d.cache_read_tokens + d.cache_write_tokens + d.output_tokens;
  t.modelMs += d.model_ms;
  t.errors += d.errors;
}

/**
 * 许多会话按项目、按天、按模型加起来，并有总数。`days` 是汇总的那些天（`windowDays` 给的）：
 * 落在它之外的数不计。一个会话在一行里只算一次。
 */
export function breakdownUsage(sessions: readonly UsageSession[], days: readonly string[]): UsageBreakdown {
  const window = new Set(days);
  const totals = emptyTotals();
  const rows = () => new Map<string, UsageRow>();
  const project = rows();
  const day = rows();
  const model = rows();
  for (const d of days) day.set(d, { key: d, ...emptyTotals() });
  for (const s of sessions) {
    const touched = new Set<UsageRow | UsageTotals>();
    const into = (map: Map<string, UsageRow>, key: string, d: DayUsage) => {
      let row = map.get(key);
      if (!row) map.set(key, (row = { key, ...emptyTotals() }));
      add(row, d);
      touched.add(row);
    };
    for (const d of s.days) {
      if (!window.has(d.day)) continue;
      add(totals, d);
      touched.add(totals);
      into(project, s.project ?? "", d);
      into(day, d.day, d);
      into(model, d.model, d);
    }
    for (const row of touched) row.sessions += 1;
  }
  const byTotal = (map: Map<string, UsageRow>) => [...map.values()].sort((a, b) => b.total - a.total || a.key.localeCompare(b.key));
  return { totals, byProject: byTotal(project), byDay: [...day.values()], byModel: byTotal(model) };
}

/**
 * 从 `sinceMs` 那天起的 `count` 天，`YYYY-MM-DD`，日界在东 `offsetMinutes` 分钟——与
 * `observability.usage.daily` 的 `since_ms`、`tz_offset_min` 同一种算法。
 */
export function windowDays(sinceMs: number, count: number, offsetMinutes: number): string[] {
  const first = Math.floor((sinceMs + offsetMinutes * 60_000) / 86_400_000);
  return Array.from({ length: Math.max(0, count) }, (_, i) => new Date((first + i) * 86_400_000).toISOString().slice(0, 10));
}

/** 这台机器此刻在 UTC 以东多少分钟：`tz_offset_min` 的值。 */
export function localOffsetMinutes(at: Date = new Date()): number {
  return -at.getTimezoneOffset();
}
