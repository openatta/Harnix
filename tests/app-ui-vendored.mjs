// The showcase's interface ships @harnix/ext-sdk and @harnix/ext-kit as built files
// (examples/apps/showcase/ui/frame/vendor/), as an unbundled application would. This builds both
// packages and checks the copies are what they build to — or, with --write, puts them there.
//
//   node tests/app-ui-vendored.mjs           # check
//   node tests/app-ui-vendored.mjs --write   # build and copy

import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const vendor = join(root, "examples", "apps", "showcase", "ui", "frame", "vendor");
const files = [
  ["app-sdk", "harnix-ext-sdk.js"],
  ["ui-kit", "harnix-ext-kit.js"],
  ["ui-kit", "harnix-ext-kit.css"],
];
const write = process.argv.includes("--write");

for (const pkg of new Set(files.map(([p]) => p))) {
  execFileSync("pnpm", ["--filter", `@harnix/${pkg}`, "run", "build"], { cwd: root, stdio: ["ignore", "ignore", "inherit"] });
}
let stale = 0;
for (const [pkg, name] of files) {
  const built = join(root, "ui", "packages", pkg, "dist", name);
  const copy = join(vendor, name);
  if (write) {
    copyFileSync(built, copy);
    console.log(`wrote ${copy.slice(root.length + 1)}`);
  } else if (!existsSync(copy) || !readFileSync(copy).equals(readFileSync(built))) {
    console.log(`FAIL: ${copy.slice(root.length + 1)} is not what @harnix/${pkg} builds to: node tests/app-ui-vendored.mjs --write`);
    stale += 1;
  } else {
    console.log(`ok — ${name}`);
  }
}
process.exit(stale ? 1 : 0);
