/**
 * What runtime applications declare inline (`src/inline/`): the templates fill from what is drawn
 * and nothing else, and what the shell registers for an application follows `harnix.apps` — in
 * when it runs, out when it stops, afresh when it changes.
 */

import { describe, expect, it } from "vitest";
import { Registry } from "@harnix/react";

import { AppsWatch, toolNames } from "../src/inline/apps.ts";
import { fill, fillValue, inputOf, localized, say } from "../src/inline/template.ts";

describe("inline templates", () => {
  it("fill placeholders from the scope, as words", () => {
    const scope = { input: { text: "hi", n: 2, deep: { a: [1, 2] } }, result: "done" };
    expect(fill("Echo · {input.text} ({input.n})", scope)).toBe("Echo · hi (2)");
    expect(fill("{input.deep.a} {input.missing}|{result}", scope)).toBe("[1,2] |done");
    // Markup is text: nothing here is HTML.
    expect(fill("{input.text}", { input: { text: "<b>x</b>" } })).toBe("<b>x</b>");
  });

  it("take the reader's locale, then English, then what there is", () => {
    expect(localized("plain")).toBe("plain");
    expect(localized({ en: "Hello", "zh-CN": "你好" })).toBe("Hello");
    expect(localized({ "zh-CN": "你好" })).toBe("你好");
    expect(localized(undefined)).toBe("");
    expect(say({ en: "Now {result.counter}" }, { result: { counter: 3 } })).toBe("Now 3");
  });

  it("keep a value's type where a param is one placeholder", () => {
    const scope = { payload: { title: "T", count: 4, tags: ["a"] } };
    expect(fillValue({ text: "{payload.title}", n: "{payload.count}", tags: "{payload.tags}", s: "x{payload.count}", k: 1 }, scope)).toEqual({
      text: "T",
      n: 4,
      tags: ["a"],
      s: "x4",
      k: 1,
    });
    expect(inputOf('{"a":1}')).toEqual({ a: 1 });
    expect(inputOf("not json")).toBe("not json");
  });
});

describe("the applications the shell draws for", () => {
  it("are the running ones, with their declarations", () => {
    const watch = new AppsWatch();
    let seen = 0;
    watch.subscribe(() => (seen += 1));
    watch.take({
      apps: [
        { id: "acme.notes", title: "Notes", enabled: true, trust: "store", runtime: { state: "running", inline: { command: [{ trigger: "/", name: "notes" }] } } },
        { id: "acme.off", enabled: false, runtime: { state: "stopped" } },
      ],
    });
    expect(seen).toBe(1);
    expect(watch.running().map((a) => a.id)).toEqual(["acme.notes"]);
    expect(watch.get("acme.off")).toMatchObject({ title: "acme.off", trust: "unsigned", running: false });
    expect(watch.get("acme.notes")?.inline.command?.[0]?.name).toBe("notes");
    watch.setMarked(false);
    expect(watch.marked()).toBe(false);
    watch.setMarked(true);
    expect(watch.marked()).toBe(true);
  });

  it("are known by their tools' names in this process and in a spawned daemon", () => {
    expect(toolNames("example.showcase", "echo")).toEqual(["app__example_showcase__echo", "mcp__app-example_showcase__echo"]);
  });

  it("have what the shell registered for them taken back", () => {
    const r = new Registry();
    r.register("command", { id: "inline.command./notes", complete: () => [] }, "acme.notes");
    r.register("command", { id: "inline.command./notes", complete: () => [] }, "acme.other");
    expect(r.unregister("command", "inline.command./notes", "acme.notes")).toBe(true);
    expect(r.unregister("command", "inline.command./notes", "acme.notes")).toBe(false);
    expect(r.at("command").map((c) => c.owner)).toEqual(["acme.other"]);
  });
});
