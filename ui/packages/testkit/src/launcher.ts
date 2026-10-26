/**
 * Start a real `harnix`.
 *
 * Each one gets everything of its own: HOME, data root, engine root, project,
 * port, recordings. A test that inherits yesterday's sessions — or a
 * developer's memories under `~/.agix` — is a test whose failures depend on
 * what somebody did yesterday (IPDrun N-003).
 */

import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import type { Topology } from "@harnix/core/protocol";

import { RECORDINGS, ROOT, sleep } from "./env.ts";

export interface LaunchOptions {
  /** Which binary: `harnix` (the empty application) or another bin of this workspace. */
  bin?: string;
  /** A fixed port. Without one, a free port is picked. */
  port?: number;
  /** Extra environment for the process (a model credential, a log level). */
  env?: Record<string, string>;
  /** Extra command-line arguments. */
  args?: string[];
  /** Served topologies. Both by default: a suite picks which one it speaks. */
  topologies?: Topology[];
  /** Play model calls back from the recordings. */
  replay?: boolean;
  /** Extra profile TOML, appended after `[transport]`. */
  profile?: string;
  /**
   * State that has to exist before the engine reads it. Some behaviour is
   * only reachable at startup — an MCP server that fails to connect — and no
   * method can provoke it.
   */
  seed?: (scratch: string) => void;
  /** Serve the interface from this directory instead of the embedded one. */
  uiDir?: string;
  /**
   * Run in this directory, a previous backend's `scratch`: its sessions,
   * ledger and data are still there. What `Backend.restart` passes.
   */
  scratch?: string;
}

export interface Backend {
  port: number;
  url: string;
  token: string;
  /** Everything this process owns; removed by `stop`. */
  scratch: string;
  /** The project its sessions run in — see `makeProject`. */
  project: string;
  /** The process id, for a test that kills it. */
  pid: number;
  log(): string;
  /** End the process and remove everything it owned. */
  stop(): void;
  /** End the process, keep its directory, and wait until it is gone. */
  halt(signal?: NodeJS.Signals): Promise<void>;
  /** End it and start another on the same directory and port. */
  restart(): Promise<Backend>;
}

/**
 * The project a test's sessions run in: a directory of its own, never this
 * repository. The engine keeps per-project state inside the project, so
 * running against the checkout made the tests depend on a developer's own
 * sessions — one remembered fact put a memory-recall call in front of every
 * turn and every fixture played back one call out of step (N-003).
 */
export function makeProject(scratch: string): string {
  const project = join(scratch, "project");
  mkdirSync(project, { recursive: true });
  writeFileSync(join(project, "Cargo.toml"), '[package]\nname = "fixture"\nversion = "0.1.0"\n');
  writeFileSync(join(project, "README.md"), "A project for the tests to run in.\n");
  return project;
}

const built = new Map<string, string>();

/**
 * The binary under test, current as of this process.
 *
 * Asked every run rather than only when the file is missing: cargo is the only
 * thing that can tell whether there is anything to rebuild, and a green run
 * against yesterday's engine is worse than no run. `HARNIX_BIN` skips the
 * build — for a caller that has just built it.
 */
export function binary(name = "harnix"): string {
  const known = built.get(name);
  if (known) return known;
  const path = join(ROOT, "target", "release", name);
  // `HARNIX_BIN` names the `harnix` binary a caller has just built; the
  // other binaries sit next to it.
  if (process.env.HARNIX_BIN) {
    const given = name === "harnix" ? process.env.HARNIX_BIN : join(dirname(process.env.HARNIX_BIN), name);
    built.set(name, given);
    return given;
  }
  const pkg = name === "harnix" ? "harnix" : name;
  execFileSync("cargo", ["build", "--release", "-p", pkg], {
    cwd: ROOT,
    stdio: existsSync(path) ? ["ignore", "ignore", "inherit"] : "inherit",
  });
  built.set(name, path);
  return path;
}

export async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : 0;
      server.close(() => resolve(port));
    });
  });
}

export async function startHarnix(options: LaunchOptions = {}): Promise<Backend> {
  const bin = binary(options.bin);
  const port = options.port ?? (await freePort());
  const reused = options.scratch !== undefined;
  const scratch = options.scratch ?? mkdtempSync(join(tmpdir(), "harnix-live-"));
  if (!reused) options.seed?.(scratch);
  const project = makeProject(scratch);
  const home = join(scratch, "home");
  mkdirSync(home, { recursive: true });

  const topologies = options.topologies ?? ["single_duplex", "split_streams"];
  const profile = join(scratch, "profile.toml");
  writeFileSync(
    profile,
    [
      "[transport]",
      `topologies = [${topologies.map((t) => JSON.stringify(t)).join(", ")}]`,
      'host = "127.0.0.1"',
      `port = ${port}`,
      "",
      options.profile ?? "",
    ].join("\n"),
  );

  const args = [
    "--profile", profile,
    "--port", String(port),
    "--agix-dir", join(scratch, "agix"),
    "--data-dir", join(scratch, "projects"),
    ...(options.uiDir ? ["--ui-dir", options.uiDir] : []),
    ...(options.replay ? ["--replay-dir", RECORDINGS] : []),
    ...(options.args ?? []),
  ];
  const child: ChildProcess = spawn(bin, args, {
    cwd: project,
    env: {
      ...process.env,
      HOME: home,
      ...options.env,
      RUST_LOG: process.env.HARNIX_TEST_LOG || "warn",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let log = "";
  child.stdout?.on("data", (d) => (log += d));
  child.stderr?.on("data", (d) => (log += d));
  let exited: number | null = null;
  child.on("exit", (code) => (exited = code ?? -1));

  const url = `http://127.0.0.1:${port}/`;
  const tokenFile = join(scratch, "projects", ".harnix", "token");
  // A restart finds the last run's token; it is rewritten before serving.
  if (reused) rmSync(tokenFile, { force: true });
  const deadline = Date.now() + 60_000;
  for (;;) {
    if (exited !== null) {
      if (!reused) rmSync(scratch, { recursive: true, force: true });
      throw new Error(`harnix exited (${exited}) before it was up:\n${log}`);
    }
    try {
      const response = await fetch(url);
      if (response.ok && existsSync(tokenFile)) break;
    } catch {
      /* not up yet */
    }
    if (Date.now() > deadline) {
      child.kill();
      throw new Error(`harnix did not start in 60s:\n${log}`);
    }
    await sleep(150);
  }
  const token = readFileSync(tokenFile, "utf8").trim();

  const halt = async (signal: NodeJS.Signals = "SIGTERM") => {
    if (exited !== null) return;
    const gone = new Promise<void>((resolve) => child.once("exit", () => resolve()));
    try {
      child.kill(signal);
    } catch {
      return;
    }
    await gone;
  };
  return {
    port,
    url,
    token,
    scratch,
    project,
    pid: child.pid ?? 0,
    log: () => log,
    stop() {
      try {
        child.kill();
      } catch {
        /* already gone */
      }
      rmSync(scratch, { recursive: true, force: true });
    },
    halt,
    async restart() {
      await halt();
      return startHarnix({ ...options, scratch, port });
    },
  };
}
