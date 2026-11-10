/** The @harnix/settings package is the `settings` extension's interface: its settings sections, by id. */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, Registry, en, type HostApi } from "@harnix/react";

import { extension, register } from "../src/index.tsx";

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
});
