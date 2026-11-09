/**
 * The recordings made by `record-fixture.mjs` still match what this build asks the model, request for
 * request (`--replay-strict`).
 *
 * Every other suite replays by position, and a request that no longer matches its recording is played
 * back anyway: change the system prompt, a tool's schema or a scene's parameters and those suites stay
 * green. These replay each recording of `recorded.ts` in the directory it was recorded in
 * (`STRICT_ROOT`), on the backend it was recorded on, with the message it was recorded with,
 * strictly, so a call that differs fails the turn and says what differed. When one goes red, the build
 * changed what it sends: re-record it (`node tests/live/record-fixture.mjs <name>`) and look at what
 * the other suites assert of it.
 *
 * Only the recordings made that way are here; the hand-made ones copy another recording's requests
 * (`fixtures/recordings/README.md`). The prompt names the operating system, so these are recorded,
 * and replay strictly, on macOS only.
 */

import { STRICT_ROOT, connect, expect, launchFor, startHarnix, test } from "@harnix/testkit";
import { join } from "node:path";

import { RECORDED, backendFor, play } from "./recorded";

const TAGS = { tag: ["@feature:sessions", "@layer:api", "@strict"] };

for (const entry of RECORDED.filter((r) => !r.varies)) {
  test(`${entry.name} replays strictly: every request is the recorded one`, TAGS, async () => {
    test.skip(process.platform !== "darwin", "recorded on macOS, and the prompt names the operating system");
    const backend = await backendFor(entry, launchFor("replay", { replayStrict: true, fixedScratch: join(STRICT_ROOT, entry.name) }), startHarnix);
    try {
      const api = await connect(backend);
      const { settled } = await play(api, entry, entry.name);
      // A divergent call fails the turn with the recorder's report, which says which part differed.
      expect(settled.params.error, JSON.stringify(settled.params.error)).toBeUndefined();
      // The calls beside the turn (its title, the memory extraction) fail quietly; the log says so.
      expect(backend.log(), "a call beside the turn diverged").not.toContain("diverges from recording");
      api.close();
    } finally {
      await backend.dispose();
    }
  });
}
