/**
 * The composer's models come from hello, not from any extension's settings: whether a turn has a
 * model, which one a new session runs on, and — when there is none — the settings section that
 * says it covers models, whoever registered it.
 */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import type { Hello } from "@harnix/core";

import { openSettingsFor } from "../src/settings";
import { scriptedApp, tick } from "./scripted";

// What these tests read of `hello`; the rest of the generated shape is not drawn here.
const hello = (engine: Partial<Hello["engine"]>): Hello => ({ protocol_version: 1, contrib_api_version: 3, engine, scenes: [], commands: [] }) as unknown as Hello;

afterEach(cleanup);

describe("models from hello", () => {
  it("say whether a turn has a model, and which one a new session runs on", () => {
    const { app } = scriptedApp();
    // Not read: not cried wolf about.
    app.set({ hello: hello({ model: "claude-x", providers: null }) });
    expect(app.modelReady()).toBe(true);
    // Nothing configured, no key: no model, whatever the engine started with.
    app.set({ hello: hello({ model: "claude-x", has_credentials: false, providers: { providers: {}, default_provider: null } }) });
    expect(app.modelReady()).toBe(false);
    expect(app.defaultModel()).toBeNull();
    // The default provider's model.
    app.set({ hello: hello({ model: "claude-x", providers: { providers: { acme: { default_model: "m-1", models: ["m-2"] } }, default_provider: "acme" } }) });
    expect(app.modelReady()).toBe(true);
    expect(app.defaultModel()).toBe("m-1");
  });

  it("are read again from hello when the settings close", async () => {
    const { app, client } = scriptedApp();
    app.set({ hello: hello({ model: "claude-x", providers: { providers: {}, default_provider: null } }) });
    client.answer = (method) => (method === "harnix.hello" ? hello({ providers: { providers: { acme: { default_model: "m-1", models: [] } }, default_provider: "acme" } }) : {});
    openSettingsFor(app, "models");
    app.closeModal();
    await tick();
    expect(client.calls.map((c) => c.method)).toContain("harnix.hello");
    expect(app.defaultModel()).toBe("m-1");
    expect(client.calls.some((c) => c.method === "settings.describe")).toBe(false);
  });

  it("send the reader to the section that covers them, else to the first", () => {
    const { app } = scriptedApp();
    openSettingsFor(app, "models");
    expect(app.snapshot.modal?.key).toBe("settings");
    app.closeModal();
    app.registry.register("settings.section", { id: "keys", label: () => "Keys", render: () => null }, "acme");
    app.registry.register("settings.section", { id: "llm", label: () => "LLM", render: () => null, covers: ["models"] }, "acme");
    openSettingsFor(app, "models");
    const { container } = render(<>{app.snapshot.modal?.content}</>);
    expect(container.querySelector('[data-state="current"]')?.getAttribute("data-section")).toBe("llm");
  });
});
