// The interface's layering rules (U1–U3), asserted rather than described.
//
//   U1  the shell's core names no first-party space and imports no first-party UI package
//   U2  a first-party UI package imports only React, `@harnix/react` and `@harnix/core`
//   U3  no package mentions the old private channels (`__client`, `__chain`, `app.client`,
//       `onNotify`, `FRAME_HUB`)
//
// A rule that does not hold yet is in `PENDING`, by rule and file, until the work that makes it
// hold lands. The table can only get shorter: a violation that is not listed fails, and so does
// a listed one that is gone — the list never keeps promising work that is already done.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const packages = join(root, "ui", "packages");

// The first-party UI packages: a space or a part of the system space, each its own package.
const FIRST_PARTY = ["flow", "system", "harnix-app", "settings", "plugins", "devices", "packages", "observability"];
const SPACE_IDS = ["system", "sessions", "projects", "flow", "harnix-app"];
const PRIVATE = ["__client", "__chain", "app.client", "onNotify", "FRAME_HUB"];
const ALLOWED_IMPORTS = ["react", "react-dom", "react/jsx-runtime", "@harnix/react", "@harnix/core"];

// Violations that exist now, as `<rule> <file>`: none. A rule that stops holding is listed here
// only with the work that makes it hold again. `node tests/ui-layering.mjs --list` prints what is
// violated.
export const PENDING = [];

function files(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name === "dist") continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...files(path));
    else if (/\.(ts|tsx|mjs|js|jsx)$/.test(name)) out.push(path);
  }
  return out;
}

/** Source with its comments blanked: what the code says, not what its prose does. */
function code(text) {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:"'`\\])\/\/[^\n]*/g, "$1");
}

const imports = (text) =>
  [...text.matchAll(/(?:from|import\()\s*["']([^"']+)["']/g)].map((m) => m[1]).filter((spec) => !spec.startsWith("."));

const packageDirs = readdirSync(packages).filter((n) => statSync(join(packages, n)).isDirectory());

export function violations() {
  const found = new Set();
  const add = (rule, file) => found.add(`${rule} ${relative(root, file)}`);
  for (const pkg of packageDirs) {
    const src = join(packages, pkg, "src");
    if (!statSync(src, { throwIfNoEntry: false })?.isDirectory()) continue;
    for (const file of files(src)) {
      const text = code(readFileSync(file, "utf8"));
      if (PRIVATE.some((word) => text.includes(word))) add("U3", file);
      if (pkg === "shell") {
        const quoted = SPACE_IDS.some((id) => new RegExp(`(["'\`])${id}\\1`).test(text));
        const firstParty = imports(text).some((spec) => FIRST_PARTY.some((p) => spec === `@harnix/${p}` || spec.startsWith(`@harnix/${p}/`)));
        if (quoted || firstParty) add("U1", file);
      }
      if (FIRST_PARTY.includes(pkg) && imports(text).some((spec) => !ALLOWED_IMPORTS.includes(spec))) add("U2", file);
    }
  }
  return [...found].sort();
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const now = violations();
  if (process.argv.includes("--list")) {
    console.log(now.map((v) => `  "${v}",`).join("\n"));
    process.exit(0);
  }
  const unlisted = now.filter((v) => !PENDING.includes(v));
  const stale = PENDING.filter((v) => !now.includes(v));
  for (const v of unlisted) console.error(`ui-layering: ${v} — a violation that is not pending`);
  for (const v of stale) console.error(`ui-layering: ${v} — pending, and no longer violated: remove it from PENDING`);
  if (unlisted.length || stale.length) process.exit(1);
  console.log(`ui-layering: ok (${PENDING.length} pending)`);
}
