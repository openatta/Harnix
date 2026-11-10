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

// Violations that exist now, as `<rule> <file>`. U1 goes when the first-party UI packages leave
// the shell (0.5.0 P17), U3 with the private channels (P2 for `__client`, P16 for `FRAME_HUB`).
// `node tests/ui-layering.mjs --list` prints what is violated.
export const PENDING = [
  "U1 ui/packages/shell/src/Shell.tsx",
  "U1 ui/packages/shell/src/Sidebar.tsx",
  "U1 ui/packages/shell/src/app.ts",
  "U1 ui/packages/shell/src/builtins.tsx",
  "U1 ui/packages/shell/src/commands.ts",
  "U1 ui/packages/shell/src/constants.ts",
  "U1 ui/packages/shell/src/dialogs.tsx",
  "U1 ui/packages/shell/src/envelope.tsx",
  "U1 ui/packages/shell/src/flow.ts",
  "U1 ui/packages/shell/src/host.ts",
  "U1 ui/packages/shell/src/inline/Grouping.tsx",
  "U1 ui/packages/shell/src/projects.tsx",
  "U1 ui/packages/shell/src/sessions.tsx",
  "U1 ui/packages/shell/src/settings.tsx",
  "U1 ui/packages/shell/src/system/agents.tsx",
  "U1 ui/packages/shell/src/system/console.tsx",
  "U1 ui/packages/shell/src/system/navigate.ts",
  "U1 ui/packages/shell/src/system/pane/instance.tsx",
  "U1 ui/packages/shell/src/system/pane/session.tsx",
  "U1 ui/packages/shell/src/system/pane/space.tsx",
  "U3 ui/packages/app-sdk/src/protocol.ts",
  "U3 ui/packages/core/src/client/client.ts",
  "U3 ui/packages/core/src/state/controller.ts",
  "U3 ui/packages/shell/src/Sidebar.tsx",
  "U3 ui/packages/shell/src/app.ts",
  "U3 ui/packages/shell/src/dialogs.tsx",
  "U3 ui/packages/shell/src/envelope.tsx",
  "U3 ui/packages/shell/src/host.ts",
  "U3 ui/packages/shell/src/inline/index.tsx",
  "U3 ui/packages/shell/src/surfaces/bridge.ts",
  "U3 ui/packages/shell/src/surfaces/register.tsx",
  "U3 ui/packages/shell/src/system/apps.tsx",
  "U3 ui/packages/shell/src/system/data.ts",
  "U3 ui/packages/shell/src/system/pane/calls.tsx",
  "U3 ui/packages/shell/src/system/pane/context.tsx",
  "U3 ui/packages/shell/src/system/pane/frames.tsx",
  "U3 ui/packages/shell/src/system/pane/space.tsx",
  "U3 ui/packages/shell/src/system/pane/tasks.tsx",
  "U3 ui/packages/shell/src/system/pane/turns.tsx",
  "U3 ui/packages/shell/src/system/view.ts",
  "U3 ui/packages/shell/src/topics.ts",
  "U3 ui/packages/testkit/src/api.ts",
];

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
