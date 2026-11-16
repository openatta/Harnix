/** The @harnix/settings package is the `settings` extension's interface: its settings sections, by id. */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, Registry, en, resetSetupSteps, type HostApi } from "@harnix/react";

import { MODEL_STEP, ModelStep, apiTypeOf, engineStore, extension, register } from "../src/index.tsx";

afterEach(cleanup);

describe("@harnix/settings", () => {
  it("registers its settings sections, owned by the extension", async () => {
    const calls: string[] = [];
    const host = {
      call: async (method: string) => {
        calls.push(method);
        return {};
      },
      session: { current: () => null, context: () => null },
      shell: { notice: () => undefined },
      dialogs: { confirm: async () => false },
    } as unknown as HostApi;
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    expect(extension).toBe("settings");
    const sections = r.at("settings.section");
    expect(sections.map((s) => s.id)).toEqual(["engine","scenes","models","mcp"]);
    expect(sections.every((s) => s.owner === "settings" && typeof s.label() === "string")).toBe(true);
    // Each draws, and reads what it shows through the host.
    // Not connected: what reads hello waits, the rest draws.
    const client = { status: "closed", detail: undefined, onStatus: () => () => undefined, call: async () => ({}) } as unknown as HarnixClient;
    for (const section of sections) render(<HarnixUiProvider client={client} strings={en}>{section.render()}</HarnixUiProvider>);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls.length).toBeGreaterThan(0);
  });

  it("shows each MCP server with the tool count and transport Agix reports", async () => {
    const host = {
      call: async () => ({ servers: [{ name: "files", transport: "stdio", tool_count: 3 }] }),
      session: { current: () => null, context: () => null },
      shell: { notice: () => undefined },
      dialogs: { confirm: async () => false },
    } as unknown as HostApi;
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    const mcp = r.at("settings.section").find((s) => s.id === "mcp")!;
    const client = { status: "closed", detail: undefined, onStatus: () => () => undefined, call: async () => ({}) } as unknown as HarnixClient;
    const { findByText } = render(<HarnixUiProvider client={client} strings={en}>{mcp.render()}</HarnixUiProvider>);
    expect(await findByText("3 tools · stdio")).toBeTruthy();
    expect(await findByText("connected")).toBeTruthy();
  });

  it("says when the project tier wrote a setting the engine does not use", async () => {
    const field = { key: "permission_mode", type: "select", kind: "choice", section: "engine", options: ["default", "plan"], effective: "default", source: "project", withheld: true, tiers: { global: "default", scene: null, project: "bypassPermissions" } };
    const host = {
      call: async () => ({ paths: { global: "/g", project: "/p" }, fields: [field] }),
      session: { current: () => null, context: () => null },
      shell: { notice: () => undefined },
      dialogs: { confirm: async () => false },
    } as unknown as HostApi;
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    const engine = r.at("settings.section").find((s) => s.id === "engine")!;
    const client = { status: "closed", detail: undefined, onStatus: () => () => undefined, call: async () => ({}) } as unknown as HarnixClient;
    const { findByText } = render(<HarnixUiProvider client={client} strings={en}>{engine.render()}</HarnixUiProvider>);
    expect(await findByText(/not in force: an untrusted project can only tighten settings/)).toBeTruthy();
  });

  it("draws a section's engine fields from their declarations and writes one into the chosen tier, at the revision read", async () => {
    const fields = [
      { key: "permission_mode", type: "select", section: "engine", options: ["default", "plan"], default: "default", writable: ["global", "scene", "project"], effective: "default", source: null, withheld: false, tiers: { global: null, scene: null, project: null } },
      { key: "memory_enabled", type: "toggle", section: "engine", default: true, writable: ["global", "scene", "project"], effective: true, source: null, withheld: false, tiers: { global: null, scene: null, project: null } },
      { key: "model.max_tokens", type: "number", section: "models", min: 1, max: 1000000, writable: ["global", "scene", "project"], effective: 8000, source: "global", withheld: false, tiers: { global: 8000, scene: null, project: null } },
    ];
    const calls: [string, Record<string, unknown>][] = [];
    const host = {
      call: async (method: string, params: Record<string, unknown>) => {
        calls.push([method, params]);
        return method === "settings.describe" ? { paths: { global: "/g/settings.json", scene: "/s/settings.json" }, fields, revision: "rev-1" } : {};
      },
      session: { current: () => null, context: () => null },
      shell: { notice: () => undefined },
      dialogs: { confirm: async () => false },
    } as unknown as HostApi;
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    const engine = r.at("settings.section").find((s) => s.id === "engine")!;
    const client = { status: "closed", detail: undefined, onStatus: () => () => undefined, call: async () => ({}) } as unknown as HarnixClient;
    render(<HarnixUiProvider client={client} strings={en}>{engine.render()}</HarnixUiProvider>);
    // The engine section's two, not the models section's.
    expect(await screen.findByTestId("settings-permission_mode")).toBeTruthy();
    expect(screen.getByTestId("settings-memory_enabled")).toBeTruthy();
    expect(screen.queryByTestId("settings-model.max_tokens")).toBeNull();
    // The scene tier, then a write there.
    fireEvent.change(screen.getByTestId("settings-tier"), { target: { value: "scene" } });
    fireEvent.click(screen.getByTestId("settings-memory_enabled"));
    await waitFor(() => expect(calls.some(([m]) => m === "settings.write")).toBe(true));
    const [, params] = calls.find(([m]) => m === "settings.write")!;
    expect(params).toMatchObject({ tier: "scene", changes: { memory_enabled: false }, revision: "rev-1" });
  });

  it("the engine's store reads describe and writes write", async () => {
    const calls: [string, unknown][] = [];
    const host = { call: async (method: string, params: unknown) => (calls.push([method, params]), { fields: [] }) } as unknown as HostApi;
    const store = engineStore(host, { scene: "code", project_root: "/p" }, "project");
    await store.load();
    await store.save({ memory_enabled: null });
    expect(calls).toEqual([
      ["settings.describe", { scene: "code", project_root: "/p" }],
      ["settings.write", { scene: "code", project_root: "/p", tier: "project", changes: { memory_enabled: null } }],
    ]);
  });

  it("a provider's type is kept as it is: a known one, a new one, and one the table lacks", () => {
    expect(apiTypeOf({ api_type: "openai_responses" }).id).toBe("openai_responses");
    expect(apiTypeOf({ api_type: "anthropic" }).id).toBe("anthropic");
    expect(apiTypeOf({}).id).toBe("anthropic");
    expect(apiTypeOf({ api_type: "mystery" })).toMatchObject({ id: "mystery", label: "" });
  });
});

describe("the first-run model step", () => {
  afterEach(() => {
    localStorage.clear();
    resetSetupSteps();
  });

  /** A host whose hello says what `engine` is now, and whose dialogs can be closed. */
  function stepHost(engine: Record<string, unknown>) {
    const modal = new Set<(open: boolean) => void>();
    const opened: (string | undefined)[] = [];
    const host = {
      engine,
      opened,
      closeModal: () => modal.forEach((fn) => fn(false)),
      call: async (method: string) => (method === "harnix.hello" ? { engine: host.engine } : {}),
      modal: { onChange: (fn: (open: boolean) => void) => (modal.add(fn), () => modal.delete(fn)) },
      settings: { open: (section?: string) => void opened.push(section) },
    };
    return host as unknown as HostApi & { engine: Record<string, unknown>; opened: (string | undefined)[]; closeModal(): void };
  }

  const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

  it("is a home card of the extension's, first", () => {
    const r = new Registry();
    r.activate(extension, () => register(r, stepHost({})));
    expect(r.at("home.card").map((c) => [c.id, c.order, c.owner, c.title])).toEqual([[MODEL_STEP, 10, "settings", undefined]]);
  });

  it("asks for a model while none is configured, and says it is done once one is", async () => {
    const host = stepHost({ has_credentials: false, providers: { providers: {}, default_provider: null } });
    const page = render(<ModelStep host={host} />);
    await settle();
    const step = () => page.container.querySelector(`[data-step="${MODEL_STEP}"]`) as HTMLElement | null;
    expect(step()?.dataset.state).toBe("todo");
    fireEvent.click(page.getByTestId("setup-model-open"));
    expect(host.opened).toEqual(["models"]);

    // A provider is added in the settings; hello is read again when the panel closes.
    host.engine = { has_credentials: true, providers: { providers: { anthropic: { default_model: "claude-x" } }, default_provider: "anthropic" } };
    host.closeModal();
    await settle();
    expect(step()?.dataset.state).toBe("done");
    expect(step()!.textContent).toContain("claude-x");
  });

  it("draws nothing when a model was there from the start", async () => {
    const host = stepHost({ has_credentials: true, providers: { providers: {}, default_provider: null } });
    const page = render(<ModelStep host={host} />);
    await settle();
    expect(page.container.innerHTML).toBe("");
  });
});
