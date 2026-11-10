/**
 * The contribution registry (concept §5.6): shape checks at registration, newest-first claims, a
 * failing contribution that costs only itself, and a point list that is the generated catalog's.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { POINTS, Registry } from "../src/contrib";
import { askedTool, rowTitle, toolAsks } from "../src/components/blocks";

const tool = (name: string) => ({ key: 1, kind: "tool" as const, id: "t", name, status: "ok" as const });

describe("registry", () => {
  it("has exactly the points the generated catalog lists", () => {
    // Vitest runs in the package directory; under jsdom `import.meta.url` is not a file URL.
    const doc = readFileSync(join(process.cwd(), "../../../docs/contribution_points.md"), "utf8");
    // The shell's points: the first generated table (the second lists the extensions' own).
    const table = doc.slice(doc.indexOf("<!-- BEGIN GENERATED TABLE"), doc.indexOf("<!-- END GENERATED TABLE"));
    const catalog = [...table.matchAll(/^\| `([a-z.]+)` \|/gm)].map((m) => m[1]);
    expect([...POINTS].sort()).toEqual([...new Set(catalog)].sort());
  });

  it("refuses a missing id and a missing function, and says why; an undefined point waits, then goes to the diagnostics", () => {
    const r = new Registry();
    expect(r.register("no.such.point", { id: "x" }, "pkg")).toBe(true);
    expect(r.register("tool.row", { match: () => true, render: () => null } as never, "pkg")).toBe(false);
    expect(r.register("tool.row", { id: "x", match: () => true } as never, "pkg")).toBe(false);
    expect(r.list("no.such.point")).toEqual([]);
    r.settle();
    const reasons = r.refusals().map((x) => `${x.point}: ${x.reason}`);
    expect(reasons).toEqual([
      "tool.row: a contribution needs an `id`",
      "tool.row: missing render",
      "no.such.point: no one defines the point `no.such.point`",
    ]);
    expect(r.at("tool.row")).toHaveLength(0);
    // Settled once: defining it later brings nothing back, and is not told twice.
    r.definePoint("no.such.point", { kind: "register" }, "no");
    r.settle();
    expect(r.list("no.such.point")).toEqual([]);
    expect(r.refusals()).toHaveLength(3);
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

describe("a tool's one name: its row's, on the card and in the announcement", () => {
  const echo = { ...tool("app__acme__echo"), id: "e1" };
  const rows = [
    { match: (b: { name: string }) => b.name.startsWith("app__"), title: () => "Any app" },
    { match: (b: { name: string }) => b.name === "app__acme__echo", title: () => "Acme echo" },
  ];
  const prompt = (over: Record<string, unknown>) => ({ kind: "prompt", prompt_type: "permission", prompt_id: "p", ...over }) as never;

  it("is the newest row's that claims the call and says one, else the display name", () => {
    expect(rowTitle(echo, rows)).toBe("Acme echo");
    expect(rowTitle(echo, [rows[1]!, { match: () => true, title: () => null }])).toBe("app__acme__echo");
    expect(rowTitle(tool("Edit"), rows)).toBe("Update");
  });

  it("names the call a prompt is about by its row, and a prompt with no call by its tool", () => {
    const asks = toolAsks([echo], (b) => rowTitle(b, rows));
    expect(askedTool(prompt({ tool_name: "app__acme__echo", tool_use_id: "e1" }), asks)).toBe("Acme echo");
    expect(askedTool(prompt({ tool_name: "Edit" }), asks)).toBe("Update");
    expect(askedTool(prompt({}), asks)).toBe("");
  });
});

describe("contribution API 3", () => {
  const action = (id: string, at: ("header" | "menu" | "tool")[], match = () => true) => ({ id, at, label: () => id, match, run: () => undefined });
  const target = { kind: "session" as const, session: { instance: "main", sessionId: "s" }, space: "sessions" };

  it("takes a session action for the places its `at` names, and asks each by its match", () => {
    const r = new Registry();
    expect(r.register("session.action", action("both", ["header", "menu"]), "a")).toBe(true);
    expect(r.register("session.action", action("tool", ["tool"]), "a")).toBe(true);
    expect(r.register("session.action", action("never", ["header"], () => false), "a")).toBe(true);
    expect(r.register("session.action", action("nowhere", [] as never), "a")).toBe(false);
    expect(r.register("session.action", action("odd", ["footer" as never]), "a")).toBe(false);
    expect(r.actions("header", target).map((a) => a.id)).toEqual(["both"]);
    expect(r.actions("menu", target).map((a) => a.id)).toEqual(["both"]);
    expect(r.actions("tool", { ...target, kind: "tool", toolUseId: "t", tool: "Bash" }).map((a) => a.id)).toEqual(["tool"]);
  });

  it("drops a session action whose match throws, and keeps the rest", async () => {
    const r = new Registry();
    r.register("session.action", action("broken", ["header"], () => { throw new Error("no"); }), "a");
    r.register("session.action", action("fine", ["header"]), "a");
    expect(r.actions("header", target).map((a) => a.id)).toEqual(["fine"]);
    await Promise.resolve();
    expect(r.refusals()).toEqual([{ point: "session.action", owner: "a", id: "broken", reason: "no" }]);
  });

  it("refuses a space in contribution API 2's flat shape, naming the group", () => {
    const r = new Registry();
    expect(r.register("space", { id: "old", icon: "x", title: () => "Old", sidebar: () => null } as never, "pkg")).toBe(false);
    expect(r.register("space", { id: "new", icon: "x", title: () => "New", sidebar: { render: () => null }, center: { mode: "hidden" } }, "pkg")).toBe(true);
    expect(r.refusals()[0]!.reason).toContain("`sidebar` is a group");
  });

  it("lets an extension define a point of its own and others fill it, before or after it is defined", () => {
    const r = new Registry();
    r.register("system.console.tab", { id: "early" }, "flow");
    expect(r.list("system.console.tab")).toEqual([]);
    r.activate("system", () => {
      expect(r.definePoint("system.console.tab", { kind: "register" })).toBe(true);
    });
    r.register("system.console.tab", { id: "late" }, "other");
    r.settle();
    expect(r.list("system.console.tab").map((c) => c.id)).toEqual(["early", "late"]);
    expect(r.definitions()).toEqual([{ name: "system.console.tab", owner: "system", kind: "register" }]);
    expect(r.refusals()).toEqual([]);
  });

  it("refuses a point outside the definer's name, a kernel point's name, and one defined twice", () => {
    const r = new Registry();
    r.activate("system", () => {
      expect(r.definePoint("flow.tab", { kind: "render" })).toBe(false);
      expect(r.definePoint("system", { kind: "render" })).toBe(false);
      expect(r.definePoint("system.tab", { kind: "render" })).toBe(true);
      expect(r.definePoint("system.tab", { kind: "render" })).toBe(false);
    });
    expect(r.definePoint("space", { kind: "register" }, "space")).toBe(false);
    expect(r.refusals().map((x) => x.point)).toEqual(["flow.tab", "system", "system.tab", "space"]);
  });

  it("calls orElse when the point is never defined, inside the registering extension's batch", () => {
    const r = new Registry();
    let fell = 0;
    r.activate("flow", () => {
      r.register("system.console.tab", { id: "flows" }, "flow", {
        orElse: () => {
          fell += 1;
          r.register("space", { id: "flow", icon: "flow", title: () => "Flows" }, "flow");
        },
      });
    });
    expect(fell).toBe(0);
    r.settle();
    expect(fell).toBe(1);
    expect(r.at("space").map((c) => c.id)).toEqual(["flow"]);
    expect(r.refusals()).toEqual([]);
    // What orElse registered went in under the same batch: withdrawn with it.
    r.withdraw("flow");
    expect(r.at("space")).toEqual([]);
    r.settle();
    expect(fell).toBe(1);
  });

  it("does not call orElse when the point is defined", () => {
    const r = new Registry();
    let fell = 0;
    r.activate("system", () => r.definePoint("system.console.tab", { kind: "register" }));
    r.register("system.console.tab", { id: "flows" }, "flow", { orElse: () => (fell += 1) });
    r.settle();
    expect(fell).toBe(0);
    expect(r.list("system.console.tab").map((c) => c.id)).toEqual(["flows"]);
  });

  it("withdraws a whole batch: what the extension registered under any owner, its points, its icons", () => {
    const r = new Registry();
    r.activate("system", () => {
      r.definePoint("system.inspector.tab", { kind: "render" });
      r.register("space", { id: "system", icon: "cpu", title: () => "System" }, "system");
      r.register("session.action", action("open-in-system", ["header"]), "builtin");
      r.icons({ chip: '<path d="M0 0h1"/>' }, "system");
    });
    r.register("system.inspector.tab", { id: "from-flow" }, "flow");
    r.register("space", { id: "sessions", icon: "chat", title: () => "Sessions" }, "shell");
    r.settle();
    let told = 0;
    r.subscribe(() => (told += 1));
    expect(r.withdraw("system")).toBe(2);
    expect(told).toBe(1);
    expect(r.at("space").map((c) => c.id)).toEqual(["sessions"]);
    expect(r.at("session.action")).toEqual([]);
    expect(r.isDefined("system.inspector.tab")).toBe(false);
    expect(r.list("system.inspector.tab")).toEqual([]);
    expect(r.icon("system.chip")).toBeUndefined();
    // Defined again (the extension came back): what others put in is there again.
    r.activate("system", () => r.definePoint("system.inspector.tab", { kind: "render" }));
    expect(r.list("system.inspector.tab").map((c) => c.id)).toEqual(["from-flow"]);
  });
});
