/**
 * What @harnix/observability registers: the trajectory as a view of the centre, the context and
 * usage tabs of the right column, and the trajectory as a page of the system space's inspector —
 * which is nowhere when that space is not there. The trajectory lists a session's calls by turn,
 * opens a call's inspector, picks out the tool call a person came for, and says which run it covers.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { Registry, RegistryProvider } from "@harnix/react";

import { TRAJECTORY_VIEW, register } from "../src/index";
import { NOT_DECLARED, call, scripted } from "./host";

afterEach(cleanup);

const tick = () => new Promise((r) => setTimeout(r, 0));

const TRACE = {
  calls: [
    call(0, { tools: [{ id: "tu-a", name: "Bash", input_preview: '{"command":"ls"}' }], output: { chars: 0, thinking_chars: 0, tool_uses: 1 } }),
    call(1, { step: 1, output: { chars: 12, thinking_chars: 0, tool_uses: 0, text_preview: "listed them" } }),
    call(2, { turn_id: null, purpose: "title" }),
  ],
  run: { created_at: "2025-11-15T09:46:40Z", generation: 1, earlier_run: true },
};
const OUTLINE = { turns: [{ turn_id: "t1", seq: 0, at: "2025-11-15T09:46:41Z", prompt: "list the files", reply: "listed them" }] };

function answering(method: string, params: unknown): unknown {
  switch (method) {
    case "observability.trace":
      return TRACE;
    case "observability.turns":
      return { turns: [{ turn: 0, calls: [0, 1], auxiliary: 1, started_at: 1000, duration_ms: 1500, ttft_ms: 100, model_ms: 1000, tool_ms: 500, input_tokens: 20, output_tokens: 10, cache_hit: 0.5, tokens_per_s: 12, context_tokens: 300, context_delta: 300 }] };
    case "harnix.session.outline":
      return OUTLINE;
    case "engine.session.entries":
      return { entries: [{ ns: "harnix.feedback", event: "rating", payload: { turn_id: "t1", rating: "up", at: 1, by: "d" } }] };
    case "observability.trace.call":
      return {
        input: { system: [{ source: "scene.main", text: "You are…" }], tools: [{ name: "Bash" }], messages: [{ role: "user", content: "list the files" }], model: "m", max_tokens: 100 },
        output: { text: "", thinking_chars: 0, tool_uses: [{ id: "tu-a", name: "Bash", input: { command: "ls" } }], tool_results: [{ id: "tu-a", text: "a.txt", chars: 5, is_error: false }], status: "ok" },
      };
    default:
      return NOT_DECLARED;
  }
}

describe("what is registered", () => {
  it("the trajectory view, two tabs with guide entries; the inspector page only where the system space defines its point", () => {
    const alone = new Registry();
    alone.activate("observability", () => register(alone, scripted(() => ({})).host));
    alone.settle();
    expect(alone.at("conversation.view").map((v) => v.id)).toEqual([TRAJECTORY_VIEW]);
    expect(alone.at("sidebar.tab").map((t) => [t.kind, t.priority, t.guide?.length ?? 0])).toEqual([
      ["context", "builtin", 1],
      ["usage", "builtin", 1],
    ]);
    expect(alone.list("system.inspector.tab")).toEqual([]);

    const r = new Registry();
    r.definePoint("system.inspector.tab", { kind: "render" }, "system");
    r.activate("observability", () => register(r, scripted(() => ({})).host));
    r.settle();
    expect(r.list<{ id: string; tools?: boolean }>("system.inspector.tab").map((t) => [t.id, t.tools ?? false])).toEqual([[TRAJECTORY_VIEW, true]]);
  });
});

describe("the trajectory", () => {
  function drawn(host: ReturnType<typeof scripted>["host"], extra: { tool?: string } = {}) {
    const r = new Registry();
    r.definePoint("system.inspector.tab", { kind: "render" }, "system");
    r.register("space", { id: "sessions", icon: "chat", title: () => "Sessions" }, "test");
    r.activate("observability", () => register(r, host));
    r.settle();
    const view = r.at("conversation.view")[0]!;
    const page = r.list<{ render(t: unknown): React.ReactNode }>("system.inspector.tab")[0]!;
    const body = extra.tool ? page.render({ kind: "session", instance: "main", sessionId: "s1", tool: extra.tool }) : view.render({ instance: "main", sessionId: "s1" });
    return render(<RegistryProvider registry={r}>{body}</RegistryProvider>);
  }

  it("lists the turn's words, its calls with their tools and the call beside it, the feedback on the last call, and the run", async () => {
    const s = scripted(answering);
    const { container } = drawn(s.host);
    await act(tick);
    expect(container.querySelector('[data-testid="trajectory"]')).not.toBeNull();
    // Every read of the session claims the front space's.
    expect(s.calls.find((c) => c.method === "observability.trace")?.params).toEqual({ instance: "main", session_id: "s1", space: "sessions" });
    expect(container.querySelectorAll('[data-testid="turn-row"]')).toHaveLength(1);
    expect(container.querySelector('[data-testid="traj-user"]')?.textContent).toContain("list the files");
    const calls = container.querySelectorAll('[data-testid="system-call"]');
    expect(calls).toHaveLength(2);
    expect(container.querySelector('[data-testid="system-call-tool"]')?.textContent).toContain("Bash");
    expect(container.querySelector('[data-testid="traj-aux"]')?.getAttribute("data-purpose")).toBe("title");
    expect(container.querySelector('[data-testid="traj-feedback"]')?.getAttribute("data-rating")).toBe("up");
    expect(calls[1]?.getAttribute("data-feedback")).toBe("up");
    expect(container.querySelector('[data-testid="run-note"]')?.getAttribute("data-earlier")).toBe("true");
    // The turn's sums, on its row: the cache's share, the context as a change or a total.
    expect(container.querySelector('[data-testid="turn-cache"]')?.textContent).toBe("50%");
    expect(container.querySelector('[data-testid="turn-context"]')?.textContent).toBe("+300");
    fireEvent.click(container.querySelector('[data-testid="turns-total"]')!);
    expect(container.querySelector('[data-testid="turn-context"]')?.textContent).toBe("300");
    expect(container.querySelector('[data-testid="calls-timeline"] [data-testid="timeline-call"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="traj-earlier"]')).toBeNull();
  });

  it("narrows to the failed calls or the ones that called a tool, searches, and opens a call's inspector beside the rows", async () => {
    const s = scripted((method, params) => (method === "observability.trace" ? { ...TRACE, calls: [TRACE.calls[0], { ...TRACE.calls[1], status: "error", error: { message: "overloaded" } }] } : answering(method, params)));
    const { container } = drawn(s.host);
    await act(tick);
    const calls = () => container.querySelectorAll('[data-testid="system-call"]');
    fireEvent.click(container.querySelector('[data-filter="failed"]')!);
    expect([...calls()].map((c) => c.getAttribute("data-call"))).toEqual(["1"]);
    fireEvent.click(container.querySelector('[data-filter="tools"]')!);
    expect([...calls()].map((c) => c.getAttribute("data-call"))).toEqual(["0"]);
    fireEvent.click(container.querySelector('[data-filter="all"]')!);
    expect(calls()).toHaveLength(2);
    // Folded away, a search still finds the tool call.
    fireEvent.click(container.querySelector('[data-testid="traj-fold-turns"]')!);
    expect(calls()).toHaveLength(0);
    fireEvent.change(container.querySelector('[data-testid="calls-search"]')!, { target: { value: "ls" } });
    expect([...calls()].map((c) => c.getAttribute("data-call"))).toEqual(["0"]);
    fireEvent.change(container.querySelector('[data-testid="calls-search"]')!, { target: { value: "nothing says this" } });
    expect(container.querySelector('[data-testid="calls-none"]')).not.toBeNull();
    fireEvent.change(container.querySelector('[data-testid="calls-search"]')!, { target: { value: "" } });
    fireEvent.click(container.querySelector('[data-testid="traj-fold-turns"]')!);

    fireEvent.click(container.querySelector('[data-testid="jump-to-failure"]')!);
    await act(tick);
    const detail = container.querySelector('[data-testid="call-detail"]')!;
    expect(detail.getAttribute("data-call")).toBe("1");
    // Beside the rows, not in their place.
    expect(calls()).toHaveLength(2);
    expect(container.querySelector('[data-testid="call-compare"]')).not.toBeNull();
    fireEvent.click(container.querySelector('[data-testid="call-back"]')!);
    expect(container.querySelector('[data-testid="call-detail"]')).toBeNull();
  });

  it("in the system space's inspector: picks out the call that asked for the tool a person came for, and offers the way into the session", async () => {
    const s = scripted(answering, { space: "system" });
    const { container } = drawn(s.host, { tool: "tu-a" });
    await act(tick);
    // Opened on the call: the compact inspector takes the list's place.
    const detail = container.querySelector('[data-testid="call-detail"]');
    expect(detail?.getAttribute("data-call")).toBe("0");
    expect(s.calls.find((c) => c.method === "observability.trace")?.params).toMatchObject({ space: "system" });
    fireEvent.click(container.querySelector('[data-testid="call-back"]')!);
    expect(container.querySelector('[data-testid="system-call"][data-located="true"]')?.getAttribute("data-call")).toBe("0");
    expect(container.querySelector('[data-testid="system-call-tool"][data-located="true"]')?.getAttribute("data-tool")).toBe("tu-a");
    fireEvent.click(container.querySelector('[data-testid="traj-open-session"]')!);
    await act(tick);
    expect(s.opened).toEqual([{ instance: "main", sessionId: "s1", options: { switchSpace: true, view: TRAJECTORY_VIEW } }]);
  });

  it("reads again when a turn of the session settles, and says a failure in its own words", async () => {
    const s = scripted(answering);
    const { container } = drawn(s.host);
    await act(tick);
    const reads = () => s.calls.filter((c) => c.method === "observability.trace").length;
    expect(reads()).toBe(1);
    act(() => s.settle("other"));
    await act(tick);
    expect(reads()).toBe(1);
    act(() => s.settle("s1"));
    await act(tick);
    expect(reads()).toBe(2);

    const failing = scripted((method, params) => (method === "observability.trace" ? Object.assign(new Error("busy"), { code: -32015 }) : answering(method, params)));
    const failed = drawn(failing.host);
    await act(tick);
    expect(failed.container.querySelector('[data-testid="system-calls-failed"]')?.getAttribute("data-code")).toBe("-32015");
  });
});
