/**
 * What the extension-package suites share: a `harnix` that runs packages, and the showcase
 * (`examples/packages/showcase`) packed for it.
 *
 * `harnix` runs packages in its default build, on both carriers: in this process (`wasm`, the
 * default) and each in a process of its own (`proc`). Which one a backend uses is the run's
 * (`pnpm live --carrier`, testkit's `carrier()`), unless a suite's profile says. Beside it, in a
 * target directory of their own (`target/packages`) so the default build stays as it is: `harnix-pack`,
 * the author's packing tool, built with the engine's compiler; and `harnix-testapp` with both
 * carriers (`packages-store.ts`). `scripts/check.sh` builds all three and says so (`HARNIX_BIN`);
 * run alone, a suite has cargo build them.
 *
 * The showcase is packed once per change to its sources (or to the WIT, or to the tools): its
 * component built for wasm32-wasip2, compiled for this binary's engine, zipped with its manifest,
 * `agent/` and `ui/`. The package is cached under `target/packages/showcase/`.
 */

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { crc32, inflateRawSync } from "node:zlib";

import { type Backend, type LaunchOptions, type ModelMode, type TestClient, ROOT, binary, carrier, expect, hasModel, launchFor, loadEnv, sleep, startHarnix, test } from "@harnix/testkit";

export const TARGET = join(ROOT, "target", "packages");
export const SHOWCASE = join(ROOT, "examples", "packages", "showcase");
export const SHOWCASE_ID = "example.showcase";
/** The showcase's second space: the shell's session list, grouped as its backend answers. */
export const SHOWCASE_CHATS = "example.showcase.chats";

/**
 * A release build in `target/packages`, unless the caller built it already (`HARNIX_BIN` says the
 * caller built everything): cargo is asked every run, as testkit's `binary()` does.
 */
export function buildBeside(pkg: string, args: string[]): void {
  if (process.env.HARNIX_BIN) return;
  execFileSync("cargo", ["build", "--release", "--target-dir", TARGET, "-p", pkg, ...args], {
    cwd: ROOT,
    stdio: ["ignore", "ignore", "inherit"],
  });
}

let built: { bin: string; pack: string } | null = null;

/** `harnix` as it ships, and the packing tool beside the test application. */
export function appsTools(): { bin: string; pack: string } {
  if (built) return built;
  buildBeside("harnix-package", ["--features", "compile", "--bin", "harnix-pack"]);
  built = { bin: binary("harnix"), pack: join(TARGET, "release", "harnix-pack") };
  return built;
}

/** Every file under `dir` but build output, hashed with its path. */
function hashTree(hash: ReturnType<typeof createHash>, dir: string, root = dir): void {
  for (const name of readdirSync(dir).sort()) {
    if (name === "target" || name === "node_modules") continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) hashTree(hash, path, root);
    else {
      hash.update(relative(root, path));
      hash.update(readFileSync(path));
    }
  }
}

let packed: { path: string; fingerprint: string } | null = null;

/** The showcase, packed for the apps binary's engine: the `.hxext`, and the fingerprint. */
export function showcasePackage(): { path: string; fingerprint: string } {
  if (packed) return packed;
  const { pack } = appsTools();
  const hash = createHash("sha256");
  hashTree(hash, SHOWCASE);
  hashTree(hash, join(ROOT, "crates", "wasm", "wit"));
  hash.update(String(statSync(pack).mtimeMs));
  const dir = join(TARGET, "showcase", hash.digest("hex").slice(0, 16));
  const out = join(dir, "showcase.hxext");
  const fingerprintFile = join(dir, "fingerprint");
  if (!existsSync(out) || !existsSync(fingerprintFile)) {
    mkdirSync(dir, { recursive: true });
    const componentTarget = join(TARGET, "showcase", "component");
    execFileSync("cargo", ["build", "--release", "--target", "wasm32-wasip2"], {
      cwd: join(SHOWCASE, "component"),
      env: { ...process.env, CARGO_TARGET_DIR: componentTarget },
      stdio: ["ignore", "ignore", "inherit"],
    });
    const wasm = join(componentTarget, "wasm32-wasip2", "release", "showcase_tools.wasm");
    const printed = execFileSync(pack, ["pack", SHOWCASE, "--wasm", wasm, "-o", out], { encoding: "utf8" });
    writeFileSync(fingerprintFile, printed.trim().split("\n").at(-1)!);
  }
  packed = { path: out, fingerprint: readFileSync(fingerprintFile, "utf8").trim() };
  return packed;
}

/** The `[ext.packages]` line that puts packages on the run's carrier, unless `own` (the rest of
 *  the section, and the profile's other tables after it) names one in that section: an
 *  instance's `carrier` under its own table is another key. */
export function onCarrier(own = ""): string {
  const section = own.split(/^\s*\[/m)[0];
  return /^\s*carrier\s*=/m.test(section) ? "" : `carrier = "${carrier()}"\n`;
}

/** A backend of `harnix` (or `bin`) running packages, `dev` unless said otherwise, on the run's
 *  carrier unless the profile names one; replaying unless given another mode. */
export function startApps(
  options: {
    bin?: string;
    dev?: boolean;
    profile?: string;
    seed?: LaunchOptions["seed"];
    mode?: ModelMode;
    env?: Record<string, string>;
    topologies?: LaunchOptions["topologies"];
  } = {},
): Promise<Backend> {
  const bin = options.bin ?? appsTools().bin;
  const own = options.profile ?? "";
  return startHarnix(
    launchFor(options.mode ?? "replay", {
      bin,
      seed: options.seed,
      env: options.env,
      topologies: options.topologies,
      profile: `
[ext.packages]
dev = ${options.dev ?? true}
${onCarrier(own)}${own}`,
    }),
  );
}

/**
 * The live run's model as a provider on the settings page, for an extension's own engine: that
 * engine has the product's providers and nothing of its environment, and the run hands `main` its
 * model by environment alone. A provider that changes is pushed to the engine at once.
 */
export async function provideModel(api: TestClient): Promise<void> {
  const env = loadEnv();
  test.skip(!hasModel(env), "no model in .env");
  const config: Record<string, string> = { api_type: "anthropic", api_key: env.ANTHROPIC_AUTH_TOKEN || env.ANTHROPIC_API_KEY || "" };
  if (env.ANTHROPIC_BASE_URL) config.base_url = env.ANTHROPIC_BASE_URL;
  if (env.ANTHROPIC_MODEL) config.default_model = env.ANTHROPIC_MODEL;
  const set = await api.call("settings.setProvider", { provider_id: "live-run", config, default_provider: "live-run" });
  expect(set.reloaded_instances, "the extension's engine was not given the provider").toContain(`proc.${SHOWCASE_ID}`);
}

/** What the showcase's last `recall {later}` timer came to (its `state.recalled`), once it fired. */
export async function recalledOf(api: TestClient): Promise<any> {
  const deadline = Date.now() + 10_000;
  for (;;) {
    const recalled = (await api.call(`${SHOWCASE_ID}.state`)).recalled;
    if (recalled) return recalled;
    if (Date.now() > deadline) throw new Error("the showcase's recall timer never fired");
    await sleep(50);
  }
}

/* ── zips, for the packages that should be refused ──────────────────────── */

export type Entries = Map<string, Buffer>;

/** The entries of a zip (stored or deflated). */
export function readZip(path: string): Entries {
  const data = readFileSync(path);
  let end = data.length - 22;
  while (end >= 0 && data.readUInt32LE(end) !== 0x06054b50) end -= 1;
  const count = data.readUInt16LE(end + 10);
  let at = data.readUInt32LE(end + 16);
  const entries: Entries = new Map();
  for (let i = 0; i < count; i += 1) {
    const method = data.readUInt16LE(at + 10);
    const compressed = data.readUInt32LE(at + 20);
    const nameLength = data.readUInt16LE(at + 28);
    const extra = data.readUInt16LE(at + 30);
    const comment = data.readUInt16LE(at + 32);
    const local = data.readUInt32LE(at + 42);
    const name = data.toString("utf8", at + 46, at + 46 + nameLength);
    const start = local + 30 + data.readUInt16LE(local + 26) + data.readUInt16LE(local + 28);
    const raw = data.subarray(start, start + compressed);
    if (!name.endsWith("/")) entries.set(name, method === 8 ? inflateRawSync(raw) : Buffer.from(raw));
    at += 46 + nameLength + extra + comment;
  }
  return entries;
}

/** A zip of these entries, stored. */
export function writeZip(path: string, entries: Entries): void {
  const locals: Buffer[] = [];
  const centrals: Buffer[] = [];
  let offset = 0;
  for (const [name, body] of entries) {
    const nameBytes = Buffer.from(name, "utf8");
    const crc = crc32(body);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(body.length, 18);
    local.writeUInt32LE(body.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(body.length, 20);
    central.writeUInt32LE(body.length, 24);
    central.writeUInt16LE(nameBytes.length, 28);
    central.writeUInt32LE(offset, 42);
    locals.push(local, nameBytes, body);
    centrals.push(central, nameBytes);
    offset += 30 + nameBytes.length + body.length;
  }
  const directory = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.size, 8);
  end.writeUInt16LE(entries.size, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, Buffer.concat([...locals, directory, end]));
}

/** The showcase package with `edit` applied to its entries, at `path`. */
export function variant(path: string, edit: (entries: Entries) => void): string {
  const entries = readZip(showcasePackage().path);
  edit(entries);
  writeZip(path, entries);
  return path;
}
