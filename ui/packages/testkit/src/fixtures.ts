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
import { type ModelMode, ROOT, hasModel, loadEnv, modelMode } from "./env.ts";
import { type Backend, type LaunchOptions, startHarnix } from "./launcher.ts";

/** The topology the API suites speak this run: `HARNIX_TOPOLOGY`, duplex by default. */
export function topology(): Topology {
  return process.env.HARNIX_TOPOLOGY === "split_streams" ? "split_streams" : "single_duplex";
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
    // The interface as built (`pnpm --filter @harnix/shell build`), from disk rather than the copy
    // compiled in, so a front-end change is tested without rebuilding the binary.
    uiDir: join(ROOT, "ui", "dist"),
    ...extra,
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
}

export const test = base.extend<TestFixtures, WorkerFixtures>({
  // An option, so a project can pin it: the interface suites always replay.
  mode: [modelMode(), { scope: "worker", option: true }],

  harnix: [
    async ({ mode }, use, workerInfo) => {
      // The interface picks its own topology, so the backend offers only the one under test: a page
      // told to prefer one duplex connection has to fall back to split streams when that is all
      // there is.
      const only = workerInfo.project.name === "ui" && process.env.HARNIX_TOPOLOGY ? [topology()] : undefined;
      const backend = await startHarnix(launchFor(mode, only ? { topologies: only } : {}));
      await use(backend);
      backend.stop();
    },
    { scope: "worker", timeout: 300_000 },
  ],

  // What the API suites called, against what the backend says a client may
  // call. Each worker writes its share; the global teardown adds them up.
  apiCoverage: [
    async ({ harnix }, use, workerInfo) => {
      await use();
      const seen = coverage();
      if (!seen.called.length) return;
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

  api: async ({ harnix }, use) => {
    const client = await connect(harnix, { topology: topology() });
    await use(client);
    client.close();
  },
});

export { expect } from "@playwright/test";
