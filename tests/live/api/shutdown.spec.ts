/**
 * The way out (ipdrun-02, ipdrun-03).
 *
 * SIGTERM is how a service manager, a desktop shell, `kill` and a test harness ask a process to go,
 * and it takes the same way out as Ctrl-C: every module's `stop`, the instances, the process's own
 * uploads. That way out is bounded — a second signal, or the grace period, ends it — so a `stop`
 * that never returns cannot keep the process up. And the testkit's `dispose()` waits for all of it
 * before removing the directory the process was still writing into.
 */

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";

import { type Backend, ROOT, alive, expect, gone, sleep, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:modules", "@layer:api"] };

/** The `lifecycle` module's data directory: where its `stop` leaves its marks. */
const marks = (backend: Backend) => join(backend.state, "modules", "lifecycle");

test.describe.configure({ mode: "serial" });

for (const signal of ["SIGTERM", "SIGINT"] as const) {
  test(`${signal} runs every module's stop before the process exits`, TAGS, async () => {
    const backend = await startTestApp();
    try {
      const uploads = join(backend.state, "uploads", String(backend.pid));
      expect(existsSync(uploads)).toBe(true);
      await backend.halt(signal);
      expect(existsSync(join(marks(backend), "stopped"))).toBe(true);
      // The process's own scratch goes with it.
      expect(existsSync(uploads)).toBe(false);
    } finally {
      await backend.dispose();
    }
  });
}

test("a stop that never returns is cut short by a second signal", TAGS, async () => {
  const backend = await startTestApp({ profile: "\n[modules.lifecycle]\nhang_on_stop = true\n" });
  try {
    process.kill(backend.pid, "SIGTERM");
    // In its `stop`, and staying there.
    for (let i = 0; i < 100 && !existsSync(join(marks(backend), "stopping")); i += 1) await sleep(50);
    expect(existsSync(join(marks(backend), "stopping"))).toBe(true);
    await sleep(300);
    expect(alive(backend.pid)).toBe(true);

    const asked = Date.now();
    await backend.halt("SIGTERM");
    expect(Date.now() - asked).toBeLessThan(1_000);
    expect(existsSync(join(marks(backend), "stopped"))).toBe(false);
  } finally {
    await backend.dispose();
  }
});

test("dispose waits for the process to finish before removing its directory", TAGS, async () => {
  const backend = await startTestApp({ profile: "\n[modules.lifecycle]\nslow_stop_files = 20\n" });
  await backend.dispose();
  expect(alive(backend.pid)).toBe(false);
  expect(existsSync(backend.scratch)).toBe(false);
});

test("stop still returns at once, without waiting for the process", TAGS, async () => {
  const backend = await startTestApp({ profile: "\n[modules.lifecycle]\nslow_stop_files = 20\n" });
  expect(() => backend.stop()).not.toThrow();
  // Whatever it left behind is the next run's to ignore; the process itself goes.
  for (let i = 0; i < 100 && alive(backend.pid); i += 1) await sleep(50);
  expect(alive(backend.pid)).toBe(false);
  await backend.dispose();
});

// A run that ends without disposing of its backends — Ctrl-C, a script that throws — does not leave
// them running: the testkit ends every backend it started, and what that backend spawned, as the
// process that started it goes.
for (const how of ["SIGINT", "exit"] as const) {
  test(`a process that started a backend and then ${how === "exit" ? "exits" : "is interrupted"} takes the backend with it`, TAGS, async () => {
    const launcher = join(ROOT, "ui", "packages", "testkit", "src", "launcher.ts");
    const script = [
      `import { startHarnix } from ${JSON.stringify(launcher)};`,
      `const backend = await startHarnix({ env: { ANTHROPIC_API_KEY: "leak-test" } });`,
      `console.log(JSON.stringify({ pid: backend.pid }));`,
      how === "exit" ? "process.exit(0);" : "setInterval(() => {}, 1000);",
    ].join("\n");
    const child = spawn(process.execPath, ["--input-type=module", "-e", script], { stdio: ["ignore", "pipe", "inherit"] });
    const line = await new Promise<string>((resolve, reject) => {
      let out = "";
      child.stdout!.on("data", (d) => {
        out += d;
        if (out.includes("\n")) resolve(out.split("\n")[0]!);
      });
      child.on("exit", (code) => reject(new Error(`the script exited (${code}) before its backend was up: ${out}`)));
    });
    const { pid } = JSON.parse(line);
    expect(alive(pid)).toBe(true);
    if (how === "SIGINT") child.kill("SIGINT");
    expect(await gone(pid, 15_000), `the backend ${pid} outlived the process that started it`).toBe(true);
  });
}
