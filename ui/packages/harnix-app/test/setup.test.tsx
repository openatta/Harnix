/**
 * The Harnix app's first-run steps on the empty page: a project (done once there is one — shown as
 * done when it was made here, not drawn when there already was one), and what a conversation starts
 * with — the default scene, the permission mode the engine's settings give, `code` offered beside
 * it — until it is closed. Neither says anything of trust: that is the banner's.
 */

import type { ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, HostProvider, Registry, en, resetSetupSteps, type HostApi } from "@harnix/react";

import { PROJECT_STEP, SCENE_STEP, extension, register } from "../src/index.tsx";

afterEach(() => {
  cleanup();
  localStorage.clear();
  resetSetupSteps();
});

type Call = [string, Record<string, unknown>];

const HELLO = {
  engine: { scene: "harnix", active_scenes: ["code", "harnix"] },
  scenes: [
    { scene: "code", name: "Code", description: "a coding agent", active: true, requires_project: true },
    { scene: "harnix", name: "Harnix", description: "a general assistant", active: true },
  ],
};

function fakeHost(workspaces: { id: string; path: string; title?: string }[]) {
  const calls: Call[] = [];
  const opened: ((close: (r: unknown) => void) => ReactNode)[] = [];
  const sections: (string | undefined)[] = [];
  const switched: string[] = [];
  const host = {
    calls,
    opened,
    sections,
    switched,
    workspaces,
    call: async (method: string, params: Record<string, unknown> = {}) => {
      calls.push([method, params]);
      if (method === "harnix.sessions.list") return { sessions: [] };
      if (method === "harnix-app.workspaces.list") return { workspaces: host.workspaces, prefs: {}, sessions: {} };
      if (method === "harnix-app.recentProjects") return {};
      if (method === "harnix-app.listDirectory") return { path: "/work", breadcrumbs: [], entries: [] };
      if (method === "settings.describe") return { fields: [{ key: "permission_mode", effective: "acceptEdits" }] };
      return {};
    },
    subscribe: () => () => undefined,
    events: { onTurnSettled: () => () => undefined },
    session: { current: () => null, context: () => null, onChange: () => () => undefined },
    spaces: { switch: async (id: string) => void switched.push(id) },
    settings: { open: (section?: string) => void sections.push(section) },
    shell: { notice: () => undefined },
    dialogs: { open: async (render: (close: (r: unknown) => void) => ReactNode) => void opened.push(render) },
    defaults: { instance: "main", space: "sessions" },
  };
  return host as unknown as HostApi & { calls: Call[]; opened: typeof opened; sections: typeof sections; switched: string[]; workspaces: typeof workspaces };
}

const client = {
  status: "open",
  detail: undefined,
  onStatus: () => () => undefined,
  call: async (method: string) => (method === "harnix.hello" ? HELLO : {}),
} as unknown as HarnixClient;

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

async function mounted(workspaces: { id: string; path: string; title?: string }[]) {
  const host = fakeHost(workspaces);
  const r = new Registry();
  r.activate(extension, () => register(r, host));
  const card = (id: string) => r.at("home.card").find((c) => c.id === id)!;
  const page = render(
    <HarnixUiProvider client={client} strings={en}>
      <HostProvider host={host}>
        <div data-card="project">{card(PROJECT_STEP).render()}</div>
        <div data-card="scene">{card(SCENE_STEP).render()}</div>
      </HostProvider>
    </HarnixUiProvider>,
  );
  await settle();
  await settle();
  return { host, r, page };
}

describe("the first-run steps", () => {
  it("are two home cards of the extension's, after the model's", async () => {
    const { r } = await mounted([]);
    const cards = r.at("home.card");
    expect(cards.map((c) => [c.id, c.order, c.owner])).toEqual([
      [PROJECT_STEP, 20, "harnix-app"],
      [SCENE_STEP, 30, "harnix-app"],
    ]);
    // No title: a step that is done draws nothing, and the shell gives it no place.
    expect(cards.every((c) => c.title === undefined)).toBe(true);
  });

  it("asks for a project until there is one, then says it is done", async () => {
    const { host, page } = await mounted([]);
    const step = () => page.container.querySelector(`[data-step="${PROJECT_STEP}"]`) as HTMLElement | null;
    expect(step()?.dataset.state).toBe("todo");
    expect(step()!.textContent).not.toMatch(/trust/i);

    fireEvent.click(page.getByTestId("setup-project-add"));
    expect(host.opened).toHaveLength(1);
    // The dialog adds one, and the list is read again.
    host.workspaces.push({ id: "w1", path: "/work/alpha", title: "alpha" });
    const added = render(<HostProvider host={host}>{host.opened[0]!(() => undefined)}</HostProvider>);
    fireEvent.change(added.container.querySelector("input[type=text]")!, { target: { value: "alpha" } });
    fireEvent.click(added.getByTestId("project-add"));
    await settle();
    await settle();
    expect(host.calls.some(([m, p]) => m === "harnix-app.projects.create" && p.name === "alpha")).toBe(true);
    expect(step()?.dataset.state).toBe("done");
    expect(step()!.textContent).toContain("alpha");
    fireEvent.click(page.getByTestId("setup-project-open"));
    await settle();
    expect(host.switched).toEqual(["projects"]);
  });

  it("draws nothing for a project when there already was one, nor once it is closed", async () => {
    const { page } = await mounted([{ id: "w1", path: "/work/alpha" }]);
    expect(page.container.querySelector('[data-card="project"]')!.innerHTML).toBe("");

    cleanup();
    resetSetupSteps();
    const again = await mounted([]);
    fireEvent.click(again.page.container.querySelector(`[data-step="${PROJECT_STEP}"] [data-testid="setup-dismiss"]`)!);
    expect(again.page.container.querySelector('[data-card="project"]')!.innerHTML).toBe("");
    expect(JSON.parse(localStorage.getItem("harnix.setup.dismissed")!)).toEqual([PROJECT_STEP]);
  });

  it("says the default scene and permission mode, offers code, and is gone once read", async () => {
    const { host, page } = await mounted([]);
    expect(page.getByTestId("setup-scene-default").dataset.scene).toBe("harnix");
    expect(page.getByTestId("setup-scene-default").textContent).toContain("a general assistant");
    // The mode in force is the engine settings', for the default scene.
    expect(host.calls.find(([m]) => m === "settings.describe")?.[1]).toEqual({ scene: "harnix", project_root: null });
    expect(page.getByTestId("setup-scene-mode").dataset.mode).toBe("acceptEdits");
    expect(page.getByTestId("setup-scene-offer").textContent).toContain("Code");

    // `code` opens the new-session dialog on it.
    fireEvent.click(page.getByTestId("setup-scene-code"));
    await settle();
    const dialog = render(
      <HarnixUiProvider client={client} strings={en}>
        <HostProvider host={host}>{host.opened.at(-1)!(() => undefined)}</HostProvider>
      </HarnixUiProvider>,
    );
    await settle();
    expect((dialog.getByTestId("new-scene") as HTMLSelectElement).value).toBe("code");

    fireEvent.click(page.getByTestId("setup-scene-settings"));
    expect(host.sections).toEqual(["permissions"]);
    fireEvent.click(page.getByTestId("setup-scene-got"));
    expect(page.container.querySelector('[data-card="scene"]')!.innerHTML).toBe("");
  });
});
