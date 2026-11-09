/**
 * A channel's life, from the outside: what ends one, what brings one back, and what one can reach.
 *
 *   - an event stream that dropped is opened again and still receives (split streams);
 *   - a paired device is still paired after a restart;
 *   - on a reachable node, revoking a device closes its WebSocket and refuses its split credential
 *     at once — not only its next handshake;
 *   - shutting down tells an open connection it is going away;
 *   - the static face serves a package's files and nothing beside them.
 */

import { execFileSync, spawn } from "node:child_process";
import { createHash, generateKeyPairSync, sign as nodeSign } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { CONTRIB_API_VERSION, PROTOCOL_VERSION } from "@harnix/core/protocol";
import {
  type Backend,
  RECORDINGS,
  ROOT,
  binary,
  connect,
  expect,
  freePort,
  launchFor,
  sleep,
  startHarnix,
  test,
  topology,
} from "@harnix/testkit";

const CONNECTION = { tag: ["@feature:connection", "@layer:api"] };
const AUTHZ = { tag: ["@feature:authz", "@layer:api"] };
const PACKAGES = { tag: ["@feature:packages", "@layer:api"] };

test.describe.configure({ mode: "serial" });

const hello = (token: string, topology: string, extra: Record<string, unknown> = {}) => ({
  token,
  protocol_version: PROTOCOL_VERSION,
  contrib_api_version: CONTRIB_API_VERSION,
  topology,
  ...extra,
});

/** Read an event stream's `data:` lines until `match` takes one, or the stream ends. */
async function readUntil(response: Response, match: (frame: any) => boolean, ms = 15_000): Promise<any | null> {
  const reader = response.body!.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  const deadline = Date.now() + ms;
  try {
    while (Date.now() < deadline) {
      const next = await Promise.race([
        reader.read(),
        sleep(deadline - Date.now()).then(() => ({ done: true, value: undefined, timeout: true })),
      ]);
      if ((next as any).timeout) throw new Error("the stream said nothing in time");
      if (next.done) return null;
      buffer += decoder.decode(next.value, { stream: true });
      let cut: number;
      while ((cut = buffer.indexOf("\n\n")) !== -1) {
        const chunk = buffer.slice(0, cut);
        buffer = buffer.slice(cut + 2);
        const data = chunk.split("\n").filter((l) => l.startsWith("data:")).map((l) => l.slice(5).trim()).join("\n");
        if (!data) continue;
        const frame = JSON.parse(data);
        if (match(frame)) return frame;
      }
    }
    throw new Error("the stream said nothing in time");
  } finally {
    reader.cancel().catch(() => {});
  }
}

test.describe("on loopback", () => {
  let backend: Backend;
  test.beforeAll(async () => {
    backend = await startHarnix(launchFor("replay"));
  });
  test.afterAll(async () => {
    await backend?.dispose();
  });

  // Split streams by nature; the backend serves both topologies, so this runs under either.
  test("an event stream that dropped is opened again and still receives", CONNECTION, async () => {
    const at = (path: string) => `${backend.url.replace(/\/$/, "")}${path}`;
    const shaken = await (await fetch(at("/handshake"), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(hello(backend.token, "split_streams")),
    })).json();
    const credential = shaken.credential as string;
    expect(credential).toBeTruthy();
    const rpc = async (method: string, params: unknown = {}) => {
      const response = await fetch(at(`/rpc?token=${encodeURIComponent(credential)}`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
      });
      return response.json();
    };
    const stream = () => {
      const abort = new AbortController();
      return { abort, response: fetch(at(`/events/host?token=${encodeURIComponent(credential)}`), { signal: abort.signal }) };
    };

    const first = stream();
    expect((await first.response).status).toBe(200);
    expect((await rpc("harnix.subscribe", { topic: "harnix.process" })).result).toBeTruthy();
    // The connection drops, as a proxy's idle timeout or a laptop's sleep drops it.
    first.abort.abort();
    await sleep(200);

    // The browser opens it again on its own. That used to be 409 for good — and a browser does
    // not retry a refusal.
    const second = stream();
    const reopened = await second.response;
    expect(reopened.status).toBe(200);
    // A value computed afresh goes to who already watches it: this client, on the new stream.
    expect((await rpc("harnix.subscribe", { topic: "harnix.process" })).result).toBeTruthy();
    const frame = await readUntil(reopened, (f) => f.method === "harnix.topic" && f.params?.topic === "harnix.process");
    expect(frame?.params?.value?.uptime_secs).toBeGreaterThanOrEqual(0);
  });

  test("shutting down tells an open connection it is going away", CONNECTION, async () => {
    const own = await startHarnix(launchFor("replay"));
    try {
      const socket = new WebSocket(`ws://127.0.0.1:${own.port}/ws?token=${own.token}`);
      const opened = new Promise<void>((resolve, reject) => {
        socket.onopen = () => resolve();
        socket.onerror = () => reject(new Error("no connection"));
      });
      await opened;
      const answered = new Promise<any>((resolve) => (socket.onmessage = (e) => resolve(JSON.parse(String(e.data)))));
      socket.send(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "harnix.handshake", params: hello(own.token, "single_duplex") }));
      expect((await answered).result?.credential).toBeTruthy();
      const closed = new Promise<CloseEvent>((resolve) => (socket.onclose = (e) => resolve(e)));
      await own.halt("SIGTERM");
      const event = await Promise.race([closed, sleep(10_000).then(() => null)]);
      expect(event, "the connection was not closed").not.toBeNull();
      expect(event!.code).toBe(1001);
    } finally {
      await own.dispose();
    }
  });

  test("a paired device is still paired after a restart", AUTHZ, async () => {
    const own = await startHarnix(launchFor("replay"));
    let restarted: Backend | null = null;
    try {
      const api = await connect(own, { topology: topology() });
      const grant = await api.call("harnix.devices.pair.begin", { label: "phone" });
      const { device } = await api.call("harnix.devices.pair.complete", { code: grant.code, public_key: "dGVzdC1rZXk=" });
      api.close();
      // Kept like the token: in the state directory, readable by its owner only.
      const file = join(own.state, "devices.json");
      expect(JSON.parse(readFileSync(file, "utf8")).map((d: any) => d.id)).toContain(device.id);
      if (process.platform !== "win32") {
        const mode = execFileSync("stat", process.platform === "darwin" ? ["-f", "%Lp", file] : ["-c", "%a", file], { encoding: "utf8" }).trim();
        expect(mode).toBe("600");
      }

      restarted = await own.restart();
      const again = await connect(restarted, { topology: topology() });
      const { devices } = await again.call("harnix.devices.list");
      expect(devices.map((d: any) => d.id)).toContain(device.id);
      // And revoking it is kept as well.
      expect((await again.call("harnix.devices.revoke", { device_id: device.id })).revoked).toBe(true);
      again.close();
      restarted = await restarted.restart();
      const third = await connect(restarted, { topology: topology() });
      expect((await third.call("harnix.devices.list")).devices.map((d: any) => d.id)).not.toContain(device.id);
      third.close();
    } finally {
      await (restarted ?? own).dispose();
    }
  });

  test("a package's interface path reaches its files and nothing beside them", PACKAGES, async () => {
    const api = await connect(backend, { topology: topology() });
    try {
      const name = "demo-rows-v2";
      const zip = readFileSync(join(ROOT, "tests", "fixtures", "packages", `${name}.zip`));
      const grant = await api.call("harnix.plugins.upload", { name: `${name}.zip` });
      const uploaded = await fetch(`${backend.url.replace(/\/$/, "")}${grant.url}`, { method: "POST", body: zip });
      expect(uploaded.ok).toBe(true);
      await api.call("harnix.plugins.install", {
        path: grant.path, name, version: "1.0.0", checksum: createHash("sha256").update(zip).digest("hex"),
      });
      const at = (path: string) => fetch(`http://127.0.0.1:${backend.port}${path}`);
      expect((await at(`/plugins/${name}/ui/rows.js`)).status).toBe(200);
      // An absolute path after `/ui/` used to replace the package's directory: any file the
      // process could read, with no token.
      for (const path of [`/plugins/${name}/ui//etc/hosts`, `/plugins/${name}/ui//etc/passwd`, `/plugins/${name}/ui/%2e%2e/%2e%2e/manifest.json`]) {
        const response = await at(path);
        expect(response.status, path).toBe(404);
        expect(await response.text(), path).not.toContain("localhost");
      }
    } finally {
      api.close();
    }
  });
});

/**
 * A reachable node: bound to `0.0.0.0` over TLS, so a paired device has to sign a challenge. The
 * testkit's backends are loopback-only, so this one is started here.
 */
test.describe("on a reachable node", () => {
  let child: ReturnType<typeof spawn>;
  let scratch: string;
  let base: string;
  let port: number;
  let token: string;
  let code: string;
  let log = "";
  const previousTls = process.env.NODE_TLS_REJECT_UNAUTHORIZED;

  test.beforeAll(async () => {
    // The certificate is self-signed; only this describe talks to it.
    process.env.NODE_TLS_REJECT_UNAUTHORIZED = "0";
    scratch = realpathSync(mkdtempSync(join(tmpdir(), "harnix-live-reachable-")));
    const home = join(scratch, "home");
    mkdirSync(home, { recursive: true });
    execFileSync("openssl", [
      "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
      "-keyout", join(scratch, "key.pem"), "-out", join(scratch, "cert.pem"),
      "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1",
    ], { stdio: "ignore" });
    port = await freePort();
    writeFileSync(join(scratch, "profile.toml"), [
      "[transport]", 'topologies = ["single_duplex", "split_streams"]', 'host = "0.0.0.0"', `port = ${port}`, "",
    ].join("\n"));
    mkdirSync(join(scratch, "project"), { recursive: true });
    child = spawn(binary(), [
      "--profile", join(scratch, "profile.toml"), "--port", String(port),
      "--tls-cert", join(scratch, "cert.pem"), "--tls-key", join(scratch, "key.pem"),
      "--agix-dir", join(scratch, "agix"), "--data-dir", join(scratch, "projects"),
      "--replay-dir", RECORDINGS,
    ], {
      cwd: join(scratch, "project"),
      env: { ...process.env, HOME: home, ANTHROPIC_API_KEY: "live-tests-no-model", RUST_LOG: "warn" },
      stdio: ["ignore", "pipe", "pipe"],
    });
    child.stdout?.on("data", (d) => (log += d));
    child.stderr?.on("data", (d) => (log += d));
    base = `https://127.0.0.1:${port}`;
    const deadline = Date.now() + 60_000;
    for (;;) {
      const printed = /pairing code → ([A-Z0-9]+)/.exec(log)?.[1];
      try {
        const up = await fetch(`${base}/challenge`);
        if (up.ok && printed) {
          code = printed;
          break;
        }
      } catch {
        /* not up yet */
      }
      if (Date.now() > deadline || child.exitCode !== null) throw new Error(`the reachable node did not start:\n${log}`);
      await sleep(150);
    }
    token = readFileSync(join(home, ".harnix", "token"), "utf8").trim();
  });

  test.afterAll(async () => {
    if (child && child.exitCode === null) {
      const gone = new Promise<void>((resolve) => child.once("exit", () => resolve()));
      child.kill("SIGTERM");
      await Promise.race([gone, sleep(15_000).then(() => child.kill("SIGKILL"))]);
    }
    if (scratch) rmSync(scratch, { recursive: true, force: true });
    if (previousTls === undefined) delete process.env.NODE_TLS_REJECT_UNAUTHORIZED;
    else process.env.NODE_TLS_REJECT_UNAUTHORIZED = previousTls;
  });

  test("revoking a device closes its WebSocket and refuses its split credential", AUTHZ, async () => {
    const { publicKey, privateKey } = generateKeyPairSync("ed25519");
    const publicB64 = publicKey.export({ type: "spki", format: "der" }).subarray(12).toString("base64");
    const post = (path: string, body: unknown) =>
      fetch(`${base}${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    const proof = async () => {
      const challenge = await (await fetch(`${base}/challenge`)).json();
      const signature = nodeSign(null, Buffer.from(challenge.challenge, "base64"), privateKey).toString("base64");
      return { challenge_id: challenge.challenge_id, signature };
    };

    const paired = await post("/pair", { token, code, public_key: publicB64 });
    expect(paired.status, await paired.clone().text()).toBe(200);
    const deviceId = (await paired.json()).device.id as string;

    // Its split credential.
    const shaken = await post("/handshake", hello(token, "split_streams", { device_id: deviceId, ...(await proof()) }));
    expect(shaken.status).toBe(200);
    const credential = (await shaken.json()).credential as string;
    const rpc = (method: string, params: unknown = {}) =>
      post(`/rpc?token=${encodeURIComponent(credential)}`, { jsonrpc: "2.0", id: 1, method, params });
    expect((await rpc("harnix.devices.list")).status).toBe(200);
    const events = await fetch(`${base}/events/host?token=${encodeURIComponent(credential)}`);
    expect(events.status).toBe(200);

    // And a WebSocket of the same device.
    const socket = new WebSocket(`wss://127.0.0.1:${port}/ws?token=${token}`);
    await new Promise<void>((resolve, reject) => {
      socket.onopen = () => resolve();
      socket.onerror = () => reject(new Error("no connection"));
    });
    const answered = new Promise<any>((resolve) => (socket.onmessage = (e) => resolve(JSON.parse(String(e.data)))));
    socket.send(JSON.stringify({
      jsonrpc: "2.0", id: 1, method: "harnix.handshake",
      params: hello(token, "single_duplex", { device_id: deviceId, ...(await proof()) }),
    }));
    expect((await answered).result?.credential, "the device's WebSocket was not admitted").toBeTruthy();
    const closed = new Promise<CloseEvent>((resolve) => (socket.onclose = (e) => resolve(e)));

    // Revoked through its own credential: the answer comes back, and then nothing else does.
    const revoked = await (await rpc("harnix.devices.revoke", { device_id: deviceId })).json();
    expect(revoked.result?.revoked).toBe(true);

    const refused = await rpc("harnix.devices.list");
    expect(refused.status).toBe(401);
    expect((await refused.json()).error?.code).toBe(-31001);
    const event = await Promise.race([closed, sleep(10_000).then(() => null)]);
    expect(event, "the revoked device's WebSocket stayed open").not.toBeNull();
    expect(event!.code).toBe(1008);
    // Its event stream ended too.
    expect(await readUntil(events, () => false, 10_000)).toBeNull();
    // And it cannot come back.
    const again = await post("/handshake", hello(token, "split_streams", { device_id: deviceId, ...(await proof()) }));
    expect(again.status).toBe(401);
  });
});
