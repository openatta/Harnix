/**
 * Playwright fixtures: a `harnix` per worker, a connected `api` per test.
 *
 * One backend for a worker, not one per test: starting the engine costs
 * seconds and the suites share nothing but the process — every test that
 * needs a session makes its own. A client per test, not per suite: the event
 * log is per client, and sharing one let a test match a frame an earlier test
 * produced.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { test as base } from "@playwright/test";
import type { Topology } from "@harnix/core/protocol";

import { type TestClient, connect, coverage } from "./api.ts";
import { type ModelMode, ROOT, hasModel, loadEnv, modelMode, topology } from "./env.ts";
import { type Backend, type LaunchOptions, divergenceCount, divergencesSince, startHarnix } from "./launcher.ts";

export { topology };

/**
 * An interface test's backend offers only this run's topology, so the page has to get there by
 * itself: a page prefers one duplex connection, and offered both it would speak duplex in the
 * split-streams run too. An API test picks its own topology (`connect`), and some of them open
 * the other topology's door on purpose, so an API backend keeps both.
 */
function servedForRun(): Topology[] | undefined {
  if (!process.env.HARNIX_TOPOLOGY) return undefined;
  let project = "";
  try {
    project = test.info().project.name;
  } catch {
    /* outside a test or hook: not an interface test's backend */
  }
  return project === "ui" ? [topology()] : undefined;
}

/**
 * How a backend for this run is started, given the model mode.
 *
 * Replay is the default for anything model-shaped: recorded calls make an
 * agent's behaviour deterministic and free. `live` uses `.env`.
 */
export function launchFor(mode: ModelMode, extra: LaunchOptions = {}): LaunchOptions {
  const env = loadEnv();
  const model = mode === "live" && hasModel(env);
  return {
    replay: mode === "replay",
    // `pnpm live --strict`: a request that no longer matches its recording fails the call, rather
    // than being reported and played back anyway.
    replayStrict: process.env.HARNIX_REPLAY_STRICT === "1",
    // The interface as built (`pnpm --filter @harnix/shell build`), from disk rather than the copy
    // compiled in, so a front-end change is tested without rebuilding the binary.
    uiDir: join(ROOT, "ui", "dist"),
    ...extra,
    topologies: extra.topologies ?? servedForRun(),
    env: { ...(model ? env : { ANTHROPIC_API_KEY: "live-tests-no-model" }), ...extra.env },
  };
}

export interface WorkerFixtures {
  mode: ModelMode;
  harnix: Backend;
  apiCoverage: void;
}

export interface TestFixtures {
  api: TestClient;
  replayReport: void;
}

export const test = base.extend<TestFixtures, WorkerFixtures>({
  // An option, so a project can pin it: the interface suites always replay.
  mode: [modelMode(), { scope: "worker", option: true }],

  harnix: [
    async ({ mode }, use, workerInfo) => {
      // The interface picks its own topology, so its backend offers only the one under test (see
      // `servedForRun`; a worker fixture has no test to ask, so the project is read here).
      const only = workerInfo.project.name === "ui" && process.env.HARNIX_TOPOLOGY ? [topology()] : undefined;
      const backend = await startHarnix(launchFor(mode, only ? { topologies: only } : {}));
      await use(backend);
      await backend.dispose();
    },
    { scope: "worker", timeout: 300_000 },
  ],

  // What the API suites called, against what the backend says a client may
  // call. Each worker writes its share; the global teardown adds them up.
  apiCoverage: [
    async ({ harnix }, use, workerInfo) => {
      await use();
      const seen = coverage();
      // The interface suites call a few methods to clean up after themselves; that is not the API
      // coverage this counts.
      if (workerInfo.project.name !== "api" || !seen.called.length) return;
      let reachable: string[] = [];
      try {
        // The topology the backend offers: an interface suite's backend may serve only one.
        const probe = await connect(harnix, { topology: topology() });
        reachable = (await probe.call("harnix.reachable")).methods;
        probe.close();
      } catch {
        /* a backend that went away takes its list with it */
      }
      const dir = join(ROOT, "test-results", ".api-coverage");
      mkdirSync(dir, { recursive: true });
      writeFileSync(
        join(dir, `${workerInfo.workerIndex}-${process.pid}.json`),
        JSON.stringify({ reachable, ...seen }),
      );
    },
    { scope: "worker", auto: true },
  ],

  // `HARNIX_REPLAY_REPORT=1`: a replayed call that diverged from its recording fails the test it
  // happened in, though it was played back. Stricter than reading logs, gentler than `--strict`,
  // which fails the call itself: the test still runs to the end and says what else differed.
  replayReport: [
    async ({}, use) => {
      const from = divergenceCount();
      await use();
      if (process.env.HARNIX_REPLAY_REPORT !== "1") return;
      const found = divergencesSince(from);
      if (found.length) throw new Error(`${found.length} replayed call(s) diverged from their recording:\n${found.join("\n")}`);
    },
    { auto: true },
  ],

  api: async ({ harnix }, use) => {
    const client = await connect(harnix, { topology: topology() });
    await use(client);
    client.close();
  },
});

export { expect } from "@playwright/test";
