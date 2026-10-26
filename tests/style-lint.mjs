// Static checks on the styles that a rendering test cannot make.
//
// A rule that never matches is invisible to every other test — which is how
// an unsized glyph once shipped as a plus sign the height of its button.
// These are the cheap invariants that would have caught it.
//
//   node tests/style-lint.mjs

import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const packages = join(root, "ui", "packages");

const files = (dir, pattern) => {
  const out = [];
  const walk = (d) => {
    for (const entry of readdirSync(d, { withFileTypes: true })) {
      const path = join(d, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== "node_modules") walk(path);
      } else if (pattern.test(entry.name)) out.push([path, readFileSync(path, "utf8")]);
    }
  };
  walk(dir);
  return out;
};

const sheets = [...files(join(packages, "react", "src"), /\.css$/), ...files(join(packages, "shell", "src"), /\.css$/)];
const css = sheets.map(([, body]) => body).join("\n");
const sources = [...files(join(packages, "react", "src"), /\.tsx?$/), ...files(join(packages, "shell", "src"), /\.tsx?$/)];

const fail = (m) => { console.log("FAIL:", m); process.exitCode = 1; };
const ok = (m) => console.log("ok —", m);
const rel = (path) => path.slice(root.length + 1);

/** Does any rule give this class a width? (`.glyph`, `.mark`, …) */
function sized(className) {
  const rule = new RegExp(`\\.${className}\\b[^{]*\\{[^}]*\\bwidth\\s*:`, "s");
  return rule.test(css);
}

/* ── every glyph carries at least one class that gives it a size ─────────── */
// Per call site, not per class: `<Glyph className="glyph folder">` is sized by
// `.glyph` while `.folder` only switches it on and off.
const sites = new Set();
for (const [path, source] of sources) {
  for (const [, cls] of source.matchAll(/<Glyph\b[^>]*\bclassName="([^"]*)"/g)) {
    const classes = cls.trim().split(/\s+/).filter(Boolean);
    if (!classes.length) fail(`${rel(path)}: a Glyph with an empty class — nothing can size it`);
    else if (!classes.some(sized)) fail(`${rel(path)}: <Glyph className="${cls}"> has no class with a width rule`);
    else sites.add(cls);
  }
  for (const match of source.matchAll(/<Glyph\b(?![^>]*\bclassName=)[^>]*>/g)) {
    fail(`${rel(path)}: ${match[0]} has no class — nothing sizes it`);
  }
}
ok(`${sites.size} glyph class shapes, every one sized: ${[...sites].sort().join(" | ")}`);

/* ── no shouty uppercase on text that will be CJK ───────────────────────── */
const uppercase = [...css.matchAll(/text-transform:\s*uppercase/g)].length;
if (uppercase) fail(`${uppercase} rule(s) force uppercase — it does nothing for CJK but spread it`);
else ok("no uppercase transforms");

/* ── tokens only: no raw colors outside the token sheet ─────────────────── */
const rawColors = [];
for (const [path, body] of sheets) {
  if (path.endsWith("tokens.css")) continue;
  for (const [, color] of body.matchAll(/:\s*(#[0-9a-fA-F]{3,8}\b|rgba?\([^)]*\))/g)) {
    // rgba() inside a shadow or a gradient is a shape, not a palette entry.
    if (/^rgba?\(/.test(color) && /(shadow|gradient|mask)/i.test(body.slice(0, body.indexOf(color)).split("\n").slice(-6).join("\n"))) continue;
    rawColors.push(`${rel(path)}: ${color}`);
  }
}
if (rawColors.length) fail(`raw colors outside tokens.css: ${rawColors.slice(0, 6).join(", ")}`);
else ok("styles use tokens only");

/* ── no literal colors in components either ─────────────────────────────── */
const inline = [];
for (const [path, source] of sources) {
  for (const [, color] of source.matchAll(/style=\{\{[^}]*?(#[0-9a-fA-F]{3,8}\b|rgba?\([^)]*\))/g)) inline.push(`${rel(path)}: ${color}`);
}
if (inline.length) fail(`colors written into components: ${inline.slice(0, 6).join(", ")}`);
else ok("no colors in inline styles");

console.log(process.exitCode ? "\nSTYLE LINT FAILED" : "\nSTYLE LINT PASSED");
process.exit(process.exitCode || 0);
