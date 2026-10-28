// The application scripts catch the drift they exist for (roadmap M2-10).
//
// Each one is shown a drift made on purpose, in a scratch directory, and has
// to say so: sync-app-deps a hand-edited block, check-lock a version that
// moved, check-pins an agix/ commit no remote has.

import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, realpathSync as realpath, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { block, rewrite } from "../scripts/sync-app-deps.mjs";
import { drift } from "../scripts/check-lock.mjs";
import { judge } from "../scripts/check-pins.mjs";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
let failures = 0;
const ok = (m) => console.log(`ok — ${m}`);
const fail = (m) => { console.log(`FAIL: ${m}`); failures += 1; };
const expect = (cond, yes, no) => (cond ? ok(yes) : fail(no));

/* ── sync-app-deps ─────────────────────────────────────────────────────── */
const template = readFileSync(join(root, "templates/app/Cargo.toml"), "utf8");
const synced = rewrite(template, block());
expect(synced.includes('harnix-hub       = { path = "harnix/crates/hub" }'),
  "sync-app-deps rebases Harnix's crates on the application root",
  "the generated block does not point at harnix/crates");
expect(synced.includes('daemon    = { path = "agix/daemon" }'),
  "and leaves agix/ where it sits, beside harnix/", "agix/ was rebased");
expect(rewrite(synced, block()) === synced, "and writing it twice changes nothing", "the block is not stable");
const tampered = synced.replace('tokio = { version = "1"', 'tokio = { version = "0.2"');
expect(rewrite(tampered, block()) !== tampered,
  "a hand-edited line is drift --check reports", "a hand-edited block passed");

/* ── check-lock ────────────────────────────────────────────────────────── */
const lock = readFileSync(join(root, "Cargo.lock"), "utf8");
expect(drift(lock, lock).length === 0, "check-lock: the same lock file has no drift", "a lock file drifts from itself");
const moved = lock.replace(/(name = "serde_json"\nversion = ")[^"]+/, "$10.0.1");
const found = drift(moved, lock);
expect(found.some((d) => d.name === "serde_json"),
  "check-lock: a moved version is reported", "check-lock missed a moved version");

/* ── check-pins ────────────────────────────────────────────────────────── */
const scratch = mkdtempSync(join(tmpdir(), "harnix-pins-"));
const sh = (cwd, ...args) => execFileSync("git", ["-C", cwd, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
const commit = (repo, message) => {
  writeFileSync(join(repo, "file"), `${message}\n`);
  sh(repo, "add", "file");
  sh(repo, "-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", message);
  return sh(repo, "rev-parse", "HEAD");
};
try {
  // Agix upstream with two commits, and the application's clone of it.
  const agixUp = join(scratch, "agix-up");
  mkdirSync(agixUp);
  sh(agixUp, "init", "-q", "-b", "main");
  const a1 = commit(agixUp, "a1");
  const a2 = commit(agixUp, "a2");
  const app = join(scratch, "app");
  mkdirSync(app);
  execFileSync("git", ["clone", "-q", agixUp, join(app, "agix")]);
  // Harnix pins a1, and its commit is on its remote.
  const harnixUp = join(scratch, "harnix-up");
  mkdirSync(harnixUp);
  sh(harnixUp, "init", "-q", "-b", "main");
  sh(harnixUp, "update-index", "--add", "--cacheinfo", `160000,${a1},agix`);
  commit(harnixUp, "harnix pins a1");
  execFileSync("git", ["clone", "-q", harnixUp, join(app, "harnix")]);

  const agix = join(app, "agix");
  sh(agix, "checkout", "-q", a1);
  let verdict = judge(app);
  expect(!verdict.problems.length && !verdict.warnings.length, "check-pins: the same pin passes",
    `the same pin did not pass: ${JSON.stringify(verdict)}`);

  sh(agix, "checkout", "-q", a2);
  verdict = judge(app);
  expect(!verdict.problems.length && verdict.warnings.length === 1,
    "check-pins: a newer upstream commit warns on a development branch", `got ${JSON.stringify(verdict)}`);
  verdict = judge(app, { release: true });
  expect(verdict.problems.length === 1, "and fails a release", `a release passed: ${JSON.stringify(verdict)}`);

  commit(agix, "local only");
  verdict = judge(app);
  expect(verdict.problems.some((p) => p.includes("no Agix remote")),
    "check-pins: a commit only this machine has fails", `got ${JSON.stringify(verdict)}`);

  sh(agix, "checkout", "-q", a1);
  commit(join(app, "harnix"), "a local patch to harnix");
  verdict = judge(app);
  expect(verdict.problems.some((p) => p.includes("no Harnix remote")),
    "check-pins: a harnix/ pointer no remote has fails", `got ${JSON.stringify(verdict)}`);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}

/* ── build.sh: the application's own pnpm workspace ────────────────────── */
// An application with its own workspace (spaces/*/ui, tests) installs it before the shell is
// built, or the shell build cannot resolve a space's third-party imports (ipdrun-32). Run with
// stand-ins for pnpm, cargo and Harnix's scripts that only write down how they were called.
{
  const app = mkdtempSync(join(tmpdir(), "harnix-build-sh-"));
  try {
    mkdirSync(join(app, "scripts"), { recursive: true });
    mkdirSync(join(app, "harnix", "scripts"), { recursive: true });
    mkdirSync(join(app, "bin"), { recursive: true });
    writeFileSync(join(app, "scripts", "build.sh"), readFileSync(join(root, "templates/app/scripts/build.sh")));
    for (const s of ["sync-app-deps.mjs", "check-pins.mjs", "check-lock.mjs"]) {
      writeFileSync(join(app, "harnix", "scripts", s), "");
    }
    const log = join(app, "calls.log");
    for (const tool of ["pnpm", "cargo"]) {
      writeFileSync(join(app, "bin", tool), `#!/bin/sh\necho "${tool} $* @ $(pwd -P)" >> "${log}"\n`, { mode: 0o755 });
    }
    const build = () => {
      writeFileSync(log, "");
      execFileSync("bash", [join(app, "scripts", "build.sh")], {
        cwd: app,
        env: { ...process.env, PATH: `${join(app, "bin")}:${process.env.PATH}` },
        stdio: ["ignore", "ignore", "inherit"],
      });
      return readFileSync(log, "utf8").trim().split("\n");
    };
    const rootInstall = (calls) =>
      calls.findIndex((c) => /^pnpm install --frozen-lockfile @ /.test(c) && c.endsWith(` @ ${realpath(app)}`));
    let calls = build();
    expect(rootInstall(calls) < 0, "build.sh: without its own workspace an app installs only harnix/",
      `it installed at the root anyway: ${calls.join(" | ")}`);
    writeFileSync(join(app, "pnpm-workspace.yaml"), "packages:\n  - spaces/*/ui\n");
    calls = build();
    const shell = calls.findIndex((c) => c.includes("--filter @harnix/shell build"));
    expect(rootInstall(calls) >= 0 && rootInstall(calls) < shell,
      "build.sh: with its own workspace the app's packages are installed before the shell is built",
      `calls were: ${calls.join(" | ")}`);
  } finally {
    rmSync(app, { recursive: true, force: true });
  }
}

if (failures) {
  console.log(`\nAPP SCRIPTS FAILED (${failures})`);
  process.exit(1);
}
console.log("\nAPP SCRIPTS PASSED");
