// Static checks on the styles that a rendering test cannot make.
//
// A rule that never matches is invisible to every other test — which is how
// an unsized glyph once shipped as a plus sign the height of its button.
// These are the cheap invariants that would have caught it. The last one is
// about words rather than styles: what the comments cite, and the retired names.
//
//   node tests/style-lint.mjs

import { execFileSync } from "node:child_process";
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

/* ── the words on the screen are the glossary's ─────────────────────────── */
// docs/glossary.md: one word, one meaning. A word it retired, or one it says not to mix up, does
// not come back through a dictionary. The values only — a key, a comment or a code name may keep
// an old spelling.
const BANNED = [
  [/子 ?agent/, "子 Agent"],
  [/执行器/, "执行者 (Executor)"],
  [/出站条件/, "出口条件 (ExitCond)"],
  [/工位/, "站 (Station)"],
  [/配套/, "件 (a pack's parts)"],
  [/等批准|待审批|等审批/, "待批准 (an open permission ask)"],
  [/模拟运行/, "试运行 (dry run)"],
  [/空应用|最小应用/, "Harnix app"],
  [/\b(?:[Ee]mpty|[Mm]inimal) application\b/, "Harnix app"],
  [/\bAttaCore\b/, "Agix"],
  [/\bsub agent\b|\bsubagent\b/i, "Sub-agent"],
  [/\b(?:Does|Will) not take\b/, "say what happens: not loaded"],
];
const dictionaries = [
  join(packages, "react", "src", "i18n", "en.ts"),
  join(packages, "react", "src", "i18n", "zh-CN.ts"),
  join(packages, "flow", "src", "strings.ts"),
];
const offTerm = [];
let words = 0;
for (const path of dictionaries) {
  const body = readFileSync(path, "utf8");
  for (const [, key, value] of body.matchAll(/^\s+"([^"]+)":\s*("(?:[^"\\]|\\.)*"),?$/gm)) {
    words += 1;
    const text = JSON.parse(value);
    for (const [word, instead] of BANNED) if (word.test(text)) offTerm.push(`${rel(path)} ${key}: "${text.match(word)[0]}" — the glossary says ${instead}`);
  }
}
if (offTerm.length) fail(`words the glossary does not use: ${offTerm.slice(0, 8).join("; ")}`);
else if (words < 500) fail(`only ${words} dictionary entries read: has the dictionaries' shape changed?`);
else ok(`${words} dictionary entries, none in a word the glossary retired (${BANNED.length} checked)`);

/* ── comments cite what a reader can open ───────────────────────────────── */
// The 0.4 design record, its roadmap and its review rounds left the repository (CLAUDE.md); a
// comment that cites `design §7.3` or `review #4` sends the reader nowhere. A comment says its
// reason, or names a document that is here: `concept §3.2`, `harnix-sdk.md §17.6`. And the words
// the glossary retired do not come back in the code by another door.
const tracked = execFileSync("git", ["ls-files", "-z", "--", "crates", "examples", "templates", "ui/packages", "tests", "scripts"], {
  cwd: root,
  encoding: "utf8",
}).split("\0").filter(Boolean);
const textual = /\.(rs|toml|ts|tsx|mjs|js|css|html|json|sh|py|wit|yaml|yml|md)$/;
const CITATIONS = [
  [new RegExp("设计 ?" + "§"), "the design record (设计 §x)"],
  [new RegExp("[Dd]esign " + "§"), "the design record (design §x)"],
  [new RegExp("review " + "#\\d"), "a review round (review #N)"],
];
const RETIRED = [
  [new RegExp("roadmap " + "M"), "the roadmap (roadmap Mx-y)"],
  [new RegExp("[Ee]mpty " + "application"), "the empty application — the Harnix app"],
  [new RegExp("空" + "应用"), "空应用 — Harnix app"],
];
const cited = [];
let read = 0;
for (const file of tracked) {
  if (file === "tests/style-lint.mjs" || !textual.test(file) || file.includes("/recordings/") && file.endsWith(".jsonl")) continue;
  let body;
  try {
    body = readFileSync(join(root, file), "utf8");
  } catch {
    continue; // listed but not on disk: a deleted file not yet committed
  }
  read += 1;
  const rules = file.startsWith("crates/") || file.startsWith("examples/") ? [...CITATIONS, ...RETIRED] : CITATIONS;
  const lines = body.split("\n");
  lines.forEach((line, i) => {
    for (const [pattern, what] of rules) if (pattern.test(line)) cited.push(`${file}:${i + 1} cites ${what}`);
  });
  // A citation a comment wraps (`design` at the end of one line, `§11.4` at the start of the next).
  const joined = body.replace(/\s*\n\s*(?:\/\/[!/]?|\*|#)?\s*/g, " ");
  for (const [pattern, what] of rules) {
    const global = new RegExp(pattern.source, "g");
    const wrapped = (joined.match(global) ?? []).length - lines.filter((l) => pattern.test(l)).length;
    if (wrapped > 0) cited.push(`${file} cites ${what} across a line break`);
  }
}
if (cited.length) fail(`citations a reader cannot follow, or retired words:\n  ${cited.join("\n  ")}`);
else ok(`${read} source files, none citing the removed design record or using a retired word`);

console.log(process.exitCode ? "\nSTYLE LINT FAILED" : "\nSTYLE LINT PASSED");
process.exit(process.exitCode || 0);
