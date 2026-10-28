/**
 * The API suites' shape, run under Playwright Test.
 *
 * A suite is `{ tests: { title: async ({ client, backend, env, model, replay, ...shared }) => … } }`
 * with optional `setup` / `teardown` and three flags. That shape predates the
 * move to Playwright (M1) and is kept: the bodies moved over unchanged, so a
 * migrated suite is the same test, not a rewrite that might mean something
 * else. New suites can use `test` and the `api` fixture directly.
 */

import { type TestClient, Inconclusive, connect } from "./api.ts";
import { hasModel, loadEnv } from "./env.ts";
import { launchFor, test, topology } from "./fixtures.ts";
import { type Backend, startHarnix } from "./launcher.ts";

export interface SuiteContext {
  client: TestClient;
  backend: Backend;
  env: Record<string, string>;
  /** A live model is configured and asked for. */
  model: boolean;
  /** Model calls are played back from recordings. */
  replay: boolean;
  [shared: string]: any;
}

export interface Suite {
  /** Calls a model: runs replayed by default, skipped when the run has no model at all. */
  needsModel?: boolean;
  /** Something a recording cannot stand in for: runs only with `--live`. */
  needsLiveModel?: boolean;
  /**
   * Its value is that the same tool is called at the same point every run, so
   * it gets a replaying backend of its own whatever mode the run is in —
   * against a live model it would be a different test wearing the same name.
   */
  replayed?: boolean;
  setup?: (ctx: { backend: Backend; env: Record<string, string> }) => Promise<Record<string, any>>;
  teardown?: (ctx: { backend: Backend; env: Record<string, string>; [shared: string]: any }) => Promise<unknown>;
  tests: Record<string, (ctx: SuiteContext) => Promise<unknown>>;
}

export interface SuiteMeta {
  /** The functional module (design §4.3) this suite covers: `@feature:<id>`. */
  feature: string;
  /** Per-test overrides, for a suite whose tests belong to more than one. */
  features?: Record<string, string>;
}

export function defineSuite(name: string, meta: SuiteMeta, suite: Suite): void {
  test.describe(name, () => {
    const env = loadEnv();
    let shared: Record<string, any> = {};
    let own: Backend | null = null;

    test.beforeAll(async ({ harnix, mode }) => {
      test.skip(Boolean(suite.needsModel) && mode === "none", "no model and no recordings");
      test.skip(Boolean(suite.needsLiveModel) && !(mode === "live" && hasModel(env)), "needs --live");
      if (suite.replayed && mode !== "replay") own = await startHarnix(launchFor("replay"));
      if (suite.setup) shared = await suite.setup({ backend: own ?? harnix, env });
    });

    test.afterAll(async ({ harnix }) => {
      if (suite.teardown) await suite.teardown({ backend: own ?? harnix, env, ...shared }).catch(() => {});
      await own?.dispose();
      own = null;
    });

    for (const [title, run] of Object.entries(suite.tests)) {
      const feature = meta.features?.[title] ?? meta.feature;
      test(title, { tag: [`@feature:${feature}`, "@layer:api"] }, async ({ harnix, mode }) => {
        const backend = own ?? harnix;
        const client = await connect(backend, { topology: topology() });
        try {
          await run({
            client,
            backend,
            env,
            model: mode === "live" && hasModel(env),
            replay: mode === "replay" || Boolean(own),
            ...shared,
          });
        } catch (e) {
          if (e instanceof Inconclusive) {
            test.info().annotations.push({ type: "inconclusive", description: e.message });
            test.skip(true, `inconclusive: ${e.message}`);
          }
          throw e;
        } finally {
          client.close();
        }
      });
    }
  });
}
