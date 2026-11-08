#!/usr/bin/env node
// An application's pin (design §11.3).
//
//   node harnix/scripts/check-pins.mjs [app-root] [--release]
//
// `harnix/` pins a Harnix commit, and that Harnix pins its engine, checked out
// as `harnix/agix`. What is checked out there against Harnix's pin:
//
// | harnix/agix against Harnix's pin       | result                                   |
// |----------------------------------------|------------------------------------------|
// | the same                               | ok                                       |
// | another commit the engine's remote has | warning; a failure with --release        |
// | a commit no remote has                 | failure                                  |
//
// And `harnix/` itself must point at a commit a Harnix remote has: no
// long-lived patches in an application (§11.5). An application still on the
// old layout (its own `agix/` beside `harnix/`) is judged the same way on that
// checkout, and told to move.

import { execFileSync } from "node:child_process";
import { realpathSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { MOVE_HINT, engineAt } from "./sync-app-deps.mjs";

const harnixRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const git = (cwd, ...args) =>
  execFileSync("git", ["-C", cwd, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();

const tryGit = (cwd, ...args) => {
  try {
    return git(cwd, ...args);
  } catch {
    return null;
  }
};

/** Whether any remote-tracking branch of this repository contains a commit. */
export function onARemote(repo, commit) {
  const branches = tryGit(repo, "branch", "-r", "--contains", commit);
  return Boolean(branches && branches.length);
}

/** What the rule says about one application. */
export function judge(appRoot, { release = false } = {}) {
  const harnix = join(appRoot, "harnix");
  const where = engineAt(appRoot);
  const agix = join(appRoot, where);
  const problems = [];
  const warnings = [];
  if (where === "agix") warnings.push(MOVE_HINT);

  const harnixHead = tryGit(harnix, "rev-parse", "HEAD");
  if (!harnixHead) problems.push("harnix/ is not a git checkout");
  else if (!onARemote(harnix, harnixHead)) {
    problems.push(`harnix/ is at ${harnixHead.slice(0, 10)}, which no Harnix remote has`);
  }

  const pinned = tryGit(harnix, "ls-tree", "HEAD", "agix")?.split(/\s+/)[2];
  const ours = tryGit(agix, "rev-parse", "HEAD");
  if (!pinned) problems.push("harnix/ pins no engine commit");
  if (!ours) problems.push(`${where}/ is not checked out — git submodule update --init --recursive`);
  if (pinned && ours && pinned !== ours) {
    if (!onARemote(agix, ours)) {
      problems.push(`${where}/ is at ${ours.slice(0, 10)}, which no engine remote has`);
    } else if (release) {
      problems.push(`${where}/ is at ${ours.slice(0, 10)}; Harnix was tested with ${pinned.slice(0, 10)} — release with the same, or move Harnix to this one first`);
    } else {
      warnings.push(`${where}/ is at ${ours.slice(0, 10)}; Harnix was tested with ${pinned.slice(0, 10)}`);
    }
  }
  return { problems, warnings };
}

if (process.argv[1] && realpathSync(resolve(process.argv[1])) === realpathSync(fileURLToPath(import.meta.url))) {
  const args = process.argv.slice(2);
  const appRoot = resolve(args.find((a) => !a.startsWith("--")) ?? join(harnixRoot, ".."));
  const { problems, warnings } = judge(appRoot, { release: args.includes("--release") });
  for (const w of warnings) console.warn(`check-pins: warning: ${w}`);
  for (const p of problems) console.error(`check-pins: ${p}`);
  if (problems.length) process.exit(1);
  console.log(warnings.length ? "check-pins: ok, with warnings" : "check-pins: ok");
}
