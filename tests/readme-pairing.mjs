// The two READMEs are one document in two languages, and the failure mode is
// silent: someone edits the English side, the Chinese side keeps saying the old
// thing, and nobody notices because nothing is broken — it is just wrong.
//
// So the pair records the git blob hash of each side as of the last time they
// were confirmed to say the same thing. Edit either one and this fails until
// you bring the other along and re-record:
//
//   node tests/readme-pairing.mjs --write
//
// Borrowed from DeepSeek Harness, which keeps the same record in
// `README.i18n.yaml`. Neither side is the original: both are authoritative, and
// re-recording is a claim that you checked.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const record = join(root, "README.i18n.yaml");
const write = process.argv.includes("--write");

const fail = (m) => { console.log("FAIL:", m); process.exitCode = 1; };
const ok = (m) => console.log("ok —", m);

/** The same hash `git ls-files -s` reports, computed without touching the index. */
const blob = (file) =>
  execFileSync("git", ["hash-object", "--", join(root, file)], { encoding: "utf8" }).trim();

const parse = (text) => {
  const pairs = {};
  for (const line of text.split("\n")) {
    const m = /^([\w.\-/]+):\s*([0-9a-f]{40})\s*$/.exec(line);
    if (m) pairs[m[1]] = m[2];
  }
  return pairs;
};

const HEADER = `# Bilingual-pair consistency record: the git blob hash of each side as of the last
# confirmed-consistent state. Both languages carry equal authority; after editing either
# side, bring the other along and re-record with:
#   node tests/readme-pairing.mjs --write
`;

// Each pair is [English, Chinese]; the user guide is one more document in two languages.
const PAIRS = [
  ["README.md", "docs/README.zh-CN.md"],
  ["user-guide/README.md", "user-guide/README.zh-CN.md"],
];
const FILES = PAIRS.flat();
const current = Object.fromEntries(FILES.map((f) => [f, blob(f)]));

if (write) {
  writeFileSync(record, `${HEADER}${FILES.map((f) => `${f}: ${current[f]}`).join("\n")}\n`);
  console.log("recorded:", FILES.map((f) => `${f} ${current[f].slice(0, 8)}`).join(" · "));
  process.exit(0);
}

const recorded = parse(readFileSync(record, "utf8"));
for (const pair of PAIRS) {
  const moved = pair.filter((f) => recorded[f] !== current[f]);
  if (moved.length === pair.length) {
    ok(`${pair[0]} and ${pair[1]} moved together — re-record with --write once you have checked them`);
  } else if (moved.length === 0) {
    ok(`${pair[0]} matches its recorded pair`);
  } else {
    const stale = pair.filter((f) => !moved.includes(f));
    fail(`${moved.join(", ")} changed but ${stale.join(", ")} did not — the pair has drifted`);
  }
}

// A structural check the hashes cannot make: each side must point at the other,
// or the switcher is a dead end.
const links = {
  "README.md": "(docs/README.zh-CN.md)",
  "docs/README.zh-CN.md": "(../README.md)",
  "user-guide/README.md": "(README.zh-CN.md)",
  "user-guide/README.zh-CN.md": "(README.md)",
};
for (const file of FILES) {
  const text = readFileSync(join(root, file), "utf8");
  if (!text.includes(links[file])) fail(`${file} has no link to its counterpart`);
  else ok(`${file} links to its counterpart`);
}

// The two guides must have the same shape: the same number of sections and tables.
const shape = (file) => {
  const text = readFileSync(join(root, file), "utf8");
  return [(text.match(/^## /gm) || []).length, (text.match(/^\|---/gm) || []).length];
};
const [en, zh] = PAIRS[1].map(shape);
if (en.join() !== zh.join()) fail(`user-guide sections/tables differ: ${en} vs ${zh}`);
else ok(`user-guide has the same sections and tables in both languages (${en[0]} sections, ${en[1]} tables)`);

// Images are shared between the two, so a missing one breaks both at once.
const images = new Set();
// Each side's image paths are relative to where that side lives.
for (const file of FILES) {
  const text = readFileSync(join(root, file), "utf8");
  for (const m of text.matchAll(/!\[[^\]]*\]\(([^)]+)\)/g)) images.add(join(dirname(file), m[1]));
}
for (const image of images) {
  try {
    readFileSync(join(root, image));
    ok(`image present: ${image}`);
  } catch {
    fail(`image referenced but missing: ${image}`);
  }
}

console.log(process.exitCode ? "\nREADME PAIRING FAILED" : "\nREADME PAIRING PASSED");
