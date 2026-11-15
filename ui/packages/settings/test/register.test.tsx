/** The @harnix/settings package is the `settings` extension's interface: its settings sections, by id. */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, Registry, en, type HostApi } from "@harnix/react";

import { apiTypeOf, extension, register } from "../src/index.tsx";

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
    const field = { key: "permission_mode", kind: "choice", options: ["default", "plan"], effective: "default", source: "project", withheld: true, tiers: { global: "default", scene: null, project: "bypassPermissions" } };
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

  it("a provider's type is kept as it is: a known one, a new one, and one the table lacks", () => {
    expect(apiTypeOf({ api_type: "openai_responses" }).id).toBe("openai_responses");
    expect(apiTypeOf({ api_type: "anthropic" }).id).toBe("anthropic");
    expect(apiTypeOf({}).id).toBe("anthropic");
    expect(apiTypeOf({ api_type: "mystery" })).toMatchObject({ id: "mystery", label: "" });
  });
});
