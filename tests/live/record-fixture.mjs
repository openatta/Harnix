// Record one fixture against a real model.
//
//   node tests/live/record-fixture.mjs calls-a-tool "Use the Bash tool to run: echo hi"
//   node tests/live/record-fixture.mjs --scene code streams-output "…"
//
// Needs `.env`. Answers any permission prompt with permit, so a recording of a
// tool call actually contains the call rather than stopping at the ask.
//
// The backend is the one the suites start (`startHarnix`, `connect`): the same
// binary (`HARNIX_BIN`), flags, HOME and project layout, and the scene the
// suites replay in (`harnix` unless `--scene` says otherwise). It runs in
// `<STRICT_ROOT>/<name>` rather than a temporary directory, because the prompt
// names the directories: replayed there with the same message
// (`api/strict-replay.spec.ts`), the requests are the recorded ones byte for
// byte, and `--strict` passes.

import { cpSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { RECORDINGS, STRICT_ROOT, hasModel, loadEnv } from "../../ui/packages/testkit/src/env.ts";
import { startHarnix } from "../../ui/packages/testkit/src/launcher.ts";
import { connect, runTurn } from "../../ui/packages/testkit/src/api.ts";

/**
 * Several recordings as one: the first one's header, then each one's records, its sequence numbers
 * moved past the ones before it (a chunk names its call by number, so they must not collide).
 */
function merge(froms, to) {
  mkdirSync(join(to, "blobs"), { recursive: true });
  const lines = [];
  let offset = 0;
  for (const [i, from] of froms.entries()) {
    cpSync(join(from, "blobs"), join(to, "blobs"), { recursive: true, force: false });
    const records = readFileSync(join(from, "calls.jsonl"), "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l));
    let top = 0;
    for (const record of records) {
      if (record.type === "recording") {
        if (i === 0) lines.push(JSON.stringify(record));
        continue;
      }
      for (const key of ["seq", "seq0", "call"]) {
        if (typeof record[key] === "number") {
          record[key] += offset;
          top = Math.max(top, record[key] + (record.texts?.length ?? record.args?.length ?? 1));
        }
      }
      lines.push(JSON.stringify(record));
    }
    offset = Math.max(offset, top) + 1;
  }
  writeFileSync(join(to, "calls.jsonl"), lines.join("\n") + "\n");
}

const argv = process.argv.slice(2);
let scene = "harnix";
const at = argv.indexOf("--scene");
if (at !== -1) [, scene] = argv.splice(at, 2);
const [name, prompt] = argv;
if (!name || !prompt || !scene) {
  console.error('usage: node tests/live/record-fixture.mjs [--scene <scene>] <name> "<prompt>"');
  process.exit(2);
}

const env = loadEnv();
if (!hasModel(env)) {
  console.error("no model in .env — a fixture is a recording of a real call");
  process.exit(2);
}

const backend = await startHarnix({ env, topologies: ["single_duplex"], fixedScratch: join(STRICT_ROOT, name) });
try {
  const api = await connect(backend, { topology: "single_duplex" });
  const session = await api.call("session.create", { scene, project_root: backend.project });
  await api.call("harnix.attach", { session_id: session.session_id });
  const { asks } = await runTurn(api, { session_id: session.session_id }, prompt, { timeout: 180_000 });
  const kinds = (kind) => api.eventsOfKind(kind, session.session_id).length;
  console.log(`  scene=${scene}  tool_use=${kinds("tool_use")}  prompts=${asks.length}  settled=true`);
  api.close();

  // Ended before copying: the engine writes the recording out as it stops.
  await backend.halt();
  const recorded = join(backend.scratch, "agix", "recordings");
  const sessions = readdirSync(recorded);
  if (!sessions.includes(session.session_id)) throw new Error(`no recording for ${session.session_id} in ${sessions.join(", ")}`);
  const to = join(RECORDINGS, name);
  rmSync(to, { recursive: true, force: true });
  // The session first, then any it started — a sub-agent records as a session of its own — in one
  // recording: replay binds live sessions to recorded ones in the order they first call.
  merge([session.session_id, ...sessions.filter((s) => s !== session.session_id).sort()].map((s) => join(recorded, s)), to);
  console.log(`  → tests/live/fixtures/recordings/${name}${sessions.length > 1 ? ` (${sessions.length} sessions)` : ""}`);
} finally {
  await backend.dispose();
}
