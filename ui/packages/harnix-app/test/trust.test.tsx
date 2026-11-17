/**
 * A project's trust in the projects space: the shield on a trusted project, its menu offering to
 * trust or to stop trusting, the dialog listing what trust lets in before anything is written, and
 * the banner over a conversation in a project that is not trusted — gone once closed, for good.
 */

import type { ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { HostProvider, Registry, type HostApi, type MenuItem } from "@harnix/react";

import { extension, register } from "../src/index.tsx";

afterEach(() => {
  cleanup();
  localStorage.clear();
});

type Call = [string, Record<string, unknown>];

const WORKSPACES = [
  { id: "w1", path: "/work/alpha", title: "alpha" },
  { id: "w2", path: "/work/beta", title: "beta" },
];
const SESSIONS = [{ session_id: "s1", instance: "main", name: "one", project_root: "/work/alpha" }];

const item = (key: string, category: string, name: string | null) => ({ key, category, name });
const ALPHA = {
  project_root: "/work/alpha",
  trust: "untrusted",
  trust_source: "default",
  matched: null,
  requires_trust: ["hooks_config", "mcp_servers.github"],
  items: [item("hooks_config", "hooks", null), item("mcp_servers.github", "mcp", "github")],
  categories: ["hooks", "mcp"],
  withheld: [item("hooks_config", "hooks", null), item("mcp_servers.github", "mcp", "github")],
};
const BETA = { project_root: "/work/beta", trust: "trusted", trust_source: "host", matched: "/work/beta", requires_trust: [], items: [], categories: [], withheld: [] };

function fakeHost() {
  const calls: Call[] = [];
  const notices: string[] = [];
  const menus: MenuItem[][] = [];
  const opened: ((close: (r: unknown) => void) => ReactNode)[] = [];
  const statuses: Record<string, unknown> = { "/work/alpha": ALPHA, "/work/beta": BETA };
  const host = {
    calls,
    notices,
    menus,
    opened,
    call: async (method: string, params: Record<string, unknown> = {}) => {
      calls.push([method, params]);
      if (method === "harnix.sessions.list") return { sessions: SESSIONS };
      if (method === "harnix-app.workspaces.list") return { workspaces: WORKSPACES, prefs: {}, sessions: {} };
      if (method === "harnix-app.projects.status") return statuses[params.project_root as string];
      if (method === "harnix-app.projects.trust") {
        const root = params.project_root as string;
        const was = statuses[root] as typeof ALPHA;
        statuses[root] = { ...was, trust: params.trusted ? "trusted" : "untrusted", withheld: params.trusted ? [] : was.items };
        return statuses[root];
      }
      return {};
    },
    subscribe: () => () => undefined,
    events: { onTurnSettled: () => () => undefined },
    session: { current: () => null, context: () => null, onChange: () => () => undefined, menu: () => undefined, open: () => undefined },
    shell: { notice: (text: string) => notices.push(text) },
    dialogs: {
      menu: async (_anchor: unknown, items: MenuItem[]) => void menus.push(items),
      open: async (render: (close: (r: unknown) => void) => ReactNode) => void opened.push(render),
    },
    defaults: { instance: "main", space: "sessions" },
  };
  return host as unknown as HostApi & { calls: Call[]; notices: string[]; menus: MenuItem[][]; opened: typeof opened };
}

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

async function mounted() {
  const host = fakeHost();
  const r = new Registry();
  r.activate(extension, () => register(r, host));
  await settle();
  const space = r.at("space").find((s) => s.id === "projects")!;
  return { host, space };
}

describe("a project's trust", () => {
  it("puts a shield on a trusted project, and its menu offers to trust or to stop trusting", async () => {
    const { host, space } = await mounted();
    const page = render(<HostProvider host={host}>{space.sidebar!.render!()}</HostProvider>);
    await settle();
    const project = (id: string) => page.container.querySelector(`[data-project="${id}"]`)!;
    expect(project("w2").querySelector('[data-testid="project-trusted"]')).not.toBeNull();
    expect(project("w1").querySelector('[data-testid="project-trusted"]')).toBeNull();
    // Each project's trust is read once.
    expect(host.calls.filter(([m]) => m === "harnix-app.projects.status").map(([, p]) => p.project_root).sort()).toEqual(["/work/alpha", "/work/beta"]);

    fireEvent.click(project("w1").querySelector('[data-testid="project-more"]')!);
    fireEvent.click(project("w2").querySelector('[data-testid="project-more"]')!);
    expect(host.menus.map((items) => items.map((i) => i.id))).toEqual([
      ["new", "trust", "rename", "remove"],
      ["new", "untrust", "rename", "remove"],
    ]);

    // Stopping needs no confirmation.
    await act(async () => void (await host.menus[1]!.find((i) => i.id === "untrust")!.run()));
    expect(host.calls.at(-1)).toEqual(["harnix-app.projects.trust", { project_root: "/work/beta", trusted: false }]);
    expect(host.notices.at(-1)).toContain("beta");
    expect(project("w2").querySelector('[data-testid="project-trusted"]')).toBeNull();
  });

  it("lists what trust lets in, and writes it only once that is confirmed", async () => {
    const { host, space } = await mounted();
    const page = render(<HostProvider host={host}>{space.sidebar!.render!()}</HostProvider>);
    await settle();
    fireEvent.click(page.container.querySelector('[data-project="w1"] [data-testid="project-more"]')!);
    await act(async () => void (await host.menus[0]!.find((i) => i.id === "trust")!.run()));
    expect(host.opened).toHaveLength(1);

    let closed: unknown = "open";
    const dialog = render(<HostProvider host={host}>{host.opened[0]!((r) => (closed = r))}</HostProvider>);
    const kinds = [...dialog.getByTestId("trust-categories").querySelectorAll("li")].map((li) => [li.dataset.category, li.querySelector(".sub")!.textContent]);
    expect(kinds).toEqual([
      ["hooks", "hooks_config"],
      ["mcp", "github"],
    ]);
    expect(host.calls.some(([m]) => m === "harnix-app.projects.trust")).toBe(false);

    fireEvent.click(dialog.getByTestId("trust-confirm"));
    await settle();
    expect(host.calls.at(-1)).toEqual(["harnix-app.projects.trust", { project_root: "/work/alpha", trusted: true }]);
    expect(closed).toBe(true);
    expect(page.container.querySelector('[data-project="w1"] [data-testid="project-trusted"]')).not.toBeNull();
  });

  it("offers trust over a conversation in an untrusted project until it is closed for that project", async () => {
    const { host, space } = await mounted();
    const banner = () => render(<HostProvider host={host}>{space.conversation!.pinned!({ instance: "main", sessionId: "s1" })}</HostProvider>);
    const first = banner();
    await settle();
    expect(first.getByTestId("trust-banner").textContent).toContain("MCP");
    fireEvent.click(first.getByTestId("trust-banner-trust"));
    await settle();
    expect(host.opened).toHaveLength(1);

    fireEvent.click(first.getByTestId("trust-banner-dismiss"));
    expect(first.queryByTestId("trust-banner")).toBeNull();
    cleanup();
    // Closed, it stays closed: on this page and the next.
    expect(banner().queryByTestId("trust-banner")).toBeNull();
    expect(JSON.parse(localStorage.getItem("harnix.trust.dismissed")!)).toEqual(["/work/alpha"]);
  });
});
