/**
 * @harnix/testkit —— LIVE 测试（设计 §13）：起真的 harnix，经 @harnix/core 说协议，从 Playwright 驱动。
 */

export { ROOT, RECORDINGS, type ModelMode, hasModel, loadEnv, modelMode, sleep } from "./env.ts";
export { type Backend, type LaunchOptions, binary, freePort, makeProject, startHarnix } from "./launcher.ts";
export { type Frame, type TestClient, Inconclusive, connect, coverage, finish, inconclusive } from "./api.ts";
export { type TestFixtures, type WorkerFixtures, expect, launchFor, test, topology } from "./fixtures.ts";
export { type Suite, type SuiteContext, type SuiteMeta, defineSuite } from "./suite.ts";

/** The old harness's name for `startHarnix`, so migrated suites read as they did. */
export { startHarnix as startBackend } from "./launcher.ts";
