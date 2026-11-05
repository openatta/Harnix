/**
 * The station graph's layout: pure, no DOM.
 *
 * Stations are put in columns (layers) by the longest path over `needs` — a station sits one column
 * right of its latest prerequisite — and ordered within a column by a few barycenter sweeps (each
 * node moves toward the mean position of its neighbours), keeping the order with the fewest
 * crossings. Flow runs left to right. Rework edges (`rework_to`, `on_unmet.rework_to`) point back
 * upstream; they are returned on their own, to be drawn as dashed arcs, and take no part in layering.
 */

import type { Station } from "./types";

export type LayoutStation = Pick<Station, "id" | "needs" | "rework_to" | "on_unmet">;

export interface LayoutOptions {
  nodeWidth?: number;
  nodeHeight?: number;
  /** Between columns. */
  gapX?: number;
  /** Between nodes of one column. */
  gapY?: number;
  /** Around the whole graph; the top one also holds the back edges' arcs. */
  margin?: number;
  /** Barycenter sweeps (each one down and up). */
  sweeps?: number;
}

export interface LaidNode {
  id: string;
  layer: number;
  /** Position within its layer, from the top. */
  order: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

/** A `needs` edge: `from` is the prerequisite. */
export interface LaidEdge {
  from: string;
  to: string;
}

/** A rework edge: from the station that may send work back, to where it goes. */
export interface BackEdge {
  from: string;
  to: string;
  kind: "rework" | "on_unmet";
}

export interface Layout {
  nodes: LaidNode[];
  byId: Record<string, LaidNode>;
  layers: string[][];
  edges: LaidEdge[];
  back: BackEdge[];
  width: number;
  height: number;
}

const ARC_ROOM = 48;

const DEFAULTS: Required<LayoutOptions> = { nodeWidth: 184, nodeHeight: 72, gapX: 64, gapY: 24, margin: 40, sweeps: 4 };

/** A station's prerequisites: as written, else the previous station in the list (none for the first). */
export function needsOf(stations: readonly LayoutStation[], index: number): string[] {
  const s = stations[index];
  if (!s) return [];
  if (Array.isArray(s.needs)) return s.needs;
  const prev = stations[index - 1];
  return prev ? [prev.id] : [];
}

/** The `needs` edges among stations that exist, prerequisite first, without duplicates. */
export function forwardEdges(stations: readonly LayoutStation[]): LaidEdge[] {
  const ids = new Set(stations.map((s) => s.id));
  const seen = new Set<string>();
  const out: LaidEdge[] = [];
  stations.forEach((s, i) => {
    for (const n of needsOf(stations, i)) {
      const key = `${n}\u0000${s.id}`;
      if (!ids.has(n) || n === s.id || seen.has(key)) continue;
      seen.add(key);
      out.push({ from: n, to: s.id });
    }
  });
  return out;
}

/** The rework edges among stations that exist. */
export function backEdges(stations: readonly LayoutStation[]): BackEdge[] {
  const ids = new Set(stations.map((s) => s.id));
  const out: BackEdge[] = [];
  for (const s of stations) {
    for (const to of s.rework_to ?? []) if (ids.has(to)) out.push({ from: s.id, to, kind: "rework" });
    const auto = s.on_unmet?.rework_to;
    if (auto && ids.has(auto)) out.push({ from: s.id, to: auto, kind: "on_unmet" });
  }
  return out;
}

/** Each station's column: 0 for a root, else one past its latest prerequisite. A cycle counts as a root where it closes. */
export function layersOf(stations: readonly LayoutStation[], edges: readonly LaidEdge[]): Map<string, number> {
  const preds = new Map<string, string[]>(stations.map((s) => [s.id, []]));
  for (const e of edges) preds.get(e.to)?.push(e.from);
  const layer = new Map<string, number>();
  const visiting = new Set<string>();
  const visit = (id: string): number => {
    const known = layer.get(id);
    if (known !== undefined) return known;
    if (visiting.has(id)) return -1; // a cycle: the definition is invalid, draw it anyway
    visiting.add(id);
    let l = 0;
    for (const p of preds.get(id) ?? []) l = Math.max(l, visit(p) + 1);
    visiting.delete(id);
    layer.set(id, l);
    return l;
  };
  for (const s of stations) visit(s.id);
  return layer;
}

/** Centred position within a column, so columns of different heights line up on their middles. */
function centred(layers: readonly string[][]): Map<string, number> {
  const pos = new Map<string, number>();
  for (const ids of layers) ids.forEach((id, i) => pos.set(id, i - (ids.length - 1) / 2));
  return pos;
}

/** Pairs of edges between the same two columns whose ends are in opposite orders. */
export function countCrossings(layers: readonly string[][], edges: readonly LaidEdge[], layerOf: Map<string, number>): number {
  const pos = centred(layers);
  let n = 0;
  for (let i = 0; i < edges.length; i += 1) {
    for (let j = i + 1; j < edges.length; j += 1) {
      const a = edges[i]!;
      const b = edges[j]!;
      if (layerOf.get(a.from) !== layerOf.get(b.from) || layerOf.get(a.to) !== layerOf.get(b.to)) continue;
      const d1 = pos.get(a.from)! - pos.get(b.from)!;
      const d2 = pos.get(a.to)! - pos.get(b.to)!;
      if (d1 * d2 < 0) n += 1;
    }
  }
  return n;
}

function sweep(layers: string[][], neighbours: Map<string, string[]>, range: number[]): void {
  for (const l of range) {
    const ids = layers[l]!;
    // Positions as they stand when this column is reached.
    const pos = centred(layers);
    const bary = new Map<string, number>();
    for (const id of ids) {
      const ns = (neighbours.get(id) ?? []).filter((n) => pos.has(n));
      bary.set(id, ns.length ? ns.reduce((sum, n) => sum + pos.get(n)!, 0) / ns.length : pos.get(id)!);
    }
    // Stable: ties keep their order (the definition's, at first).
    layers[l] = ids
      .map((id, i) => ({ id, i }))
      .sort((a, b) => bary.get(a.id)! - bary.get(b.id)! || a.i - b.i)
      .map((x) => x.id);
  }
}

export function layoutFlow(stations: readonly LayoutStation[], options: LayoutOptions = {}): Layout {
  const o = { ...DEFAULTS, ...options };
  const edges = forwardEdges(stations);
  const back = backEdges(stations);
  const layerOf = layersOf(stations, edges);
  const depth = stations.length ? Math.max(...stations.map((s) => layerOf.get(s.id) ?? 0)) + 1 : 0;

  let layers: string[][] = Array.from({ length: depth }, () => []);
  for (const s of stations) layers[layerOf.get(s.id) ?? 0]!.push(s.id);

  const preds = new Map<string, string[]>();
  const succs = new Map<string, string[]>();
  for (const e of edges) {
    preds.set(e.to, [...(preds.get(e.to) ?? []), e.from]);
    succs.set(e.from, [...(succs.get(e.from) ?? []), e.to]);
  }
  const down = Array.from({ length: Math.max(0, depth - 1) }, (_, i) => i + 1);
  const up = Array.from({ length: Math.max(0, depth - 1) }, (_, i) => depth - 2 - i);

  let best = layers.map((l) => [...l]);
  let fewest = countCrossings(best, edges, layerOf);
  for (let i = 0; i < o.sweeps && fewest > 0; i += 1) {
    sweep(layers, preds, down);
    sweep(layers, succs, up);
    const c = countCrossings(layers, edges, layerOf);
    if (c < fewest) {
      fewest = c;
      best = layers.map((l) => [...l]);
    }
  }
  layers = best;

  const rows = layers.reduce((m, l) => Math.max(m, l.length), 0);
  // Room over the top row for the rework arcs.
  const top = o.margin + (back.length ? ARC_ROOM : 0);
  const stepX = o.nodeWidth + o.gapX;
  const stepY = o.nodeHeight + o.gapY;
  const nodes: LaidNode[] = [];
  layers.forEach((ids, layer) => {
    ids.forEach((id, order) => {
      const row = order - (ids.length - 1) / 2 + (rows - 1) / 2;
      nodes.push({ id, layer, order, x: o.margin + layer * stepX, y: top + row * stepY, width: o.nodeWidth, height: o.nodeHeight });
    });
  });
  // Nodes in the definition's order: what a reader tabs through.
  const index = new Map(stations.map((s, i) => [s.id, i]));
  nodes.sort((a, b) => index.get(a.id)! - index.get(b.id)!);
  const byId = Object.fromEntries(nodes.map((n) => [n.id, n]));
  return {
    nodes,
    byId,
    layers,
    edges,
    back,
    width: depth ? 2 * o.margin + depth * stepX - o.gapX : 2 * o.margin,
    height: top + o.margin + (rows ? rows * stepY - o.gapY : 0),
  };
}

const r = (n: number) => Math.round(n * 10) / 10;

/** A `needs` edge as an SVG path: out of the prerequisite's right side, into the station's left. */
export function edgePath(layout: Layout, e: LaidEdge): string {
  const a = layout.byId[e.from];
  const b = layout.byId[e.to];
  if (!a || !b) return "";
  const x1 = a.x + a.width;
  const y1 = a.y + a.height / 2;
  const x2 = b.x;
  const y2 = b.y + b.height / 2;
  const dx = Math.max(24, (x2 - x1) / 2);
  return `M${r(x1)},${r(y1)} C${r(x1 + dx)},${r(y1)} ${r(x2 - dx)},${r(y2)} ${r(x2)},${r(y2)}`;
}

/**
 * A rework edge as an SVG path: an arc over the top, out of the station that sends work back and
 * into the one it goes to. `lane` lifts arcs that would otherwise lie on one another; a station
 * that reworks itself gets a small loop.
 */
export function backEdgePath(layout: Layout, e: BackEdge, lane = 0): string {
  const a = layout.byId[e.from];
  const b = layout.byId[e.to];
  if (!a || !b) return "";
  if (a.id === b.id) {
    const x = a.x + a.width - 28;
    const y = a.y;
    const h = 18 + lane * 6;
    return `M${r(x)},${r(y)} C${r(x)},${r(y - h)} ${r(x + 24)},${r(y - h)} ${r(x + 24)},${r(y)}`;
  }
  const x1 = a.x + a.width / 2 - 8;
  const y1 = a.y;
  const x2 = b.x + b.width / 2 + 8;
  const y2 = b.y;
  const lift = Math.min(30 + Math.abs(x1 - x2) * 0.12, 120) + lane * 8;
  const top = Math.max(4, Math.min(y1, y2) - lift);
  return `M${r(x1)},${r(y1)} C${r(x1)},${r(top)} ${r(x2)},${r(top)} ${r(x2)},${r(y2)}`;
}

/** Lanes for back edges, so two arcs between the same pair of columns do not overlap. */
export function backLanes(layout: Layout): number[] {
  const used = new Map<string, number>();
  return layout.back.map((e) => {
    const a = layout.byId[e.from];
    const b = layout.byId[e.to];
    const key = `${a?.layer}:${b?.layer}`;
    const lane = used.get(key) ?? 0;
    used.set(key, lane + 1);
    return lane;
  });
}
