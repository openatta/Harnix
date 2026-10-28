/**
 * One process per state directory (ipdrun-09).
 *
 * The token, the devices, the instance ledger and every module's data are written by one process.
 * A second one on the same directory — another port, the same HOME — is refused with who holds it,
 * and the first is left alone. The lock goes with its process however it ends, so a restart after
 * `kill -9` starts at once.
 */

import { type ChildProcess, spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { binary, connect, expect, freePort, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:connection", "@layer:api"] };

/** Run to its end: the exit code and everything it printed, or a failure after `ms`. */
function finished(child: ChildProcess, ms: number): Promise<{ code: number | null; out: string }> {
  let out = "";
  child.stdout?.on("data", (d) => (out += d));
  child.stderr?.on("data", (d) => (out += d));
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error(`still running after ${ms} ms:\n${out}`));
    }, ms);
    child.on("exit", (code) => {
      clearTimeout(timer);
      resolve({ code, out });
    });
  });
}

test.describe.configure({ mode: "serial" });

test("a second process on the same state directory is refused, naming the first", TAGS, async () => {
  const first = await startTestApp();
  try {
    const token = readFileSync(join(first.state, "token"), "utf8");
    const lock = JSON.parse(readFileSync(join(first.state, "harnix.lock"), "utf8"));
    expect(lock).toEqual({ pid: first.pid, url: first.url });

    const second = spawn(binary("harnix-testapp"), [
      "--profile", join(first.scratch, "profile.toml"),
      "--port", String(await freePort()),
      "--agix-dir", join(first.scratch, "agix"),
      "--data-dir", join(first.scratch, "projects"),
    ], {
      cwd: first.project,
      env: { ...process.env, HOME: join(first.scratch, "home"), RUST_LOG: "warn" },
      stdio: ["ignore", "pipe", "pipe"],
    });
    const { code, out } = await finished(second, 5_000);
    expect(code).not.toBe(0);
    expect(out).toContain(`pid ${first.pid}`);
    expect(out).toContain(first.url);

    // The first goes on as it was.
    expect(readFileSync(join(first.state, "token"), "utf8")).toBe(token);
    const api = await connect(first);
    expect((await api.call("harnix.hello")).protocol_version).toBeTruthy();
    api.close();
  } finally {
    await first.dispose();
  }
});

test("after the holder is killed outright, the next one starts at once", TAGS, async () => {
  const first = await startTestApp();
  await first.halt("SIGKILL");
  const next = await first.restart();
  try {
    const lock = JSON.parse(readFileSync(join(next.state, "harnix.lock"), "utf8"));
    expect(lock.pid).toBe(next.pid);
  } finally {
    await next.dispose();
  }
});
