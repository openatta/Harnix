/**
 * The home page: its blocks by order — this package's and others' between them, a wide one marked,
 * one that throws costing only itself — each told how many things wait and given the way to them;
 * the recent sessions, newest first and the archived left out, each opening in its own space; and
 * first, while anything of it is unfinished, the first run: the missing model said, and the
 * `home.card` steps the empty page draws.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import type { HarnixClient } from "@harnix/core";
import { HarnixUiProvider, HostProvider, Registry, dismissSetupStep, en, resetSetupSteps, type HostApi } from "@harnix/react";

import { HOME, HOME_PANELS, INBOX, PROJECT_STEP, SCENE_STEP, extension, recentSessions, register, type HomeContext } from "../src/index.tsx";

afterEach(() => {
  cleanup();
  localStorage.clear();
  resetSetupSteps();
});

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const SESSIONS = [
  { session_id: "s-old", instance: "main", name: "older", project_root: "/work/alpha", last_active: "2026-01-01T00:00:00Z" },
  { session_id: "s-new", instance: "main", name: "newer", project_root: null, last_active: "2026-03-01T00:00:00Z" },
];

/** What hello says of the engine: a provider configured, or none. */
const WITH_MODEL = { scene: "harnix", providers: { default_provider: "a", providers: { a: { default_model: "m" } } } };
const NO_MODEL = { scene: "harnix", providers: { providers: {} } };

function fakeHost(options: { engine?: unknown; workspaces?: unknown[] } = {}) {
  const modal = new Set<(open: boolean) => void>();
  const host = {
    opened: [] as unknown[],
    switched: [] as string[],
    asked: [] as unknown[],
    sections: [] as (string | undefined)[],
    engine: options.engine ?? WITH_MODEL,
    closeModal: () => modal.forEach((fn) => fn(false)),
    call: async (method: string) => {
      if (method === "harnix.sessions.list") return { sessions: SESSIONS };
      if (method === "harnix-app.workspaces.list") return { workspaces: options.workspaces ?? [], prefs: {}, sessions: {} };
      if (method === "harnix.hello") return { engine: host.engine, scenes: [{ scene: "harnix", name: "Harnix", active: true }] };
      return {};
    },
    modal: {
      isOpen: () => false,
      onChange: (fn: (open: boolean) => void) => (modal.add(fn), () => modal.delete(fn)),
    },
    settings: { open: (section?: string) => void host.sections.push(section) },
    subscribe: () => () => undefined,
    events: { onTurnSettled: () => () => undefined, onSessionEvent: () => () => undefined, onHostEvent: () => () => undefined },
    session: { current: () => null, context: () => null, onChange: () => () => undefined, open: async (ref: unknown, options: unknown) => void host.opened.push([ref, options]) },
    spaces: { current: () => HOME, switch: async (id: string) => void host.switched.push(id) },
    dialogs: { menu: async () => undefined, open: async (_render: unknown, options: unknown) => (host.asked.push(options), null) },
    shell: { notice: () => undefined },
    defaults: { instance: "main", space: "sessions" },
  };
  return host as unknown as HostApi & { opened: unknown[]; switched: string[]; asked: unknown[]; sections: (string | undefined)[]; engine: unknown; closeModal(): void };
}

/** The home page, drawn as the shell draws it: inside the page's client, which says hello. */
async function drawHome(host: HostApi, r: Registry) {
  const home = r.at("space").find((s) => s.id === HOME)!;
  const client = { status: "open", detail: undefined, onStatus: () => () => undefined, call: (method: string) => host.call(method, {}) } as unknown as HarnixClient;
  const page = render(
    <HarnixUiProvider client={client} strings={en}>
      <HostProvider host={host}>{home.center!.overview!()}</HostProvider>
    </HarnixUiProvider>,
  );
  await settle();
  await settle();
  return page;
}

describe("the home page", () => {
  it("draws the blocks by order, others' between this package's; tells each what waits; one that throws costs only itself", async () => {
    const host = fakeHost();
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    const told: HomeContext[] = [];
    r.activate("test", () => {
      r.register(HOME_PANELS, { id: "test.stats", order: 10, wide: true, title: () => "Stats", render: (c: HomeContext) => (told.push(c), <span data-testid="test-stats">{c.waiting}</span>) }, "test");
      r.register(HOME_PANELS, { id: "test.broken", order: 25, title: () => "Broken", render: () => { throw new Error("no"); } }, "test");
      // Something waits, from a source of 待我处理.
      r.register(INBOX, { id: "test.waits", items: () => [{ id: "w1", kind: "confirm", label: "Confirm", title: "a run" }], subscribe: () => () => undefined, open: () => undefined }, "test");
    });
    r.settle();
    const fail = vi.spyOn(r, "fail");
    await settle();
    const home = r.at("space").find((s) => s.id === HOME)!;
    const page = render(<HostProvider host={host}>{home.center!.overview!()}</HostProvider>);
    await settle();
    const blocks = [...page.container.querySelectorAll<HTMLElement>('[data-testid="home-block"]')];
    expect(blocks.map((el) => el.dataset.block)).toEqual(["harnix-app.setup", "test.stats", "harnix-app.inbox", "test.broken", "harnix-app.sessions"]);
    expect(blocks[1]!.classList.contains("wide")).toBe(true);
    expect(fail).toHaveBeenCalledWith(HOME_PANELS, expect.objectContaining({ id: "test.broken" }), expect.any(Error));

    // The count of what waits, and the way to the whole list.
    expect(page.getByTestId("test-stats").textContent).toBe("1");
    told.at(-1)!.openWaiting();
    expect(host.asked).toHaveLength(1);

    // The recent sessions, newest first, each opening in its own space; then the way to them all.
    const rows = page.getAllByTestId("home-session");
    expect(rows.map((el) => el.dataset.sid)).toEqual(["s-new", "s-old"]);
    fireEvent.click(rows[0]!);
    expect(host.opened).toEqual([[{ instance: "main", sessionId: "s-new" }, { switchSpace: true }]]);
    fireEvent.click(page.getByTestId("home-sessions-all"));
    expect(host.switched).toEqual(["sessions"]);
  });

  it("recent sessions leave the archived out and stop at the limit", () => {
    const rows = [
      { session_id: "a", instance: "main", last_active: "2026-01-02T00:00:00Z" },
      { session_id: "b", instance: "main", last_active: "2026-01-03T00:00:00Z", archived: true },
      { session_id: "c", instance: "main", last_active: "2026-01-01T00:00:00Z" },
    ] as never;
    expect(recentSessions(rows).map((s) => s.session_id)).toEqual(["a", "c"]);
    expect(recentSessions(rows, 1).map((s) => s.session_id)).toEqual(["a"]);
  });
});

describe("the first run, on the home page", () => {
  it("says no model is configured, with the way to it, and draws the unfinished steps the empty page does", async () => {
    const host = fakeHost({ engine: NO_MODEL });
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    // Another package's step, as the settings' model step is: drawn here by its order.
    r.activate("test", () => r.register("home.card", { id: "test.step", order: 10, render: () => <div data-testid="test-step">step</div> }, "test"));
    r.settle();
    const page = await drawHome(host, r);
    const block = page.container.querySelector<HTMLElement>('[data-block="harnix-app.setup"]')!;
    expect(block.classList.contains("wide")).toBe(true);
    expect(block.querySelector("h3")!.textContent).toBe("Get started");
    fireEvent.click(page.getByTestId("model-setup-open"));
    expect(host.sections).toEqual(["models"]);
    const cards = [...block.querySelectorAll<HTMLElement>(".hx-home-card")].map((el) => el.dataset.card);
    expect(cards).toEqual(["test.step", PROJECT_STEP, SCENE_STEP]);
    expect(block.querySelector(`[data-card="${PROJECT_STEP}"] [data-state="todo"]`)).toBeTruthy();
    // A model configured since, read again once the settings dialog closes: the note goes.
    (host as unknown as { engine: unknown }).engine = WITH_MODEL;
    await act(async () => host.closeModal());
    await settle();
    expect(page.queryByTestId("model-setup")).toBeNull();
  });

  it("draws nothing in the block once every step is done or closed, so the block is not shown", async () => {
    const host = fakeHost({ workspaces: [{ id: "w1", path: "/work/alpha" }] });
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    r.settle();
    dismissSetupStep(SCENE_STEP);
    const page = await drawHome(host, r);
    const body = page.getByTestId("home-setup");
    expect(page.queryByTestId("model-setup")).toBeNull();
    // Each card is there and empty: the stylesheet hides an empty card, and a block with none left.
    expect([...body.children].every((el) => el.innerHTML === "")).toBe(true);
    expect(body.children.length).toBe(2);
  });
});
