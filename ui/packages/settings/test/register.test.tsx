/** The @harnix/settings package is the `settings` extension's interface: its settings sections, by id. */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, Registry, en, type HostApi } from "@harnix/react";

import { apiTypeOf, engineStore, extension, maskedKey, register } from "../src/index.tsx";

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
    expect(sections.map((s) => s.id)).toEqual(["engine","scenes","models","task-models","mcp"]);
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

  it("a stored key is shown as its last four at most, whatever it is handed", () => {
    expect(maskedKey("***abcd")).toBe("***abcd");
    expect(maskedKey("***")).toBe("***");
    expect(maskedKey("sk-0123456789abcd")).toBe("***abcd");
    expect(maskedKey("short")).toBe("***");
    expect(maskedKey("***0123456789abcd")).toBe("***abcd");
    expect(maskedKey(null)).toBeNull();
    expect(maskedKey("")).toBeNull();
  });

  /** The models section, over providers given here; `calls` is what it asked. */
  function models(providers: Record<string, unknown>, calls: [string, Record<string, unknown>][] = []) {
    const host = {
      call: async (method: string, params: Record<string, unknown>) => {
        calls.push([method, params]);
        return method === "settings.describe" ? { paths: { global: "/g/settings.json" }, fields: [], providers: { providers, default_provider: Object.keys(providers)[0] } } : {};
      },
      session: { current: () => null, context: () => null },
      shell: { notice: () => undefined },
      dialogs: { confirm: async () => false },
    } as unknown as HostApi;
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    const section = r.at("settings.section").find((s) => s.id === "models")!;
    const client = { status: "closed", detail: undefined, onStatus: () => () => undefined, call: async () => ({}) } as unknown as HarnixClient;
    return render(<HarnixUiProvider client={client} strings={en}>{section.render()}</HarnixUiProvider>);
  }

  it("a provider's key is shown masked, or as the environment variable it is read from", async () => {
    const { container } = models({
      stored: { api_type: "anthropic", api_key: "***wxyz" },
      env: { api_type: "openai_compatible", api_key: null, api_key_env: "DEEPSEEK_API_KEY", base_url: "https://api.deepseek.com" },
      none: { api_type: "anthropic", api_key: null },
    });
    await screen.findByTestId("providers");
    const key = (id: string) => container.querySelector(`[data-provider="${id}"] [data-testid="provider-key"]`)!;
    expect(key("stored").getAttribute("data-state")).toBe("stored");
    expect(key("stored").textContent).toBe("key ***wxyz");
    expect(key("env").getAttribute("data-state")).toBe("env");
    expect(key("env").textContent).toBe("key from $DEEPSEEK_API_KEY");
    expect(key("none").getAttribute("data-state")).toBe("missing");
  });

  it("an OpenAI-compatible provider's thinking switch and its key's variable are written with it", async () => {
    const calls: [string, Record<string, unknown>][] = [];
    const { container } = models({ ds: { api_type: "openai_compatible", base_url: "https://api.deepseek.com", api_key: "***abcd", default_model: "deepseek-chat" } }, calls);
    await screen.findByTestId("providers");
    fireEvent.click(container.querySelector('[data-provider="ds"] [data-testid="provider-edit"]')!);
    const form = await screen.findByTestId("provider-form");
    // The stored key is not shown, only its last four.
    expect((form.querySelector('[data-testid="provider-api_key"]') as HTMLInputElement).placeholder).toContain("***abcd");
    fireEvent.change(form.querySelector('[data-testid="provider-thinking_toggle"]')!, { target: { value: "deepseek" } });
    fireEvent.click(form.querySelector('[data-testid="provider-reasoning_passback"]')!);
    fireEvent.change(form.querySelector('[data-testid="provider-api_key_env"]')!, { target: { value: "DEEPSEEK_API_KEY" } });
    fireEvent.click(form.querySelector('[data-testid="provider-save"]')!);
    await waitFor(() => expect(calls.some(([m]) => m === "settings.setProvider")).toBe(true));
    const [, params] = calls.find(([m]) => m === "settings.setProvider")!;
    expect(params).toMatchObject({ provider_id: "ds", config: { thinking_toggle: "deepseek", reasoning_passback: true, api_key_env: "DEEPSEEK_API_KEY" } });
    expect((params.config as Record<string, unknown>).api_key).toBeUndefined();
  });

  it("an Anthropic provider is not sent the OpenAI-compatible thinking fields", async () => {
    const calls: [string, Record<string, unknown>][] = [];
    const { container } = models({ a: { api_type: "anthropic", api_key: "***abcd", default_model: "m" } }, calls);
    await screen.findByTestId("providers");
    fireEvent.click(container.querySelector('[data-provider="a"] [data-testid="provider-edit"]')!);
    const form = await screen.findByTestId("provider-form");
    expect(form.querySelector('[data-testid="provider-thinking_toggle"]')).toBeNull();
    fireEvent.click(form.querySelector('[data-testid="provider-save"]')!);
    await waitFor(() => expect(calls.some(([m]) => m === "settings.setProvider")).toBe(true));
    const config = calls.find(([m]) => m === "settings.setProvider")![1].config as Record<string, unknown>;
    expect("thinking_toggle" in config || "reasoning_passback" in config).toBe(false);
    expect(config.api_key_env).toBe("");
  });
});
