// Every Harnix crate states its own version, licence, edition and toolchain
// floor, and they agree.
//
// Inherited from the workspace, these would be whatever workspace the crates
// are built in — and an application builds them as members of its own (see
// Cargo.toml). The check is what keeps "they agree" true without the
// inheritance that used to guarantee it.
//
// The SDK document's version table says the same version as the crates, and
// the same contract versions as the code.
//
// And each Harnix release stands on exactly one Agix release: the submodule is
// pinned to a commit that carries an `Agix-v…` tag, and is checked out there,
// so the Cargo.lock written from it is the pinned one.

import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
// `crates/ext/<id>` holds the first-party extensions and `crates/carrier/<name>` the carriers;
// those directories themselves are no crate.
const nested = ["ext", "carrier"];
const crates = readdirSync(join(root, "crates"))
  .filter((c) => !nested.includes(c))
  .concat(nested.flatMap((dir) => readdirSync(join(root, "crates", dir)).map((c) => `${dir}/${c}`)));
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
// The extension guide's version table names the release it describes; a release that forgets it leaves the
// document quoting an old one as current. `scripts/set-version.sh` rewrites that cell with the crates.
const version = [...(seen.version?.values() ?? [])][0]?.replace(/"/g, "");
const guide = readFileSync(join(root, "docs", "harnix-ext.md"), "utf8");
const documented = guide.match(/^\| Harnix 版本 \| `([^`]+)` \|/m)?.[1];
if (!documented) fail("docs/harnix-ext.md has no `| Harnix 版本 | `…` |` row in its version table");
else if (version && documented !== version) {
  fail(`docs/harnix-ext.md says Harnix ${documented}; the crates are ${version} (scripts/set-version.sh)`);
}
// The other cells are constants in the code, and nothing rewrites them: a bump that forgets
// the table leaves an application reading the old contract as current.
const constants = [
  ["扩展 API", "crates/ext/api/src/lib.rs", "EXT_API_VERSION"],
  ["贡献点 API", "crates/contract/src/handshake.rs", "CONTRIB_API_VERSION"],
  ["协议", "crates/contract/src/handshake.rs", "PROTOCOL_VERSION"],
  ["扩展包接口", "crates/package/src/manifest.rs", "HARNIX_API"],
  ["扩展线", "crates/wasm-port/src/lib.rs", "EXT_WIRE_VERSION"],
  ["流程库 API", "crates/flow/src/lib.rs", "FLOW_API_VERSION"],
];
for (const [row, file, name] of constants) {
  const code = readFileSync(join(root, file), "utf8").match(new RegExp(`pub const ${name}: u32 = (\\d+);`))?.[1];
  const cell = guide.match(new RegExp(`^\\| ${row} \\| \`([^\`]+)\` \\|`, "m"))?.[1];
  if (!code) fail(`${file} no longer defines \`pub const ${name}: u32\``);
  else if (!cell) fail(`docs/harnix-ext.md has no \`| ${row} | \`…\` |\` row in its version table`);
  else if (cell !== code) fail(`docs/harnix-ext.md says ${row} ${cell}; ${name} is ${code} (${file})`);
}
// The flow library writes every dependency's version itself, so the directory can be copied into
// another workspace (crates/flow/docs/embed.en.md). Where Harnix's workspace names the same crate,
// the two say the same version: a bump on one side alone would build two copies of it here.
const deps = (text, ...sections) => {
  const out = new Map();
  let inside = false;
  for (const line of text.split("\n")) {
    const header = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (header) { inside = sections.includes(header[1].trim()); continue; }
    if (!inside) continue;
    const m = line.match(/^\s*([A-Za-z0-9_-]+)\s*=\s*(.+)$/);
    if (!m) continue;
    const value = m[2].trim();
    const version = value.startsWith('"') ? value.match(/^"([^"]*)"/)?.[1] : value.match(/\bversion\s*=\s*"([^"]*)"/)?.[1];
    out.set(m[1], version ?? null);
  }
  return out;
};
const flowManifest = readFileSync(join(root, "crates", "flow", "Cargo.toml"), "utf8");
const workspaceDeps = deps(readFileSync(join(root, "Cargo.toml"), "utf8"), "workspace.dependencies");
for (const [name, version] of deps(flowManifest, "dependencies", "dev-dependencies", "build-dependencies")) {
  if (version === null) fail(`crates/flow/Cargo.toml: \`${name}\` states no version of its own`);
  else if (workspaceDeps.has(name) && workspaceDeps.get(name) !== version) {
    fail(`crates/flow/Cargo.toml: \`${name}\` is "${version}", the workspace's is "${workspaceDeps.get(name)}"`);
  }
}
const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
const pinned = git("ls-files", "-s", "agix").split(/\s+/)[1];
const checkedOut = git("-C", "agix", "rev-parse", "HEAD");
if (pinned !== checkedOut) fail(`agix/ is checked out at ${checkedOut.slice(0, 7)}, not the pinned ${pinned?.slice(0, 7)} (git submodule update)`);
const agix = git("-C", "agix", "tag", "--points-at", pinned, "--list", "Agix-v*").split("\n").filter(Boolean);
if (agix.length !== 1) fail(`the pinned Agix ${pinned?.slice(0, 7)} carries ${agix.length ? agix.join(", ") : "no Agix-v… tag"}, not exactly one release`);
if (failed) process.exit(1);
console.log(`ok — ${crates.length} crates, version ${[...seen.version.values()][0]}, on ${agix[0]}`);
