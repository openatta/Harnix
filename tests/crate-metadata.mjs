// Every Harnix crate states its own version, licence, edition and toolchain
// floor, and they agree.
//
// Inherited from the workspace, these would be whatever workspace the crates
// are built in — and an application builds them as members of its own (see
// Cargo.toml). The check is what keeps "they agree" true without the
// inheritance that used to guarantee it.

import { readFileSync, readdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const crates = readdirSync(join(root, "crates"));
const field = (text, key) => text.match(new RegExp(`^${key}\\s*=\\s*(.+)$`, "m"))?.[1].trim();

let failed = false;
const fail = (msg) => { console.log(`FAIL: ${msg}`); failed = true; };
const seen = {};
for (const crate of crates) {
  const text = readFileSync(join(root, "crates", crate, "Cargo.toml"), "utf8");
  for (const key of ["version", "edition", "rust-version", "license"]) {
    const value = field(text, key);
    if (!value) fail(`crates/${crate} does not state ${key}`);
    else if (value.includes("workspace")) fail(`crates/${crate} inherits ${key} from the workspace`);
    else (seen[key] ??= new Map()).set(crate, value);
  }
}
for (const [key, values] of Object.entries(seen)) {
  const distinct = new Set(values.values());
  if (distinct.size > 1) {
    fail(`crates disagree on ${key}: ${[...values].map(([c, v]) => `${c}=${v}`).join(", ")}`);
  }
}
if (seen.license && [...seen.license.values()].some((v) => v !== '"Apache-2.0"')) {
  fail("every crate is Apache-2.0");
}
if (failed) process.exit(1);
console.log(`ok — ${crates.length} crates, version ${[...seen.version.values()][0]}`);
