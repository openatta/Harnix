/**
 * Where the flows go: a tab of `system.console.tab` when that point is defined — a flow clicked
 * there a space shown inside the console's space — and the flow space of its own when nobody
 * defines it. Decided by the point, not by which packages run.
 */

import { describe, expect, it } from "vitest";
import { Registry, type HostApi } from "@harnix/react";

import { extension, register } from "../src/register";

const off = () => () => undefined;
const host = {
  call: async () => null,
  subscribe: off,
  session: { current: () => null, onChange: off, open: async () => undefined },
  spaces: { current: () => "flow", switch: async () => undefined },
  details: { open: () => undefined, tab: () => "", onTab: off },
  composer: { insert: () => undefined },
  redraw: () => undefined,
  t: (k: string) => k,
} as unknown as HostApi;

describe("the flow package", () => {
  it("is the flow extension's interface", () => {
    expect(extension).toBe("flow");
  });

  it("with a console defined: its tab, and the flow space inside the console's space", () => {
    const r = new Registry();
    r.activate("system", () => r.definePoint("system.console.tab", { kind: "register" }));
    r.activate("flow", () => register(r, host));
    r.settle();
    expect(r.list<{ id: string; about(): string }>("system.console.tab").map((c) => [c.id, c.owner])).toEqual([["flow.flows", "flow"]]);
    const flow = r.at("space").find((c) => c.id === "flow");
    expect(flow?.within).toBe("system");
    expect((flow?.details?.tabs as { id: string }[]).map((x) => x.id)).toEqual(["flow.reference"]);
    expect(r.refusals()).toEqual([]);
  });

  it("with nobody defining it: the flow space of its own, and nothing left in the diagnostics", () => {
    const r = new Registry();
    r.activate("flow", () => register(r, host));
    expect(r.at("space").some((c) => c.id === "flow")).toBe(false);
    r.settle();
    const flow = r.at("space").find((c) => c.id === "flow");
    expect(flow?.within).toBeUndefined();
    expect((flow?.details?.tabs as { id: string }[]).map((x) => x.id)).toEqual(["flow.flows", "flow.runs", "flow.reference"]);
    expect(r.refusals()).toEqual([]);
  });

  it("a section of the 用量 tab where the observability package defines it, and none, quietly, where not", () => {
    const r = new Registry();
    r.activate("observability", () => r.definePoint("observability.usage.section", { kind: "render" }));
    r.activate("flow", () => register(r, host));
    r.settle();
    expect(r.list<{ id: string }>("observability.usage.section").map((c) => [c.id, c.owner])).toEqual([["flow.usage", "flow"]]);
    const alone = new Registry();
    alone.activate("flow", () => register(alone, host));
    alone.settle();
    expect(alone.list("observability.usage.section")).toEqual([]);
    expect(alone.refusals()).toEqual([]);
  });

  it("goes with its extension: everything it registered, as one batch", () => {
    const r = new Registry();
    r.activate("flow", () => register(r, host));
    r.settle();
    expect(r.at("space").some((c) => c.id === "flow")).toBe(true);
    r.withdraw("flow");
    expect(r.at("space").some((c) => c.id === "flow")).toBe(false);
  });
});
