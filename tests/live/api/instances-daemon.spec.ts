/**
 * Daemon instances (design §5.7): an Agix in another process is an instance
 * that can go away and come back.
 *
 * Killed, it is `degraded` rather than gone; the daemon this Harnix spawned
 * is started again, every session somebody was watching is resynced, and a
 * watcher that attaches again has all of it. A daemon that speaks another
 * protocol is refused at the handshake and says so. A spawned daemon does
 * not outlive the Harnix that spawned it — not even one killed outright.
 */

import { createServer, type Server } from "node:net";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { type Backend, type TestClient, connect, expect, sleep, test } from "@harnix/testkit";

import { runOn, until } from "./instances";
import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:instances", "@layer:api"] };

/**
 * A daemon from some other Agix: it answers `daemon.ping` with a protocol
 * version nobody speaks.
 */
function fakeDaemon(socket: string): Server {
  const server = createServer((conn) => {
    let buffer = "";
    conn.on("data", (chunk) => {
      buffer += chunk;
      for (let at = buffer.indexOf("\n"); at >= 0; at = buffer.indexOf("\n")) {
        const line = buffer.slice(0, at);
        buffer = buffer.slice(at + 1);
        const request = JSON.parse(line);
        const result = request.method === "daemon.ping"
          ? { pong: true, version: "9.9.9", protocol_version: 999, capabilities: [] }
          : {};
        conn.write(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }) + "\n");
      }
    });
  });
  server.listen(socket);
  return server;
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

let dir: string;
let fake: Server;
let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  // Short: a unix socket path has a length limit.
  dir = mkdtempSync(join(tmpdir(), "hx-"));
  fake = fakeDaemon(join(dir, "old.sock"));
  backend = await startTestApp({
    profile: `
[[instances]]
id = "ci"
carrier = "spawn"

[[instances]]
id = "old"
carrier = "attach"
[instances.attach]
socket = "${join(dir, "old.sock")}"
`,
  });
});
test.afterAll(() => {
  backend?.stop();
  fake?.close();
  rmSync(dir, { recursive: true, force: true });
});
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

test("a daemon speaking another protocol is refused, and says why", TAGS, async () => {
  const old = await until(api, "old", "down");
  expect(old.carrier).toBe("attach");
  expect(old.status.reason).toContain("protocol version 999");
  const error = await api.refused("harnix.instances.sessions", { instance: "old" });
  expect(error?.message).toContain("protocol version 999");
});

test("a killed daemon degrades its instance, comes back, and its sessions resync", TAGS, async () => {
  const before = await until(api, "ci", "ready");
  const sessionId = await runOn(api, "ci");
  const watched = await api.call("harnix.subscribe", { topic: "harnix.instances" });
  expect(watched.value.instances.some((i: any) => i.id === "ci")).toBe(true);

  process.kill(before.pid, "SIGKILL");
  await api.waitFor(
    (f) => f.method === "harnix.topic" && f.params.topic === "harnix.instances"
      && f.params.value.instances.some((i: any) => i.id === "ci" && i.status.state === "degraded"),
    { timeout: 20_000, describe: "ci to be shown degraded" },
  );
  const after = await until(api, "ci", "ready");
  expect(after.pid).not.toBe(before.pid);
  await api.waitFor(
    (f) => f.method === "harnix.session_resync" && f.params.session_id === sessionId && f.params.instance === "ci",
    { timeout: 20_000, describe: "the watched session to be resynced" },
  );

  // Attaching again, the watcher has everything: the transcript is the
  // daemon's, and survived it.
  const attached = await api.call("harnix.attach", { instance: "ci", session_id: sessionId });
  expect(attached.history_total).toBeGreaterThanOrEqual(2);
  const history = await api.call("session.history", { instance: "ci", session_id: sessionId });
  expect(JSON.stringify(history.messages)).toMatch(/fixture ready/i);

  // And it takes turns again. The resume passes the recorder again — a
  // recorder is never written to the log, so the call that brings the
  // session back is where it comes from (Agix 0.2.8) — and the turn replays.
  api.events.length = 0;
  const sent = await api.call("harnix.send", { instance: "ci", session_id: sessionId, message: "Say exactly: fixture ready" });
  expect(sent.turn_id).toBeTruthy();
  const settled = await api.waitFor(
    (f) => f.method === "harnix.turn_settled" && f.params.session_id === sessionId && f.params.turn_id === sent.turn_id,
    { timeout: 60_000, describe: "a turn after the restart to settle" },
  );
  expect(settled.params.error, "the resumed session replayed rather than called a model").toBeUndefined();
});

test("a spawned daemon does not outlive Harnix, even one killed outright", TAGS, async () => {
  const ci = await until(api, "ci", "ready");
  expect(alive(ci.pid)).toBe(true);
  api.close();
  await backend.halt("SIGKILL");
  const deadline = Date.now() + 10_000;
  while (alive(ci.pid) && Date.now() < deadline) await sleep(100);
  expect(alive(ci.pid), `daemon ${ci.pid} is still running`).toBe(false);
});
