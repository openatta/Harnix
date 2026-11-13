/** The @harnix/packages package is the `packages` extension's interface: its settings section and
 *  its console tab, owned by the extension; the tab follows the list's topic and stops with it. */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, Registry, en, type HostApi } from "@harnix/react";

import { extension, register } from "../src/index.tsx";

afterEach(cleanup);

const client = { status: "closed", detail: undefined, onStatus: () => () => undefined, call: async () => ({}) } as unknown as HarnixClient;

function fakeHost() {
  const calls: string[] = [];
  const topics = new Map<string, (value: unknown) => void>();
  const host = {
    call: async (method: string) => {
      calls.push(method);
      return {};
    },
    subscribe: (topic: string, _params: unknown, fn: (value: unknown) => void) => {
      topics.set(topic, fn);
      return () => topics.delete(topic);
    },
    shell: { notice: () => undefined },
    dialogs: { confirm: async () => false, open: async () => null },
  } as unknown as HostApi;
  return { host, calls, topics };
}

describe("@harnix/packages", () => {
  it("registers its settings section and its console tab, owned by the extension", async () => {
    const { host, calls } = fakeHost();
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    expect(extension).toBe("packages");
    const sections = r.at("settings.section");
    expect(sections.map((s) => [s.id, s.owner])).toEqual([["apps", "packages"]]);
    render(<HarnixUiProvider client={client} strings={en}>{sections[0]!.render()}</HarnixUiProvider>);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toEqual(["packages.list", "packages.catalog"]);
  });

  it("puts its tab into a console that is defined, and nowhere when none is", () => {
    const { host } = fakeHost();
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    r.settle();
    expect(r.list("system.console.tab")).toEqual([]);
    expect(r.refusals()).toEqual([]);

    const defined = new Registry();
    defined.activate("system", () => defined.definePoint("system.console.tab", { kind: "register" }));
    defined.activate(extension, () => register(defined, host));
    defined.settle();
    expect(defined.list("system.console.tab").map((c) => [c.id, c.owner])).toEqual([["apps", "packages"]]);
  });

  it("draws the tab from the list's topic, and stops following it when it goes", () => {
    const { host, topics } = fakeHost();
    const r = new Registry();
    r.activate("system", () => r.definePoint("system.console.tab", { kind: "register" }));
    r.activate(extension, () => register(r, host));
    const tab = r.list<{ render(): React.ReactNode }>("system.console.tab")[0]!;
    const { unmount } = render(<HarnixUiProvider client={client} strings={en}>{tab.render()}</HarnixUiProvider>);
    const push = topics.get("packages.list")!;
    act(() =>
      push({
        packages: [{ id: "acme.notes", version: "0.1.0", title: "Notes", enabled: true, registered: true, runtime: { state: "running", calls: { calls: 3, errors: 1, hub_calls: 0, refused: [], recent_errors: [] } } }],
      }),
    );
    const row = screen.getByTestId("system-app");
    expect(row.dataset.app).toBe("acme.notes");
    expect(row.dataset.state).toBe("running");
    expect(row.querySelector('[data-field="calls"]')?.textContent).toBe("3 · 1");
    unmount();
    expect(topics.has("packages.list")).toBe(false);
  });
});
