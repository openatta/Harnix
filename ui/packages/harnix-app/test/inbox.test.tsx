/**
 * 待我处理: the hub's asks as items, the sources registered at `harnix-app.inbox` put together and
 * followed, one project's part of them, the entry at the top of the left column opening the list,
 * a row opening where it is answered, and the system space's count linked to it.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { HostProvider, Registry, type HostApi } from "@harnix/react";
import type { InboxRow } from "@harnix/core";
import type { ReactNode } from "react";

import { INBOX, Inbox, askItem, extension, register, sameDir, type InboxItem, type InboxSource } from "../src/index.tsx";

afterEach(() => {
  cleanup();
  localStorage.clear();
});

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const ROW: InboxRow = {
  instance: "main",
  session_id: "s1",
  space: "sessions",
  waiting: "approval",
  prompt: { kind: "prompt", prompt_id: "p1", tool_name: "Bash", tool_use_id: "u1" },
  name: "fix the build",
  project_root: "/work/alpha",
  started_by: "user",
};

/** A host whose `harnix.inbox` topic the test pushes, and whose dialogs and opens it records. */
function fakeHost() {
  const topics = new Map<string, (value: unknown) => void>();
  const opened: unknown[][] = [];
  const shown: ReactNode[] = [];
  const host = {
    topics,
    opened,
    shown,
    push: (topic: string, value: unknown) => act(() => topics.get(topic)?.(value)),
    call: async (method: string) => {
      if (method === "harnix.sessions.list") return { sessions: [] };
      if (method === "harnix-app.workspaces.list") return { workspaces: [], prefs: {}, sessions: {} };
      return {};
    },
    subscribe: (topic: string, _params: unknown, fn: (value: unknown) => void) => {
      topics.set(topic, fn);
      return () => topics.delete(topic);
    },
    events: { onTurnSettled: () => () => undefined, onSessionEvent: () => () => undefined, onHostEvent: () => () => undefined },
    session: { current: () => null, context: () => null, onChange: () => () => undefined, open: async (...args: unknown[]) => void opened.push(args) },
    spaces: { current: () => "sessions" },
    dialogs: {
      open: async (render: (close: (v: unknown) => void) => ReactNode) => {
        shown.push(render(() => undefined));
        return null;
      },
      menu: async () => undefined,
    },
    shell: { notice: () => undefined },
    defaults: { instance: "main", space: "sessions" },
  };
  return host as unknown as HostApi & { opened: unknown[][]; shown: ReactNode[]; push(topic: string, value: unknown): void; topics: Map<string, unknown> };
}

/** A source the test changes. */
function source(id: string, items: InboxItem[]) {
  const listeners = new Set<() => void>();
  const s: InboxSource & { set(next: InboxItem[]): void; opened: string[] } = {
    id,
    items: () => items,
    subscribe: (fn) => {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    open: (item) => void s.opened.push(item.id),
    opened: [],
    set(next) {
      items = next;
      for (const fn of [...listeners]) fn();
    },
  };
  return s;
}

describe("the hub's asks", () => {
  it("say what they wait for, what is waiting and a line more", () => {
    const permission = askItem(ROW, 0);
    expect(permission).toMatchObject({ id: "main/s1/p1", kind: "approval", title: "fix the build", detail: "Bash", project_root: "/work/alpha" });
    const question = askItem({ ...ROW, waiting: "answer", name: null, prompt: { kind: "prompt", prompt_id: "p2", prompt_type: "question", question: "Which  branch?\nmain or dev" } }, 1);
    expect(question.kind).toBe("answer");
    expect(question.detail).toBe("Which branch? main or dev");
    expect(question.title).not.toBe("");
    const form = askItem({ ...ROW, waiting: "answer", prompt: { kind: "prompt", prompt_id: "p3", prompt_type: "mcp_elicitation", server: "github", message: "Pick a repo" } }, 2);
    expect(form).toMatchObject({ kind: "elicitation", detail: "github · Pick a repo" });
    const plan = askItem({ ...ROW, waiting: "plan", prompt: { kind: "prompt", prompt_id: "p4", tool_name: "ExitPlanMode" } }, 3);
    expect(plan).toMatchObject({ kind: "plan", detail: null });
  });

  it("a trailing slash does not make another directory", () => {
    expect(sameDir("/work/alpha/", "/work/alpha")).toBe(true);
    expect(sameDir("/work/alpha", "/work/alphabet")).toBe(false);
    expect(sameDir(null, "/work/alpha")).toBe(false);
  });
});

describe("the list", () => {
  it("puts the sources together, follows each, keeps one project's part, and lets go of one withdrawn", () => {
    const r = new Registry();
    r.activate("harnix-app", () => r.definePoint(INBOX, { kind: "register" }));
    const a = source("a.asks", [{ id: "1", kind: "approval", label: "Approval", title: "one", project_root: "/work/alpha" }]);
    const b = source("b.waiting", [{ id: "2", kind: "confirm", label: "Confirm", title: "two", project_root: "/work/beta/" }]);
    r.activate("a", () => r.register(INBOX, a, "a"));
    r.activate("b", () => r.register(INBOX, b, "b"));
    r.settle();
    const inbox = new Inbox(r);
    const told = vi.fn();
    inbox.subscribe(told);
    inbox.start();
    expect(inbox.getSnapshot().map((e) => e.item.id)).toEqual(["1", "2"]);
    expect(inbox.of("/work/beta").map((e) => e.item.id)).toEqual(["2"]);
    b.set([]);
    expect(inbox.getSnapshot().map((e) => e.item.id)).toEqual(["1"]);
    // The same items again: nothing to draw.
    const before = inbox.getSnapshot();
    told.mockClear();
    a.set([{ id: "1", kind: "approval", label: "Approval", title: "one", project_root: "/work/alpha" }]);
    expect(told).not.toHaveBeenCalled();
    expect(inbox.getSnapshot()).toBe(before);
    r.withdraw("a");
    expect(inbox.getSnapshot()).toEqual([]);
    inbox.stop();
  });

  it("a source whose items throw costs only itself", () => {
    const r = new Registry();
    r.activate("harnix-app", () => r.definePoint(INBOX, { kind: "register" }));
    const good = source("a.asks", [{ id: "1", kind: "approval", label: "Approval", title: "one" }]);
    const broken: InboxSource = { ...source("b.broken", []), items: () => { throw new Error("no"); } };
    r.activate("a", () => r.register(INBOX, good, "a"));
    r.activate("b", () => r.register(INBOX, broken, "b"));
    r.settle();
    const fail = vi.spyOn(r, "fail");
    const inbox = new Inbox(r);
    inbox.start();
    expect(inbox.getSnapshot().map((e) => e.item.id)).toEqual(["1"]);
    expect(fail).toHaveBeenCalledWith(INBOX, expect.objectContaining({ id: "b.broken" }), expect.any(Error));
  });
});

describe("in the Harnix app", () => {
  async function drawn(host: HostApi, extra?: (r: Registry) => void) {
    const r = new Registry();
    r.activate(extension, () => register(r, host));
    extra?.(r);
    r.settle();
    const space = r.at("space").find((s) => s.id === "sessions")!;
    const page = render(<HostProvider host={host}>{space.sidebar!.render!()}</HostProvider>);
    await settle();
    return { r, page };
  }

  it("the left column's entry shows while anything waits, and opens the list; a row opens its session where it belongs", async () => {
    const host = fakeHost();
    const { page } = await drawn(host);
    expect(page.queryByTestId("inbox-open")).toBeNull();
    host.push("harnix.inbox", { items: [ROW] });
    const entry = page.getByTestId("inbox-open");
    expect(entry.dataset.count).toBe("1");
    fireEvent.click(entry);
    await settle();
    expect(host.shown).toHaveLength(1);
    const list = render(<HostProvider host={host}>{host.shown[0]}</HostProvider>);
    const row = list.getByTestId("inbox-item");
    expect(row.dataset.kind).toBe("approval");
    expect(row.textContent).toContain("fix the build");
    expect(row.textContent).toContain("alpha");
    fireEvent.click(row);
    await settle();
    expect(host.opened).toEqual([[{ instance: "main", sessionId: "s1" }, { switchSpace: true }]]);
    // Answered: nothing waits, and the entry goes.
    host.push("harnix.inbox", { items: [] });
    expect(page.queryByTestId("inbox-open")).toBeNull();
  });

  it("where the system space defines its metrics link, the count opens the list", async () => {
    const host = fakeHost();
    const { r } = await drawn(host, (r) => r.activate("system", () => r.definePoint("system.metrics.link", { kind: "register" })));
    const links = r.list<{ metric: string; open(): void }>("system.metrics.link");
    expect(links.map((l) => [l.id, l.metric])).toEqual([["harnix-app.inbox", "waiting"]]);
    links[0]!.open();
    await settle();
    expect(host.shown).toHaveLength(1);
  });
});
