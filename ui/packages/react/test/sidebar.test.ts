/**
 * Tab types (harnix-ext.md §22.7): which type claims an address, which registration of a kind is
 * in force, and what the registry refuses.
 */

import { describe, expect, it } from "vitest";
import { Registry } from "../src/contrib";
import { candidates, claim, effectiveTypes, guideEntries, parseResource, patternMatches, resourceAddress, typeOfKind, type SidebarTabType } from "../src/sidebar";

const type = (id: string, over: Partial<SidebarTabType> = {}): SidebarTabType => ({ id, kind: id, title: () => id, render: () => null, ...over });

describe("resource addresses", () => {
  it("are made and read back", () => {
    expect(resourceAddress("file", "/src/a.ts")).toBe("harnix-resource://file/src/a.ts");
    expect(parseResource("harnix-resource://file/src/a.ts")).toEqual({ type: "file", path: "src/a.ts" });
    expect(parseResource("harnix-resource://changes")).toEqual({ type: "changes", path: "" });
    expect(parseResource("https://x/y")).toBeNull();
    expect(parseResource("harnix-resource:///x")).toBeNull();
  });

  it("match a whole address with a pattern that has a colon, and a path at any depth without one", () => {
    const a = "harnix-resource://file/docs/Notes/README.md";
    expect(patternMatches("harnix-resource://file/**", a)).toBe(true);
    expect(patternMatches("harnix-resource://file/docs/**", a)).toBe(true);
    expect(patternMatches("harnix-resource://image/**", a)).toBe(false);
    expect(patternMatches("*.md", a)).toBe(true);
    expect(patternMatches("readme.*", a)).toBe(true);
    expect(patternMatches("notes/*.md", a)).toBe(true);
    expect(patternMatches("docs/*.md", a)).toBe(false);
    expect(patternMatches("docs/**/*.md", a)).toBe(true);
    expect(patternMatches("*.ts", a)).toBe(false);
    expect(patternMatches("*.md", "not-a-resource")).toBe(false);
  });
});

describe("which type is in force", () => {
  it("keeps one extension and one builtin per kind, the extension's in force until it goes", () => {
    const builtin = type("b.preview", { kind: "preview", priority: "builtin" });
    const ext = type("x.preview", { kind: "preview" });
    expect(effectiveTypes([builtin, ext]).map((t) => t.id)).toEqual(["x.preview"]);
    expect(effectiveTypes([ext, builtin]).map((t) => t.id)).toEqual(["x.preview"]);
    expect(effectiveTypes([builtin]).map((t) => t.id)).toEqual(["b.preview"]);
    expect(typeOfKind([builtin, ext], "preview")?.id).toBe("x.preview");
    expect(typeOfKind([builtin], "other")).toBeNull();
  });

  it("the registry refuses a second type at the same tier of a kind, and takes the other tier", () => {
    const r = new Registry();
    expect(r.register("sidebar.tab", type("a.files", { kind: "files", priority: "builtin" }), "a")).toBe(true);
    expect(r.register("sidebar.tab", type("b.files", { kind: "files", priority: "fallback" }), "b")).toBe(false);
    expect(r.register("sidebar.tab", type("c.files", { kind: "files" }), "c")).toBe(true);
    expect(r.register("sidebar.tab", type("d.files", { kind: "files" }), "d")).toBe(false);
    expect(r.refusals().map((x) => `${x.id}: ${x.reason}`)).toEqual([
      "b.files: `files` already has a builtin type (a.files, a)",
      "d.files: `files` already has a extension type (c.files, c)",
    ]);
    // The extension's goes with its owner: the builtin is in force again.
    expect(typeOfKind(r.at("sidebar.tab"), "files")?.id).toBe("c.files");
    r.withdraw("c");
    expect(typeOfKind(r.at("sidebar.tab"), "files")?.id).toBe("a.files");
  });

  it("the registry checks the shape: a kind, a known priority and scope, string patterns, guide entries", () => {
    const r = new Registry();
    expect(r.register("sidebar.tab", { id: "x", title: () => "", render: () => null } as never, "p")).toBe(false);
    expect(r.register("sidebar.tab", type("p1", { priority: "high" as never }), "p")).toBe(false);
    expect(r.register("sidebar.tab", type("s1", { scope: "global" as never }), "p")).toBe(false);
    expect(r.register("sidebar.tab", type("pat", { patterns: ["*.md", 3] as never }), "p")).toBe(false);
    expect(r.register("sidebar.tab", type("g", { guide: [{ id: "x" }] as never }), "p")).toBe(false);
    expect(r.register("sidebar.tab", { id: "no-render", kind: "k", title: () => "" } as never, "p")).toBe(false);
    expect(r.refusals().map((x) => x.reason)).toEqual([
      "a tab type needs a `kind`",
      'a tab type\'s priority is extension, builtin, fallback, not "high"',
      'a tab type\'s scope is session or space, not "global"',
      "a tab type's `patterns` are strings",
      "a tab type's `guide` entries are `{ id, title() }`",
      "missing render",
    ]);
  });
});

describe("claiming an address", () => {
  const text = type("b.text", { kind: "text", priority: "fallback", patterns: ["harnix-resource://file/**"] });
  const markdown = type("b.markdown", { kind: "markdown", priority: "builtin", patterns: ["*.md"] });
  const image = type("b.image", { kind: "image", priority: "builtin", patterns: ["*.png", "*.jpg"] });
  const docs = type("x.docs", { kind: "docs", patterns: ["docs/**"], canOpen: (a) => !a.endsWith("SKIP.md") });
  const types = [text, markdown, image, docs];

  it("orders the candidates by tier, then by the matched pattern's length, then by registration", () => {
    expect(candidates(types, "harnix-resource://file/docs/a.md").map((t) => t.kind)).toEqual(["docs", "markdown", "text"]);
    expect(candidates(types, "harnix-resource://file/src/a.md").map((t) => t.kind)).toEqual(["markdown", "text"]);
    expect(candidates(types, "harnix-resource://file/src/a.rs").map((t) => t.kind)).toEqual(["text"]);
    expect(candidates(types, "harnix-resource://other/x").map((t) => t.kind)).toEqual([]);
    // Longer pattern first within a tier: a more specific builtin beats a generic one.
    const specific = type("b.changelog", { kind: "changelog", priority: "builtin", patterns: ["CHANGELOG.md"] });
    expect(candidates([markdown, specific], "harnix-resource://file/CHANGELOG.md").map((t) => t.kind)).toEqual(["changelog", "markdown"]);
    // Same tier and length: registration order.
    const twin = type("b.twin", { kind: "twin", priority: "builtin", patterns: ["*.md"] });
    expect(candidates([twin, markdown], "harnix-resource://file/a.md").map((t) => t.kind)).toEqual(["twin", "markdown"]);
  });

  it("lets canOpen strike a type out, and claims by kind when asked to", () => {
    expect(claim(types, "harnix-resource://file/docs/SKIP.md")?.kind).toBe("markdown");
    expect(claim(types, "harnix-resource://file/docs/a.md")?.kind).toBe("docs");
    expect(claim(types, "harnix-resource://file/docs/a.md", "text")?.kind).toBe("text");
    expect(claim(types, "harnix-resource://file/docs/SKIP.md", "docs")).toBeNull();
    expect(claim(types, "harnix-resource://file/a.md", "nope")).toBeNull();
    expect(claim(types, "harnix-resource://nothing/claims/this")).toBeNull();
  });

  it("lists the guide entries of the types in force, by order", () => {
    const a = type("a", { guide: [{ id: "one", title: () => "One", order: 20 }] });
    const b = type("b", { guide: [{ id: "two", title: () => "Two", order: 10 }, { id: "three", title: () => "Three" }] });
    const shadowed = type("bb", { kind: "b", priority: "builtin", guide: [{ id: "hidden", title: () => "Hidden", order: 0 }] });
    expect(guideEntries([a, b, shadowed]).map((e) => `${e.kind}:${e.id}`)).toEqual(["b:two", "a:one", "b:three"]);
  });
});
