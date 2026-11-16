/**
 * The points the projects space defines (`harnix-app.project.items`, `harnix-app.project.panels`,
 * `harnix-app.session.badge`), and the tree against a kind another package registers: its rows under the open project between
 * the conversations by activity, marked current when their session is open; a row clicked opened
 * by its kind; its menu; and the project's "+" offering a conversation and what the kind makes.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { HostProvider, Registry, type HostApi, type MenuItem } from "@harnix/react";

import { INBOX, PROJECT_ITEMS, PROJECT_PANELS, SESSION_BADGE, extension, interleave, register, type ProjectItemKind, type ProjectItemRow, type SessionBadge } from "../src/index.tsx";

afterEach(() => {
  cleanup();
  localStorage.clear();
});

const WORKSPACES = [{ id: "w1", path: "/work/alpha", title: "alpha" }];
const SESSIONS = [
  { session_id: "s-old", instance: "main", name: "older", project_root: "/work/alpha", last_active: "2026-01-01T00:00:00Z" },
  { session_id: "s-new", instance: "main", name: "newer", project_root: "/work/alpha", last_active: "2026-03-01T00:00:00Z" },
];

function fakeHost() {
  const menus: MenuItem[][] = [];
  let current: { instance: string; sessionId: string } | null = null;
  const host = {
    menus,
    setCurrent: (c: typeof current) => (current = c),
    call: async (method: string) => {
      if (method === "harnix.sessions.list") return { sessions: SESSIONS };
      if (method === "harnix-app.workspaces.list") return { workspaces: WORKSPACES, prefs: {}, sessions: {} };
      if (method === "harnix-app.projects.status") return null;
      return {};
    },
    subscribe: () => () => undefined,
    events: { onTurnSettled: () => () => undefined, onSessionEvent: () => () => undefined, onHostEvent: () => () => undefined },
    session: { current: () => current, context: () => null, onChange: () => () => undefined, open: async () => undefined },
    spaces: { current: () => "projects" },
    dialogs: { menu: async (_anchor: unknown, items: MenuItem[]) => void menus.push(items), open: async () => null },
    shell: { notice: () => undefined },
    defaults: { instance: "main", space: "sessions" },
  };
  return host as unknown as HostApi & { menus: MenuItem[][]; setCurrent: (c: typeof current) => void };
}

/** A kind of item, with rows that can be changed from the test. */
function taskKind() {
  const listeners = new Set<() => void>();
  let rows: ProjectItemRow[] = [{ id: "r-1", title: "review repo", at: Date.parse("2026-02-01T00:00:00Z"), running: true, session: { instance: "main", sessionId: "main-1" } }];
  const kind: ProjectItemKind & { set(next: ProjectItemRow[]): void; opened: string[]; created: string[] } = {
    id: "test.task",
    title: () => "Tasks",
    rows: (root) => (root === "/work/alpha" ? rows : []),
    subscribe: (fn) => {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    render: (row) => <span data-testid="task-body">{row.title}</span>,
    open: (row) => void kind.opened.push(row.id),
    menu: (row) => [{ id: "x", label: `about ${row.id}`, run: () => undefined }],
    create: { label: () => "New task", icon: "fork", run: (ctx) => void kind.created.push(ctx.root) },
    opened: [],
    created: [],
    set(next) {
      rows = next;
      for (const fn of [...listeners]) fn();
    },
  };
  return kind;
}

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

async function drawn(host: HostApi, kind?: ProjectItemKind) {
  // The project open on this page, as the store reads it when it starts.
  localStorage.setItem("harnix.project", JSON.stringify("w1"));
  const r = new Registry();
  r.activate(extension, () => register(r, host));
  if (kind) r.activate("test", () => r.register(PROJECT_ITEMS, kind, "test"));
  r.settle();
  const space = r.at("space").find((s) => s.id === "projects")!;
  const page = render(<HostProvider host={host}>{space.sidebar!.render!()}</HostProvider>);
  await settle();
  return { r, page };
}

describe("the projects space's points", () => {
  it("defines the item kinds, the overview panels, the session rows' badges and the inbox's sources for others to fill", () => {
    const r = new Registry();
    const host = fakeHost();
    r.activate(extension, () => register(r, host));
    expect(r.definitions().map((d) => [d.name, d.owner, d.kind])).toEqual([
      [PROJECT_ITEMS, "harnix-app", "register"],
      [PROJECT_PANELS, "harnix-app", "render"],
      [SESSION_BADGE, "harnix-app", "render"],
      [INBOX, "harnix-app", "register"],
    ]);
    expect(r.refusals()).toEqual([]);
  });

  it("orders items and conversations together by activity, newest first, what says no time last", () => {
    const sessions = SESSIONS.map((s) => ({ ...s, archived: false })) as never;
    const kind = taskKind();
    const items = [
      { kind: { ...kind, owner: "test" }, row: { id: "a", title: "a", at: Date.parse("2026-02-01T00:00:00Z") } },
      { kind: { ...kind, owner: "test" }, row: { id: "b", title: "b", at: null } },
    ];
    const order = interleave(sessions, items).map((e) => (e.kind === "session" ? e.session.session_id : e.item.row.id));
    expect(order).toEqual(["s-new", "a", "s-old", "b"]);
  });

  it("draws a registered kind's rows under the open project, between the conversations, following its changes", async () => {
    const host = fakeHost();
    const kind = taskKind();
    const { page } = await drawn(host, kind);
    const rows = () => [...page.container.querySelectorAll<HTMLElement>('[data-testid="session-row"], [data-testid="project-item"]')].map((el) => el.dataset.sid ?? el.dataset.item);
    expect(rows()).toEqual(["s-new", "r-1", "s-old"]);
    const item = page.getByTestId("project-item");
    expect(item.dataset.kind).toBe("test.task");
    expect(page.getByTestId("task-body").textContent).toBe("review repo");
    // Its session open: the row is the current one.
    act(() => host.setCurrent({ instance: "main", sessionId: "main-1" }));
    // The count on the project is conversations and items together.
    expect(page.container.querySelector(".prj .c")?.textContent).toBe("3");
    fireEvent.click(item);
    expect(kind.opened).toEqual(["r-1"]);
    // Its menu, on the row's button.
    fireEvent.click(page.getByTestId("project-item-more"));
    await settle();
    expect(host.menus.at(-1)?.map((m) => m.label)).toEqual(["about r-1"]);
    // The rows change: the tree follows.
    act(() => kind.set([]));
    expect(rows()).toEqual(["s-new", "s-old"]);
  });

  it("the project's + offers a conversation, then what each kind makes; the project's menu offers them too", async () => {
    const host = fakeHost();
    const kind = taskKind();
    const { page } = await drawn(host, kind);
    fireEvent.click(page.getByTestId("project-new"));
    await settle();
    const labels = host.menus.at(-1)!.map((m) => m.id);
    expect(labels).toEqual(["session", "item:test.task"]);
    await host.menus.at(-1)![1]!.run();
    expect(kind.created).toEqual(["/work/alpha"]);
    fireEvent.click(page.getByTestId("project-more"));
    await settle();
    expect(host.menus.at(-1)!.map((m) => m.id).slice(0, 2)).toEqual(["new", "item:test.task"]);
  });

  it("with nothing registered: the conversations alone, and + offers only a conversation", async () => {
    const host = fakeHost();
    const { page } = await drawn(host);
    expect(page.queryAllByTestId("project-item")).toEqual([]);
    fireEvent.click(page.getByTestId("project-new"));
    await settle();
    expect(host.menus.at(-1)!.map((m) => m.id)).toEqual(["session"]);
  });

  it("a kind whose rows throw loses only itself", async () => {
    const host = fakeHost();
    const broken: ProjectItemKind = { ...taskKind(), id: "test.broken", rows: () => { throw new Error("no"); } };
    localStorage.setItem("harnix.project", JSON.stringify("w1"));
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    r.activate("test", () => r.register(PROJECT_ITEMS, broken, "test"));
    r.settle();
    const fail = vi.spyOn(r, "fail");
    const space = r.at("space").find((s) => s.id === "projects")!;
    const page = render(<HostProvider host={host}>{space.sidebar!.render!()}</HostProvider>);
    await settle();
    expect(page.getAllByTestId("session-row")).toHaveLength(2);
    expect(fail).toHaveBeenCalledWith(PROJECT_ITEMS, expect.objectContaining({ id: "test.broken" }), expect.any(Error));
  });

  it("a session's row draws the badges registered for it, between its name and its time; one that throws loses only itself", async () => {
    const host = fakeHost();
    localStorage.setItem("harnix.project", JSON.stringify("w1"));
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    const asked: string[] = [];
    const clock: SessionBadge = {
      id: "test.clock",
      render: (session) => {
        asked.push(`${session.instance}/${session.sessionId}`);
        return session.sessionId === "s-new" ? <span data-testid="test-clock" /> : null;
      },
    };
    const broken: SessionBadge = { id: "test.broken", render: () => { throw new Error("no"); } };
    r.activate("test", () => {
      r.register(SESSION_BADGE, clock, "test");
      r.register(SESSION_BADGE, broken, "test");
    });
    r.settle();
    const fail = vi.spyOn(r, "fail");
    const space = r.at("space").find((s) => s.id === "projects")!;
    const page = render(<HostProvider host={host}>{space.sidebar!.render!()}</HostProvider>);
    await settle();
    expect(asked).toEqual(expect.arrayContaining(["main/s-old", "main/s-new"]));
    const row = page.getAllByTestId("session-row").find((el) => el.getAttribute("data-sid") === "s-new")!;
    const badges = row.querySelector('[data-testid="session-badges"]')!;
    expect(badges.querySelector('[data-testid="test-clock"]')).not.toBeNull();
    expect(badges.nextElementSibling?.classList.contains("time")).toBe(true);
    expect(fail).toHaveBeenCalledWith(SESSION_BADGE, expect.objectContaining({ id: "test.broken" }), expect.any(Error));
  });
});
