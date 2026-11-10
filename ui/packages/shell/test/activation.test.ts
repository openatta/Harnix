/**
 * Contribution API 3 on the page: the compiled-in UI packages activated by what hello says is
 * enabled, registered late and withdrawn as a whole batch as `harnix.ui` changes; a point nobody
 * defines going to `orElse` or the diagnostics; the frames following `harnix.ui` the same way; and
 * what the host API gained — events, navigation, the extensions and hello's defaults, a frame's
 * subscription asked as its extension.
 */

import { describe, expect, it } from "vitest";
import type { ExtensionEntry, HarnixClient } from "@harnix/core";
import { Registry, type HostApi } from "@harnix/react";

import { activate, type UiPackage } from "../src/activation";
import { App } from "../src/app";
import { hostFor } from "../src/host";
import { followSurfaces } from "../src/surfaces/register";
import { topicsOf } from "../src/topics";
import { ScriptedClient, space, tick } from "./scripted";

const ext = (id: string, enabled: boolean, bundle?: string): ExtensionEntry => ({ id, trust: "origin", enabled, ...(bundle ? { bundle } : {}) });

function setup() {
  localStorage.clear();
  const client = new ScriptedClient();
  const app = new App(client as unknown as HarnixClient, new Registry());
  return { app, client, host: hostFor(app) };
}

/** The first-party shape: a system package defining a point, a flow package filling it or else
 *  a space of its own, and the application's own package. */
function packages(): UiPackage[] {
  return [
    {
      extension: "system",
      register(r) {
        r.definePoint("system.console.tab", { kind: "register" });
        r.register("space", space("system", { place: "bottom" }), "system");
        r.register("session.action", { id: "system.open", at: ["header"], label: () => "Open", match: () => true, run: () => undefined }, "system");
      },
    },
    {
      extension: "flow",
      register(r) {
        r.register("system.console.tab", { id: "flows" }, "flow", {
          orElse: () => r.register("space", space("flow"), "flow"),
        });
      },
    },
    { register: (r) => r.register("space", space("notes"), "hello-app") },
  ];
}

describe("activation by hello.extensions", () => {
  it("registers, before anything is drawn, the packages whose extension is enabled, and the application's own", () => {
    const { app, host } = setup();
    activate(app, host, packages(), [ext("system", true), ext("flow", true)]);
    app.registry.settle();
    expect(app.registry.at("space").map((c) => c.id)).toEqual(["system", "notes"]);
    expect(app.registry.list("system.console.tab").map((c) => c.id)).toEqual(["flows"]);
    expect(app.registry.refusals()).toEqual([]);
  });

  it("leaves out a disabled extension's package; a point it would have defined sends its fillers to orElse", () => {
    const { app, host } = setup();
    activate(app, host, packages(), [ext("system", false), ext("flow", true)]);
    app.registry.settle();
    // orElse runs once every registration is in: after the application's own.
    expect(app.registry.at("space").map((c) => c.id)).toEqual(["notes", "flow"]);
    expect(app.registry.at("session.action")).toEqual([]);
  });

  it("goes by an extension's bundle when it names one", () => {
    const { app, host } = setup();
    activate(app, host, packages(), [ext("acme.system", true, "system")]);
    expect(app.registry.at("space").map((c) => c.id)).toContain("system");
  });

  it("an undefined point with no orElse goes to the diagnostics, not the page", () => {
    const { app, host } = setup();
    activate(app, host, [{ extension: "w", register: (r) => r.register("nobody.point", { id: "x" }, "w") }], [ext("w", true)]);
    app.registry.settle();
    expect(app.registry.refusals()).toEqual([{ point: "nobody.point", owner: "w", id: "x", reason: "no one defines the point `nobody.point`" }]);
  });
});

describe("following harnix.ui", () => {
  it("registers a package late when its extension comes up, and withdraws the whole batch when it goes", () => {
    const { app, host } = setup();
    const all = packages();
    activate(app, host, all, [ext("system", false), ext("flow", true)]);
    app.registry.settle();
    expect(app.registry.at("space").map((c) => c.id)).toEqual(["notes", "flow"]);

    // system comes up: its package registers; the point is there for whoever registers next.
    expect(activate(app, host, all, [ext("system", true), ext("flow", true)])).toBe(true);
    app.registry.settle();
    expect(app.registry.at("space").map((c) => c.id)).toEqual(["notes", "flow", "system"]);
    expect(app.registry.isDefined("system.console.tab")).toBe(true);

    // and goes down: everything its register did goes, under whatever owner it named.
    expect(activate(app, host, all, [ext("system", false), ext("flow", true)])).toBe(true);
    app.registry.settle();
    expect(app.registry.at("space").map((c) => c.id)).toEqual(["notes", "flow"]);
    expect(app.registry.at("session.action")).toEqual([]);
    expect(app.registry.isDefined("system.console.tab")).toBe(false);
    // The same again changes nothing, and the application's own package registered once.
    expect(activate(app, host, all, [ext("system", false), ext("flow", true)])).toBe(false);
    expect(app.registry.at("space").filter((c) => c.id === "notes")).toHaveLength(1);
  });

  it("puts an application's frames on the page as harnix.ui lists them, and takes them all back when it goes", async () => {
    const { app } = setup();
    const entry = {
      plugin: "acme.notes",
      point: "space",
      api_version: 3,
      frame: "/apps/acme.notes/ui/a.html",
      surface: { id: "space", space: "acme.notes", title: { en: "Notes" }, center: "hidden" },
      interface: { hub: [], links: false, trust: "store", title: "Notes", spaces: ["acme.notes"] },
    };
    await followSurfaces(app, { contributions: [entry] });
    expect(app.registry.at("space").map((c) => [c.id, c.owner, c.center?.mode])).toEqual([["acme.notes", "acme.notes", "hidden"]]);
    await followSurfaces(app, { contributions: [entry] });
    expect(app.registry.at("space")).toHaveLength(1);
    await followSurfaces(app, { contributions: [] });
    expect(app.registry.at("space")).toEqual([]);
  });
});

describe("the host API", () => {
  it("tells the extensions and hello's defaults, and follows the extensions as they change", async () => {
    const { app, client, host } = setup();
    expect(host.defaults).toEqual({ instance: "system", space: null });
    app.set({ hello: { defaults: { instance: "main", space: "work" } } as never, extensions: [ext("system", true)] });
    expect(host.defaults).toEqual({ instance: "main", space: "work" });
    expect(host.extensions.list().map((e) => e.id)).toEqual(["system"]);
    let told = 0;
    host.extensions.onChange(() => (told += 1));
    client.answer = (method, params) => (method === "harnix.topics.subscribe" ? { topic: params.topic, value: { extensions: [ext("system", false)], contributions: [] } } : {});
    (app as unknown as { followUi(): void }).followUi();
    await tick();
    await tick();
    expect(host.extensions.list()).toEqual([ext("system", false)]);
    expect(told).toBe(1);
  });

  it("hands a session's events over parsed, with the engine's own and the session's trust", () => {
    const { client, host } = setup();
    const seen: unknown[] = [];
    const off = host.events.onSessionEvent({ instance: "main", sessionId: "s" }, (e) => seen.push(e));
    const push = (params: Record<string, unknown>) => client.push("harnix.event", params);
    push({ instance: "main", session_id: "s", seq: 1, turn_id: "t", event: { kind: "text_delta", text: "hi" } });
    push({ instance: "main", session_id: "other", seq: 1, turn_id: "t", event: { kind: "text_delta", text: "no" } });
    push({ instance: "main", session_id: "s", seq: 2, turn_id: "t", event: { kind: "new_kind" }, trust: "extension" });
    off();
    push({ instance: "main", session_id: "s", seq: 3, turn_id: "t", event: { kind: "text_delta", text: "late" } });
    expect(seen).toEqual([
      { raw: { kind: "text_delta", text: "hi" }, parsed: { kind: "text", text: "hi" }, trust: "product" },
      { raw: { kind: "new_kind" }, parsed: { kind: "other", raw: "new_kind" }, trust: "extension" },
    ]);
  });

  it("navigates to a space and hands its accept the selection", async () => {
    const { app, host } = setup();
    const accepted: unknown[] = [];
    app.registry.register("space", space("sessions"), "test");
    app.registry.register("space", space("inspect", { accept: (s) => accepted.push(s) }), "test");
    await (host as HostApi).navigate.open({ space: "inspect", selection: { tool: "t1" } });
    expect(app.snapshot.space).toBe("inspect");
    expect(accepted).toEqual([{ tool: "t1" }]);
    await expect(host.navigate.open({ space: "nowhere" })).rejects.toThrow();
  });

  it("asks a frame's subscription as its extension, and keeps one subscription on the connection", async () => {
    const { client } = setup();
    client.answer = (method, params) => (method === "harnix.topics.subscribe" ? { topic: params.topic, value: 1 } : {});
    const topics = topicsOf(client as unknown as HarnixClient);
    const off = await topics.subscribeAs("acme.notes", "harnix.spaces", {}, () => undefined);
    const subs = client.calls.filter((c) => c.method === "harnix.topics.subscribe");
    expect(subs.length).toBeGreaterThan(0);
    expect(subs.every((c) => c.params.$ext === "acme.notes")).toBe(true);
    expect(topics.count("harnix.spaces", {})).toBe(1);
    client.answer = () => {
      throw { code: -31000, message: "not in the extension interface's `hub`" };
    };
    await expect(topics.subscribeAs("acme.notes", "harnix.instances", {}, () => undefined)).rejects.toMatchObject({ code: -31000 });
    expect(topics.count("harnix.instances", {})).toBe(0);
    off();
  });
});
