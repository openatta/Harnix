// `templates/app` builds, offline, as a fresh application would (M2-10).
//
// The template is copied into a scratch directory with `harnix/` and
// `agix/` beside each other (links to this checkout and its submodule), the
// dependency block is generated, and the application is built with
// `--offline`: a layout that needed the network to resolve Agix (design
// §11.2's first row) fails here rather than in someone's CI. Then the binary
// is asked its versions, and the lock file is compared with Harnix's.

import { execFileSync } from "node:child_process";
import { cpSync, mkdtempSync, rmSync, symlinkSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const app = mkdtempSync(join(tmpdir(), "harnix-app-"));
const run = (cmd, args, cwd = app) =>
  execFileSync(cmd, args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });

try {
  cpSync(join(root, "templates", "app"), app, { recursive: true });
  symlinkSync(root, join(app, "harnix"));
  symlinkSync(join(root, "agix"), join(app, "agix"));
  // Start from Harnix's lock, as an application that just added the
  // submodules would after its first `cargo generate-lockfile` — and so the
  // build needs nothing the registry cache does not already have.
  copyFileSync(join(root, "Cargo.lock"), join(app, "Cargo.lock"));

  console.log(run("node", [join(root, "scripts", "sync-app-deps.mjs"), app]).trim());
  console.log(run("node", [join(root, "scripts", "sync-app-deps.mjs"), app, "--check"]).trim());
  execFileSync("cargo", ["build", "--offline", "-p", "myapp"], {
    cwd: app,
    stdio: "inherit",
    env: { ...process.env, CARGO_TARGET_DIR: process.env.HARNIX_TEMPLATE_TARGET ?? join(app, "target") },
  });
  const target = process.env.HARNIX_TEMPLATE_TARGET ?? join(app, "target");
  const versions = run(join(target, "debug", "myapp"), ["--version"]);
  for (const line of ["myapp 0.1.0\n", "harnix ", "agix ", "module API ", "contribution API ", "protocol "]) {
    if (!versions.includes(line)) throw new Error(`--version does not say "${line}":\n${versions}`);
  }
  console.log("ok — the application builds offline and says every version:");
  console.log(versions.replace(/^/gm, "    "));
  console.log(run("node", [join(root, "scripts", "check-lock.mjs"), app]).trim());
  console.log("\nAPP TEMPLATE PASSED");
} finally {
  rmSync(app, { recursive: true, force: true });
}
