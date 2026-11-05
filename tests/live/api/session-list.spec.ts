/**
 * Past the engine's in-memory cap (`session_cap`, 32), every session is listed once.
 *
 * The engine lists the sessions it holds, then reads the rest from each project's history it has
 * served, and does not skip what that reading already gave. A project reached under two spellings
 * of one folder — macOS's `/var/…` and `/private/var/…` — was read twice, so each session it had
 * let go of came back twice (Agix, `.local/agix-issue-08-session-list-duplicates.md`). The testkit
 * now hands the engine real paths; this keeps it so.
 */

import { connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

test("forty sessions in the test project are listed once each", TAGS, async () => {
  const backend = await startHarnix(launchFor("replay"));
  const api = await connect(backend);
  try {
    const made = new Set<string>();
    for (let i = 0; i < 40; i += 1) {
      const created = await api.call("session.create", { space: "sessions", project_root: backend.project });
      made.add(created.session_id);
    }
    const { sessions } = await api.call("harnix.sessions", { space: "sessions" });
    const ids = sessions.map((s: { session_id: string }) => s.session_id).filter((id: string) => made.has(id));
    expect(ids.length).toBe(40);
    expect(new Set(ids).size).toBe(40);
  } finally {
    api.close();
    await backend.dispose();
  }
});
