/**
 * The framework of a flow in one picture (the reference tab, above the guide): a definition from a
 * pack becoming a run that goes through stations (a fork and a join), one station opened — its work
 * (task → executor: main Agent, sub-agent, program, external) with the rules it is told and the
 * permissions each tool call is judged against, its exit (output contract, gate, a person's
 * confirmation) — and what meets the outside: hooks out to other systems, external tasks in, the
 * application's capabilities feeding programs and gates.
 *
 * Each element is coloured by the guide's group it belongs to (`--hx-flow-g1`…`g5`, the same as the
 * grouped guide below), and the legend names the groups. Clicking an element (or pressing Enter)
 * calls `onPick` with its group and the term of the guide's row about it.
 *
 * Laid out top to bottom, to read at the right column's width; it scales with it.
 */

import { useId, type KeyboardEvent, type ReactNode } from "react";
import { useT } from "@harnix/react";

import { GUIDE_GROUPS, type GuideGroup } from "./model";

/** What a click on an element asks for: its group, and the term of its row in the guide. */
export interface FrameworkPick {
  id: string;
  group: GuideGroup;
  /** The row's term in the guide's language (its first cell's bold text). */
  term: string | null;
}

/** One element of the picture: where it is, its group, the guide's term for it. */
interface Element {
  id: string;
  group: GuideGroup;
  x: number;
  y: number;
  w: number;
  h: number;
  /** Whether a guide row is about it: its term is `flowui.fw.<id>.term`, in the page's language. */
  term: boolean;
  /** A chip: a smaller, filled box. */
  chip?: boolean;
}

const W = 360;

/** The elements, top to bottom. */
const ELEMENTS: Element[] = [
  { id: "pack", group: 5, x: 10, y: 10, w: 160, h: 40, term: true },
  { id: "authoring", group: 5, x: 190, y: 10, w: 160, h: 40, term: true },
  { id: "flow", group: 1, x: 90, y: 74, w: 180, h: 34, term: true },
  { id: "run", group: 2, x: 10, y: 134, w: 210, h: 40, term: true },
  { id: "hook", group: 4, x: 240, y: 134, w: 110, h: 40, term: true },
  { id: "s1", group: 1, x: 150, y: 196, w: 60, h: 24, term: true },
  { id: "s2", group: 1, x: 80, y: 240, w: 60, h: 24, term: true },
  { id: "s3", group: 1, x: 220, y: 240, w: 60, h: 24, term: true },
  { id: "s4", group: 1, x: 150, y: 284, w: 60, h: 24, term: true },
  { id: "work", group: 1, x: 18, y: 352, w: 176, h: 152, term: true },
  { id: "task", group: 1, x: 30, y: 378, w: 152, h: 24, term: true, chip: true },
  { id: "executor", group: 2, x: 30, y: 414, w: 152, h: 24, term: true, chip: true },
  { id: "main", group: 2, x: 30, y: 448, w: 74, h: 20, term: true, chip: true },
  { id: "sub", group: 2, x: 108, y: 448, w: 74, h: 20, term: true, chip: true },
  { id: "program", group: 4, x: 30, y: 474, w: 74, h: 20, term: true, chip: true },
  { id: "external", group: 4, x: 108, y: 474, w: 74, h: 20, term: true, chip: true },
  { id: "rule", group: 3, x: 206, y: 372, w: 138, h: 40, term: true },
  { id: "permission", group: 3, x: 206, y: 424, w: 138, h: 40, term: true },
  { id: "exit", group: 1, x: 18, y: 520, w: 326, h: 64, term: true },
  { id: "output", group: 1, x: 28, y: 548, w: 98, h: 26, term: true, chip: true },
  { id: "gate", group: 3, x: 132, y: 548, w: 98, h: 26, term: true, chip: true },
  { id: "confirm", group: 3, x: 236, y: 548, w: 98, h: 26, term: true, chip: true },
  { id: "capabilities", group: 4, x: 10, y: 612, w: 170, h: 40, term: true },
  { id: "outside", group: 4, x: 190, y: 612, w: 160, h: 40, term: true },
];

/** The opened station's frame. */
const STATION = { x: 10, y: 326, w: 340, h: 266 };
const LEGEND_Y = 672;
const H = LEGEND_Y + GUIDE_GROUPS.length * 20 + 8;

const byId = new Map(ELEMENTS.map((e) => [e.id, e]));

/** The elements with a second line (`flowui.fw.<id>.sub`). */
const SUB = new Set(["pack", "authoring", "run", "hook", "rule", "permission", "capabilities", "outside"]);

function at(id: string, side: "top" | "bottom" | "left" | "right"): [number, number] {
  const e = byId.get(id)!;
  switch (side) {
    case "top":
      return [e.x + e.w / 2, e.y];
    case "bottom":
      return [e.x + e.w / 2, e.y + e.h];
    case "left":
      return [e.x, e.y + e.h / 2];
    case "right":
      return [e.x + e.w, e.y + e.h / 2];
  }
}

/** The arrows: [from, to, label?, dashed?] with the points spelt out. */
type Arrow = { d: string; label?: string; lx?: number; ly?: number; dashed?: boolean; group?: GuideGroup };

function line(a: [number, number], b: [number, number]): string {
  return `M${a[0]},${a[1]} L${b[0]},${b[1]}`;
}

export interface FlowFrameworkProps {
  onPick?(pick: FrameworkPick): void;
}

export function FlowFramework({ onPick }: FlowFrameworkProps) {
  const t = useT();
  const marker = `hx-fw-arrow-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
  const pick = (e: Element) => onPick?.({ id: e.id, group: e.group, term: e.term ? t(`flowui.fw.${e.id}.term`) : null });
  const key = (e: Element) => (ev: KeyboardEvent) => {
    if (ev.key === "Enter" || ev.key === " ") {
      ev.preventDefault();
      pick(e);
    }
  };
  const arrows: Arrow[] = [
    { d: line(at("pack", "bottom"), [at("flow", "top")[0] - 30, at("flow", "top")[1]]) },
    { d: line([at("authoring", "bottom")[0], at("authoring", "bottom")[1]], [at("flow", "top")[0] + 30, at("flow", "top")[1]]), dashed: true, label: t("flowui.fw.writes"), lx: 284, ly: 66 },
    { d: line(at("flow", "bottom"), [at("flow", "bottom")[0], at("run", "top")[1]]), label: t("flowui.fw.starts"), lx: at("flow", "bottom")[0] + 6, ly: 124 },
    { d: line(at("run", "right"), at("hook", "left")), group: 4, label: t("flowui.fw.events"), lx: 222, ly: 129 },
    { d: line([115, at("run", "bottom")[1]], at("s1", "top")), label: t("flowui.fw.through"), lx: 24, ly: 190 },
    { d: line(at("s1", "bottom"), at("s2", "top")) },
    { d: line(at("s1", "bottom"), at("s3", "top")) },
    { d: line(at("s2", "bottom"), at("s4", "top")) },
    { d: line(at("s3", "bottom"), at("s4", "top")) },
    { d: line(at("task", "bottom"), at("executor", "top")) },
    { d: line(at("rule", "left"), [at("work", "right")[0], at("rule", "left")[1]]), dashed: true, group: 3 },
    { d: line(at("permission", "left"), [at("work", "right")[0], at("permission", "left")[1]]), dashed: true, group: 3 },
    { d: line(at("work", "bottom"), [at("work", "bottom")[0], STATION.y + 194]) },
    { d: line([at("capabilities", "top")[0] - 40, at("capabilities", "top")[1]], at("program", "bottom")), group: 4, label: t("flowui.fw.feeds"), lx: 64, ly: 606 },
    { d: line([at("capabilities", "top")[0] + 50, at("capabilities", "top")[1]], at("gate", "bottom")), group: 4 },
    { d: line(at("outside", "top"), at("external", "bottom")), group: 4, label: t("flowui.fw.claims"), lx: 276, ly: 606 },
  ];
  const box = (e: Element, children: ReactNode) => (
    <g
      key={e.id}
      className={`hx-fw-el hx-fw-g${e.group}${e.chip ? " hx-fw-chip" : ""}`}
      role="button"
      tabIndex={0}
      aria-label={`${t("flowui.fw." + e.id)} — ${t("flowui.guide.group." + e.group)}`}
      data-testid={`flow-fw-${e.id}`}
      data-group={e.group}
      onClick={() => pick(e)}
      onKeyDown={key(e)}
    >
      <rect x={e.x} y={e.y} width={e.w} height={e.h} rx={e.chip ? 5 : 8} ry={e.chip ? 5 : 8} />
      {children}
    </g>
  );
  const text = (e: Element) => {
    const hasSub = SUB.has(e.id);
    const sub = hasSub ? t("flowui.fw." + e.id + ".sub") : "";
    // A box that holds others names itself at its top.
    if (e.id === "work" || e.id === "exit") {
      return (
        <text className="hx-fw-title" x={e.x + 10} y={e.y + 16}>
          {t("flowui.fw." + e.id)}
        </text>
      );
    }
    return (
      <>
        <text className={e.chip ? "hx-fw-chip-text" : "hx-fw-title"} x={e.x + e.w / 2} y={e.y + (hasSub ? e.h / 2 - 2 : e.h / 2 + 4)} textAnchor="middle">
          {t("flowui.fw." + e.id)}
        </text>
        {hasSub ? (
          <text className="hx-fw-sub" x={e.x + e.w / 2} y={e.y + e.h / 2 + 12} textAnchor="middle">
            {sub}
          </text>
        ) : null}
      </>
    );
  };
  // Containers first, so what they hold is drawn over them.
  const order = ["work", "exit"];
  const drawn = [...ELEMENTS.filter((e) => order.includes(e.id)), ...ELEMENTS.filter((e) => !order.includes(e.id))];
  return (
    <figure className="hx-flow-fw" data-testid="flow-framework">
      <svg className="hx-flow-fw-svg" viewBox={`0 0 ${W} ${H}`} role="img" aria-label={t("flowui.fw.title")} preserveAspectRatio="xMidYMin meet">
        <defs>
          <marker id={marker} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
            <path d="M0,0 L10,5 L0,10 z" className="hx-fw-arrowhead" />
          </marker>
        </defs>
        {/* The opened station: the last of the chain, drawn large. */}
        <path className="hx-fw-zoom" d={`M${at("s4", "bottom")[0]},${at("s4", "bottom")[1]} L${STATION.x + STATION.w / 2},${STATION.y}`} />
        <rect className="hx-fw-station" x={STATION.x} y={STATION.y} width={STATION.w} height={STATION.h} rx={10} ry={10} />
        <text className="hx-fw-caption" x={STATION.x + 10} y={STATION.y + 16}>
          {t("flowui.fw.station.opened")}
        </text>
        {arrows.map((a, i) => (
          <g key={i} className={`hx-fw-arrow${a.group ? ` hx-fw-g${a.group}` : ""}${a.dashed ? " hx-fw-dashed" : ""}`}>
            <path d={a.d} markerEnd={`url(#${marker})`} />
            {a.label ? (
              <text className="hx-fw-label" x={a.lx} y={a.ly}>
                {a.label}
              </text>
            ) : null}
          </g>
        ))}
        {drawn.map((e) => box(e, text(e)))}
        <text className="hx-fw-caption" x={10} y={LEGEND_Y - 4}>
          {t("flowui.fw.legend")}
        </text>
        {GUIDE_GROUPS.map((g, i) => {
          const y = LEGEND_Y + 4 + i * 20;
          const e: Element = { id: `group-${g}`, group: g, x: 10, y, w: 340, h: 16, term: false };
          return (
            <g
              key={g}
              className={`hx-fw-legend hx-fw-g${g}`}
              role="button"
              tabIndex={0}
              aria-label={t("flowui.guide.group." + g)}
              data-testid={`flow-fw-legend-${g}`}
              data-group={g}
              onClick={() => pick(e)}
              onKeyDown={key(e)}
            >
              <rect x={10} y={y} width={14} height={14} rx={3} ry={3} />
              <text className="hx-fw-legend-text" x={32} y={y + 11}>
                {t("flowui.guide.group." + g)}
              </text>
            </g>
          );
        })}
      </svg>
    </figure>
  );
}
