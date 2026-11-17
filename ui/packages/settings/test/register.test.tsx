/** The @harnix/settings package is the `settings` extension's interface: its settings sections, by id. */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, Registry, en, resetSetupSteps, setLocale, type HostApi } from "@harnix/react";

import { MODEL_STEP, ModelStep, apiTypeOf, engineStore, extension, maskedKey, register } from "../src/index.tsx";

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
    expect(sections.map((s) => s.id)).toEqual(["engine", "scenes", "models", "task-models", "mcp", "permissions", "sandbox", "network", "privacy", "advanced"]);
    expect(sections.every((s) => s.owner === "settings" && typeof s.label() === "string")).toBe(true);
    // Each draws, and reads what it shows through the host.
    // Not connected: what reads hello waits, the rest draws.
    const client = { status: "closed", detail: undefined, onStatus: () => () => undefined, call: async () => ({}) } as unknown as HarnixClient;
    for (const section of sections) render(<HarnixUiProvider client={client} strings={en}>{section.render()}</HarnixUiProvider>);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls.length).toBeGreaterThan(0);
  });

  it("shows each MCP server with the tool count and transport the extension reports", async () => {
    const calls: string[] = [];
    const host = {
      call: async (method: string) => {
        calls.push(method);
        return {
          servers: [{ name: "files", tier: "global", enabled: true, transport: "stdio", command: "fs", args: [], env: {}, headers: {}, unsupported: [], status: { state: "connected", tool_count: 3, auth: { state: "none", scopes: [] } } }],
          paths: { global: "/g/settings.json", scene: null, project: null },
          status_read: true,
        };
      },
      session: { current: () => null, context: () => null },
      shell: { notice: () => undefined },
      dialogs: { confirm: async () => false },
    } as unknown as HostApi;
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    const mcp = r.at("settings.section").find((s) => s.id === "mcp")!;
    const client = { status: "closed", detail: undefined, onStatus: () => () => undefined, call: async () => ({}) } as unknown as HarnixClient;
    const { findByText } = render(<HarnixUiProvider client={client} strings={en}>{mcp.render()}</HarnixUiProvider>);
    expect(await findByText("3 tools")).toBeTruthy();
    expect(await findByText("connected")).toBeTruthy();
    expect(calls).toContain("settings.mcp.list");
  });

  it("says when the project tier wrote a setting the engine does not use", async () => {
    const field = { key: "permission_mode", type: "select", kind: "choice", section: "permissions", options: ["default", "plan"], effective: "default", source: "project", withheld: true, tiers: { global: "default", scene: null, project: "bypassPermissions" } };
    const host = {
      call: async () => ({ paths: { global: "/g", project: "/p" }, fields: [field] }),
      session: { current: () => null, context: () => null },
      shell: { notice: () => undefined },
      dialogs: { confirm: async () => false },
    } as unknown as HostApi;
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    const permissions = r.at("settings.section").find((s) => s.id === "permissions")!;
    const client = { status: "closed", detail: undefined, onStatus: () => () => undefined, call: async () => ({}) } as unknown as HarnixClient;
    const { findByText } = render(<HarnixUiProvider client={client} strings={en}>{permissions.render()}</HarnixUiProvider>);
    expect(await findByText(/not in force: an untrusted project can only tighten settings/)).toBeTruthy();
  });

  it("draws a section's engine fields from their declarations and writes one into the chosen tier, at the revision read", async () => {
    const fields = [
      { key: "permission_mode", type: "select", section: "permissions", options: ["default", "plan"], default: "default", writable: ["global", "scene", "project"], effective: "default", source: null, withheld: false, tiers: { global: null, scene: null, project: null } },
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
    // The engine section's own, not the permissions section's or the models section's.
    expect(await screen.findByTestId("settings-memory_enabled")).toBeTruthy();
    expect(screen.queryByTestId("settings-permission_mode")).toBeNull();
    expect(screen.queryByTestId("settings-model.max_tokens")).toBeNull();
    // The scene tier, then a write there.
    fireEvent.change(screen.getByTestId("settings-tier"), { target: { value: "scene" } });
    fireEvent.click(screen.getByTestId("settings-memory_enabled"));
    await waitFor(() => expect(calls.some(([m]) => m === "settings.write")).toBe(true));
    const [, params] = calls.find(([m]) => m === "settings.write")!;
    expect(params).toMatchObject({ tier: "scene", changes: { memory_enabled: false }, revision: "rev-1" });
  });

  it("sends a reader from the network section to the web's routes, which are set with the other routes", async () => {
    const fields = [
      { key: "sandbox.allowed_domains", type: "list", section: "network", writable: ["global", "scene", "project"], effective: null, source: null, withheld: false, tiers: { global: null, scene: null, project: null } },
      { key: "task_models.web_search", type: "route", section: "task-models", routes: [], writable: ["global", "scene", "project"], effective: null, source: null, withheld: false, tiers: { global: null, scene: null, project: null } },
    ];
    const opened: (string | undefined)[] = [];
    const host = {
      call: async () => ({ paths: { global: "/g/settings.json" }, fields }),
      session: { current: () => null, context: () => null },
      settings: { open: (section?: string) => opened.push(section) },
      shell: { notice: () => undefined },
      dialogs: { confirm: async () => false },
    } as unknown as HostApi;
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    const network = r.at("settings.section").find((s) => s.id === "network")!;
    const client = { status: "closed", detail: undefined, onStatus: () => () => undefined, call: async () => ({}) } as unknown as HarnixClient;
    render(<HarnixUiProvider client={client} strings={en}>{network.render()}</HarnixUiProvider>);
    expect(await screen.findByTestId("settings-sandbox.allowed_domains")).toBeTruthy();
    expect(screen.queryByTestId("settings-task_models.web_search")).toBeNull();
    fireEvent.click(screen.getByTestId("settings-web-routes"));
    expect(opened).toEqual(["task-models"]);
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

  const client = { status: "closed", detail: undefined, onStatus: () => () => undefined, call: async () => ({}) } as unknown as HarnixClient;
  const hostOf = (call: (method: string, params: Record<string, unknown>) => unknown) =>
    ({
      call: async (method: string, params: Record<string, unknown>) => call(method, params),
      session: { current: () => null, context: () => null },
      shell: { notice: () => undefined },
      dialogs: { confirm: async () => false },
    }) as unknown as HostApi;

  it("writes the engine's language, in the global tier, when the language is chosen", async () => {
    const calls: [string, Record<string, unknown>][] = [];
    const host = hostOf((method, params) => (calls.push([method, params]), {}));
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    setLocale("en");
    setLocale("auto");
    expect(calls.filter(([m]) => m === "settings.write")).toEqual([
      ["settings.write", { tier: "global", changes: { language: "en" } }],
      ["settings.write", { tier: "global", changes: { language: null } }],
    ]);
    // Registered again, it still writes once a choice.
    r.activate(extension, () => register(r, host));
    calls.length = 0;
    setLocale("zh-CN");
    expect(calls).toHaveLength(1);
    setLocale("auto");
  });

  const flag = { key: "telemetry_enabled", type: "toggle", section: "privacy", default: false, writable: ["global"], hint: "settings.hint.telemetry_enabled", effective: true, source: "global", withheld: false, tiers: { global: true, scene: null, project: null } };

  it("says what telemetry records and writes the switch into the global tier whichever tier was last chosen", async () => {
    const calls: [string, Record<string, unknown>][] = [];
    const host = hostOf((method, params) => (calls.push([method, params]), method === "settings.describe" ? { paths: { global: "/g/settings.json", scene: "/s/settings.json" }, fields: [flag], revision: "r1" } : {}));
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    const privacy = r.at("settings.section").find((s) => s.id === "privacy")!;
    render(<HarnixUiProvider client={client} strings={en}>{privacy.render()}</HarnixUiProvider>);
    expect(screen.getByTestId("privacy-explain").textContent).toMatch(/no prompts, no message text/);
    expect(await screen.findByTestId("settings-telemetry_enabled")).toBeTruthy();
    expect(screen.queryByTestId("settings-tier")).toBeNull();
    expect(screen.getByTestId("settings-file-global").textContent).toBe("/g/settings.json");
    expect(screen.queryByTestId("settings-file-scene")).toBeNull();
    fireEvent.click(screen.getByTestId("settings-telemetry_enabled"));
    await waitFor(() => expect(calls.some(([m]) => m === "settings.write")).toBe(true));
    expect(calls.find(([m]) => m === "settings.write")![1]).toMatchObject({ tier: "global", changes: { telemetry_enabled: false }, revision: "r1" });
  });

  it("shows the advanced values with the tier that set them and the file to edit, and offers no control", async () => {
    const advanced = (key: string, effective: unknown, source: string | null) => ({ key, type: "json", section: "advanced", writable: [], effective, source, tiers: { global: source === "global" ? effective : null, scene: null, project: null } });
    const fields = [advanced("execution", { max_agent_depth: 3 }, "global"), advanced("hooks_config", { PreToolUse: 2 }, "project"), advanced("team", null, null), flag];
    const host = hostOf((method) => (method === "settings.describe" ? { paths: { global: "/g/settings.json", project: "/p/.agix/settings.json" }, fields, revision: "r1" } : {}));
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    const section = r.at("settings.section").find((s) => s.id === "advanced")!;
    render(<HarnixUiProvider client={client} strings={en}>{section.render()}</HarnixUiProvider>);
    expect((await screen.findByTestId("settings-execution")).textContent).toBe('{"max_agent_depth":3}');
    expect(screen.getByTestId("settings-execution").dataset.source).toBe("global");
    expect(screen.getByTestId("settings-hooks_config").textContent).toBe('{"PreToolUse":2}');
    expect(screen.getByTestId("settings-hooks_config").dataset.source).toBe("project");
    expect(screen.getByTestId("settings-team").textContent).toBe("not set");
    // Only the advanced fields; no input to change one; a path for each tier that has a file.
    expect(screen.queryByTestId("settings-telemetry_enabled")).toBeNull();
    expect(document.querySelector("[data-testid=advanced] input, [data-testid=advanced] select")).toBeNull();
    expect(screen.getByTestId("settings-file-global").textContent).toBe("/g/settings.json");
    expect(screen.getByTestId("settings-file-project").textContent).toBe("/p/.agix/settings.json");
    expect(screen.getByTestId("settings-file-copy-project")).toBeTruthy();
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
