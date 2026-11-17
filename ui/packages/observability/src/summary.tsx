/**
 * The usage summary (`UsageSummaryTab`, for whoever has a page for it): what every session of every instance has used
 * over the last days — totals, a bar a day, and tables by project, by day and by model — summed from
 * the recordings (`observability.usage.daily`), and below them the sections other packages add to
 * the point this package defines, `observability.usage.section` (the flows, by flow).
 *
 * A recording covers a session's latest run only, so these are not totals over every run; a
 * sleeping instance gives what was read of it before. The sessions asked about are those the
 * instances list (`harnix.instances`, `harnix.instances.sessions`) that were active in the window.
 */

import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  breakdownUsage,
  localOffsetMinutes,
  windowDays,
  type InstanceRow,
  type SessionDays as ReadDays,
  type SessionRow,
  type UsageBreakdown,
  type UsageRow,
  type UsageSession,
} from "@harnix/core";
import { Bars, Contributed, FigureTable, compact, millis, useContributions, useT, type FigureColumn, type HostApi } from "@harnix/react";

import { Failure, type Observed } from "./shared.tsx";

/** The point this package defines: a section of the 用量 tab. */
export const USAGE_SECTION = "observability.usage.section";

/** How many days the summary covers. */
export const SUMMARY_DAYS = 30;
/** One `observability.usage.daily` call's most. */
const BATCH = 200;
/** How long the first value of an instance's session list is waited for. */
const LISTED_WITHIN = 10_000;

/** What a section is told: the window the tab sums, its first day's start (ms) and how many days. */
export interface UsageSectionContext {
  since: number;
  days: number;
}

/** A section of the 用量 tab (`observability.usage.section`). */
export interface UsageSection {
  id: string;
  title(): string;
  order?: number;
  render(context: UsageSectionContext): ReactNode;
}

interface Summary {
  since: number;
  days: string[];
  sessions: UsageSession[];
  breakdown: UsageBreakdown;
  /** Instances that could not be read now. */
  unread: string[];
  /** Sessions whose figures are the ones read before (their instance sleeps). */
  stale: number;
}

/** A topic's first value, or undefined when none comes in time. */
function firstValue<T>(host: HostApi, topic: string, params: Record<string, unknown> | undefined): Promise<T | undefined> {
  return new Promise((resolve) => {
    let done = false;
    let off: (() => void) | null = null;
    const finish = (value: T | undefined) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      resolve(value);
      // Unsubscribed after the call that delivered the value has returned.
      queueMicrotask(() => off?.());
    };
    const timer = setTimeout(() => finish(undefined), LISTED_WITHIN);
    off = host.subscribe(topic, params, (v) => finish(v as T));
  });
}

/** Whether a session was active since `since`: one whose last activity is unknown is asked about. */
function activeSince(row: SessionRow, since: number): boolean {
  const at = row.last_active ? Date.parse(row.last_active) : NaN;
  return Number.isNaN(at) || at >= since;
}

/** Every listed session active in the window, its days read in batches; an instance that cannot be read is named. */
export async function readSummary(host: HostApi, offset: number, now = Date.now()): Promise<Summary> {
  const days = windowDays(now - (SUMMARY_DAYS - 1) * 86_400_000, SUMMARY_DAYS, offset);
  let since = Date.parse(`${days[0]}T00:00:00Z`) - offset * 60_000;
  const listed = await firstValue<{ instances?: InstanceRow[] }>(host, "harnix.instances", {});
  const instances = (listed?.instances ?? []).map((i) => i.id);
  const sessions: UsageSession[] = [];
  const unread: string[] = [];
  let stale = 0;
  await Promise.all(
    instances.map(async (instance) => {
      const rows = (await firstValue<{ sessions?: SessionRow[] }>(host, "harnix.instances.sessions", { instance }))?.sessions ?? [];
      const asked = rows.filter((row) => activeSince(row, since));
      const project = new Map(asked.map((row) => [row.session_id, row.project_root ?? null]));
      for (let at = 0; at < asked.length; at += BATCH) {
        const ids = asked.slice(at, at + BATCH).map((row) => row.session_id);
        try {
          const answer = await host.call<{ since_ms?: number; sessions?: Record<string, ReadDays> }>("observability.usage.daily", {
            instance,
            session_ids: ids,
            days: SUMMARY_DAYS,
            tz_offset_min: offset,
          });
          if (typeof answer.since_ms === "number") since = answer.since_ms;
          for (const [sessionId, read] of Object.entries(answer.sessions ?? {})) {
            if (read.stale) stale += 1;
            sessions.push({ instance, sessionId, project: project.get(sessionId) ?? null, days: read.days ?? [] });
          }
        } catch {
          if (!unread.includes(instance)) unread.push(instance);
          return;
        }
      }
    }),
  );
  // The backend's window when it answered (the same unless a midnight came between).
  const window = windowDays(since, SUMMARY_DAYS, offset);
  return { since, days: window, sessions, breakdown: breakdownUsage(sessions, window), unread: unread.sort(), stale };
}

/** The last part of a path, for a project's name. */
function basename(path: string): string {
  const parts = path.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || path;
}

export function UsageSummaryTab({ observed }: { observed: Observed }) {
  const tr = useT();
  const host = observed.host;
  const [summary, setSummary] = useState<Summary | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [loading, setLoading] = useState(true);
  const [round, setRound] = useState(0);
  useEffect(() => {
    let alive = true;
    setLoading(true);
    readSummary(host, localOffsetMinutes())
      .then((s) => alive && (setSummary(s), setError(null)))
      .catch((e: unknown) => alive && setError(e ?? "error"))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [host, round]);
  const reload = useCallback(() => setRound((n) => n + 1), []);
  const sections = [...useContributions<UsageSection>(USAGE_SECTION, observed.registry)].sort((a, b) => (a.order ?? 100) - (b.order ?? 100));
  const context: UsageSectionContext | null = summary ? { since: summary.since, days: SUMMARY_DAYS } : null;
  return (
    <div className="obs-summary" data-testid="usage-summary" data-loading={loading || undefined}>
      <div className="obs-summary-head">
        <span className="label">{tr("usageAll.window", { days: SUMMARY_DAYS })}</span>
        <span className="sp" />
        <button type="button" className="btn sm outline" data-testid="usage-summary-refresh" disabled={loading} onClick={reload}>
          {loading ? tr("usageAll.reading") : tr("usageAll.refresh")}
        </button>
      </div>
      {error ? <Failure error={error} testId="usage-summary-failed" /> : null}
      {summary ? <Figures summary={summary} /> : null}
      {context
        ? sections.map((section) => (
            <Contributed key={`${section.owner}/${section.id}`} registry={observed.registry} point={USAGE_SECTION} contribution={section}>
              <Section section={section} context={context} />
            </Contributed>
          ))
        : null}
      <div className="note" data-testid="usage-summary-note">
        {tr("usageAll.covers")}
      </div>
    </div>
  );
}

/** A contributed section: its title, then what it renders (inside its guard: one that throws loses only itself). */
function Section({ section, context }: { section: UsageSection; context: UsageSectionContext }) {
  return (
    <section className="obs-summary-section" data-section={section.id}>
      <div className="label">{section.title()}</div>
      {section.render(context)}
    </section>
  );
}

/** The columns every table of the summary shares, after its first. */
function figureColumns<R extends UsageRow>(tr: ReturnType<typeof useT>, sessions = true): FigureColumn<R>[] {
  const n = (id: keyof UsageRow, label: string, show: (v: number) => string = compact): FigureColumn<R> => ({
    id,
    label,
    numeric: true,
    value: (r) => r[id] as number,
    render: (r) => show(r[id] as number),
  });
  return [
    ...(sessions ? [n("sessions", tr("usageAll.col.sessions"), String)] : []),
    {
      id: "calls",
      label: tr("usageAll.col.calls"),
      numeric: true,
      value: (r) => r.calls,
      render: (r) => (r.auxiliary ? `${r.calls} +${r.auxiliary}` : String(r.calls)),
      title: (r) => (r.errors ? tr("obs.stat.errors", { count: r.errors }) : undefined),
    },
    n("input", tr("usageAll.col.input")),
    n("output", tr("usageAll.col.output")),
    n("cacheRead", tr("usageAll.col.cacheRead")),
    n("cacheWrite", tr("usageAll.col.cacheWrite")),
    n("total", tr("usageAll.col.total")),
    n("modelMs", tr("usageAll.col.modelMs"), millis),
  ];
}

function Figures({ summary }: { summary: Summary }) {
  const tr = useT();
  const { totals, byProject, byDay, byModel } = summary.breakdown;
  const cells: [string, string, string][] = [
    ["sessions", tr("usageAll.col.sessions"), String(totals.sessions)],
    ["calls", tr("usageAll.col.calls"), totals.auxiliary ? `${totals.calls} +${totals.auxiliary}` : String(totals.calls)],
    ["total", tr("usageAll.col.total"), compact(totals.total)],
    ["input", tr("usageAll.col.input"), compact(totals.input)],
    ["output", tr("usageAll.col.output"), compact(totals.output)],
    ["cacheRead", tr("usageAll.col.cacheRead"), compact(totals.cacheRead)],
    ["cacheWrite", tr("usageAll.col.cacheWrite"), compact(totals.cacheWrite)],
    ["model", tr("usageAll.col.modelMs"), millis(totals.modelMs)],
  ];
  const rest = figureColumns<UsageRow>(tr);
  const named = (label: string, name: (r: UsageRow) => string, title?: (r: UsageRow) => string | undefined): FigureColumn<UsageRow> => ({
    id: "key",
    label,
    value: (r) => r.key,
    render: name,
    title,
  });
  return (
    <>
      <div className="stat-grid" data-testid="usage-summary-totals">
        {cells.map(([id, label, value]) => (
          <div key={id} className="cell" data-cell={id}>
            <div className="v">{value}</div>
            <div className="l">{label}</div>
          </div>
        ))}
      </div>
      {summary.unread.length ? (
        <div className="note" data-testid="usage-summary-unread">
          {tr("usageAll.unread", { instances: summary.unread.join(", ") })}
        </div>
      ) : null}
      {summary.stale ? <div className="note">{tr("usageAll.stale", { count: summary.stale })}</div> : null}
      <div className="label">{tr("usageAll.perDay")}</div>
      <Bars
        testId="usage-summary-bars"
        label={tr("usageAll.perDay")}
        bars={byDay.map((r) => ({ key: r.key, label: r.key.slice(5), value: r.total, title: tr("usageAll.barTip", { day: r.key, tokens: compact(r.total), calls: r.calls }) }))}
      />
      {totals.calls + totals.auxiliary === 0 ? (
        <div className="note" data-testid="usage-summary-none">
          {tr("usageAll.none")}
        </div>
      ) : (
        <>
          <div className="label">{tr("usageAll.byProject")}</div>
          <FigureTable
            testId="usage-by-project"
            rows={byProject}
            rowKey={(r) => r.key}
            dataKey={(r) => r.key}
            columns={[named(tr("usageAll.col.project"), (r) => (r.key ? basename(r.key) : tr("usageAll.noProject")), (r) => r.key || undefined), ...rest]}
            sort={{ column: "total", descending: true }}
          />
          <div className="label">{tr("usageAll.byModel")}</div>
          <FigureTable
            testId="usage-by-model"
            rows={byModel}
            rowKey={(r) => r.key}
            dataKey={(r) => r.key}
            columns={[named(tr("usageAll.col.model"), (r) => r.key || tr("usageAll.noModel")), ...rest]}
            sort={{ column: "total", descending: true }}
          />
          <div className="label">{tr("usageAll.byDay")}</div>
          <FigureTable
            testId="usage-by-day"
            rows={byDay.filter((r) => r.calls + r.auxiliary > 0)}
            rowKey={(r) => r.key}
            dataKey={(r) => r.key}
            columns={[named(tr("usageAll.col.day"), (r) => r.key), ...rest]}
            sort={{ column: "key", descending: true }}
          />
        </>
      )}
    </>
  );
}
