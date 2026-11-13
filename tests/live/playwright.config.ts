/**
 * The LIVE suites, one runner (testing.md §1).
 *
 *   pnpm live                         both projects, replayed
 *   pnpm live --project api           the API suites alone
 *   pnpm live --live                  against the model in `.env`
 *   pnpm live --topology split_streams
 *   pnpm live --carrier process       the package suites, each package in a process of its own
 *
 * `api` opens no browser: it drives the backend through `@harnix/core`.
 * `ui` drives the interface in chromium. Both start the real binary through
 * the same fixtures (`@harnix/testkit`), one backend per worker.
 */

import { defineConfig, devices } from "@playwright/test";
import type { WorkerFixtures } from "@harnix/testkit";

import { CARRIER_PINNED, CARRIER_SUITES } from "./features.mjs";

// On the process carrier only the suites that hold on both run (`features.mjs`): the rest never
// start a package, so another run of them would test nothing new.
const onProcess = process.env.HARNIX_CARRIER === "process";
const carrierOnly = onProcess
  ? { testMatch: CARRIER_SUITES.map((g) => `**/${g}`), testIgnore: CARRIER_PINNED.map((g) => `**/${g}`) }
  : {};

export default defineConfig<{}, WorkerFixtures>({
  // One worker, for load rather than for anything shared: every backend picks a free port and
  // each worker writes its own share of the coverage. A worker starts a backend, many suites
  // start more of their own, and every one is an engine with threads of its own; two workers on
  // one machine made the deadlines measure the machine instead of the code.
  workers: 1,
  fullyParallel: false,
  globalTeardown: "./teardown.ts",
  reporter: process.env.CI ? "list" : [["list"]],
  outputDir: "../../test-results/live",
  projects: [
    {
      name: "api",
      testDir: "api",
      ...carrierOnly,
      // A turn waits up to three minutes on a live upstream.
      timeout: 300_000,
    },
    {
      name: "ui",
      testDir: "ui",
      ...carrierOnly,
      // Above every wait inside a test (their expects allow up to 30 s, a page opening 15 s more):
      // a test that runs out of budget says "Test timeout exceeded", not what it was waiting for.
      // Replay keeps it cheap — a wide budget costs nothing while the test is green.
      timeout: 120_000,
      // One more try, so one slow moment on a loaded machine does not fail the layer — and the
      // test that needed it is reported as flaky (scripts/check.sh lists them), not passed. Two
      // failures in a row are a failure.
      retries: 1,
      use: {
        ...devices["Desktop Chrome"],
        // A layout test that fails because a model was slow is a test people
        // learn to ignore: the interface always replays.
        mode: "replay",
        // Failures here are usually layout, and a screenshot is the only
        // useful artifact for that.
        screenshot: "only-on-failure",
        trace: "retain-on-failure",
      },
    },
  ],
});
