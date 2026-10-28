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

  it("takes a command for the @ menu, and refuses a trigger that is neither / nor @", () => {
    const r = new Registry();
    expect(r.register("command", { id: "docs", trigger: "@", complete: () => [] }, "pkg")).toBe(true);
    expect(r.register("command", { id: "slash", complete: () => [] }, "pkg")).toBe(true);
    expect(r.register("command", { id: "hash", trigger: "#", complete: () => [] } as never, "pkg")).toBe(false);
    expect(r.at("command").map((c) => [c.id, c.trigger ?? "/"])).toEqual([
      ["docs", "@"],
      ["slash", "/"],
    ]);
    expect(r.refusals().map((x) => x.reason)).toEqual(['a command\'s trigger is "/" or "@", not "#"']);
  });

  it("takes an application's icons under its own name, and refuses a script, a handler, a link", () => {
    const r = new Registry();
    r.icons({ ship: "<path d='M3 3h18'/><circle cx=\"12\" cy=\"12\" r=\"3\"/>" }, "testapp");
    expect(r.icon("testapp.ship")).toBe('<path d="M3 3h18"/><circle cx="12" cy="12" r="3"/>');
    expect(r.icon("ship")).toBeUndefined();
    r.icons(
      {
        bad: "<path d='M0 0'/><script>alert(1)</script>",
        loud: "<path d='M0 0' onload='alert(1)'/>",
        linked: "<use href='#x'/>",
        painted: "<path d='M0 0' fill='url(#g)'/>",
      },
      "testapp",
    );
    for (const name of ["bad", "loud", "linked", "painted"]) expect(r.icon(`testapp.${name}`)).toBeUndefined();
    expect(r.refusals().map((x) => `${x.id}: ${x.reason}`)).toEqual([
      "testapp.bad: <script> is not a shape an icon may have",
      "testapp.loud: onload is not an attribute an icon may have",
      "testapp.linked: <use> is not a shape an icon may have",
      "testapp.painted: fill may not refer to anything",
    ]);
  });
});
