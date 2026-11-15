/** The session list draws each session once, however often the backend lists it. */

import { describe, expect, it } from "vitest";

import { scriptedApp } from "./scripted";

describe("the session list", () => {
  it("draws a session the engine listed twice once", async () => {
    const { app, client } = scriptedApp();
    const row = { instance: "main", session_id: "s1", last_active: "" };
    client.answer = (method) => (method === "harnix.sessions.list" ? { sessions: [row, { ...row }, { ...row, session_id: "s2" }] } : {});
    await app.refreshSessions();
    expect(app.snapshot.sessions.map((s) => s.session_id)).toEqual(["s1", "s2"]);
  });

  it("an earlier refresh that answers after a later one does not put the older list back", async () => {
    const { app, client } = scriptedApp();
    let release: () => void = () => undefined;
    const late = new Promise<void>((resolve) => (release = resolve));
    let calls = 0;
    client.answer = (method) => {
      if (method !== "harnix.sessions.list") return {};
      calls += 1;
      // The first list is the old one, and it is slow; the second already has the new session.
      return calls === 1 ? late.then(() => ({ sessions: [] })) : { sessions: [{ instance: "main", session_id: "new" }] };
    };
    const first = app.refreshSessions();
    await app.refreshSessions();
    expect(app.snapshot.sessions.map((s) => s.session_id)).toEqual(["new"]);
    release();
    await first;
    expect(app.snapshot.sessions.map((s) => s.session_id)).toEqual(["new"]);
  });

  it("is the front space's: the backend space it claims is the one listed", async () => {
    const { app, client } = scriptedApp();
    app.registry.register("space", { id: "projects", icon: "folder", title: () => "Projects", sessions: "sessions" }, "test");
    await app.switchSpace("projects");
    expect(client.calls.filter((c) => c.method === "harnix.sessions.list").at(-1)?.params).toEqual({ space: "sessions" });
    await app.switchSpace("b");
    expect(client.calls.filter((c) => c.method === "harnix.sessions.list").at(-1)?.params).toEqual({ space: "b" });
  });
});
