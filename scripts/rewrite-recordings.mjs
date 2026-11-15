#!/usr/bin/env node
// Rewrite the 0.4.2 wire names and tool names in recorded model calls, and give each blob it changed
// its new digest.
//
//   node scripts/rewrite-recordings.mjs [--check] [<recordings-dir>…]
//
// A recording keeps the bulky parts of its calls (system prompt blocks, the tool table, messages)
// as blobs named by the first 16 hex digits of the SHA-256 of their bytes, and `calls.jsonl`
// refers to them by that name. A method or topic that is renamed and was said in one — a tool's
// description, the flow guide a `flow_guide` call returned — changes the blob, so its name and
// every reference to it change with it. Strict replay compares requests by those digests: a blob
// edited by hand under its old name no longer matches.
//
// The default directory is `tests/live/fixtures/recordings`; with none to change, nothing is
// written. `--check` writes nothing either way and exits 1 when something would change.

import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, renameSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));

const family = (prefix, names) => names.split(/\s+/).map((n) => prefix + n);

/** The 0.4.2 names that moved, as `[old, new]` (design: 9.2, 9.3). */
const RENAMED = [
  ...family("harnix.settings.", "describe set setProvider").map((o) => [o, o.slice("harnix.".length)]),
  ...family("harnix.plugins.", "upload install uninstall enable disable reload list").map((o) => [o, o.slice("harnix.".length)]),
  ...family("harnix.devices.", "list pair.begin pair.complete revoke").map((o) => [o, o.slice("harnix.".length)]),
  ...family(
    "harnix.system.",
    "context messages.drop messages.edit tasks.stop tasks.message tasks.output compact clear model rename goal goal.set goal.update instances.stop instances.restart instances.create instances.templates",
  ).map((o) => [o, o.slice("harnix.".length)]),
  ...family("harnix.system.", "trace trace.call turns usage calls").map((o) => [o, "observability." + o.slice("harnix.system.".length)]),
  ["harnix.requestHeaders", "observability.calls"],
  ["harnix.process", "system.process"],
  ["harnix.upload.begin", "harnix.bulk.upload"],
  ...family("harnix.workspaces.", "list create update reorder remove").map((o) => [o, "harnix-app." + o.slice("harnix.".length)]),
  ...family("harnix.sessions.", "archive export").map((o) => [o, "harnix-app." + o.slice("harnix.".length)]),
  ...family("harnix.", "prefs.set search listDirectory recentProjects files projects.create").map((o) => [o, "harnix-app." + o.slice("harnix.".length)]),
  ["harnix.sessions", "harnix.sessions.list"],
  ["harnix.sessions.rename", "harnix.session.rename"],
  ["harnix.sessions.setModel", "harnix.session.setModel"],
  ["harnix.sessions.tasks", "harnix.session.tasks"],
  ["harnix.sessions.move", "harnix.session.move"],
  ["harnix.entries.append", "harnix.session.entries.append"],
  ["harnix.prompts.answerer", "harnix.session.answerer"],
  ["harnix.subscribe", "harnix.topics.subscribe"],
  ["harnix.unsubscribe", "harnix.topics.unsubscribe"],
  ...family("harnix.", "attach detach send queue.remove queue.edit").map((o) => [o, "harnix.session." + o.slice("harnix.".length)]),
  ...family(
    "app.flow.",
    "list get history defs def schema catalog validate save delete parts part part.save part.delete hooks focus session sessions guide pause resume dry_run.start dry_run.step dry_run.run dry_run.list start cancel proposal claim heartbeat submit fail retry confirm force rework release runs run dry_runs",
  ).map((o) => [o, o.slice("app.".length)]),
];

/** The engine's own names the hub's rows now spell `engine.<name>`, and the hub's intercepts. */
const ENGINE = [
  ...family("session.", "create interrupt close delete fork").map((o) => [o, "harnix." + o]),
  ["session.respondToPrompt", "harnix.session.respond"],
  ...family(
    "session.",
    "list get history entries resume tasks taskOutput inspectContext goal stopTask messageTask dropMessages editMessage compact clear setModel rename setGoal updateGoal export run_turn subscribe unsubscribe",
  ).map((o) => [o, "engine." + o]),
  ...family("scene.", "list describe activate").map((o) => [o, "engine." + o]),
  ...family("config.", "get getProvider set setProvider update reload").map((o) => [o, "engine." + o]),
  ...family("plugin.", "list install uninstall enable disable reload").map((o) => [o, "engine." + o]),
  ...family("daemon.", "status ping doctor subscribeEvents shutdown").map((o) => [o, "engine." + o]),
  ...family("import.", "run list").map((o) => [o, "engine." + o]),
  ...family("mcp.", "status addServer").map((o) => [o, "engine." + o]),
  ["commands.list", "engine.commands.list"],
];

const esc = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const alternation = (pairs) => pairs.map(([old]) => esc(old)).sort((a, b) => b.length - a.length).join("|");
// A name is replaced where it stands whole; the engine's own, only where it is a whole quoted
// string, because a recorded prompt also talks about Agix's methods as Agix's.
const whole = new RegExp(`(?<![\\w.$@/-])(${alternation(RENAMED)})(?![\\w-]|\\.[A-Za-z_])`, "g");
const quoted = new RegExp(`(?<=")(${alternation(ENGINE)})(?=")`, "g");
const to = new Map([...RENAMED, ...ENGINE]);

/** `text` with every renamed name said by its new one. */
export function renamed(text) {
  return text.replace(whole, (name) => to.get(name)).replace(quoted, (name) => to.get(name));
}

/**
 * A package's tools are `ext__<id>__<tool>` in a session (0.4.2: `app__`), and `mcp__ext-<id>__<tool>`
 * in a spawned daemon (0.4.2: `mcp__app-`). Whole names only: a `__` before it is another word's.
 */
export function renamedTools(text) {
  return text.replace(/(?<![\w])app__(?=[a-z][a-z0-9_]*__)/g, "ext__").replace(/(?<![\w])mcp__app-(?=[a-z][a-z0-9_]*__)/g, "mcp__ext-");
}

/** The name a blob is kept under: the first 16 hex digits of the SHA-256 of its bytes. */
export function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex").slice(0, 16);
}

/**
 * One recording directory: each blob that says a renamed name is written under its new digest, and
 * `calls.jsonl` (and any other `.jsonl` beside it) refers to that. The changes, as `old → new`.
 */
export function rewrite(dir, { write = true } = {}) {
  const blobs = join(dir, "blobs");
  if (!existsSync(blobs)) return [];
  const moved = new Map();
  for (const name of readdirSync(blobs)) {
    const path = join(blobs, name);
    const before = readFileSync(path, "utf8");
    const after = renamedTools(renamed(before));
    if (after === before) continue;
    moved.set(name, digest(Buffer.from(after, "utf8")));
    if (write) {
      writeFileSync(path, after);
      renameSync(path, join(blobs, moved.get(name)));
    }
  }
  const changes = [...moved].map(([from, now]) => `${from} → ${now}`);
  for (const file of readdirSync(dir).filter((f) => f.endsWith(".jsonl"))) {
    const path = join(dir, file);
    const before = readFileSync(path, "utf8");
    let text = renamedTools(before);
    for (const [from, now] of moved) text = text.replaceAll(`"${from}"`, `"${now}"`);
    if (text === before) continue;
    if (!moved.size) changes.push(`${file}: tool names`);
    if (write) writeFileSync(path, text);
  }
  return changes;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const args = process.argv.slice(2);
  const check = args.includes("--check");
  const given = args.filter((a) => !a.startsWith("--"));
  const top = given.length ? given : [join(root, "tests", "live", "fixtures", "recordings")];
  const dirs = top.flatMap((d) => readdirSync(d).map((n) => join(d, n)).filter((p) => statSync(p).isDirectory()));
  let changed = 0;
  for (const dir of dirs) {
    const moves = rewrite(dir, { write: !check });
    changed += moves.length;
    for (const move of moves) console.log(`${dir.slice(root.length + 1)}: ${move}`);
  }
  console.log(`${dirs.length} recordings, ${changed} blobs ${check ? "to change" : "rewritten"}`);
  process.exit(check && changed ? 1 : 0);
}
