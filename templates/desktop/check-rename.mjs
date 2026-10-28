#!/usr/bin/env node
// Everything an app renames in this template is a named constant, and the README's rename list
// names it (ipdrun-31). A literal left in the code is a rename that an app's copy misses: the
// state directory's fallback was a bare "harnix" in `local.rs`, and an app whose profile is built
// into its binary waited sixty seconds for a token in `~/.harnix` that it never wrote.
//
//   node check-rename.mjs            # this template
//   node check-rename.mjs <dir>      # an app's copy of it (a renamed copy passes too)

import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(process.argv[2] ?? dirname(fileURLToPath(import.meta.url)));
const read = (path) => readFileSync(join(root, path), "utf8");

const config = read("src-tauri/src/config.rs");
const local = read("src-tauri/src/local.rs");
const readme = read("README.md");
const problems = [];

for (const name of ["ENV_PREFIX", "SIDECAR", "PRODUCT_ID"]) {
  if (!new RegExp(`const ${name}: &str = "[^"]+";`).test(config)) problems.push(`config.rs has no \`${name}\` constant`);
  if (!readme.includes(`\`${name}\``)) problems.push(`the README's rename list does not name \`${name}\``);
}
const stateDir = /fn state_dir\([\s\S]*?\n\}/.exec(local)?.[0] ?? "";
if (!stateDir) problems.push("local.rs has no `state_dir`");
if (/"[a-z][a-z0-9-]*"\.to_string\(\)/.test(stateDir)) problems.push("`state_dir` falls back to a literal product id, not `PRODUCT_ID`");
if (!stateDir.includes("PRODUCT_ID")) problems.push("`state_dir` does not fall back to `PRODUCT_ID`");

if (problems.length) {
  for (const p of problems) console.error(`  ${p}`);
  process.exit(1);
}
console.log("desktop template: every rename is a constant the README names");
