/**
 * After the run: what the API suites called, against what a client may call.
 *
 * Each worker wrote its share (`apiCoverage` in `@harnix/testkit`); this adds
 * them up and says it. "We tested the API" is a claim; "51 of 56 methods were
 * called, and here are the five that were not" is a number.
 */

import { existsSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { join } from "node:path";

import { ROOT } from "@harnix/testkit";

export default function teardown(): void {
  const dir = join(ROOT, "test-results", ".api-coverage");
  if (!existsSync(dir)) return;
  const reachable = new Set<string>();
  const called = new Set<string>();
  const errored = new Set<string>();
  const codes = new Map<number, string>();
  for (const file of readdirSync(dir)) {
    const part = JSON.parse(readFileSync(join(dir, file), "utf8"));
    for (const m of part.reachable) reachable.add(m);
    for (const m of part.called) called.add(m);
    for (const m of part.errored) errored.add(m);
    for (const [code, via] of part.codes) if (!codes.has(code)) codes.set(code, via);
  }
  rmSync(dir, { recursive: true, force: true });
  if (!reachable.size) return;

  const all = [...reachable].sort();
  const untested = all.filter((m) => !called.has(m));
  const failedCount = all.filter((m) => errored.has(m)).length;
  const neverFailed = all.filter((m) => called.has(m) && !errored.has(m));
  const pct = (n: number) => ((n / all.length) * 100).toFixed(0);

  const lines = ["", "── API coverage ──"];
  lines.push(`  ${all.length - untested.length}/${all.length} reachable methods called (${pct(all.length - untested.length)}%)`);
  if (untested.length) lines.push("  never called:", ...untested.map((m) => `    ${m}`));
  lines.push(`  ${failedCount}/${all.length} also driven into an error (${pct(failedCount)}%)`);
  if (neverFailed.length) lines.push("  happy path only:", ...neverFailed.map((m) => `    ${m}`));
  lines.push(
    `  ${codes.size} distinct error codes seen: `
      + [...codes.entries()].sort((a, b) => b[0] - a[0]).map(([c, via]) => `${c} (${via})`).join(", "),
  );
  console.log(lines.join("\n"));
}
