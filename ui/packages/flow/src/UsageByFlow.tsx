/**
 * The flows' section of the usage summary (`observability.usage.section`): the runs
 * started in the tab's window, summed per flow from the run records (`list`): how many, the tokens
 * their stations' turns spent, how long the finished ones took.
 */

import { FigureTable, useT, type FigureColumn } from "@harnix/react";

import { useDefs, useMethod } from "./hooks";
import { formatDuration, formatTokens, runsOf, tokensOf, usageByFlow, type FlowUsageRow } from "./model";

/** The newest runs read: enough for any month a person runs. */
export const USAGE_RUNS = 2000;

export function UsageByFlow({ module, since }: { module: string; since: number }) {
  const tr = useT();
  const listed = useMethod<unknown>(module, "list", { limit: USAGE_RUNS });
  const defs = useDefs(module);
  if (listed.error) return <div className="note" data-testid="usage-by-flow-failed">{listed.error}</div>;
  if (listed.value === undefined) return null;
  const runs = runsOf(listed.value);
  const rows = usageByFlow(runs, since);
  // Every run read is in the window: older ones may be past the newest read.
  const cut = runs.length >= USAGE_RUNS && runs.every((r) => Number(r.created_at ?? 0) >= since);
  if (!rows.length) return <div className="note" data-testid="usage-by-flow-none">{tr("flowui.usage.none")}</div>;
  const title = (flow: string) => defs.list.find((d) => d.name === flow)?.title || flow;
  const tokens = (id: string, label: string, of: (r: FlowUsageRow) => number): FigureColumn<FlowUsageRow> => ({
    id,
    label,
    numeric: true,
    value: of,
    render: (r) => formatTokens(of(r)) || "0",
  });
  const columns: FigureColumn<FlowUsageRow>[] = [
    { id: "flow", label: tr("flowui.usage.flow"), value: (r) => title(r.flow), title: (r) => r.flow },
    { id: "runs", label: tr("flowui.usage.runs"), numeric: true, value: (r) => r.runs, render: (r) => (r.finished === r.runs ? String(r.runs) : `${r.runs} (${r.finished})`), title: () => tr("flowui.usage.runsTip") },
    { id: "turns", label: tr("flowui.usage.turns"), numeric: true, value: (r) => r.usage.turns ?? 0 },
    tokens("input", tr("usageAll.col.input"), (r) => r.usage.input_tokens ?? 0),
    tokens("output", tr("usageAll.col.output"), (r) => r.usage.output_tokens ?? 0),
    tokens("cacheRead", tr("usageAll.col.cacheRead"), (r) => r.usage.cache_read_input_tokens ?? 0),
    tokens("cacheWrite", tr("usageAll.col.cacheWrite"), (r) => r.usage.cache_creation_input_tokens ?? 0),
    tokens("total", tr("usageAll.col.total"), (r) => tokensOf(r.usage)),
    { id: "elapsed", label: tr("flowui.usage.elapsed"), numeric: true, value: (r) => r.elapsedMs, render: (r) => formatDuration(r.elapsedMs) || "—" },
  ];
  return (
    <>
      <FigureTable testId="usage-by-flow" rows={rows} rowKey={(r) => r.flow} dataKey={(r) => r.flow} columns={columns} sort={{ column: "total", descending: true }} />
      <div className="note">{tr("flowui.usage.note")}</div>
      {cut ? <div className="note" data-testid="usage-by-flow-cut">{tr("flowui.usage.cut", { count: USAGE_RUNS })}</div> : null}
    </>
  );
}
