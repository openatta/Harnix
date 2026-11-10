/**
 * What the Harnix app's interface registers, through the host API alone: the two spaces over one
 * backend space, the grouping by time, archiving and export on the session menu for the sessions
 * they apply to, the `@` files of the open session's project, and a new conversation in the
 * projects space running in the open project.
 */

import { afterEach, describe, expect, it } from "vitest";
import { Registry, t, type ActionTarget, type HostApi } from "@harnix/react";

import { extension, register } from "../src/index.tsx";

type Call = [string, Record<string, unknown>];

const WORKSPACES = [{ id: "w1", path: "/work/alpha", title: "alpha" }];
const SESSIONS = [
  { session_id: "s1", instance: "main", name: "one", project_root: "/work/alpha", last_active: new Date().toISOString() },
  { session_id: "s2", instance: "main", name: "two", project_root: null, last_active: "2020-01-01T00:00:00Z" },
];

function fakeHost(answers: Record<string, unknown> = {}) {
  const calls: Call[] = [];
  let current: { instance: string; sessionId: string } | null = null;
  const host = {
    calls,
    setCurrent: (c: typeof current) => (current = c),
    call: async (method: string, params: Record<string, unknown> = {}) => {
      calls.push([method, params]);
      if (method in answers) return answers[method];
      if (method === "harnix.sessions.list") return { sessions: SESSIONS };
      if (method === "harnix-app.workspaces.list") return { workspaces: WORKSPACES, prefs: {}, sessions: { s2: { archived: true } } };
      return {};
    },
    subscribe: () => () => undefined,
    events: { onTurnSettled: () => () => undefined, onSessionEvent: () => () => undefined, onHostEvent: () => () => undefined },
    session: { current: () => current, context: () => null },
    shell: { notice: () => undefined },
    defaults: { instance: "main", space: "sessions" },
  };
  return host as unknown as HostApi & { calls: Call[]; setCurrent: (c: typeof current) => void };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => localStorage.clear());

describe("@harnix/harnix-app", () => {
  it("is the harnix-app extension's interface: two spaces over the session space, and the grouping", async () => {
    const r = new Registry();
    const host = fakeHost();
    r.activate(extension, () => register(r, host));
    const spaces = r.at("space");
    expect(spaces.map((s) => [s.id, s.sessions ?? s.id, s.order])).toEqual([
      ["sessions", "sessions", 10],
      ["projects", "sessions", 20],
    ]);
    expect(spaces.every((s) => s.owner === "harnix-app" && s.store !== undefined)).toBe(true);
    expect(r.grouping()?.id).toBe("harnix-app.time");
    await settle();
    const groups = r.grouping()!.group(SESSIONS as never);
    expect(groups.map((g) => g.key)).toEqual(["today", "earlier"]);
    expect(host.calls.some(([m, p]) => m === "harnix.sessions.list" && p.space === "sessions")).toBe(true);
  });

  it("archives only the session space's sessions, with the backend space they belong to", async () => {
    const r = new Registry();
    const host = fakeHost();
    r.activate(extension, () => register(r, host));
    r.register("space", { id: "elsewhere", icon: "chat", title: () => "Elsewhere" }, "test");
    await settle();
    const archive = r.at("session.action").find((a) => a.id === "harnix-app.archive")!;
    const exporting = r.at("session.action").find((a) => a.id === "harnix-app.export")!;
    const inProjects: ActionTarget = { kind: "session", session: { instance: "main", sessionId: "s2" }, space: "projects" };
    const elsewhere: ActionTarget = { kind: "session", session: { instance: "main", sessionId: "x" }, space: "elsewhere" };
    const tool: ActionTarget = { ...inProjects, kind: "tool", toolUseId: "t1", tool: "Bash" };
    expect(archive.at).toEqual(["menu"]);
    expect([archive.match(inProjects), archive.match(elsewhere), archive.match(tool)]).toEqual([true, false, false]);
    expect([exporting.match(inProjects), exporting.match(elsewhere), exporting.match(tool)]).toEqual([true, true, false]);
    // s2 is archived: the action takes it back out.
    expect(archive.label(inProjects)).toBe(t("dialog.unarchive"));
    await archive.run(inProjects);
    expect(host.calls.find(([m]) => m === "harnix-app.sessions.archive")?.[1]).toEqual({ instance: "main", session_id: "s2", space: "sessions", archived: false });
  });

  it("offers the open session's project files after @, and nothing without a project", async () => {
    const r = new Registry();
    const host = fakeHost({ "harnix-app.files": { files: [{ path: "src/main.rs", size: 2048 }] } });
    r.activate(extension, () => register(r, host));
    await settle();
    const files = r.at("command").find((c) => c.id === "harnix-app.files")!;
    expect(files.trigger).toBe("@");
    host.setCurrent({ instance: "main", sessionId: "s1" });
    expect(await files.complete("ma")).toEqual([{ label: "src/main.rs", description: "2.0 KB", replacement: "@src/main.rs " }]);
    expect(host.calls.find(([m]) => m === "harnix-app.files")?.[1]).toEqual({ project_root: "/work/alpha", query: "ma" });
    host.setCurrent({ instance: "main", sessionId: "s2" });
    expect(await files.complete("ma")).toEqual([]);
    expect(await files.complete("session:x")).toEqual([]);
  });

  it("starts a conversation in the projects space in the open project", async () => {
    const r = new Registry();
    const host = fakeHost();
    r.activate(extension, () => register(r, host));
    await settle();
    const projects = r.at("space").find((s) => s.id === "projects")!;
    expect(projects.conversation?.newSessionDefaults?.()).toBeUndefined();
    const sessions = r.at("space").find((s) => s.id === "sessions")!;
    expect(sessions.conversation?.newSessionDefaults).toBeUndefined();
    // Opening a project is kept for the next page too.
    localStorage.setItem("harnix.project", JSON.stringify("w1"));
    const again = new Registry();
    again.activate(extension, () => register(again, host));
    await settle();
    expect(again.at("space").find((s) => s.id === "projects")!.conversation?.newSessionDefaults?.()).toEqual({ project_root: "/work/alpha" });
  });

  it("filters by name as the box is typed into, and searches the transcripts on Enter", async () => {
    const r = new Registry();
    const host = fakeHost({ "harnix-app.search": { hits: [{ session_id: "s1", snippet: "…" }], truncated: false } });
    r.activate(extension, () => register(r, host));
    const sessions = r.at("space").find((s) => s.id === "sessions")!;
    const snapshot = () => sessions.store!.getSnapshot() as { search: { query: string; hits: unknown } };
    sessions.sidebar!.search!("on");
    expect(snapshot().search).toMatchObject({ query: "on", hits: null });
    sessions.sidebar!.search!("on", true);
    await settle();
    expect(snapshot().search.hits).toEqual([{ session_id: "s1", snippet: "…" }]);
    sessions.sidebar!.search!("");
    expect(snapshot().search).toMatchObject({ query: "", hits: null });
  });
});
