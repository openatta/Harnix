/**
 * The station graph: one picture in two modes.
 *
 * Definition mode (no run): each station shows its title, its work (executor kind, task count, `×N`
 * for `foreach`), its skill, its exit-condition kinds, and a marker when it names policies or
 * permissions of its own (xops-04). Run mode (a
 * `RunView`): the same graph coloured by each station's status, with its round, its task progress,
 * how long it took and its tokens (xops-03 §4), and a mark when it was forced through. `needs` edges
 * are solid, rework edges dashed arcs.
 *
 * Each station is a button (`data-testid="flow-node-<id>"`, `data-status` in run mode); clicking it
 * or pressing Enter calls `onSelect`.
 */

import { useId, useMemo, type KeyboardEvent } from "react";
import { useT } from "@harnix/react";

import { backEdgePath, backLanes, edgePath, layoutFlow, type LayoutOptions } from "./layout";
import { exitKinds, formatDuration, formatTokens, stationElapsed, stationStyle, stationTitle, stationUsage, taskProgress, tokensOf, workLabel, workOf } from "./model";
import type { FlowDef, RunView, Station, StationStatus } from "./types";

export interface FlowGraphProps {
  def: FlowDef;
  /** Given: run mode. */
  view?: RunView | null;
  selected?: string | null;
  onSelect?(station: string): void;
  layout?: LayoutOptions;
}

/** Cut to about `units` Latin characters' width, counting a CJK character as two. */
export function clipText(text: string, units: number): string {
  let used = 0;
  let out = "";
  for (const ch of text) {
    const w = /[⺀-鿿가-힯＀-￯]/.test(ch) ? 2 : 1;
    if (used + w > units) return `${out.replace(/\s+$/, "")}…`;
    used += w;
    out += ch;
  }
  return out;
}

export function FlowGraph({ def, view, selected, onSelect, layout: options }: FlowGraphProps) {
  const t = useT();
  const arrow = `hx-flow-arrow-${useId().replace(/[^A-Za-z0-9_-]/g, "")}`;
  const stations = def.stations ?? [];
  const layout = useMemo(() => layoutFlow(stations, options), [stations, options]);
  const lanes = useMemo(() => backLanes(layout), [layout]);
  const byId = useMemo(() => new Map(stations.map((s) => [s.id, s])), [stations]);
  const run = view?.record ?? null;
  const units = Math.floor(((options?.nodeWidth ?? 184) - 20) / 7);
  const now = Date.now();

  const label = (s: Station): string => {
    const title = stationTitle(s);
    const head = title === s.id ? s.id : `${title} (${s.id})`;
    if (!run) {
      const parts = [workLabel(s) || t("flowui.graph.gate"), ...exitKinds(s).map((k) => t("flowui.exit." + k))];
      return `${head}: ${parts.join(", ")}`;
    }
    const rec = run.stations[s.id];
    const status: StationStatus = rec?.status ?? "pending";
    const { done, total } = taskProgress(rec);
    const parts = [t("flowui.status." + status)];
    if (rec && rec.round > 1) parts.push(t("flowui.graph.round", { n: rec.round }));
    if (total) parts.push(t("flowui.graph.progress", { done, total }));
    if (rec?.forced) parts.push(t("flowui.graph.forced"));
    return `${head}: ${parts.join(", ")}`;
  };

  const key = (id: string) => (e: KeyboardEvent<SVGGElement>) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onSelect?.(id);
    }
  };

  return (
    <div className="hx-flow-graph" data-testid="flow-graph" data-mode={run ? "run" : "definition"}>
      <svg
        className="hx-flow-svg"
        width={layout.width}
        height={layout.height}
        viewBox={`0 0 ${layout.width} ${layout.height}`}
        role="group"
        aria-label={def.title || def.name}
      >
        <defs>
          <marker id={arrow} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
            <path className="hx-flow-arrowhead" d="M0,0 L10,5 L0,10 z" />
          </marker>
        </defs>
        <g className="hx-flow-edges">
          {layout.edges.map((e) => (
            <path key={`${e.from}>${e.to}`} className="hx-flow-edge" d={edgePath(layout, e)} markerEnd={`url(#${arrow})`} data-edge={`${e.from}>${e.to}`} />
          ))}
          {layout.back.map((e, i) => (
            <path
              key={`${e.kind}:${e.from}>${e.to}`}
              className={`hx-flow-back hx-flow-back-${e.kind}`}
              d={backEdgePath(layout, e, lanes[i])}
              markerEnd={`url(#${arrow})`}
              data-back={`${e.from}>${e.to}`}
            >
              <title>{t(e.kind === "rework" ? "flowui.graph.reworkEdge" : "flowui.graph.autoReworkEdge", { from: e.from, to: e.to })}</title>
            </path>
          ))}
        </g>
        {layout.nodes.map((n) => {
          const s = byId.get(n.id)!;
          const rec = run?.stations[s.id];
          const status: StationStatus | undefined = run ? (rec?.status ?? "pending") : undefined;
          const style = run ? stationStyle(status) : { className: "hx-flow-def" };
          const work = workOf(s);
          const { done, total } = taskProgress(rec);
          const unmet = view?.unmet?.[s.id]?.length ?? 0;
          const line2 = run
            ? [t("flowui.status." + status), rec && rec.round > 1 ? t("flowui.graph.round", { n: rec.round }) : ""].filter(Boolean).join(" · ")
            : [workLabel(s) || t("flowui.graph.gate"), ...work.skills].join(" · ");
          const elapsed = run ? stationElapsed(rec, now) : null;
          const tokens = run ? tokensOf(stationUsage(rec)) : 0;
          const line3 = run
            ? [
                total ? `${done}/${total}` : "",
                formatDuration(elapsed),
                tokens ? t("flowui.tokens", { n: formatTokens(tokens) }) : "",
                unmet && status !== "passed" ? t("flowui.graph.unmet", { n: unmet }) : "",
              ]
                .filter(Boolean)
                .join(" · ")
            : exitKinds(s).join(" · ");
          return (
            <g
              key={n.id}
              className={`hx-flow-node ${style.className}${selected === n.id ? " hx-flow-selected" : ""}`}
              transform={`translate(${n.x},${n.y})`}
              role="button"
              tabIndex={0}
              aria-label={label(s)}
              aria-pressed={selected === n.id}
              data-testid={`flow-node-${n.id}`}
              data-status={status}
              data-elapsed={elapsed ?? undefined}
              data-tokens={run ? tokens : undefined}
              onClick={() => onSelect?.(n.id)}
              onKeyDown={key(n.id)}
            >
              <rect className="hx-flow-box" width={n.width} height={n.height} rx={8} ry={8} />
              <text className="hx-flow-title" x={10} y={21}>
                {clipText(stationTitle(s), units - 3)}
              </text>
              <text className="hx-flow-line" x={10} y={41}>
                {clipText(line2, units)}
              </text>
              <text className="hx-flow-line hx-flow-faint" x={10} y={59}>
                {clipText(line3, units)}
              </text>
              {!run && ((s.policies?.length ?? 0) > 0 || (s.permissions?.length ?? 0) > 0) ? (
                <circle className="hx-flow-hook" cx={n.width - 12} cy={12} r={4} data-testid={`flow-guarded-${n.id}`}>
                  <title>
                    {t("flowui.graph.guarded", {
                      list: [...(s.policies ?? []), ...(s.permissions ?? []).map((p) => `${p.effect} ${p.tools.join(", ")}`)].join("; "),
                    })}
                  </title>
                </circle>
              ) : null}
              {!run && s.hold ? (
                <text className="hx-flow-mark" x={n.width - 22} y={21} textAnchor="end">
                  {t("flowui.graph.hold")}
                </text>
              ) : null}
              {rec?.forced ? (
                <text className="hx-flow-mark hx-flow-forced" x={n.width - 10} y={21} textAnchor="end" data-testid={`flow-forced-${n.id}`}>
                  {t("flowui.graph.forced")}
                </text>
              ) : null}
            </g>
          );
        })}
      </svg>
    </div>
  );
}
