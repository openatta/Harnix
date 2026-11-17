/** The @harnix/packages package is the `packages` extension's interface: its settings section and
 *  its block of the diagnostics, owned by the extension; the runtime view follows the list's topic
 *  and stops with it. */

import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, Registry, en, type HostApi } from "@harnix/react";

import { PackagesTab, extension, register } from "../src/index.tsx";

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
  it("registers its settings section and its diagnostics block, owned by the extension, and nothing else", async () => {
    const { host, calls } = fakeHost();
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    expect(extension).toBe("packages");
    const sections = r.at("settings.section");
    expect(sections.map((s) => [s.id, s.owner, s.within])).toEqual([
      ["packages", "packages", undefined],
      ["packages.runtime", "packages", "diagnostics"],
    ]);
    r.settle();
    expect(r.refusals()).toEqual([]);
    render(<HarnixUiProvider client={client} strings={en}>{sections[0]!.render()}</HarnixUiProvider>);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toEqual(["packages.list", "packages.catalog"]);
  });

  it("draws the runtime view from the list's topic, and stops following it when it goes", () => {
    const { host, topics } = fakeHost();
    const { unmount } = render(<HarnixUiProvider client={client} strings={en}><PackagesTab host={host} /></HarnixUiProvider>);
    const push = topics.get("packages.list")!;
    act(() =>
      push({
        packages: [{ id: "acme.notes", version: "0.1.0", title: "Notes", enabled: true, registered: true, runtime: { state: "running", calls: { calls: 3, errors: 1, hub_calls: 0, refused: [], recent_errors: [] } } }],
      }),
    );
    const row = screen.getByTestId("packages-runtime-row");
    expect(row.dataset.package).toBe("acme.notes");
    expect(row.dataset.state).toBe("running");
    expect(row.querySelector('[data-field="calls"]')?.textContent).toBe("3 · 1");
    unmount();
    expect(topics.has("packages.list")).toBe(false);
  });

  it("says in red that extension processes are unsandboxed, and shows a process's last lines of stderr", () => {
    const { host, topics } = fakeHost();
    render(<HarnixUiProvider client={client} strings={en}><PackagesTab host={host} /></HarnixUiProvider>);
    const push = topics.get("packages.list")!;
    const row = { id: "acme.notes", version: "0.1.0", enabled: true, registered: true, runtime: { state: "faulted", process: { stderr: ["one", "two"] } } };
    act(() => push({ packages: [{ ...row, sandbox: "off" }], runtime: { sandbox: { mode: "off" } } }));
    expect(screen.getByTestId("packages-sandbox").dataset.mode).toBe("off");
    expect(screen.getByTestId("packages-runtime-row").dataset.sandbox).toBe("off");
    act(() => screen.getByTestId("packages-runtime-row").click());
    expect(screen.getByTestId("packages-runtime-stderr").textContent).toBe("one\ntwo");

    act(() => push({ packages: [row], runtime: { sandbox: { mode: "unavailable", why: "no sandbox-exec" } } }));
    expect(screen.getByTestId("packages-sandbox").textContent).toContain("no sandbox-exec");
    act(() => push({ packages: [row], runtime: { sandbox: { mode: "on", mechanism: "sandbox-exec" } } }));
    expect(screen.queryByTestId("packages-sandbox")).toBeNull();
  });
});
