/**
 * A settings section registered `within` one of the shell's own sections is a block of that section,
 * not an entry of the list: drawn after the section's own content under its label, each in its own
 * guard, and opened by its id as the section it is in. A section the shell does not have is refused.
 */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";

import { openSettings, settingsSections } from "../src/settings";
import { scriptedApp } from "./scripted";

afterEach(cleanup);

function drawn(app: ReturnType<typeof scriptedApp>["app"]) {
  return render(<>{app.snapshot.modal?.content}</>).container;
}

describe("a settings section within the diagnostics", () => {
  it("is a block of the diagnostics after the shell's own content, not an entry of the list", () => {
    const { app } = scriptedApp();
    app.registry.register("settings.section", { id: "acme.keys", label: () => "Keys", render: () => <p>keys</p> }, "acme");
    app.registry.register("settings.section", { id: "acme.health", within: "diagnostics", label: () => "Health", render: () => <p data-testid="acme-health">fine</p> }, "acme");
    expect(app.registry.refusals()).toEqual([]);
    const ids = settingsSections(app);
    expect(ids).toContain("acme.keys");
    expect(ids.indexOf("acme.health")).toBeGreaterThan(ids.indexOf("about"));
    openSettings(app, "diagnostics");
    const page = drawn(app);
    const nav = [...page.querySelectorAll('[data-testid="settings-nav"] [data-section]')].map((b) => b.getAttribute("data-section"));
    expect(nav).toContain("acme.keys");
    expect(nav).not.toContain("acme.health");
    const block = page.querySelector('[data-testid="settings-within"][data-section="acme.health"]');
    expect(block?.querySelector("h3")?.textContent).toBe("Health");
    expect(block?.querySelector('[data-testid="acme-health"]')?.textContent).toBe("fine");
  });

  it("opens, by its id, the section it is in", () => {
    const { app } = scriptedApp();
    app.registry.register("settings.section", { id: "acme.health", within: "diagnostics", label: () => "Health", render: () => null }, "acme");
    openSettings(app, "acme.health");
    expect(drawn(app).querySelector('[data-state="current"]')?.getAttribute("data-section")).toBe("diagnostics");
  });

  it("costs only itself when it throws", () => {
    const { app } = scriptedApp();
    app.registry.register("settings.section", { id: "acme.broken", within: "diagnostics", label: () => "Broken", render: () => { throw new Error("no"); } }, "acme");
    app.registry.register("settings.section", { id: "acme.health", within: "diagnostics", label: () => "Health", render: () => <p data-testid="acme-health">fine</p> }, "acme");
    openSettings(app, "diagnostics");
    const page = drawn(app);
    expect(page.querySelector('[data-testid="acme-health"]')).not.toBeNull();
    expect(page.querySelector('[data-testid="settings-page"]')).not.toBeNull();
  });

  it("is refused within a section the shell does not have", () => {
    const { app } = scriptedApp();
    expect(app.registry.register("settings.section", { id: "acme.x", within: "general" as never, label: () => "X", render: () => null }, "acme")).toBe(false);
    expect(app.registry.refusals().map((r) => r.reason)).toEqual([expect.stringContaining("drawn within diagnostics")]);
  });
});
