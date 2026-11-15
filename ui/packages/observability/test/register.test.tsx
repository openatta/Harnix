/**
 * The observability tabs: registered into the system space's inspector, dropped where nobody
 * defines it, the calls tab picking out the tool call a person came for.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import { Registry, RegistryProvider, type HostApi } from "@harnix/react";

import { register } from "../src/index";

afterEach(cleanup);

const tick = () => new Promise((r) => setTimeout(r, 0));

/** Just what the tabs reach: calls, the turn events, and the defaults. */
function host(answer: (method: string) => unknown): HostApi {
  const none = () => () => undefined;
  return {
    call: async (method: string) => answer(method),
    events: { onTurnSettled: none, onSessionEvent: none, onHostEvent: none },
    defaults: { instance: "main", space: "sessions" },
  } as unknown as HostApi;
}

const call = (index: number, tool: string) => ({
  index,
  session_id: "s1",
  turn: 1,
  step: index,
  model: "m",
  input: { system: [], tools: 0, messages: 1 },
  output: { chars: 0, thinking_chars: 0, tool_uses: 1 },
  tools: [{ id: tool, name: "Bash" }],
  status: "ok",
});

describe("the observability tabs", () => {
  it("go to the system space's inspector, and nowhere when it is not there", () => {
    const alone = new Registry();
    alone.activate("observability", () => register(alone, host(() => ({}))));
    alone.settle();
    expect(alone.list("system.inspector.tab")).toEqual([]);

    const r = new Registry();
    r.definePoint("system.inspector.tab", { kind: "render" }, "system");
    r.activate("observability", () => register(r, host(() => ({}))));
    r.settle();
    const tabs = r.list<{ id: string; tools?: boolean }>("system.inspector.tab");
    expect(tabs.map((t) => [t.id, t.tools ?? false])).toEqual([
      ["observability.calls", true],
      ["observability.turns", false],
    ]);
  });

  it("the calls tab lists a session's calls and scrolls to the one a person came for", async () => {
    const scrolled: string[] = [];
    Element.prototype.scrollIntoView = function (this: Element) {
      scrolled.push(this.getAttribute("data-tool") ?? "");
    };
    const r = new Registry();
    r.definePoint("system.inspector.tab", { kind: "render" }, "system");
    r.activate("observability", () => register(r, host((method) => (method === "observability.trace" ? { calls: [call(0, "tu-a"), call(1, "tu-b")] } : {}))));
    r.settle();
    const calls = r.list<{ render(t: unknown): React.ReactNode }>("system.inspector.tab")[0]!;
    const { container } = render(<RegistryProvider registry={r}>{calls.render({ kind: "session", instance: "main", sessionId: "s1", tool: "tu-b" })}</RegistryProvider>);
    await act(async () => {
      await tick();
    });
    expect(container.querySelector('[data-testid="system-calls"]')).not.toBeNull();
    expect(container.querySelector('[data-tool="tu-b"]')).not.toBeNull();
    expect(scrolled).toEqual(["tu-b"]);
  });

  it("the calls tab says which run it covers, and that an earlier one is gone", async () => {
    const r = new Registry();
    r.definePoint("system.inspector.tab", { kind: "render" }, "system");
    const run = { created_at: "2025-11-15T09:46:40Z", generation: 1, earlier_run: true };
    r.activate("observability", () => register(r, host((method) => (method === "observability.trace" ? { calls: [call(0, "tu-a")], run } : {}))));
    r.settle();
    const calls = r.list<{ render(t: unknown): React.ReactNode }>("system.inspector.tab")[0]!;
    const { container } = render(<RegistryProvider registry={r}>{calls.render({ kind: "session", instance: "main", sessionId: "s1" })}</RegistryProvider>);
    await act(async () => {
      await tick();
    });
    const note = container.querySelector('[data-testid="run-note"]');
    expect(note?.getAttribute("data-earlier")).toBe("true");
    expect(container.querySelector('[data-testid="system-call-tool"]')?.textContent).not.toContain("{}");
  });
});
