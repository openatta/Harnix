// Every Harnix crate states its own version, licence, edition and toolchain
// floor, and they agree.
//
// Inherited from the workspace, these would be whatever workspace the crates
// are built in — and an application builds them as members of its own (see
// Cargo.toml). The check is what keeps "they agree" true without the
// inheritance that used to guarantee it.
//
// The SDK document's version table says the same version as the crates.
//
// And each Harnix release stands on exactly one Agix release: the submodule is
// pinned to a commit that carries an `Agix-v…` tag, and is checked out there,
// so the Cargo.lock written from it is the pinned one.

import { execFileSync } from "node:child_process";
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
// The SDK's version table names the release it describes; a release that forgets it leaves the
// document quoting an old one as current. `scripts/set-version.sh` rewrites that cell with the crates.
const version = [...(seen.version?.values() ?? [])][0]?.replace(/"/g, "");
const sdk = readFileSync(join(root, "docs", "harnix-sdk.md"), "utf8");
const documented = sdk.match(/^\| Harnix 版本 \| `([^`]+)` \|/m)?.[1];
if (!documented) fail("docs/harnix-sdk.md has no `| Harnix 版本 | `…` |` row in its version table");
else if (version && documented !== version) {
  fail(`docs/harnix-sdk.md says Harnix ${documented}; the crates are ${version} (scripts/set-version.sh)`);
}
const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
const pinned = git("ls-files", "-s", "agix").split(/\s+/)[1];
const checkedOut = git("-C", "agix", "rev-parse", "HEAD");
if (pinned !== checkedOut) fail(`agix/ is checked out at ${checkedOut.slice(0, 7)}, not the pinned ${pinned?.slice(0, 7)} (git submodule update)`);
const agix = git("-C", "agix", "tag", "--points-at", pinned, "--list", "Agix-v*").split("\n").filter(Boolean);
if (agix.length !== 1) fail(`the pinned Agix ${pinned?.slice(0, 7)} carries ${agix.length ? agix.join(", ") : "no Agix-v… tag"}, not exactly one release`);
if (failed) process.exit(1);
console.log(`ok — ${crates.length} crates, version ${[...seen.version.values()][0]}, on ${agix[0]}`);
