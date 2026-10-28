/**
 * Building an instance as a caller means it (ipdrun-04, ipdrun-05).
 *
 * `ensure` is the call that needs no coordination: any number of them for one name, at once, all
 * get the one instance. And a caller that is about to use the instance can say so — `wait: "ready"`
 * answers once a spawned daemon is up, at once when it is down, and when the time runs out.
 */

import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { INSTANCE_UNAVAILABLE, INVALID_PARAMS } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:instances", "@layer:api"] };

/**
 * A daemon from some other Agix, as a program a spawn template starts: it listens where it is told
 * and answers `daemon.ping` with a protocol version nobody speaks, so its instance goes down.
 */
const FOREIGN_DAEMON = `#!/usr/bin/env node
const net = require("node:net");
const socket = process.argv[process.argv.indexOf("--socket") + 1];
net.createServer((conn) => {
  let buffer = "";
  conn.on("data", (chunk) => {
    buffer += chunk;
    for (let at = buffer.indexOf("\\n"); at >= 0; at = buffer.indexOf("\\n")) {
      const request = JSON.parse(buffer.slice(0, at));
      buffer = buffer.slice(at + 1);
      const result = request.method === "daemon.ping"
        ? { pong: true, version: "9.9.9", protocol_version: 999, capabilities: [] } : {};
      conn.write(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }) + "\\n");
    }
  });
}).listen(socket);
process.stdin.on("end", () => process.exit(0));
process.stdin.resume();
`;

let dir: string;
let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "hx-ready-"));
  const foreign = join(dir, "foreign-daemon.js");
  writeFileSync(foreign, FOREIGN_DAEMON);
  chmodSync(foreign, 0o755);
  backend = await startTestApp({
    profile: `
[[instance_templates]]
id = "worker"
carrier = "spawn"
ui = true

[[instance_templates]]
id = "foreign"
carrier = "spawn"
ui = true
program = "${foreign}"
`,
  });
});
test.afterAll(async () => {
  await backend?.dispose();
  rmSync(dir, { recursive: true, force: true });
});
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

async function listed(id: string): Promise<any[]> {
  const { instances } = await api.call("harnix.instances.list");
  return instances.filter((i: any) => i.id === id);
}

test("eight ensures of one name at once, from a module, are one in-process instance", TAGS, async () => {
  const { answers } = await api.call("app.fleet.ensure_many", { template: "fleet.side", name: "par-side", count: 8 });
  expect(answers).toHaveLength(8);
  for (const a of answers) expect(a, JSON.stringify(a)).toEqual({ id: "par-side" });
  expect(await listed("par-side")).toHaveLength(1);
});

test("eight ensures of one name at once, from a device, are one spawned instance", TAGS, async () => {
  const all = await Promise.all(
    Array.from({ length: 8 }, () => api.call("harnix.instances.ensure", { template: "worker", name: "par-worker" })));
  for (const view of all) expect(view.id).toBe("par-worker");
  expect(await listed("par-worker")).toHaveLength(1);
});

test("two creates of one name at once: one is built, the other refused", TAGS, async () => {
  const both = await Promise.allSettled([
    api.call("harnix.instances.create", { template: "worker", name: "dup-worker" }),
    api.call("harnix.instances.create", { template: "worker", name: "dup-worker" }),
  ]);
  const built = both.filter((r) => r.status === "fulfilled");
  const refused = both.filter((r) => r.status === "rejected") as PromiseRejectedResult[];
  expect(built).toHaveLength(1);
  expect(refused).toHaveLength(1);
  expect(refused[0]!.reason.code).toBe(INVALID_PARAMS);
  expect(await listed("dup-worker")).toHaveLength(1);
});

test("ensure with wait: ready answers once the daemon is up, and a session follows at once", TAGS, async () => {
  const view = await api.call("harnix.instances.ensure", { template: "worker", name: "ready-worker", wait: "ready" });
  expect(view.status.state).toBe("ready");
  const created = await api.call("session.create", { instance: "ready-worker", project_root: api.project });
  expect(created.instance).toBe("ready-worker");
  // Without `wait` the answer is the same as ever: it may still be starting.
  const plain = await api.call("harnix.instances.ensure", { template: "worker", name: "plain-worker" });
  expect(["starting", "ready"]).toContain(plain.status.state);
});

test("a module's ensure_ready is followed by a session at once", TAGS, async () => {
  const answer = await api.call("app.fleet.ready_session", { template: "worker", name: "module-worker" });
  expect(answer.status.state).toBe("ready");
  expect(answer.session_id).toBeTruthy();
  expect(answer.waited.state).toBe("ready");
});

test("a daemon that goes down fails the wait at once, saying why", TAGS, async () => {
  const asked = Date.now();
  const error = await api.refused("harnix.instances.ensure", { template: "foreign", name: "foreign-1", wait: "ready" });
  expect(error?.code).toBe(INSTANCE_UNAVAILABLE);
  expect(error?.message).toContain("protocol version 999");
  expect(Date.now() - asked).toBeLessThan(30_000);
});

test("a wait that runs out of time says so", TAGS, async () => {
  const error = await api.refused("harnix.instances.ensure",
    { template: "worker", name: "slow-worker", wait: "ready", timeout_ms: 1 });
  expect(error?.code).toBe(INSTANCE_UNAVAILABLE);
  expect(error?.message).toContain("not ready within 1 ms");
  expect(error?.data?.instance).toBe("slow-worker");
  // And harnix.instances.wait is the same wait for an instance that exists.
  const waited = await api.call("harnix.instances.wait", { instance: "slow-worker" });
  expect(waited.status.state).toBe("ready");
  expect((await api.refused("harnix.instances.ensure", { template: "worker", name: "x", wait: "soon" }))?.code)
    .toBe(INVALID_PARAMS);
});
