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
});
