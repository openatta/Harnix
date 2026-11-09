// Every name a test replays is a recording, and every recording is replayed by something.
//
//   node tests/recordings.mjs
//
// A session names its recording (`options: { recorder: { name } }`) and replay
// finds it by directory. A name that resolves to nothing fails only when that
// session first calls the model — in the middle of some other test's story —
// and a recording nothing names is a fixture nobody keeps current. Neither is
// caught by running the suites, so this reads the sources instead:
//
// - the names the suites pass: `recorder: { name: "…" }`, `join(RECORDINGS, "…")`,
//   and the string given to any helper whose parameter is called `fixture` or
//   `recording` (`newSession(page, "says-ready")`), its default included;
// - the names `harnix-testapp` gives a flow's sessions: the literal ones
//   (`Some("flow-main".into())`), and `flow-<station>` for a station a flow of
//   its declares (`- id: analyze` replays `flow-analyze`).
//
// Each of those names has to be a recording directory with a readable
// `calls.jsonl`, and each recording has to be one of them — or at least be
// named, as a string, somewhere in the suites.

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const recordings = join(root, "tests", "live", "fixtures", "recordings");

function files(dir, keep) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) out.push(...files(path, keep));
    else if (keep(path)) out.push(path);
  }
  return out;
}

const suites = ["api", "ui"].flatMap((d) => files(join(root, "tests", "live", d), (p) => /\.(ts|mjs)$/.test(p)));
const testapp = files(join(root, "crates", "testapp", "src"), (p) => p.endsWith(".rs"));

/** name → where it is used, for the message. */
const used = new Map();
const use = (name, where) => {
  if (!used.has(name)) used.set(name, where);
};
const line = (text, index) => text.slice(0, index).split("\n").length;

/** The arguments of a call whose `(` is at `open`, split at the top level. */
function argsAt(text, open) {
  const args = [];
  let depth = 0;
  let start = open + 1;
  let quote = null;
  for (let i = open + 1; i < text.length; i += 1) {
    const c = text[i];
    if (quote) {
      if (c === "\\") i += 1;
      else if (c === quote) quote = null;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") quote = c;
    else if ("([{".includes(c)) depth += 1;
    else if (")]}".includes(c)) {
      if (depth === 0) {
        args.push(text.slice(start, i).trim());
        return args;
      }
      depth -= 1;
    } else if (c === "," && depth === 0) {
      args.push(text.slice(start, i).trim());
      start = i + 1;
    }
  }
  return args;
}

const literal = (arg) => /^"([a-z0-9][a-z0-9-]*)"$/.exec(arg ?? "")?.[1];

for (const path of suites) {
  const text = readFileSync(path, "utf8");
  const at = (i) => `${relative(root, path)}:${line(text, i)}`;
  for (const m of text.matchAll(/recorder:\s*\{\s*name:\s*"([^"]+)"/g)) use(m[1], at(m.index));
  for (const m of text.matchAll(/join\(RECORDINGS,\s*"([^"]+)"/g)) use(m[1], at(m.index));

  // Helpers that take a recording's name: where in their parameters it is, and its default. One
  // that is exported is looked for in every suite, one that is not in its own file.
  for (const m of text.matchAll(/(export\s+)?(?:async\s+)?function\s+(\w+)\s*\(/g)) {
    const params = argsAt(text, m.index + m[0].length - 1);
    const position = params.findIndex((p) => /^(fixture|recording)\b/.test(p));
    if (position === -1) continue;
    const fallback = /=\s*("[^"]*")/.exec(params[position])?.[1];
    if (literal(fallback)) use(literal(fallback), at(m.index));
    const helper = m[2];
    for (const file of m[1] ? suites : [path]) {
      const body = readFileSync(file, "utf8");
      for (const call of body.matchAll(new RegExp(`\\b${helper}\\(`, "g"))) {
        const name = literal(argsAt(body, call.index + call[0].length - 1)[position]);
        if (name) use(name, `${relative(root, file)}:${line(body, call.index)}`);
      }
    }
  }
}

const stations = new Set();
for (const path of [...testapp, ...suites]) {
  const text = readFileSync(path, "utf8");
  for (const m of text.matchAll(/^\s*-\s*id:\s*([a-z0-9][a-z0-9-]*)\s*$/gm)) stations.add(m[1]);
  if (!path.endsWith(".rs")) continue;
  for (const m of text.matchAll(/Some\("(flow-[a-z0-9-]+)"\.into\(\)\)/g)) use(m[1], `${relative(root, path)}:${line(text, m.index)}`);
}

const problems = [];
const present = readdirSync(recordings).filter((d) => statSync(join(recordings, d)).isDirectory()).sort();
for (const [name, where] of [...used].sort()) {
  const calls = join(recordings, name, "calls.jsonl");
  if (!existsSync(calls)) {
    problems.push(`${where} replays \`${name}\`, which is not a recording (tests/live/fixtures/recordings/${name})`);
    continue;
  }
  try {
    const head = JSON.parse(readFileSync(calls, "utf8").split("\n")[0]);
    if (head.type !== "recording") problems.push(`${name}/calls.jsonl does not start with its recording header`);
  } catch (e) {
    problems.push(`${name}/calls.jsonl: ${e.message}`);
  }
}
// The other way round is looser: a recording some table of cases names (`["app-echo", "echo", …]`)
// is in use without being passed where the patterns above look.
const mentioned = new Set(suites.flatMap((p) => [...readFileSync(p, "utf8").matchAll(/"([a-z0-9][a-z0-9-]*)"/g)].map((m) => m[1])));
for (const name of present) {
  if (used.has(name) || mentioned.has(name)) continue;
  const station = /^flow-(.+)$/.exec(name)?.[1];
  if (station && stations.has(station)) continue;
  problems.push(`tests/live/fixtures/recordings/${name} is replayed by nothing: no test names it, and it is no flow station's`);
}

if (problems.length) {
  console.error(problems.map((p) => `  ${p}`).join("\n"));
  process.exit(1);
}
console.log(`recordings: ${present.length}, each replayed; ${used.size} names in the sources, each a recording`);
