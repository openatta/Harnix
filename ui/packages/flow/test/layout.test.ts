/**
 * The graph layout: columns by longest path over `needs` (the previous station by default), rows
 * ordered to cut crossings, rework edges kept apart as back edges.
 */

import { describe, expect, it } from "vitest";

import { backEdgePath, countCrossings, edgePath, forwardEdges, layersOf, layoutFlow, needsOf, type LayoutStation } from "../src/layout";

/** The issue's demo flow (xops-01 §3), as far as layout reads it. */
const DEMO: LayoutStation[] = [
  { id: "collect" },
  { id: "analyze" },
  { id: "lint", needs: ["collect"] },
  { id: "self-check", needs: ["analyze"], on_unmet: { rework_to: "analyze", max: 2 } },
  { id: "review", needs: ["self-check", "lint"], rework_to: ["analyze", "lint"] },
  { id: "report" },
];

const column = (l: ReturnType<typeof layoutFlow>, id: string) => l.byId[id]!.layer;

describe("needs", () => {
  it("defaults to the previous station, none for the first; an explicit list wins, empty is a root", () => {
    const s: LayoutStation[] = [{ id: "a" }, { id: "b" }, { id: "c", needs: [] }, { id: "d", needs: ["a", "b"] }];
    expect(needsOf(s, 0)).toEqual([]);
    expect(needsOf(s, 1)).toEqual(["a"]);
    expect(needsOf(s, 2)).toEqual([]);
    expect(needsOf(s, 3)).toEqual(["a", "b"]);
  });

  it("drops edges to stations that do not exist, to itself, and repeats", () => {
    const s: LayoutStation[] = [{ id: "a" }, { id: "b", needs: ["a", "a", "ghost", "b"] }];
    expect(forwardEdges(s)).toEqual([{ from: "a", to: "b" }]);
  });
});

describe("layers", () => {
  it("puts a station one column past its latest prerequisite", () => {
    const l = layoutFlow(DEMO);
    expect(column(l, "collect")).toBe(0);
    expect(column(l, "analyze")).toBe(1);
    expect(column(l, "lint")).toBe(1);
    expect(column(l, "self-check")).toBe(2);
    expect(column(l, "review")).toBe(3); // after self-check (2), not lint (1)
    expect(column(l, "report")).toBe(4);
    expect(l.layers.map((c) => c.length)).toEqual([1, 2, 1, 1, 1]);
  });

  it("keeps rework edges out of layering and returns them as back edges", () => {
    const l = layoutFlow(DEMO);
    expect(l.edges).not.toContainEqual(expect.objectContaining({ from: "review", to: "analyze" }));
    expect(l.back).toEqual([
      { from: "self-check", to: "analyze", kind: "on_unmet" },
      { from: "review", to: "analyze", kind: "rework" },
      { from: "review", to: "lint", kind: "rework" },
    ]);
  });

  it("survives a cycle (an invalid definition is still drawn)", () => {
    const s: LayoutStation[] = [{ id: "a", needs: ["b"] }, { id: "b", needs: ["a"] }];
    const layer = layersOf(s, forwardEdges(s));
    expect([...layer.keys()].sort()).toEqual(["a", "b"]);
    const l = layoutFlow(s);
    expect(l.nodes).toHaveLength(2);
  });

  it("lays out nothing for no stations", () => {
    const l = layoutFlow([]);
    expect(l.nodes).toEqual([]);
    expect(l.width).toBeGreaterThan(0);
  });
});

describe("ordering", () => {
  it("uncrosses a crossed pair with a barycenter sweep", () => {
    // a→d and b→c, with c listed before d: crossed until the sweep swaps c and d.
    const s: LayoutStation[] = [
      { id: "a", needs: [] },
      { id: "b", needs: [] },
      { id: "c", needs: ["b"] },
      { id: "d", needs: ["a"] },
    ];
    const edges = forwardEdges(s);
    const layer = layersOf(s, edges);
    expect(countCrossings([["a", "b"], ["c", "d"]], edges, layer)).toBe(1);
    const l = layoutFlow(s);
    expect(l.layers[1]).toEqual(["d", "c"]);
    expect(countCrossings(l.layers, edges, layer)).toBe(0);
  });

  it("keeps the definition's order when nothing crosses", () => {
    const s: LayoutStation[] = [{ id: "root" }, { id: "x", needs: ["root"] }, { id: "y", needs: ["root"] }, { id: "z", needs: ["root"] }];
    expect(layoutFlow(s).layers[1]).toEqual(["x", "y", "z"]);
  });

  it("returns nodes in the definition's order (the tab order)", () => {
    expect(layoutFlow(DEMO).nodes.map((n) => n.id)).toEqual(DEMO.map((s) => s.id));
  });
});

describe("geometry", () => {
  it("places columns left to right and centres short columns", () => {
    const l = layoutFlow(DEMO, { nodeWidth: 100, nodeHeight: 50, gapX: 20, gapY: 10, margin: 10 });
    const c = l.byId.collect!;
    const a = l.byId.analyze!;
    const lint = l.byId.lint!;
    expect(a.x - c.x).toBe(120);
    expect(Math.abs(lint.y - a.y)).toBe(60);
    // collect sits between analyze and lint.
    expect(c.y).toBe((a.y + lint.y) / 2);
    // every node inside the drawing
    for (const n of l.nodes) {
      expect(n.x + n.width).toBeLessThanOrEqual(l.width);
      expect(n.y + n.height).toBeLessThanOrEqual(l.height);
      expect(n.y).toBeGreaterThan(0);
    }
  });

  it("draws forward edges right side to left side, back edges as arcs that stay in the drawing", () => {
    const l = layoutFlow(DEMO);
    const path = edgePath(l, { from: "collect", to: "analyze" });
    const c = l.byId.collect!;
    expect(path.startsWith(`M${c.x + c.width},${c.y + c.height / 2}`)).toBe(true);
    for (const e of l.back) {
      const d = backEdgePath(l, e);
      const ys = [...d.matchAll(/,(-?[\d.]+)/g)].map((m) => Number(m[1]));
      expect(Math.min(...ys)).toBeGreaterThanOrEqual(0);
    }
    expect(backEdgePath(l, { from: "review", to: "review", kind: "rework" })).toMatch(/^M/);
    expect(edgePath(l, { from: "ghost", to: "review" })).toBe("");
  });
});
