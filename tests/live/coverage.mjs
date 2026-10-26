// The coverage table, from the tests' own tags (design §13.4).
//
//   node coverage.mjs           write coverage.md
//   node coverage.mjs --check   fail if coverage.md is stale, or a row is
//                               zero on a layer without saying why
//
// Counted from `playwright test --list`, so it costs no backend and no
// browser, and a test that is deleted changes the table — which `--check`
// then reports, because the committed table no longer matches.

import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { FEATURES } from "./features.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const check = process.argv.includes("--check");
const LAYERS = ["api", "ui"];

const listed = spawnSync("pnpm", ["exec", "playwright", "test", "--list", "--reporter=json"], {
  cwd: here,
  encoding: "utf8",
  maxBuffer: 64 * 1024 * 1024,
});
if (listed.status !== 0) {
  console.error(listed.stderr || listed.stdout);
  process.exit(2);
}
const report = JSON.parse(listed.stdout);

const counts = new Map(FEATURES.map((f) => [f.id, { api: 0, ui: 0 }]));
const problems = [];
(function walk(suite, path) {
  for (const spec of suite.specs ?? []) {
    const tags = spec.tags ?? [];
    const feature = tags.find((t) => t.startsWith("feature:"))?.slice(8);
    const layer = tags.find((t) => t.startsWith("layer:"))?.slice(6);
    const where = [...path, spec.title].join(" › ");
    if (!feature || !layer) problems.push(`untagged: ${where}`);
    else if (!counts.has(feature)) problems.push(`unknown feature \`${feature}\`: ${where}`);
    else if (!LAYERS.includes(layer)) problems.push(`unknown layer \`${layer}\`: ${where}`);
    else counts.get(feature)[layer] += 1;
  }
  for (const child of suite.suites ?? []) walk(child, [...path, child.title]);
})({ suites: report.suites }, []);

const cell = (f, layer) => {
  const n = counts.get(f.id)[layer];
  const owed = f[layer];
  if (n === 0 && !owed) problems.push(`${f.id} has no ${layer} tests and does not say why (features.mjs)`);
  if (n > 0 && owed?.pending) problems.push(`${f.id} has ${n} ${layer} tests but is still marked pending ${owed.pending}`);
  if (n > 0) return String(n);
  if (owed?.na) return `— ${owed.na}`;
  if (owed?.pending) return `0 · ${owed.pending}`;
  return "**0**";
};

const total = { api: 0, ui: 0 };
for (const c of counts.values()) for (const l of LAYERS) total[l] += c[l];

const lines = [
  "# LIVE 覆盖表",
  "",
  "> 生成的，不要手改：`pnpm --filter @harnix/live-tests coverage`。来源是每个用例的 `@feature`、`@layer`",
  "> 标签；行与说明见 `features.mjs`，行的定义见[设计 §4.3](../../docs/harnix-0.4-design-and-plan.md)。",
  "> `0 · Mx` 是那个里程碑会补上的，`—` 是这一层不适用。",
  "",
  "| 功能模块 | id | API | 界面 |",
  "|---|---|---|---|",
  ...FEATURES.map((f) => `| ${f.name} | \`${f.id}\` | ${cell(f, "api")} | ${cell(f, "ui")} |`),
  `| **合计** | | **${total.api}** | **${total.ui}** |`,
  "",
];
const table = lines.join("\n");
const path = join(here, "coverage.md");

if (check) {
  let committed = "";
  try {
    committed = readFileSync(path, "utf8");
  } catch {
    /* missing is stale */
  }
  if (committed !== table) problems.push("coverage.md is stale: run `pnpm --filter @harnix/live-tests coverage` and commit it");
  if (problems.length) {
    console.error(problems.map((p) => `  ${p}`).join("\n"));
    process.exit(1);
  }
  console.log(`coverage.md is current: ${total.api} API and ${total.ui} interface tests`);
} else {
  writeFileSync(path, table);
  if (problems.length) {
    console.error(problems.map((p) => `  ${p}`).join("\n"));
    process.exit(1);
  }
  console.log(`wrote coverage.md: ${total.api} API and ${total.ui} interface tests`);
}
