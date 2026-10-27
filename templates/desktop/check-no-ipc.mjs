#!/usr/bin/env node
// The desktop shell carries no business traffic (design §12, D5): the page
// talks to its server over HTTP/WS, exactly as in a browser, and the shell
// only manages the window, the child process and its config. This fails when
// something that would open a second channel appears in the shell's code or
// configuration — a command the page can invoke, the handler that routes
// them, the global object that exposes them, a custom protocol serving the
// interface, or the JS package that calls into any of it.
//
//   node check-no-ipc.mjs            # this template
//   node check-no-ipc.mjs <dir>      # an app's copy of it
//
// Capabilities are read as JSON: the only permissions any of them may grant
// are dragging the window and maximizing it on a double-click (the window has
// no title bar on macOS; the page's top bars are where it is dragged), to the
// main window, and a remote URL may only be a loopback address.
//
// Literal matches, comments included: a comment that names one of these is
// rare enough to rephrase, and a check that parses Rust would be the bigger
// thing to get wrong. Documentation (*.md) is not scanned; it has to name them
// to explain the rule.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const self = fileURLToPath(import.meta.url);
const root = resolve(process.argv[2] ?? dirname(self));

const forbidden = [
  [/tauri::command/, "a Tauri command"],
  [/invoke_handler/, "an invoke handler"],
  [/"?withGlobalTauri"?\s*:\s*true/, "the global Tauri object in the page"],
  [/register_(asynchronous_)?uri_scheme_protocol/, "a custom protocol"],
  [/@tauri-apps\/api/, "the Tauri JS API"],
];

// Build output, generated schemas and the bundled app binary are not ours;
// this script (any copy of it) names the patterns it looks for.
const skipped = new Set(["target", "gen", "binaries", "node_modules", ".git"]);
const scanned = new Set([".rs", ".toml", ".json", ".js", ".mjs", ".ts", ".html"]);

function* files(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      if (!skipped.has(name)) yield* files(path);
    } else if (scanned.has(extname(name)) && name !== basename(self)) {
      yield path;
    }
  }
}

let found = 0;
let count = 0;
for (const path of files(root)) {
  count += 1;
  readFileSync(path, "utf8")
    .split("\n")
    .forEach((line, i) => {
      for (const [pattern, what] of forbidden) {
        if (pattern.test(line)) {
          found += 1;
          console.error(`${relative(root, path)}:${i + 1}: ${what}: ${line.trim()}`);
        }
      }
    });
}

const allowedPermissions = new Set(["core:window:allow-start-dragging", "core:window:allow-internal-toggle-maximize"]);
const loopback = /^http:\/\/(127\.0\.0\.1|localhost)(:[*\d]+)?\/?\*?$/;
const capabilities = join(root, "src-tauri", "capabilities");
for (const name of statSync(capabilities, { throwIfNoEntry: false })?.isDirectory() ? readdirSync(capabilities) : []) {
  if (extname(name) !== ".json") continue;
  const path = join(capabilities, name);
  const where = relative(root, path);
  let capability;
  try {
    capability = JSON.parse(readFileSync(path, "utf8"));
  } catch (e) {
    found += 1;
    console.error(`${where}: not JSON: ${e.message}`);
    continue;
  }
  for (const one of Array.isArray(capability) ? capability : [capability]) {
    for (const permission of one.permissions ?? []) {
      const id = typeof permission === "string" ? permission : permission?.identifier;
      if (!allowedPermissions.has(id)) {
        found += 1;
        console.error(`${where}: a permission beyond dragging the window: ${JSON.stringify(permission)}`);
      }
    }
    if ((one.permissions ?? []).length && JSON.stringify(one.windows ?? []) !== JSON.stringify(["main"])) {
      found += 1;
      console.error(`${where}: window permissions for windows other than "main": ${JSON.stringify(one.windows)}`);
    }
    for (const url of one.remote?.urls ?? []) {
      if (!loopback.test(url)) {
        found += 1;
        console.error(`${where}: a remote URL that is not loopback: ${url}`);
      }
    }
  }
}

if (found > 0) {
  console.error(
    `\n${found} finding(s). The page reaches its server over HTTP/WS only; ` +
      "the shell gets no IPC beyond dragging its window (design §12).",
  );
  process.exit(1);
}
console.log(`no IPC: ${count} files under ${relative(process.cwd(), root) || "."}`);
