/**
 * The right column's context and usage tabs. Context: the report of the next call, the skills and
 * hooks, the MCP servers, what trust holds back and the way to trust the project — each part on its
 * own, a method the product lacks leaving its part out. Usage: this run's figures and the way into
 * the trajectory.
 */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { Registry, RegistryProvider, TabInfoProvider, type TabInfo } from "@harnix/react";

import { TRAJECTORY_VIEW, register, summarizeUsage } from "../src/index";
import { NOT_DECLARED, scripted } from "./host";

afterEach(cleanup);
const tick = () => new Promise((r) => setTimeout(r, 0));

const session = { instance: "main", sessionId: "s1" };

function info(kind: string, visible = true): TabInfo {
  return {
    tab: {
      id: `tab-${kind}`,
      kind,
      address: null,
      session,
      visible,
      navigation: { params: {}, revision: 1 },
      signal: new AbortController().signal,
      actions: { close: () => undefined, openResource: () => "", openTab: () => "", onClose: () => () => undefined },
    },
  };
}

function drawn(host: ReturnType<typeof scripted>["host"], kind: string) {
  const r = new Registry();
  r.register("space", { id: "sessions", icon: "chat", title: () => "Sessions" }, "test");
  r.activate("observability", () => register(r, host));
  r.settle();
  const type = r.at("sidebar.tab").find((t) => t.kind === kind)!;
  return render(
    <RegistryProvider registry={r}>
      <TabInfoProvider value={info(kind)}>{type.render()}</TabInfoProvider>
    </RegistryProvider>,
  );
}

const CONTEXT = {
  model: "m",
  system: [{ name: "scene.main", origin: "scene", tokens: 100, text: "You are…" }],
  tools: [{ name: "Bash", tokens: 50 }],
  messages: { count: 3, tokens: 200 },
  total_tokens: 350,
  measured: { tokens: 400, window: 1000 },
};

function full(method: string): unknown {
  switch (method) {
    case "observability.context":
      return CONTEXT;
    case "harnix.session.skills":
      return { instance: "main", session_id: "s1", listed: true, skills: [{ name: "deploy", description: "ship it", tier: "project", withheld: true }, { name: "notes", description: "", tier: "user", withheld: false }], dirs: [] };
    case "harnix.session.hooks":
      return { instance: "main", session_id: "s1", hooks: [{ event: "PreToolUse", source: "project", kind: "command", label: "command · Bash", in_force: false, withheld: true, providers: [] }, { event: "Stop", source: "global", kind: "prompt", label: "prompt", in_force: true, withheld: false, providers: [] }], unread: ["scene"] };
    case "settings.mcp.context":
      return { servers: [{ name: "github", tools: [{ name: "mcp__github__issues", tokens: 120, schema_bytes: 900 }], instructions: "Use issues.", status: { state: "connected", tool_count: 1, auth: { state: "none" } } }] };
    case "harnix-app.sessions.projectStatus":
      return { project_root: "/p", trust: "untrusted", categories: ["hooks", "mcp"], withheld: [{ key: "hooks_config", category: "hooks", name: null }, { key: "mcp_servers.github", category: "mcp", name: "github" }] };
    case "harnix-app.projects.trust":
      return { project_root: "/p", trust: "trusted", withheld: [] };
    default:
      return NOT_DECLARED;
  }
}

describe("the context tab", () => {
  it("shows the report, the skills and hooks with what is held back, the MCP servers, and trusts the project on asking", async () => {
    const s = scripted(full);
    const { container } = drawn(s.host, "context");
    await act(tick);
    expect(container.querySelector('[data-testid="context-report"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="context-stack"]')?.getAttribute("data-measured")).toBe("400");
    expect(container.querySelectorAll('[data-testid="system-block"]')).toHaveLength(1);
    // Every read claims the front space's.
    for (const method of ["observability.context", "harnix.session.skills", "harnix.session.hooks", "settings.mcp.context", "harnix-app.sessions.projectStatus"]) {
      expect(s.calls.find((c) => c.method === method)?.params, method).toEqual({ instance: "main", session_id: "s1", space: "sessions" });
    }
    const skills = container.querySelectorAll('[data-testid="context-skill"]');
    expect(skills).toHaveLength(2);
    expect(skills[0]?.getAttribute("data-withheld")).toBe("true");
    const hooks = container.querySelectorAll('[data-testid="context-hook"]');
    expect([...hooks].map((h) => h.getAttribute("data-in-force"))).toEqual(["false", "true"]);
    expect(container.querySelector('[data-testid="context-mcp-server"]')?.textContent).toContain("github");
    expect(container.querySelector('[data-testid="context-withheld"]')?.querySelectorAll("li")).toHaveLength(2);

    fireEvent.click(container.querySelector('[data-testid="context-trust"]')!);
    await act(tick);
    expect(s.calls.find((c) => c.method === "harnix-app.projects.trust")?.params).toEqual({ project_root: "/p", trusted: true, instance: "main" });
  });

  it("leaves out what this product has not: no report, no Harnix app", async () => {
    const s = scripted((method) => (method === "observability.context" || method.startsWith("harnix-app.") ? NOT_DECLARED : full(method)));
    const { container } = drawn(s.host, "context");
    await act(tick);
    expect(container.querySelector('[data-testid="context-report"]')).toBeNull();
    expect(container.querySelector('[data-testid="context-report-absent"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="context-withheld"]')).toBeNull();
    expect(container.querySelectorAll('[data-testid="context-skill"]')).toHaveLength(2);
  });

  it("declined, the project is not trusted; a part that fails says so without taking the tab", async () => {
    const s = scripted((method) => (method === "harnix.session.hooks" ? Object.assign(new Error("busy"), { code: -32015 }) : full(method)));
    s.confirmed = false;
    const { container } = drawn(s.host, "context");
    await act(tick);
    fireEvent.click(container.querySelector('[data-testid="context-trust"]')!);
    await act(tick);
    expect(s.calls.some((c) => c.method === "harnix-app.projects.trust")).toBe(false);
    expect(container.querySelector('[data-testid="context-hooks"] [data-code="-32015"]')).not.toBeNull();
    expect(container.querySelectorAll('[data-testid="context-skill"]')).toHaveLength(2);
  });
});

describe("the usage tab", () => {
  const usage = {
    sessions: {
      s1: {
        recording: true,
        run: { created_at: "2025-11-15T09:46:40Z", generation: 1 },
        agents: [
          { session_id: "s1", parent_session_id: null, calls: 4, auxiliary: 1, turns: 2, models: ["m"], input_tokens: 1000, cache_read_tokens: 3000, cache_write_tokens: 0, output_tokens: 500, model_ms: 8000, errors: 1, first_at: 1000, last_at: 61_000 },
          { session_id: "sub", parent_session_id: "s1", calls: 2, auxiliary: 0, turns: 1, models: ["m"], input_tokens: 100, cache_read_tokens: 0, cache_write_tokens: 0, output_tokens: 50, model_ms: 2000, errors: 0, first_at: 2000, last_at: 3000 },
        ],
      },
    },
  };
  const turns = { turns: [{ turn: 0, calls: [0, 1], auxiliary: 1, ttft_ms: 300, model_ms: 4000, tool_ms: 1500, input_tokens: 2000, output_tokens: 250, tokens_per_s: 25, cache_hit: 0.75 }, { turn: 1, calls: [2, 3], auxiliary: 0, ttft_ms: 500, model_ms: 4000, tool_ms: 500, input_tokens: 2000, output_tokens: 250, tokens_per_s: 50, cache_hit: 0.75 }] };

  it("sums the run: turns, calls, tokens, the cache's share, times, the span, the sub-agents", () => {
    const s = summarizeUsage(usage.sessions.s1.agents[0]!, [usage.sessions.s1.agents[1]!], turns.turns as never);
    expect(s).toMatchObject({ turns: 2, calls: 4, auxiliary: 1, total: 4500, cacheHit: 0.75, modelMs: 8000, toolMs: 2000, ttftMs: 400, spanMs: 60_000, errors: 1, agents: { count: 1, total: 150, modelMs: 2000 } });
    // Weighted by what each turn wrote: 250 tokens at 25/s and 250 at 50/s.
    expect(s.rate).toBeCloseTo(500 / (10 + 5));
    // Without the usage figures, the turns say what they can.
    const bare = summarizeUsage(undefined, [], turns.turns as never);
    expect(bare).toMatchObject({ turns: 2, calls: 4, total: 4500, cacheHit: null, spanMs: null });
  });

  it("draws the figures and opens the trajectory of the open session in place", async () => {
    const s = scripted((method) => (method === "observability.usage" ? usage : method === "observability.turns" ? turns : NOT_DECLARED), { current: session });
    const { container } = drawn(s.host, "usage");
    await act(tick);
    const cell = (id: string) => container.querySelector(`[data-cell="${id}"] .v`)?.textContent;
    expect(cell("turns")).toBe("2");
    expect(cell("calls")).toBe("4 +1");
    expect(cell("hit")).toBe("75%");
    expect(container.querySelector('[data-testid="usage-agents"]')).not.toBeNull();
    expect(s.calls.find((c) => c.method === "observability.turns")?.params).toEqual({ instance: "main", session_id: "s1", space: "sessions" });
    fireEvent.click(container.querySelector('[data-testid="usage-open-trajectory"]')!);
    expect(s.shown).toEqual([TRAJECTORY_VIEW]);
    expect(s.opened).toEqual([]);
  });

  it("with no recording says so, and opens another session's trajectory where it belongs", async () => {
    const s = scripted((method) => (method === "observability.usage" ? { sessions: {} } : method === "observability.turns" ? { turns: [] } : NOT_DECLARED), { current: { instance: "main", sessionId: "other" } });
    const { container } = drawn(s.host, "usage");
    await act(tick);
    expect(container.querySelector('[data-testid="usage-none"]')).not.toBeNull();
    fireEvent.click(container.querySelector('[data-testid="usage-open-trajectory"]')!);
    expect(s.opened).toEqual([{ instance: "main", sessionId: "s1", options: { switchSpace: true, view: TRAJECTORY_VIEW } }]);
  });
});
