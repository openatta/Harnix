/**
 * A project's overview: the projects space's right column while a project is open and no
 * conversation is; its blocks by order — this package's and another's between them, one that
 * throws costing only itself; the project's asks, conversations, recently changed files, settings
 * in force with their tiers, and trust; and a project opened with nothing open showing it.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, within } from "@testing-library/react";
import { HostProvider, Registry, type HostApi } from "@harnix/react";
import type { SessionEntry, SessionSkills } from "@harnix/core";
import type { ReactNode } from "react";

import { OVERVIEW, PROJECT_PANELS, extension, recentChanges, register, rulesByTier, skillsByTier } from "../src/index.tsx";

afterEach(() => {
  cleanup();
  localStorage.clear();
});

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const WORKSPACES = [{ id: "w1", path: "/work/alpha", title: "alpha" }];
const SESSIONS = [
  { session_id: "s-old", instance: "main", name: "older", project_root: "/work/alpha", last_active: "2026-01-01T00:00:00Z" },
  { session_id: "s-new", instance: "main", name: "newer", project_root: "/work/alpha", last_active: "2026-03-01T00:00:00Z" },
  { session_id: "s-else", instance: "main", name: "elsewhere", project_root: "/work/beta", last_active: "2026-03-02T00:00:00Z" },
];

const changes = (turn: string, ts: string, files: { path: string; added?: number }[]): SessionEntry =>
  ({ ns: "harnix.changes", event: "turn", ts, payload: { turn_id: turn, files: files.map((f) => ({ path: f.path, op: "modify", added: f.added ?? 1, removed: 0 })) } }) as SessionEntry;

function fakeHost() {
  let current: { instance: string; sessionId: string } | null = null;
  const calls: [string, Record<string, unknown>][] = [];
  const record: unknown[] = [];
  const host = {
    calls,
    record,
    settingsOpened: [] as unknown[],
    opened: [] as unknown[],
    tabs: [] as unknown[],
    setCurrent: (c: typeof current) => (current = c),
    call: async (method: string, params: Record<string, unknown> = {}) => {
      calls.push([method, params]);
      if (method === "harnix.sessions.list") return { sessions: SESSIONS };
      if (method === "harnix-app.workspaces.list") return { workspaces: WORKSPACES, prefs: {}, sessions: {} };
      if (method === "harnix-app.projects.status")
        return { project_root: "/work/alpha", trust: "untrusted", items: [{ key: "mcp_servers.gh", category: "mcp", name: "gh" }], categories: ["mcp"], withheld: [{ key: "mcp_servers.gh", category: "mcp", name: "gh" }] };
      if (method === "engine.session.entries")
        return {
          entries:
            params.session_id === "s-new"
              ? [changes("t2", "2026-03-01T10:00:00Z", [{ path: "/work/alpha/src/a.ts", added: 5 }])]
              : [changes("t1", "2026-01-01T10:00:00Z", [{ path: "/work/alpha/src/a.ts" }, { path: "/work/alpha/README.md" }])],
        };
      if (method === "settings.describe")
        return {
          fields: [
            { key: "permission_mode", effective: "acceptEdits", source: "project", withheld: false },
            { key: "permission_rules", effective: [], source: "project", withheld: true, tiers: { global: [{ tool: "Bash" }], scene: null, project: [{ tool: "Edit" }, { tool: "Read" }] } },
          ],
        };
      if (method === "settings.mcp.list")
        return { servers: [{ name: "gh", tier: "project", enabled: true }, { name: "off", tier: "global", enabled: false }, { name: "docs", tier: "global", enabled: true, overridden_by: "project" }] };
      if (method === "harnix.session.skills")
        return { instance: "main", session_id: "s-new", listed: true, skills: [{ name: "a", description: "", tier: "project", withheld: true }, { name: "b", description: "", tier: "builtin", withheld: false }], dirs: [] };
      return {};
    },
    subscribe: () => () => undefined,
    events: { onTurnSettled: () => () => undefined, onSessionEvent: () => () => undefined, onHostEvent: () => () => undefined },
    session: { current: () => current, context: () => null, onChange: () => () => undefined, open: async (ref: unknown) => void host.opened.push(ref) },
    spaces: { current: () => "projects" },
    sidebar: { openTab: (kind: string, options: unknown) => void host.tabs.push([kind, options]) },
    details: { open: (tab: unknown) => void record.push(tab) },
    settings: { open: (section: unknown) => void host.settingsOpened.push(section) },
    dialogs: { menu: async () => undefined, open: async () => null },
    shell: { notice: () => undefined },
    defaults: { instance: "main", space: "sessions" },
  };
  return host as unknown as HostApi & { record: unknown[]; calls: typeof calls; opened: unknown[]; tabs: unknown[]; settingsOpened: unknown[]; setCurrent(c: typeof current): void };
}

async function overview(host: HostApi, extra?: (r: Registry) => void) {
  localStorage.setItem("harnix.project", JSON.stringify("w1"));
  const r = new Registry();
  r.activate(extension, () => register(r, host));
  extra?.(r);
  r.settle();
  const space = r.at("space").find((s) => s.id === "projects")!;
  await settle();
  return { r, space };
}

describe("the pieces", () => {
  it("recent changes: each file once, its latest change, newest first", () => {
    const session = { instance: "main", sessionId: "s", name: null };
    const read = [
      { session, entries: [changes("t1", "2026-01-01T00:00:00Z", [{ path: "/p/a" }, { path: "/p/b" }])] },
      { session: { ...session, sessionId: "s2" }, entries: [changes("t2", "2026-02-01T00:00:00Z", [{ path: "/p/a", added: 9 }])] },
    ];
    const got = recentChanges(read);
    expect(got.map((c) => [c.path, c.turnId, c.added])).toEqual([
      ["/p/a", "t2", 9],
      ["/p/b", "t1", 1],
    ]);
  });

  it("rules by the tier that writes them, skills by where they come from", () => {
    expect(rulesByTier({ key: "permission_rules", tiers: { global: [1], scene: [], project: [1, 2] } })).toMatch(/1.*2/);
    expect(rulesByTier({ key: "permission_rules", tiers: {} })).toBe("");
    const skills = { listed: true, skills: [{ tier: "project" }, { tier: "project" }, { tier: "builtin" }] } as unknown as SessionSkills;
    expect(skillsByTier(skills)).toMatch(/2.*1/);
    expect(skillsByTier({ ...skills, listed: false })).toBe("");
  });
});

describe("the projects space's right column", () => {
  it("is the overview while a project is open and no conversation is", async () => {
    const host = fakeHost();
    const { space } = await overview(host);
    const tabs = () => (space.details!.tabs as () => { id: string }[])().map((x) => x.id);
    expect(tabs()).toEqual([OVERVIEW]);
    host.setCurrent({ instance: "main", sessionId: "s-new" });
    expect(tabs()).toEqual([]);
  });

  it("draws the blocks by order, another package's between this one's; one that throws costs only itself", async () => {
    const host = fakeHost();
    const { r, space } = await overview(host, (r) =>
      r.activate("test", () => {
        r.register(PROJECT_PANELS, { id: "test.tasks", order: 30, title: () => "Tasks", render: (c: { root: string }) => <span data-testid="test-panel">{c.root}</span> }, "test");
        r.register(PROJECT_PANELS, { id: "test.broken", order: 35, title: () => "Broken", render: () => { throw new Error("no"); } }, "test");
      }),
    );
    const fail = vi.spyOn(r, "fail");
    const tab = (space.details!.tabs as () => { render(): ReactNode }[])()[0]!;
    const page = render(<HostProvider host={host}>{tab.render()}</HostProvider>);
    await settle();
    await settle();
    const blocks = [...page.container.querySelectorAll<HTMLElement>('[data-testid="overview-block"]')].map((el) => el.dataset.block);
    expect(blocks).toEqual(["harnix-app.inbox", "harnix-app.sessions", "test.tasks", "harnix-app.changes", "harnix-app.settings", "harnix-app.trust"]);
    expect(page.getByTestId("test-panel").textContent).toBe("/work/alpha");
    expect(fail).toHaveBeenCalledWith(PROJECT_PANELS, expect.objectContaining({ id: "test.broken" }), expect.any(Error));

    // Its conversations, newest first; another project's not among them.
    const sessions = page.getAllByTestId("overview-session").map((el) => el.dataset.sid);
    expect(sessions).toEqual(["s-new", "s-old"]);
    fireEvent.click(page.getAllByTestId("overview-session")[0]!);
    expect(host.opened).toEqual([{ instance: "main", sessionId: "s-new" }]);

    // The files changed lately, each once, relative to the project; one opens its conversation on the changes tab.
    const files = page.getAllByTestId("overview-change").map((el) => el.dataset.path);
    expect(files).toEqual(["src/a.ts", "README.md"]);
    fireEvent.click(page.getAllByTestId("overview-change")[0]!);
    await settle();
    expect(host.tabs).toEqual([["changes", { params: { turn: "t2" } }]]);

    // The settings in force, each with where it comes from; the servers in force only.
    const setting = (id: string) => page.container.querySelector<HTMLElement>(`[data-setting="${id}"]`)!;
    expect(setting("permission_mode").textContent).toMatch(/acceptEdits|自动接受|Accept/i);
    expect(within(setting("permission_rules")).getByText(/held back|被扣下/)).toBeTruthy();
    expect(setting("mcp").textContent).toContain("gh");
    expect(setting("mcp").textContent).not.toContain("off");
    expect(setting("mcp").textContent).not.toContain("docs");
    expect(setting("skills").textContent).toMatch(/1/);
    fireEvent.click(within(setting("mcp")).getByTestId("overview-setting-open"));
    expect(host.settingsOpened).toEqual(["mcp"]);
    const skills = host.calls.find(([m]) => m === "harnix.session.skills");
    expect(skills?.[1]).toMatchObject({ session_id: "s-new", space: "sessions" });

    // Not trusted, with what that holds back, and an offer to trust it.
    expect(page.getByTestId("overview-trust").dataset.trust).toBe("untrusted");
    expect(page.getByTestId("overview-trust-action")).toBeTruthy();
  });

  it("a project opened with no conversation open shows its overview", async () => {
    localStorage.setItem("harnix.project", JSON.stringify(null));
    const host = fakeHost();
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    r.settle();
    const space = r.at("space").find((s) => s.id === "projects")!;
    const page = render(<HostProvider host={host}>{space.sidebar!.render!()}</HostProvider>);
    await settle();
    fireEvent.click(page.getByRole("treeitem", { name: /alpha/ }));
    expect(host.record).toEqual([OVERVIEW]);
  });
});
