/**
 * The host a contribution is handed (`useNest()`), against a scripted client: the current session's
 * notifications, the right column's tabs, bringing a space to the front, and topics counted across
 * every subscriber on the page.
 */

import { beforeEach, describe, expect, it } from "vitest";
import type { HarnixClient } from "@harnix/core";
import { Registry, type SpaceContribution } from "@harnix/react";

import { App } from "../src/app";
import { hostFor } from "../src/host";

type Listener = (method: string, params: Record<string, unknown>) => void;

/** A client that is the server: it answers calls with `answer` and records them. */
class ScriptedClient {
  status: "open" | "closed" | "connecting" = "open";
  calls: { method: string; params: Record<string, unknown> }[] = [];
  private notify = new Set<Listener>();
  private statusListeners = new Set<(s: string) => void>();
  answer: (method: string, params: Record<string, unknown>) => unknown = (method, params) =>
    method === "harnix.subscribe" ? { topic: params.topic, value: "now" } : {};

  call(method: string, params: Record<string, unknown> = {}): Promise<unknown> {
    this.calls.push({ method, params });
    try {
      return Promise.resolve(this.answer(method, params));
    } catch (e) {
      return Promise.reject(e);
    }
  }
  onNotify(fn: Listener): () => void {
    this.notify.add(fn);
    return () => this.notify.delete(fn);
  }
  onStatus(fn: (s: string) => void): () => void {
    this.statusListeners.add(fn);
    return () => this.statusListeners.delete(fn);
  }
  push(method: string, params: Record<string, unknown>): void {
    for (const fn of [...this.notify]) fn(method, params);
  }
  setStatus(status: "open" | "closed" | "connecting"): void {
    this.status = status;
    for (const fn of [...this.statusListeners]) fn(status);
  }
  named(method: string) {
    return this.calls.filter((c) => c.method === method);
  }
}

const tick = () => new Promise((r) => setTimeout(r, 0));

/** Await what waits for a draw, drawing as a shell would. */
async function drawn<T>(app: App, waiting: Promise<T>): Promise<T> {
  let done = false;
  void waiting.finally(() => (done = true)).catch(() => undefined);
  while (!done) {
    await tick();
    app.drew();
  }
  return waiting;
}

function space(id: string, extra: Partial<SpaceContribution> = {}): SpaceContribution {
  return { id, icon: "chat", title: () => id, ...extra };
}

let client: ScriptedClient;
let app: App;

beforeEach(() => {
  localStorage.clear();
  client = new ScriptedClient();
  app = new App(client as unknown as HarnixClient, new Registry());
  app.registry.register("space", space("sessions"), "test");
  app.registry.register("space", space("b"), "test");
});

describe("session.onChange", () => {
  it("says when the front space's current session changes, once per change", async () => {
    const host = hostFor(app);
    const seen: unknown[] = [];
    const off = host.session.onChange((current) => seen.push(current));
    const a = { instance: "system", sessionId: "a" };
    app.open(a);
    app.open({ ...a });
    app.open(null);
    expect(seen).toEqual([a, null]);
    // Another space, with a current session of its own.
    const b = { instance: "system", sessionId: "b1" };
    app.open(b, "b");
    expect(seen).toEqual([a, null]);
    await app.switchSpace("b");
    expect(seen).toEqual([a, null, b]);
    off();
    app.open(null);
    expect(seen).toHaveLength(3);
  });
});

describe("details", () => {
  it("opens at a tab, keeps the tab a space was asked for, and says when the tab changes", async () => {
    let tabs = [{ id: "t1", title: () => "one", render: () => "1" }];
    app.registry.register("space", space("tabbed", { tabs: () => tabs }), "test");
    await app.switchSpace("tabbed");
    const host = hostFor(app);
    const changes: string[] = [];
    host.details.onTab((tab) => changes.push(tab));
    expect(host.details.tab()).toBe("t1");
    // A tab the space gives only from now on.
    tabs = [...tabs, { id: "t2", title: () => "two", render: () => "2" }];
    host.details.open("t2");
    expect(host.details.isOpen()).toBe(true);
    expect(host.details.tab()).toBe("t2");
    app.selectTab("t1");
    expect(changes).toEqual(["t2", "t1"]);
    host.details.open("t2");
    host.details.close();
    expect(host.details.isOpen()).toBe(false);
    await app.switchSpace("b");
    await app.switchSpace("tabbed");
    expect(host.details.tab()).toBe("t2");
    expect(host.details.isOpen()).toBe(false);
  });

  it("a clicked tab still goes back to the first when the space is entered again", async () => {
    const tabs = [
      { id: "t1", title: () => "one", render: () => "1" },
      { id: "t2", title: () => "two", render: () => "2" },
    ];
    app.registry.register("space", space("tabbed", { tabs }), "test");
    await app.switchSpace("tabbed");
    app.selectTab("t2");
    await app.switchSpace("b");
    await app.switchSpace("tabbed");
    expect(app.shownTab()).toBe("t1");
  });

  it("a tab that is not there only opens the column, and is noted", async () => {
    app.registry.register("space", space("tabbed", { tabs: [{ id: "t1", title: () => "one", render: () => "1" }] }), "test");
    await app.switchSpace("tabbed");
    hostFor(app).details.open("nope");
    expect(app.detailsOpen()).toBe(true);
    expect(app.shownTab()).toBe("t1");
    await tick();
    expect(app.registry.refusals().some((r) => r.id === "nope")).toBe(true);
  });
});

describe("spaces", () => {
  it("switch brings a space to the front and refuses one that is not there", async () => {
    const host = hostFor(app);
    await drawn(app, host.spaces.switch("b"));
    expect(host.spaces.current()).toBe("b");
    await expect(host.spaces.switch("nope")).rejects.toThrow(/nope/);
  });

  it("session.open with switchSpace opens a session where the hub has it", async () => {
    client.answer = (method) =>
      method === "harnix.sessions" ? { sessions: [{ session_id: "s-b", space: "b" }] } : {};
    const host = hostFor(app);
    const before = app.snapshot.currentBySpace.sessions ?? null;
    await drawn(app, host.session.open({ instance: "system", sessionId: "s-b" }, { switchSpace: true }) as Promise<void>);
    expect(app.snapshot.space).toBe("b");
    expect(app.snapshot.current).toEqual({ instance: "system", sessionId: "s-b" });
    expect(app.snapshot.currentBySpace.sessions ?? null).toEqual(before);
    await expect(host.session.open({ instance: "system", sessionId: "gone" }, { switchSpace: true })).rejects.toThrow(/gone/);
  });
});

describe("subscribe", () => {
  it("counts subscribers of one topic: one subscribe, one unsubscribe, the rest keep hearing", async () => {
    const one = hostFor(app, "one");
    const two = hostFor(app, "two");
    const heardOne: unknown[] = [];
    const heardTwo: unknown[] = [];
    const offOne = one.subscribe("t.x", { a: 1, b: 2 }, (v) => heardOne.push(v));
    await tick();
    const offTwo = two.subscribe("t.x", { b: 2, a: 1 }, (v) => heardTwo.push(v));
    expect(client.named("harnix.subscribe")).toHaveLength(1);
    // The later one has the value held, without waiting for an answer of its own.
    await tick();
    expect(heardTwo).toEqual(["now"]);
    offOne();
    expect(client.named("harnix.unsubscribe")).toHaveLength(0);
    client.push("harnix.topic", { topic: "t.x", params: { a: 1, b: 2 }, value: "next" });
    expect(heardTwo).toEqual(["now", "next"]);
    expect(heardOne).toEqual(["now"]);
    offTwo();
    expect(client.named("harnix.unsubscribe")).toHaveLength(1);
  });

  it("subscribes again after a reconnect", async () => {
    const off = hostFor(app).subscribe("t.y", {}, () => undefined);
    await tick();
    client.setStatus("closed");
    client.setStatus("open");
    expect(client.named("harnix.subscribe").filter((c) => c.params.topic === "t.y")).toHaveLength(2);
    off();
  });
});
