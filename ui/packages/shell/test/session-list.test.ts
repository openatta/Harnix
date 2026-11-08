/** The session list draws each session once, however often the backend lists it. */

import { describe, expect, it } from "vitest";

import { scriptedApp } from "./scripted";

describe("the session list", () => {
  it("draws a session the engine listed twice once", async () => {
    const { app, client } = scriptedApp();
    const row = { instance: "system", session_id: "s1", last_active: "" };
    client.answer = (method) => (method === "harnix.sessions" ? { sessions: [row, { ...row }, { ...row, session_id: "s2" }] } : {});
    await app.refreshSessions();
    expect(app.snapshot.sessions.map((s) => s.session_id)).toEqual(["s1", "s2"]);
  });

  it("an earlier refresh that answers after a later one does not put the older list back", async () => {
    const { app, client } = scriptedApp();
    let release: () => void = () => undefined;
    const late = new Promise<void>((resolve) => (release = resolve));
    let calls = 0;
    client.answer = (method) => {
      if (method !== "harnix.workspaces.list") return { sessions: [] };
      calls += 1;
      // The first list is the old one, and it is slow; the second already has the new project.
      return calls === 1 ? late.then(() => ({ workspaces: [] })) : { workspaces: [{ id: "w1", path: "/p" }] };
    };
    const first = app.refreshSessions();
    await app.refreshSessions();
    expect(app.snapshot.workspaces.map((w) => w.id)).toEqual(["w1"]);
    release();
    await first;
    expect(app.snapshot.workspaces.map((w) => w.id)).toEqual(["w1"]);
  });
});
