#!/usr/bin/env node
// The two pins of an application (design §11.3).
//
//   node harnix/scripts/check-pins.mjs [app-root] [--release]
//
// `harnix/` pins a Harnix commit, and that Harnix pins an Agix commit. The
// application pins its own `agix/`, beside `harnix/`. The rule:
//
// | the app's agix/ against Harnix's pin   | result                                   |
// |----------------------------------------|------------------------------------------|
// | the same                               | ok                                       |
// | another commit an Agix remote has      | warning; a failure with --release        |
// | a commit no remote has                 | failure                                  |
//
// And `harnix/` itself must point at a commit a Harnix remote has: no
// long-lived patches in an application (§11.5).

import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

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
  const agix = join(appRoot, "agix");
  const problems = [];
  const warnings = [];

  const harnixHead = tryGit(harnix, "rev-parse", "HEAD");
  if (!harnixHead) problems.push("harnix/ is not a git checkout");
  else if (!onARemote(harnix, harnixHead)) {
    problems.push(`harnix/ is at ${harnixHead.slice(0, 10)}, which no Harnix remote has`);
  }

  const pinned = tryGit(harnix, "ls-tree", "HEAD", "agix")?.split(/\s+/)[2];
  const ours = tryGit(agix, "rev-parse", "HEAD");
  if (!pinned) problems.push("harnix/ pins no agix/ commit");
  if (!ours) problems.push("agix/ is not a git checkout");
  if (pinned && ours && pinned !== ours) {
    if (!onARemote(agix, ours)) {
      problems.push(`agix/ is at ${ours.slice(0, 10)}, which no Agix remote has`);
    } else if (release) {
      problems.push(`agix/ is at ${ours.slice(0, 10)}; Harnix was tested with ${pinned.slice(0, 10)} — release with the same, or move Harnix to this one first`);
    } else {
      warnings.push(`agix/ is at ${ours.slice(0, 10)}; Harnix was tested with ${pinned.slice(0, 10)}`);
    }
  }
  return { problems, warnings };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const appRoot = resolve(args.find((a) => !a.startsWith("--")) ?? join(harnixRoot, ".."));
  const { problems, warnings } = judge(appRoot, { release: args.includes("--release") });
  for (const w of warnings) console.warn(`check-pins: warning: ${w}`);
  for (const p of problems) console.error(`check-pins: ${p}`);
  if (problems.length) process.exit(1);
  console.log(warnings.length ? "check-pins: ok, with warnings" : "check-pins: ok");
}
