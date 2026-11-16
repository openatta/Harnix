/**
 * The time overview above the trajectory's rows: every call a bar from when it was sent to its end,
 * placed by its real time — the wait for the first chunk lighter, the decoding after it full — the
 * session's own calls on one lane and its sub-agents' on another, the tools' time between a turn's
 * calls as gaps. Dragging across it picks a stretch of time and the rows below narrow to the calls
 * active in it; the wheel zooms the stretch shown; "reset" shows the whole again.
 */

import { useRef, useState, type WheelEvent } from "react";
import { useT } from "@harnix/react";

import { seconds } from "../shared.tsx";
import { placeOf, zoomed, type Overview } from "./model.ts";

export function Timeline({
  overview,
  range,
  onRange,
  selected,
  onPick,
}: {
  overview: Overview;
  range: [number, number] | null;
  onRange: (r: [number, number] | null) => void;
  selected: number | null;
  onPick: (index: number) => void;
}) {
  const tr = useT();
  const strip = useRef<HTMLDivElement>(null);
  const whole: [number, number] = [overview.start, overview.end];
  const [shown, setShown] = useState<[number, number] | null>(null);
  const [drag, setDrag] = useState<[number, number] | null>(null);
  const view = shown ?? whole;
  const total = Math.max(view[1] - view[0], 1);
  const pct = (t: number) => `${placeOf(t, view) * 100}%`;
  const width = (from: number, to: number) => `${Math.max(placeOf(to, view) - placeOf(from, view), 0) * 100}%`;
  const at = (clientX: number) => {
    const box = strip.current!.getBoundingClientRect();
    return view[0] + Math.min(Math.max((clientX - box.left) / Math.max(box.width, 1), 0), 1) * total;
  };
  const shownRange = drag ?? range;
  const lanes = overview.bars.some((b) => b.lane === 1) ? 2 : 1;
  const onWheel = (e: WheelEvent<HTMLDivElement>) => {
    if (!strip.current) return;
    const box = strip.current.getBoundingClientRect();
    const place = (e.clientX - box.left) / Math.max(box.width, 1);
    const next = zoomed(view, whole, place, e.deltaY > 0 ? 1.25 : 0.8);
    setShown(next[0] <= whole[0] && next[1] >= whole[1] ? null : next);
  };
  return (
    <div className="traj-overview" data-testid="traj-overview">
      <div className="traj-overview-head">
        <span className="muted" data-testid="traj-start">
          {new Date(overview.start).toLocaleTimeString()}
        </span>
        <span className="muted">{tr("system.duration", { seconds: ((overview.end - overview.start) / 1000).toFixed(1) })}</span>
        {shown ? <span className="muted" data-testid="traj-zoomed">{tr("trajectory.zoomed", { seconds: (total / 1000).toFixed(1) })}</span> : null}
        <span className="sp" />
        {shown || range ? (
          <button
            type="button"
            className="btn sm outline"
            data-testid="timeline-clear"
            onClick={() => {
              setShown(null);
              onRange(null);
            }}
          >
            {tr("trajectory.reset")}
          </button>
        ) : null}
      </div>
      <div
        className={`timeline lanes-${lanes}`}
        ref={strip}
        data-testid="calls-timeline"
        title={tr("system.timelineHint")}
        onWheel={onWheel}
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          const t = at(e.clientX);
          setDrag([t, t]);
        }}
        onPointerMove={(e) => {
          if (!drag) return;
          // Captured once it moves, not on the press: a captured press's click goes to the strip, and
          // a bar clicked would not open its call.
          if (!strip.current?.hasPointerCapture?.(e.pointerId)) strip.current?.setPointerCapture?.(e.pointerId);
          setDrag([drag[0], at(e.clientX)]);
        }}
        onPointerUp={() => {
          if (!drag) return;
          const [a, b] = drag[0] <= drag[1] ? drag : [drag[1], drag[0]];
          setDrag(null);
          onRange(b - a >= total / 200 ? [a, b] : null);
        }}
      >
        {overview.gaps.map((gap) => (
          <i key={gap.key} className="gap" style={{ left: pct(gap.from), width: width(gap.from, gap.to) }} />
        ))}
        {overview.bars.map((bar) => (
          <i
            key={bar.call.index}
            className={`call lane-${bar.lane}${bar.call.purpose ? " aux" : ""}${bar.call.status === "error" ? " bad" : ""}${selected === bar.call.index ? " on" : ""}`}
            data-testid="timeline-call"
            data-call={bar.call.index}
            title={`${new Date(bar.from).toLocaleTimeString()} · ${seconds(bar.to - bar.from)}${bar.first != null ? ` · ${tr("system.ttft", { ms: bar.first - bar.from })}` : ""}`}
            style={{ left: pct(bar.from), width: width(bar.from, bar.to) }}
            onClick={(e) => {
              e.stopPropagation();
              onPick(bar.call.index);
            }}
          >
            {bar.first != null ? <b style={{ width: `${Math.min(100, ((bar.first - bar.from) / Math.max(bar.to - bar.from, 1)) * 100)}%` }} /> : null}
          </i>
        ))}
        {shownRange ? (
          <i className="range" data-testid="timeline-range" style={{ left: pct(Math.min(...shownRange)), width: width(Math.min(...shownRange), Math.max(...shownRange)) }} />
        ) : null}
      </div>
      <div className="traj-legend">
        <span className="model">{tr("system.legend.model")}</span>
        <span className="wait">{tr("system.legend.wait")}</span>
        <span className="gap">{tr("system.legend.toolsTime")}</span>
        {lanes > 1 ? <span className="agent">{tr("trajectory.legend.agents")}</span> : null}
      </div>
    </div>
  );
}
