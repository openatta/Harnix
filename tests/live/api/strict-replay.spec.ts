/**
 * The recordings made by `record-fixture.mjs` still match what this build asks the model, request for
 * request (`--replay-strict`).
 *
 * Every other suite replays by position, and a request that no longer matches its recording is played
 * back anyway: change the system prompt, a tool's schema or a scene's parameters and those suites stay
 * green. These replay the recorded message in the directory it was recorded in (`STRICT_ROOT`), strictly,
 * so a call that differs fails the turn and says what differed. When one goes red, the build changed
 * what it sends: re-record it (`node tests/live/record-fixture.mjs <name> "<message>"`, the message
 * below) and look at what the other suites assert of it.
 *
 * Only the recordings made that way are here; the hand-made ones copy another recording's requests
 * (`fixtures/recordings/README.md`). The prompt names the operating system, so these are recorded,
 * and replay strictly, on macOS only.
 */

import { STRICT_ROOT, connect, expect, launchFor, runTurn, startHarnix, test } from "@harnix/testkit";
import { join } from "node:path";

const TAGS = { tag: ["@feature:sessions", "@layer:api", "@strict"] };

/** Each recording and the message it was recorded with. */
export const STRICT: [string, string][] = [
  ["says-ready", "Say exactly: fixture ready"],
  ["calls-a-tool", "Use the Bash tool right now to run exactly: echo harnix-fixture. Call the tool. Do not answer in prose."],
  ["asks-permission", "Use the Bash tool right now to run exactly: rm -f /tmp/harnix-fixture-probe. Call the tool. Do not answer in prose."],
  [
    "runs-a-while",
    'Use the Bash tool right now to run exactly: perl -e "select(undef, undef, undef, 4)"; echo waited. Call the tool. When it returns, reply with exactly one word: waited',
  ],
  [
    "says-markdown",
    'Reply with exactly the following markdown, and nothing else: "Here it is:", then a sh code block containing "echo fixture ready", then a table with the header row "| name | value |" and the row "| answer | fixture ready |", then "See [the docs](https://example.com/docs)."',
  ],
  [
    "spawns-agent",
    "Use the Agent tool exactly once to launch a general-purpose sub-agent with the description 'list files' and the prompt 'Run ls in the current directory and reply with the file names only.' When it returns, reply with one line: 'sub-agent found:' followed by the names.",
  ],
];

for (const [name, message] of STRICT) {
  test(`${name} replays strictly: every request is the recorded one`, TAGS, async () => {
    test.skip(process.platform !== "darwin", "recorded on macOS, and the prompt names the operating system");
    const backend = await startHarnix(launchFor("replay", { replayStrict: true, fixedScratch: join(STRICT_ROOT, name) }));
    try {
      const api = await connect(backend);
      const created = await api.call("session.create", {
        scene: "harnix",
        project_root: backend.project,
        options: { recorder: { name } },
      });
      await api.call("harnix.attach", { session_id: created.session_id });
      const { settled } = await runTurn(api, { session_id: created.session_id }, message);
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
