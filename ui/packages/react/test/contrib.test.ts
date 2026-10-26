/**
 * The contribution registry (concept §5.1): shape checks at registration, newest-first claims, a
 * failing contribution that costs only itself, and a point list that is the generated catalog's.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { POINTS, Registry } from "../src/contrib";

const tool = (name: string) => ({ key: 1, kind: "tool" as const, id: "t", name, status: "ok" as const });

describe("registry", () => {
  it("has exactly the points the generated catalog lists", () => {
    // Vitest runs in the package directory; under jsdom `import.meta.url` is not a file URL.
    const doc = readFileSync(join(process.cwd(), "../../../docs/contribution_points.md"), "utf8");
    const catalog = [...doc.matchAll(/^\| `([a-z.]+)` \|/gm)].map((m) => m[1]);
    expect([...POINTS].sort()).toEqual([...new Set(catalog)].sort());
  });

  it("refuses an unknown point, a missing id and a missing function, and says why", () => {
    const r = new Registry();
    expect(r.register("no.such.point", { id: "x" }, "pkg")).toBe(false);
    expect(r.register("tool.row", { match: () => true, render: () => null }, "pkg")).toBe(false);
    expect(r.register("tool.row", { id: "x", match: () => true }, "pkg")).toBe(false);
    const reasons = r.refusals().map((x) => `${x.point}: ${x.reason}`);
    expect(reasons).toEqual([
      "no.such.point: there is no contribution point `no.such.point`",
      "tool.row: a contribution needs an `id`",
      "tool.row: missing render",
    ]);
    expect(r.at("tool.row")).toHaveLength(0);
  });

  it("lets the later registration win the claim, for what it matches and nothing else", () => {
    const r = new Registry();
    r.register("tool.row", { id: "builtin.tool", match: () => true, render: () => "builtin" }, "builtin");
    r.register("tool.row", { id: "bash", match: (b) => b.name === "Bash", render: () => "pkg" }, "pkg");
    expect(r.claim("tool.row", tool("Bash"))?.owner).toBe("pkg");
    expect(r.claim("tool.row", tool("Read"))?.owner).toBe("builtin");
  });

  it("skips a match that throws, records it, and keeps the point working", () => {
    const r = new Registry();
    r.register("tool.row", { id: "builtin.tool", match: () => true, render: () => "builtin" }, "builtin");
    r.register("tool.row", { id: "broken", match: () => { throw new Error("boom"); }, render: () => "x" }, "pkg");
    expect(r.claim("tool.row", tool("Bash"))?.id).toBe("builtin.tool");
    expect(r.refusals()).toEqual([{ point: "tool.row", owner: "pkg", id: "broken", reason: "boom" }]);
  });

  it("groups the sidebar by the last grouping registered, not a merge", () => {
    const r = new Registry();
    r.register("sidebar.group", { id: "a", group: () => [] }, "builtin");
    r.register("sidebar.group", { id: "b", group: () => [] }, "pkg");
    expect(r.grouping()?.id).toBe("b");
  });

  it("tells subscribers when something registers", () => {
    const r = new Registry();
    let told = 0;
    r.subscribe(() => (told += 1));
    r.register("command", { id: "c", complete: () => [] }, "builtin");
    expect(told).toBe(1);
    expect(r.getVersion()).toBe(1);
  });
});
