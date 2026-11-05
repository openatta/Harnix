/**
 * The built-in flow space (xops-02 §7): there only when the backend has the flow module, under the
 * id of the backend space whose conversations run `flow-author`, and only if the profile keeps it.
 */

import { describe, expect, it, vi } from "vitest";

import { BOOT_WAIT_MS, TIMED_OUT, within } from "../src/app";
import { registerBuiltins } from "../src/builtins";
import { flowSpaceOf, registerFlow } from "../src/flow";
import { scriptedApp, tick } from "./scripted";

const LIST = { spaces: [{ id: "sessions", scene: null }, { id: "flow", scene: "flow-author" }, { id: "system", scene: null }] };

describe("whether the backend has the flow space", () => {
  it("no `app.flow.defs` among the reachable methods: no flow module, no space, nothing asked", async () => {
    let asked = false;
    expect(await flowSpaceOf(["harnix.spaces.list", "harnix.hello"], async () => (asked = true))).toBe(null);
    expect(asked).toBe(false);
  });

  it("the space is the one whose scene is flow-author — `flow`, or what the application renamed it", async () => {
    const reachable = ["app.flow.defs", "harnix.spaces.list"];
    expect(await flowSpaceOf(reachable, async () => LIST)).toBe("flow");
    expect(await flowSpaceOf(reachable, async () => ({ spaces: [{ id: "flows", scene: "flow-author" }] }))).toBe("flows");
    // A module without a space for its conversations: nothing to show them in.
    expect(await flowSpaceOf(reachable, async () => ({ spaces: [{ id: "sessions", scene: null }] }))).toBe(null);
  });

  it("not knowing the spaces is the module's default id", async () => {
    expect(await flowSpaceOf(["app.flow.defs"], async () => LIST)).toBe("flow");
    expect(await flowSpaceOf(["app.flow.defs", "harnix.spaces.list"], async () => Promise.reject(new Error("no")))).toBe("flow");
  });
});

describe("registering it", () => {
  it("registers the space when detected and kept, not when the profile drops it", async () => {
    const { app, client } = scriptedApp();
    client.answer = (method) => (method === "harnix.spaces.list" ? LIST : {});
    app.set({ reachable: ["app.flow.defs", "harnix.spaces.list"] });
    await registerFlow(app, (id) => id !== "flow");
    expect(app.registry.at("space").some((c) => c.id === "flow")).toBe(false);
    expect(client.calls.some((c) => c.method === "harnix.spaces.list")).toBe(false);
    await registerFlow(app, () => true);
    const flow = app.registry.at("space").find((c) => c.id === "flow");
    expect(flow).toMatchObject({ center: "chat", owner: "flowui" });
  });

  it("`flow` in [shell] spaces names the built-in, whatever the backend space is called", async () => {
    const { app, client } = scriptedApp();
    client.answer = (method) => (method === "harnix.spaces.list" ? { spaces: [{ id: "flows", scene: "flow-author" }] } : {});
    app.set({ reachable: ["app.flow.defs", "harnix.spaces.list"] });
    await registerFlow(app, (id) => id === "flow");
    expect(app.registry.at("space").some((c) => c.id === "flows")).toBe(true);
  });

  it("registers nothing without the module", async () => {
    const { app } = scriptedApp();
    app.set({ reachable: ["harnix.spaces.list"] });
    await registerFlow(app, () => true);
    expect(app.registry.at("space").map((c) => c.id)).toEqual(["sessions", "b"]);
  });
});

describe("a backend that does not answer", () => {
  it("`within` gives up after its bound and leaves the promise to finish", async () => {
    let settle: (v: string) => void = () => {};
    const slow = new Promise<string>((r) => (settle = r));
    expect(await within(slow, 5)).toBe(TIMED_OUT);
    settle("late");
    expect(await slow).toBe("late");
    expect(await within(Promise.resolve(1), 1000)).toBe(1);
  });

  it("a spaces list that never comes does not hold the start; one that comes late registers the space then", async () => {
    const { app, client } = scriptedApp();
    let answer: (v: unknown) => void = () => {};
    client.answer = (method) => (method === "harnix.spaces.list" ? new Promise((r) => (answer = r)) : {});
    app.set({ reachable: ["app.flow.defs", "harnix.spaces.list"] });
    await registerFlow(app, () => true, 10);
    expect(app.registry.at("space").some((c) => c.id === "flow")).toBe(false);
    answer(LIST);
    await tick();
    await tick();
    expect(app.registry.at("space").some((c) => c.id === "flow")).toBe(true);
  });

  it.each([
    ["reachable never answers", true],
    ["reachable names the flow module and the spaces list never answers", false],
  ])("the start goes on to the sessions when %s", async (_, silent) => {
    vi.useFakeTimers();
    try {
      const { app, client } = scriptedApp();
      const listeners: ((s: string) => void)[] = [];
      Object.assign(client, {
        onStatus: (fn: (s: string) => void) => (listeners.push(fn), () => {}),
        connect: () => listeners.forEach((fn) => fn("open")),
      });
      client.answer = (method) =>
        method === "harnix.reachable"
          ? silent
            ? new Promise(() => {})
            : { methods: ["app.flow.defs", "harnix.spaces.list", "harnix.sessions"] }
          : method === "harnix.spaces.list"
            ? new Promise(() => {})
            : method === "harnix.sessions"
              ? { sessions: [] }
              : {};
      app.start(registerBuiltins);
      await vi.advanceTimersByTimeAsync(BOOT_WAIT_MS + 100);
      expect(client.calls.some((c) => c.method === "harnix.sessions")).toBe(true);
      expect(app.registry.at("space").some((c) => c.id === "sessions")).toBe(true);
      expect(client.calls.some((c) => c.method === "harnix.spaces.list")).toBe(!silent);
    } finally {
      vi.useRealTimers();
    }
  });
});
