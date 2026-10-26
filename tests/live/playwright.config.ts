/**
 * The LIVE suites, one runner (design §13.1).
 *
 *   pnpm live                         both projects, replayed
 *   pnpm live --project api           the API suites alone
 *   pnpm live --live                  against the model in `.env`
 *   pnpm live --topology split_streams
 *
 * `api` opens no browser: it drives the backend through `@harnix/core`.
 * `ui` drives the interface in chromium. Both start the real binary through
 * the same fixtures (`@harnix/testkit`), one backend per worker.
 */

import { defineConfig, devices } from "@playwright/test";
import type { WorkerFixtures } from "@harnix/testkit";

export default defineConfig<{}, WorkerFixtures>({
  // One worker: the API suites count coverage per process and several share
  // fixed ports for backends of their own.
  workers: 1,
  fullyParallel: false,
  globalTeardown: "./teardown.ts",
  reporter: process.env.CI ? "list" : [["list"]],
  outputDir: "../../test-results/live",
  projects: [
    {
      name: "api",
      testDir: "api",
      // A turn waits up to three minutes on a live upstream.
      timeout: 300_000,
    },
    {
      name: "ui",
      testDir: "ui",
      timeout: 30_000,
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
