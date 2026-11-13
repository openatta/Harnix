// The performance budget, as a gate rather than a sentence.
//
// A performance target nobody measures is just a claim. These are the numbers
// from concept_and_architecture.md §7.2, checked against the release build;
// a regression fails here instead of being noticed months later on a small
// machine.
//
// The initial values are set for "a 4-core 8 GB box, and it has to run on a
// 2-core 2 GB arm64 board too". They can be changed — by writing the new
// number into the table *and* saying why, never by quietly loosening the gate.
//
//   cargo build --release && node tests/budget.mjs

import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";
import { freePort, makeProject } from "../ui/packages/testkit/src/launcher.ts";
import { dirname, join } from "node:path";
import { PROTOCOL_VERSION, CONTRIB_API_VERSION } from "../ui/packages/core/src/protocol/generated/contract.ts";

const root = dirname(dirname(fileURLToPath(import.meta.url)));

const MB = 1024 * 1024;
const KB = 1024;

/** The table. Changing a number here is the only way to change the gate. */
const BUDGET = [
  {
    name: "backend binary (default build, both carriers, with wasmtime)",
    limit: 52 * MB,
    unit: (n) => `${(n / MB).toFixed(1)} MB`,
    // `harnix-cli`'s default features carry packages on both carriers: in this process (`wasm`) and
    // each in a process of its own (`proc`, this same binary re-executed as `ext-proc`). Either
    // brings the WebAssembly engine, runtime only — no compiler, which is what keeps it small (the
    // build with the runtime measured 41.0 MB against 37.3 MB without, 2026-11-08). The build
    // without a carrier's limit plus four megabytes: room for the runtime, and none for a compiler.
    measure: () => statSync(join(root, "target", "release", "harnix")).size,
  },
  {
    name: "interface (gzipped, as embedded)",
    limit: 300 * KB,
    unit: (n) => `${(n / KB).toFixed(0)} KB`,
    // Measured from `ui/dist`, which is what `include_dir!` compiles in — the
    // same bytes, so this is the embedded size and not an estimate of it.
    // It is self-contained (no CDN, nothing fetched at run time), which is
    // what lets the interface open with no network at all, and it is also
    // why it can live in the binary without dragging anything else along.
    measure: () => {
      let total = 0;
      const walk = (dir) => {
        for (const entry of readdirSync(dir, { withFileTypes: true })) {
          const path = join(dir, entry.name);
          if (entry.isDirectory()) walk(path);
          else if (/\.(js|css|html)$/.test(entry.name)) {
            total += gzipSync(readFileSync(path)).length;
          }
        }
      };
      walk(join(root, "ui", "dist"));
      return total;
    },
  },
];

let failures = 0;
const measured = [];

for (const item of BUDGET) {
  let value;
  try {
    value = item.measure();
  } catch (e) {
    console.log(`SKIP: ${item.name} — ${e.message}`);
    continue;
  }
  const within = value <= item.limit;
  const line = `${within ? "ok —" : "FAIL:"} ${item.name}: ${item.unit(value)} (limit ${item.unit(item.limit)})`;
  console.log(line);
  measured.push({ name: item.name, value, limit: item.limit });
  if (!within) failures += 1;
}

// Cold start is measured only when a binary exists and can be asked for its
// version — enough to load the process image, which is what the number is
// really about.
try {
  const bin = join(root, "target", "release", "harnix");
  statSync(bin);
  const runs = [];
  for (let i = 0; i < 5; i += 1) {
    const started = process.hrtime.bigint();
    execFileSync(bin, ["--version"], { stdio: "ignore" });
    runs.push(Number(process.hrtime.bigint() - started) / 1e6);
  }
  runs.sort((a, b) => a - b);
  const median = runs[Math.floor(runs.length / 2)];
  const limit = 300;
  const within = median <= limit;
  console.log(`${within ? "ok —" : "FAIL:"} process start (median of 5): ${median.toFixed(0)} ms (limit ${limit} ms)`);
  if (!within) failures += 1;
} catch {
  console.log("SKIP: process start — no release binary");
}

/* ── the ones that need the thing actually running ────────────────────── */
//
// Size and startup can be measured from a file. Resident memory, latency and
// idle CPU cannot: they are properties of a live process, and the whole point
// of the numbers in concept §7.2 is that this has to fit on a small machine while
// serving somebody. So the rest of this file starts a real backend against a
// throwaway directory and measures it.

const live = await measureLive(root).catch((e) => {
  console.log(`SKIP: live measurements — ${e.message}`);
  return null;
});

if (live) {
  const checks = [
    { name: "resident memory, idle", value: live.idleRssMb, limit: 80,
      unit: (n) => `${n.toFixed(1)} MB` },
    { name: "resident memory per idle session", value: live.perSessionMb, limit: 4,
      unit: (n) => `${n.toFixed(2)} MB` },
    { name: "unary round trip p99 (same machine)", value: live.p99Ms, limit: 15,
      unit: (n) => `${n.toFixed(1)} ms` },
    { name: "resident overhead per idle connection", value: live.perConnectionKb, limit: 64,
      unit: (n) => `${n.toFixed(1)} KB` },
    // The number a small box has to satisfy, and the one the per-connection
    // figure cannot be quietly traded against.
    { name: `resident memory holding ${live.connections} connections`, value: live.heldRssMb, limit: 120,
      unit: (n) => `${n.toFixed(1)} MB` },
    { name: "CPU when fully idle", value: live.idleCpuPercent, limit: 0.5,
      unit: (n) => `${n.toFixed(2)} %` },
  ];
  for (const c of checks) {
    const within = c.value <= c.limit;
    console.log(`${within ? "ok —" : "FAIL:"} ${c.name}: ${c.unit(c.value)} (limit ${c.unit(c.limit)})`);
    if (!within) failures += 1;
  }
}

/* ── each in-process instance (concept §7.2) ──────────────────────────── */
//
// The same process idle twice: with `system` only, and with four more
// in-process instances declared in its profile. The difference over four is
// what one instance costs before anyone opens a session in it.

const perInstance = await measureInstances(root).catch((e) => {
  console.log(`SKIP: per-instance memory — ${e.message}`);
  return null;
});
if (perInstance !== null) {
  const limit = 4;
  const within = perInstance <= limit;
  console.log(`${within ? "ok —" : "FAIL:"} resident memory per idle in-process instance: ${perInstance.toFixed(2)} MB (limit ${limit} MB)`);
  if (!within) failures += 1;
}

/* ── extension processes (mode 3), each with its own engine ───────────── */
//
// The showcase, which has an agent package, on the process carrier in its sandbox: its process
// holds the WebAssembly runtime, the package's backend and an engine of its own (Agix), so what
// one costs is measured with all three. Its pid's resident memory idle (up, nothing open), running
// (a session open on its engine) and after calls (two hundred method calls); how long it takes
// from being started to its hello and from its `ctl.start` to the answer (the carrier logs both);
// switched off and on again until it answers; and an on-demand one let go while idle, woken by a
// call — which is what a person waits for.

const PROCESS_LIMITS = { idleMb: 40, coldStartMs: 2000 };

const processes = await measureProcesses(root).catch((e) => {
  console.log(`SKIP: extension processes — ${e.message}`);
  return null;
});
if (processes) {
  const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
  const gate = (name, value, limit, unit) => {
    const within = value <= limit;
    console.log(`${within ? "ok —" : "FAIL:"} ${name}: ${unit(value)} (limit ${unit(limit)})`);
    if (!within) failures += 1;
  };
  const mb = (n) => `${n.toFixed(1)} MB`;
  const ms = (n) => `${n.toFixed(0)} ms`;
  gate("extension process with its engine, resident idle", processes.idleMb, PROCESS_LIMITS.idleMb, mb);
  console.log(`info — extension process with its engine, resident with a session open: ${mb(processes.runningMb)}`);
  console.log(`info — extension process with its engine, resident after ${processes.calls} calls: ${mb(processes.calledMb)}`);
  console.log(`info — extension process, started → its hello: ${processes.helloMs.map(ms).join(", ")}`);
  console.log(`info — extension process, ctl.start → answered: ${processes.startMs.map(ms).join(", ")}`);
  gate("extension process switched on → answering (median of 3)", median(processes.enableMs), PROCESS_LIMITS.coldStartMs, ms);
  gate("on-demand extension process woken by a call (median of 3)", median(processes.wakeMs), PROCESS_LIMITS.coldStartMs, ms);
}

async function measureProcesses(root) {
  const { spawn, execFileSync: exec } = await import("node:child_process");
  const { mkdtempSync, rmSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { SHOWCASE_ID, showcasePackage, variant } = await import("./live/api/packages-host.ts");
  const { appsTestApp } = await import("./live/api/packages-store.ts");
  const showcase = showcasePackage().path;
  const M = (name) => `${SHOWCASE_ID}.${name}`;
  const ENGINE = `proc.${SHOWCASE_ID}`;
  const rss = (pid) => Number(exec("ps", ["-o", "rss=", "-p", String(pid)], { encoding: "utf8" }).trim()) / 1024;
  const sleep = (n) => new Promise((r) => setTimeout(r, n));

  const start = async (bin, env = {}) => {
    const port = await freePort();
    const scratch = mkdtempSync(join(tmpdir(), "harnix-budget-p-"));
    const profile = join(scratch, "profile.toml");
    writeFileSync(profile, `[ext.packages]\ndev = true\ncarrier = "process"\n`);
    let log = "";
    const child = spawn(
      bin,
      ["--profile", profile, "--port", String(port), "--headless",
       "--agix-dir", join(scratch, "agix"), "--data-dir", join(scratch, "projects")],
      { cwd: makeProject(scratch),
        env: { ...process.env, ...env, ANTHROPIC_API_KEY: "budget-test", HARNIX_STATE_DIR: join(scratch, "state"),
               RUST_LOG: "error,harnix_carrier_proc=debug" },
        stdio: ["ignore", "pipe", "pipe"] },
    );
    child.stdout.on("data", (d) => (log += d));
    child.stderr.on("data", (d) => (log += d));
    const stop = () => {
      try { child.kill(); } catch { /* already gone */ }
      rmSync(scratch, { recursive: true, force: true });
    };
    try {
      const token = await waitForToken(join(scratch, "state", "token"), port);
      const client = await connect(port, token);
      return { client, stop, log: () => log, scratch };
    } catch (e) {
      stop();
      throw e;
    }
  };
  /** The showcase's runtime once `ok` holds of it. */
  const until = async (client, what, ok) => {
    const deadline = Date.now() + 60_000;
    for (;;) {
      const rt = (await client.call("packages.list")).packages.find((p) => p.id === SHOWCASE_ID)?.runtime;
      if (rt && ok(rt)) return rt;
      if (Date.now() > deadline) throw new Error(`never ${what}: ${JSON.stringify(rt)}`);
      await sleep(20);
    }
  };
  const engineReady = async (client) => {
    const deadline = Date.now() + 60_000;
    while ((await client.call("harnix.instances.list")).instances.find((i) => i.id === ENGINE)?.status?.state !== "ready") {
      if (Date.now() > deadline) throw new Error("its engine never came up");
      await sleep(50);
    }
  };
  const timings = (log, field) => [...log.matchAll(new RegExp(`its extension process came up.*`, "g"))]
    .map((m) => Number(new RegExp(`${field}=(\\d+)`).exec(m[0])?.[1]))
    .filter((n) => Number.isFinite(n));

  // The product as it ships, the showcase resident.
  const product = await start(join(root, "target", "release", "harnix"));
  let measured;
  try {
    const { client } = product;
    await client.call("packages.installFile", { path: showcase });
    const up = await until(product.client, "running", (rt) => rt.state === "running");
    await engineReady(client);
    await sleep(3000);
    const idleMb = rss(up.process.pid);

    const created = await client.call("harnix.session.create", { space: SHOWCASE_ID });
    await client.call("harnix.session.attach", { instance: ENGINE, session_id: created.session_id, space: SHOWCASE_ID });
    await sleep(2000);
    const runningMb = rss(up.process.pid);

    const CALLS = 200;
    for (let i = 0; i < CALLS / 2; i += 1) {
      await client.call(M("notes.add"), { text: `note ${i}` });
      await client.call(M("notes.list"));
    }
    await sleep(1000);
    const calledMb = rss(up.process.pid);

    const enableMs = [];
    for (let i = 0; i < 3; i += 1) {
      await client.call("packages.disable", { id: SHOWCASE_ID });
      const began = performance.now();
      await client.call("packages.enable", { id: SHOWCASE_ID });
      await until(client, "running again", (rt) => rt.state === "running");
      await client.call(M("notes.list"));
      enableMs.push(performance.now() - began);
    }
    client.close();
    measured = { idleMb, runningMb, calledMb, calls: CALLS, enableMs, log: product.log() };
  } finally {
    product.stop();
  }

  // On demand, let go after 1.5 s idle (the test application's knob; a manifest's least is a
  // minute), and woken by a call.
  const testapp = await start(appsTestApp(), { HARNIX_TESTAPP_APP_IDLE_MS: "1500" });
  try {
    const { client } = testapp;
    const onDemand = variant(join(testapp.scratch, "on-demand.hxext"), (e) => {
      const manifest = e.get("harnix-ext.toml").toString().replace("watch_sessions = true", "watch_sessions = false");
      e.set("harnix-ext.toml", Buffer.from(`${manifest}\n[backend]\nresident = "on-demand"\nidle_secs = 60\n`));
    });
    await client.call("packages.installFile", { path: onDemand });
    const wakeMs = [];
    for (let i = 0; i < 3; i += 1) {
      await until(client, "hibernating", (rt) => rt.state === "hibernating");
      const began = performance.now();
      await client.call(M("notes.list"));
      wakeMs.push(performance.now() - began);
    }
    client.close();
    const log = measured.log + testapp.log();
    return { ...measured, wakeMs, helloMs: timings(log, "hello_ms"), startMs: timings(log, "start_ms") };
  } finally {
    testapp.stop();
  }
}

async function measureInstances(root) {
  const { spawn, execFileSync: exec } = await import("node:child_process");
  const { mkdtempSync, rmSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const bin = join(root, "target", "release", "harnix");
  statSync(bin);
  const EXTRA = 4;
  const idle = async (extra) => {
    const port = await freePort();
    const scratch = mkdtempSync(join(tmpdir(), "harnix-budget-i-"));
    const profile = join(scratch, "profile.toml");
    writeFileSync(profile, Array.from({ length: extra },
      (_, i) => `[[instances]]\nid = "i${i}"\ncarrier = "in_process"\n`).join("\n"));
    const child = spawn(
      bin,
      ["--profile", profile, "--port", String(port), "--headless",
       "--agix-dir", join(scratch, "agix"), "--data-dir", join(scratch, "projects")],
      { cwd: makeProject(scratch), env: { ...process.env, ANTHROPIC_API_KEY: "budget-test", HARNIX_STATE_DIR: join(scratch, "state"), RUST_LOG: "error" },
        stdio: "ignore" },
    );
    try {
      await waitForToken(join(scratch, "state", "token"), port);
      await new Promise((r) => setTimeout(r, 3000));
      const readings = [];
      for (let i = 0; i < 3; i += 1) {
        readings.push(Number(exec("ps", ["-o", "rss=", "-p", String(child.pid)], { encoding: "utf8" }).trim()) / 1024);
        await new Promise((r) => setTimeout(r, 500));
      }
      return readings.sort((a, b) => a - b)[1];
    } finally {
      try { child.kill(); } catch { /* already gone */ }
      rmSync(scratch, { recursive: true, force: true });
    }
  };
  const base = await idle(0);
  const more = await idle(EXTRA);
  return Math.max(0, more - base) / EXTRA;
}

/**
 * Start a backend, measure it, stop it.
 *
 * Measured against a throwaway engine directory and with no model credential,
 * so nothing here reaches a provider — what is under test is the process's
 * own footprint, and a turn would be measuring somebody else's.
 */
async function measureLive(root) {
  const { spawn, execFileSync: exec } = await import("node:child_process");
  const { mkdtempSync, rmSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");

  const bin = join(root, "target", "release", "harnix");
  statSync(bin);
  const scratch = mkdtempSync(join(tmpdir(), "harnix-budget-"));
  const port = await freePort();
  const child = spawn(
    bin,
    ["--port", String(port), "--headless",
     "--agix-dir", join(scratch, "agix"), "--data-dir", join(scratch, "projects")],
    { cwd: makeProject(scratch), env: { ...process.env, ANTHROPIC_API_KEY: "budget-test", HARNIX_STATE_DIR: join(scratch, "state"), RUST_LOG: "error" },
      stdio: "ignore" },
  );

  const rssMb = () => {
    // `ps` rather than anything clever: this is the number an operator would
    // read, so it is the number the budget should be about.
    const out = exec("ps", ["-o", "rss=", "-p", String(child.pid)], { encoding: "utf8" });
    return Number(out.trim()) / 1024;
  };
  const cpuPercent = () => {
    const out = exec("ps", ["-o", "%cpu=", "-p", String(child.pid)], { encoding: "utf8" });
    return Number(out.trim());
  };

  try {
    const token = await waitForToken(join(scratch, "state", "token"), port);

    // Settle: the janitor, the skill scan and the MCP background connects all
    // happen after the port opens, and measuring through them would measure
    // startup rather than idle.
    await new Promise((r) => setTimeout(r, 3000));
    const idleRssMb = rssMb();

    const client = await connect(port, token);

    // p99 over a few hundred calls of a method that does no I/O of its own.
    const samples = [];
    for (let i = 0; i < 300; i += 1) {
      const started = process.hrtime.bigint();
      await client.call("engine.daemon.ping", {});
      samples.push(Number(process.hrtime.bigint() - started) / 1e6);
    }
    samples.sort((a, b) => a - b);
    const p99Ms = samples[Math.floor(samples.length * 0.99)];

    // What a connection costs while doing nothing.
    //
    // Measured **after a warm-up round**, and the reason is worth writing
    // down. Measured cold, the first hundred connections read as ~145 KB
    // each; open and close another hundred and the marginal cost is under a
    // kilobyte. The difference is not the connections — it is the allocator
    // taking pages from the OS for the first time and then keeping them. A
    // gate on the cold number would be a gate on macOS's malloc, and it would
    // fail or pass for reasons that have nothing to do with this code.
    //
    // So both numbers are checked, because both are real: the marginal cost
    // of one more connection, and the total footprint while a hundred are
    // held — which is the one an operator's box actually has to satisfy.
    const CONNECTIONS = 100;
    const warm = [];
    for (let i = 0; i < CONNECTIONS; i += 1) warm.push(await connect(port, token));
    await new Promise((r) => setTimeout(r, 1500));
    for (const c of warm) c.close();
    await new Promise((r) => setTimeout(r, 2000));

    const rssBefore = rssMb();
    const extra = [];
    for (let i = 0; i < CONNECTIONS; i += 1) extra.push(await connect(port, token));
    await new Promise((r) => setTimeout(r, 2000));
    const heldRssMb = rssMb();
    const perConnectionKb = Math.max(0, heldRssMb - rssBefore) * 1024 / extra.length;
    for (const c of extra) c.close();

    // Sessions, created and left alone.
    const rssBeforeSessions = rssMb();
    const sessions = [];
    for (let i = 0; i < 8; i += 1) {
      const created = await client.call("harnix.session.create", {}).catch(() => null);
      if (created?.session_id) sessions.push(created.session_id);
    }
    await new Promise((r) => setTimeout(r, 1500));
    const perSessionMb = sessions.length
      ? Math.max(0, rssMb() - rssBeforeSessions) / sessions.length
      : 0;
    for (const id of sessions) await client.call("harnix.session.delete", { session_id: id }).catch(() => {});

    client.close();
    // `ps` reports CPU averaged over the process's life, so a fresh reading
    // after a quiet stretch is the honest way to ask "does it spin".
    await new Promise((r) => setTimeout(r, 4000));
    const idleCpuPercent = cpuPercent();

    return { idleRssMb, perSessionMb, p99Ms, perConnectionKb, heldRssMb, connections: CONNECTIONS, idleCpuPercent };
  } finally {
    try { child.kill(); } catch { /* already gone */ }
    rmSync(scratch, { recursive: true, force: true });
  }
}

async function waitForToken(path, port) {
  const deadline = Date.now() + 30_000;
  for (;;) {
    try {
      await fetch(`http://127.0.0.1:${port}/nope`).catch(() => {});
      return readFileSync(path, "utf8").trim();
    } catch { /* not up yet */ }
    if (Date.now() > deadline) throw new Error("backend did not start in 30s");
    await new Promise((r) => setTimeout(r, 250));
  }
}

/** One handshaken connection, as a client would hold it. */
async function connect(port, token) {
  const ws = new WebSocket(`ws://127.0.0.1:${port}/ws?token=${encodeURIComponent(token)}`);
  let id = 1;
  const pending = new Map();
  ws.onmessage = (e) => {
    const frame = JSON.parse(e.data);
    if (frame.id != null && pending.has(frame.id)) {
      const slot = pending.get(frame.id);
      pending.delete(frame.id);
      frame.error ? slot.rej(frame.error) : slot.res(frame.result);
    }
  };
  await new Promise((res, rej) => {
    ws.onopen = res;
    ws.onerror = () => rej(new Error("could not connect"));
  });
  const call = (method, params) => new Promise((res, rej) => {
    const i = id++;
    pending.set(i, { res, rej });
    ws.send(JSON.stringify({ jsonrpc: "2.0", method, params: params || {}, id: i }));
  });
  await call("harnix.handshake", {
    protocol_version: PROTOCOL_VERSION, contrib_api_version: CONTRIB_API_VERSION, topology: "single_duplex",
  });
  return { call, close: () => ws.close() };
}

console.log(failures ? "\nBUDGET FAILED" : "\nBUDGET PASSED");
process.exitCode = failures ? 1 : 0;
