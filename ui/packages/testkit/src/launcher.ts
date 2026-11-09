/**
 * Start a real `harnix`.
 *
 * Each one gets everything of its own: HOME, data root, engine root, project,
 * port, recordings. A test that inherits yesterday's sessions — or a
 * developer's memories under `~/.agix` — is a test whose failures depend on
 * what somebody did yesterday (IPDrun N-003).
 */

import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import type { Topology } from "@harnix/core/protocol";

import { RECORDINGS, ROOT, sleep } from "./env.ts";

export interface LaunchOptions {
  /** Which binary: `harnix` (the Harnix app), another bin of this workspace, or an absolute
   *  path to an application built on its own. */
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
  /** With `replay`: a request that differs from the recorded one fails the call (`--replay-strict`, ipdrun-45). */
  replayStrict?: boolean;
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
  /** Everything this process owns; removed by `dispose`. */
  scratch: string;
  /** The program's own directory, `$HOME/.<product_id>`: token, ledgers, modules' data. */
  state: string;
  /** The project its sessions run in — see `makeProject`. */
  project: string;
  /** The process id, for a test that kills it. */
  pid: number;
  log(): string;
  /**
   * End the process and remove everything it owned. Does not wait for the process to exit, so a
   * backend still finishing (its modules' `stop`, the engine writing out) can race the removal and
   * leave part of the directory behind, and the next backend starts while this one is still
   * shutting down.
   *
   * @deprecated Use `await dispose()`. Kept so a suite written against it still runs.
   */
  stop(): void;
  /**
   * End the process, wait until it has exited (SIGKILL after a grace period), then remove
   * everything it owned.
   */
  dispose(): Promise<void>;
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
  // An application built elsewhere (examples/hello-app): used as it is.
  if (name.startsWith("/")) return name;
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
  const pkg = name === "harnix" ? "harnix-cli" : name;
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

/** Whether a process is still there (signal 0 asks without sending anything). */
export function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/** Wait up to `within` ms for a process to be gone; whether it is. */
export async function gone(pid: number, within = 3_000): Promise<boolean> {
  const deadline = Date.now() + within;
  while (alive(pid) && Date.now() < deadline) await sleep(100);
  return !alive(pid);
}

/* ── leaks ────────────────────────────────────────────────────────────── */
//
// A run that is interrupted — Ctrl-C, a worker that crashes, a `beforeAll` that throws after
// `startHarnix` — must not leave backends running on ports nobody knows. Every backend is started
// in a process group of its own and kept here until it exits; when this process exits, each group
// is told to end, which takes whatever the backend spawned (a daemon, a tool's process) with it.

const running = new Set<ChildProcess>();
let guarded = false;

function endGroup(child: ChildProcess, signal: NodeJS.Signals): void {
  if (!child.pid) return;
  try {
    process.kill(-child.pid, signal);
  } catch {
    /* already gone */
  }
}

function guardLeaks(): void {
  if (guarded) return;
  guarded = true;
  process.on("exit", () => {
    for (const child of running) endGroup(child, "SIGTERM");
  });
  // A signal ends node without an `exit` event unless somebody listens. Where nobody else does (a
  // script, not a Playwright worker), end the backends and then let the signal do what it would.
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"] as const) {
    if (process.listenerCount(signal) > 0) continue;
    const onSignal = () => {
      for (const child of running) endGroup(child, "SIGTERM");
      process.removeListener(signal, onSignal);
      process.kill(process.pid, signal);
    };
    process.on(signal, onSignal);
  }
}

/** How long `dispose()` lets a backend finish before it is killed: the backend's own shutdown is
 *  bounded by 10 s, so this is past it. */
const DISPOSE_GRACE_MS = 15_000;

/** `rm -rf`, again for a moment when something is still letting go of a file in it. */
async function removeTree(dir: string): Promise<void> {
  for (let attempt = 0; ; attempt += 1) {
    try {
      rmSync(dir, { recursive: true, force: true });
      return;
    } catch (e: any) {
      if (attempt >= 20 || !["ENOTEMPTY", "EBUSY", "EPERM"].includes(e?.code)) throw e;
      await sleep(100);
    }
  }
}

export async function startHarnix(options: LaunchOptions = {}): Promise<Backend> {
  const bin = binary(options.bin);
  const port = options.port ?? (await freePort());
  const reused = options.scratch !== undefined;
  // The real path: on macOS `tmpdir()` is `/var/…`, a link to `/private/var/…`, and the engine's
  // working directory is the real one. Handing it the other spelling of the same folder made it read
  // that project's history twice, and list every session past its in-memory cap twice (an Agix
  // `session.list` defect: `.local/agix-issue-08-session-list-duplicates.md`).
  const scratch = options.scratch ?? realpathSync(mkdtempSync(join(tmpdir(), "harnix-live-")));
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
    ...(options.replay && options.replayStrict ? ["--replay-strict"] : []),
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
    // Its own process group, so what it spawns can be ended with it (see `guardLeaks`).
    detached: true,
  });
  guardLeaks();
  running.add(child);
  child.on("exit", () => running.delete(child));
  let log = "";
  child.stdout?.on("data", (d) => (log += d));
  child.stderr?.on("data", (d) => (log += d));
  let exited: number | null = null;
  child.on("exit", (code) => (exited = code ?? -1));

  const url = `http://127.0.0.1:${port}/`;
  // `~/.<product_id>`: `[brand] product_id` in the profile names it, `harnix` otherwise.
  const product = /product_id\s*=\s*"([a-z][a-z0-9-]*)"/.exec(options.profile ?? "")?.[1] ?? "harnix";
  const state = join(home, `.${product}`);
  const tokenFile = join(state, "token");
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
      // Under launch links (`--launch-link`) the page is refused until a link is opened: a 401
      // that says so is a backend that is up.
      const up = response.ok || response.status === 401;
      if (up && existsSync(tokenFile)) break;
    } catch {
      /* not up yet */
    }
    if (Date.now() > deadline) {
      endGroup(child, "SIGKILL");
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
    state,
    project,
    pid: child.pid ?? 0,
    log: () => log,
    stop() {
      try {
        child.kill();
      } catch {
        /* already gone */
      }
      // A file the exiting process writes while the tree is walked fails the last `rmdir` with
      // ENOTEMPTY — on a loaded machine often enough to matter. Not waiting is the point of
      // `stop()`, so what it leaves behind stays in the temp directory.
      try {
        rmSync(scratch, { recursive: true, force: true });
      } catch (e: any) {
        if (!["ENOTEMPTY", "EBUSY", "EPERM"].includes(e?.code)) throw e;
      }
    },
    async dispose() {
      if (exited === null) {
        const gone = new Promise<void>((resolve) => child.once("exit", () => resolve()));
        try {
          child.kill("SIGTERM");
        } catch {
          /* already gone */
        }
        const late = await Promise.race([gone.then(() => false), sleep(DISPOSE_GRACE_MS).then(() => true)]);
        if (late && exited === null) {
          // The whole group: what a backend that would not stop had spawned goes with it.
          endGroup(child, "SIGKILL");
          await gone;
        }
      }
      await removeTree(scratch);
    },
    halt,
    async restart() {
      await halt();
      return startHarnix({ ...options, scratch, port });
    },
  };
}
