#!/usr/bin/env node
// Compare an application's Cargo.lock with Harnix's (design §11.3).
//
//   node harnix/scripts/check-lock.mjs [app-root]
//
// Harnix was tested against the versions in its own lock file. A package
// both lock files name, at a version the application's does not share, is
// a combination nobody has run — reported, and the exit status says so.

import { readFileSync, realpathSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const harnixRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/** name → set of versions. */
export function packages(lockText) {
  const found = new Map();
  for (const chunk of lockText.split("[[package]]").slice(1)) {
    const name = chunk.match(/^name = "([^"]+)"/m)?.[1];
    const version = chunk.match(/^version = "([^"]+)"/m)?.[1];
    if (!name || !version) continue;
    if (!found.has(name)) found.set(name, new Set());
    found.get(name).add(version);
  }
  return found;
}

/** Packages in both whose versions differ. Harnix's and Agix's own crates
 *  are left out: their versions are what the pins check. */
export function drift(appLock, harnixLock) {
  const app = packages(appLock);
  const ours = packages(harnixLock);
  const out = [];
  for (const [name, versions] of ours) {
    if (!app.has(name) || name.startsWith("harnix")) continue;
    const theirs = app.get(name);
    const same = [...versions].every((v) => theirs.has(v)) && [...theirs].every((v) => versions.has(v));
    if (!same) out.push({ name, harnix: [...versions].sort(), app: [...theirs].sort() });
  }
  return out;
}

if (process.argv[1] && realpathSync(resolve(process.argv[1])) === realpathSync(fileURLToPath(import.meta.url))) {
  const appRoot = resolve(process.argv.slice(2).find((a) => !a.startsWith("--")) ?? join(harnixRoot, ".."));
  const found = drift(
    readFileSync(join(appRoot, "Cargo.lock"), "utf8"),
    readFileSync(join(harnixRoot, "Cargo.lock"), "utf8"),
  );
  if (found.length) {
    console.error(`check-lock: ${found.length} package(s) differ from the versions Harnix was tested with:`);
    for (const d of found) console.error(`  ${d.name}: harnix ${d.harnix.join(", ")} · app ${d.app.join(", ")}`);
    process.exit(1);
  }
  console.log("check-lock: every shared package is at the version Harnix was tested with");
}
