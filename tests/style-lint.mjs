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

// The flow space is built into the shell (xops-02 §7): its package is held to the same rules.
const builtIn = ["react", "shell", "flow"];
const sheets = builtIn.flatMap((p) => files(join(packages, p, "src"), /\.css$/));
const css = sheets.map(([, body]) => body).join("\n");
const sources = builtIn.flatMap((p) => files(join(packages, p, "src"), /\.tsx?$/));

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

/* ── body text is readable: 4.5:1 on what it is read on, in both themes ──── */
// The token pairs the pages set running text in. A grey that reads as "quiet" on a designer's
// screen is a grey a good part of the readers cannot read (WCAG AA is 4.5:1 below 18px).
const tokenSheet = sheets.find(([path]) => path.endsWith(join("react", "src", "tokens.css")))?.[1] ?? "";
const themes = { light: {}, dark: {} };
for (const [, selector, body] of tokenSheet.matchAll(/(:root(?:\[data-theme="dark"\])?)\s*\{([^}]*)\}/g)) {
  const into = selector.includes("dark") ? themes.dark : themes.light;
  for (const [, name, value] of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) into[name] = value.trim();
}
themes.dark = { ...themes.light, ...themes.dark };
const resolve = (theme, name, depth = 0) => {
  const value = theme[name];
  const ref = value?.match(/^var\((--[\w-]+)\)$/);
  return ref && depth < 8 ? resolve(theme, ref[1], depth + 1) : value;
};
const luminance = (hex) => {
  const h = hex.length === 4 ? [...hex.slice(1)].map((c) => c + c).join("") : hex.slice(1);
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a, b) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};
const grounds = ["--bg-base", "--bg-sidebar", "--surface"];
const pairs = [
  ...["--label-primary", "--label-secondary", "--label-caption", "--label-tertiary", "--state-info", "--state-warn-label"].flatMap((fg) => grounds.map((bg) => [fg, bg])),
  ["--label-primary", "--bg-bubble"],
  ["--label-secondary", "--bg-bubble"],
  ["--label-caption", "--bg-bubble"],
  ["--button-primary-label", "--button-primary-fill"],
];
const unreadable = [];
let measured = 0;
for (const [theme, tokens] of Object.entries(themes)) {
  for (const [fg, bg] of pairs) {
    const [a, b] = [resolve(tokens, fg), resolve(tokens, bg)];
    if (!/^#[0-9a-fA-F]{3}([0-9a-fA-F]{3})?$/.test(a ?? "") || !/^#[0-9a-fA-F]{3}([0-9a-fA-F]{3})?$/.test(b ?? "")) {
      unreadable.push(`${theme} ${fg} on ${bg}: not a plain colour (${a} on ${b})`);
      continue;
    }
    measured += 1;
    const ratio = contrast(a, b);
    if (ratio < 4.5) unreadable.push(`${theme} ${fg} ${a} on ${bg} ${b}: ${ratio.toFixed(2)}:1`);
  }
}
if (unreadable.length) fail(`text tokens under 4.5:1 — ${unreadable.join("; ")}`);
else ok(`${measured} text/background token pairs, every one at least 4.5:1`);

console.log(process.exitCode ? "\nSTYLE LINT FAILED" : "\nSTYLE LINT PASSED");
process.exit(process.exitCode || 0);
