/**
 * Packages in processes of their own (mode 3), product side: `[ext.packages] carrier = "process"`.
 * The showcase runs in an extension process — this binary re-executed as `ext-proc` — and the
 * product registers it, keeps the process and answers it over its wire.
 *
 * What is the process carrier's and not the in-process one's is here: the process's pid and
 * starts in the runtime; its methods and topics answered over the wire; a backend calling back
 * into itself refused at once, by the chain the product works out; the process killed outright,
 * and up again with its rows never gone; killed three times soon after starting, and set aside
 * until it is switched off and on; and a process that is not what was installed — another
 * extension's hello, a manifest with one more method — refused and ended. `proc_program` is a
 * small script that starts this binary, but lies for the packages those tests install; these run
 * with `sandbox = "off"`, since the script starts a program the sandbox does not let a process run
 * and hands TAMPERED a package outside its sandbox. The next describe runs the showcase in its
 * sandbox, started as this binary: its backend, its own engine with its agent package, and a shell
 * that engine starts (a session-start hook, which needs no model; its shell tool, with a live one).
 *
 * Its own engine (the showcase has `agent/` and two `[[space]]`s) is adopted as `proc.<id>`: an
 * extension's engine, its spaces' sessions there, its content marked untrusted; the product only
 * routes. That describe also lets an on-demand variant's process go while idle and wakes it.
 * Turns there need a model: the process's own engine records and replays nothing, so the tests
 * that run one skip without a live model.
 *
 * A product of another build (another Harnix, Agix or VM engine) is refused too; that takes
 * another build, and `crates/carrier/proc/tests/process.rs` makes one by telling the carrier
 * another version.
 */

import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { EXTENSION_UNAVAILABLE, REFUSED } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, hasModel, loadEnv, modelMode, sleep, test, topology } from "@harnix/testkit";

import { SHOWCASE_CHATS, SHOWCASE_ID, appsTools, readZip, recalledOf, showcasePackage, startApps, variant } from "./packages-host";
import { appsTestApp } from "./packages-store";

const TAGS = { tag: ["@feature:packages", "@layer:api"] };

const M = (name: string) => `${SHOWCASE_ID}.${name}`;
/** Installed as another extension, whose process says hello as the showcase. */
const WRONG = "example.wrongext";
/** Installed as itself, whose process is handed a manifest with one more method. */
const TAMPERED = "example.tampered";

const entry = async (api: TestClient, id: string) => (await api.call("packages.list")).packages.find((p: any) => p.id === id);
const runtime = async (api: TestClient, id = SHOWCASE_ID) => (await entry(api, id))?.runtime;

/** The runtime once `ok` says it is what was waited for. */
async function until(api: TestClient, what: string, ok: (rt: any) => boolean, id = SHOWCASE_ID): Promise<any> {
  const deadline = Date.now() + 60_000;
  for (;;) {
    const rt = await runtime(api, id);
    if (rt && ok(rt)) return rt;
    if (Date.now() > deadline) throw new Error(`never: ${what}; last ${JSON.stringify(rt)}`);
    await sleep(100);
  }
}

const alive = (pid: number) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

/**
 * The live run's model as a provider on the settings page, for an extension's own engine: that
 * engine has the product's providers and nothing of its environment, and the run hands `main` its
 * model by environment alone. A provider that changes is pushed to the engine at once.
 */
async function provideModel(api: TestClient): Promise<void> {
  const env = loadEnv();
  test.skip(!hasModel(env), "no model in .env");
  const config: Record<string, string> = { api_type: "anthropic", api_key: env.ANTHROPIC_AUTH_TOKEN || env.ANTHROPIC_API_KEY || "" };
  if (env.ANTHROPIC_BASE_URL) config.base_url = env.ANTHROPIC_BASE_URL;
  if (env.ANTHROPIC_MODEL) config.default_model = env.ANTHROPIC_MODEL;
  const set = await api.call("settings.setProvider", { provider_id: "live-run", config, default_provider: "live-run" });
  expect(set.reloaded_instances, "the extension's engine was not given the provider").toContain(`proc.${SHOWCASE_ID}`);
}

/** The showcase renamed `id` throughout its manifest. */
function renamed(dir: string, id: string, more = ""): string {
  return variant(join(dir, `${id}.hxext`), (entries) => {
    const manifest = entries.get("harnix-ext.toml")!.toString("utf8").replaceAll(SHOWCASE_ID, id) + more;
    entries.set("harnix-ext.toml", Buffer.from(manifest));
  });
}

/** The entries of a package, unpacked at `dir`. */
function unpack(path: string, dir: string, edit: (manifest: string) => string): string {
  for (const [name, body] of readZip(path)) {
    const at = join(dir, name);
    mkdirSync(join(at, ".."), { recursive: true });
    writeFileSync(at, name === "harnix-ext.toml" ? Buffer.from(edit(body.toString("utf8"))) : body);
  }
  return dir;
}

test.describe("packages in processes of their own", () => {
  test.describe.configure({ mode: "serial" });

  let backend: Backend;
  let api: TestClient;
  /** The two packages whose processes lie. */
  let variants: { wrong: string; tampered: string };

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    const scratch = join(tmpdir(), `harnix-process-${process.pid}`);
    mkdirSync(scratch, { recursive: true });
    const wrong = renamed(scratch, WRONG);
    const tampered = renamed(scratch, TAMPERED);
    const other = unpack(tampered, join(scratch, "other"), (m) => `${m}\n[[method]]\nname = "more"\ndescription = "Not installed"\n`);
    // `ext-proc --ext <id> --pkg <dir> --run <dir> --wire-fd 3 [--engine-fd 4]`, as the host
    // starts it: the process of WRONG says it is the showcase, the process of TAMPERED reads
    // another manifest.
    const program = join(scratch, "proc-program.sh");
    writeFileSync(
      program,
      [
        "#!/bin/sh",
        'ext="$3"; pkg="$5"',
        `if [ "$3" = "${WRONG}" ]; then ext="${SHOWCASE_ID}"; fi`,
        `if [ "$3" = "${TAMPERED}" ]; then pkg="${other}"; fi`,
        'a="$1"; b="$2"; d="$4"; shift 5',
        `exec "${appsTools().bin}" "$a" "$b" "$ext" "$d" "$pkg" "$@"`,
        "",
      ].join("\n"),
    );
    chmodSync(program, 0o755);
    variants = { wrong, tampered };
    backend = await startApps({ profile: `carrier = "process"\nproc_program = "${program}"\nsandbox = "off"\n` });
    const client = await connect(backend);
    try {
      await client.call("packages.installFile", { path: showcasePackage().path });
    } finally {
      client.close();
    }
  });
  test.afterAll(async () => {
    await backend?.dispose();
  });
  test.beforeEach(async () => {
    api = await connect(backend, { topology: topology() });
  });
  test.afterEach(() => api?.close());

  test("it runs in a process of its own, registered by the product", TAGS, async () => {
    const rt = await runtime(api);
    expect(rt).toMatchObject({ state: "running", carrier: "process" });
    expect(rt.process.pid).toEqual(expect.any(Number));
    expect(rt.process.starts).toBe(1);
    expect(alive(rt.process.pid)).toBe(true);
    const listed = (await api.call("packages.list")).runtime;
    expect(listed.carrier).toBe("process");
    const extensions = (await api.call("harnix.extensions")).extensions;
    expect(extensions.find((e: any) => e.id === SHOWCASE_ID)).toMatchObject({ trust: "consent", carrier: "process", by: "packages" });
  });

  test("its methods and topics are answered over its wire", TAGS, async () => {
    await api.call("harnix.topics.subscribe", { topic: M("notes") });
    expect(await api.call(M("notes.add"), { text: "over the wire" })).toEqual({ count: 1 });
    await api.waitFor((f) => f.method === "harnix.topic" && f.params.topic === M("notes") && f.params.value?.count === 1, {
      describe: "the board, pushed from the process",
    });
    const notes = (await api.call(M("notes.list"))).notes;
    expect(notes.map((n: any) => n.text)).toEqual(["over the wire"]);
    expect(notes[0].by).toMatch(/^device:/);
  });

  test("a backend that calls back into itself is refused at once", TAGS, async () => {
    const began = Date.now();
    const answer = await api.call(M("recall"));
    const took = Date.now() - began;
    expect(answer.refused).toBe(true);
    expect(answer.message).toMatch(/re-?enter/);
    // Refused in the process within 100 ms of the call coming back (`ext_proc.rs` times that); the
    // whole round trip here is a page's call, the product, the process and back, twice.
    expect(took).toBeLessThan(1_000);
  });

  test("a backend that calls back into itself from its timer is refused at once too", TAGS, async () => {
    // No call of the product's waits on a timer: the chain starts at the process's own call, and
    // the backend, held on the firing, refuses it in its process.
    expect(await api.call(M("recall"), { later: true })).toEqual({ armed: true });
    const recalled = await recalledOf(api);
    expect(recalled.refused).toBe(true);
    expect(recalled.message).toMatch(/re-?enter/);
    expect(recalled.took_ms).toBeLessThan(100);
  });

  test("killed, it is up again on another process, its rows never gone, its storage kept", TAGS, async () => {
    const before = await runtime(api);
    process.kill(before.process.pid, "SIGKILL");
    const after = await until(api, "up again", (rt) => rt.state === "running" && rt.process.pid !== before.process.pid);
    expect(after.process.starts).toBe(before.process.starts + 1);
    expect((await entry(api, SHOWCASE_ID)).registered).toBe(true);
    expect((await api.call("harnix.reachable")).methods).toContain(M("notes.add"));
    expect((await api.call(M("notes.list"))).notes).toHaveLength(1);
  });

  test("three crashes soon after starting set it aside, until it is switched off and on", TAGS, async () => {
    // One crash is counted already, from the test before.
    for (let n = 0; n < 2; n += 1) {
      const rt = await until(api, "up", (rt) => rt.state === "running");
      process.kill(rt.process.pid, "SIGKILL");
      if (n === 0) await until(api, "up again", (now) => now.state === "running" && now.process.pid !== rt.process.pid);
    }
    const faulted = await until(api, "set aside", (rt) => rt.state === "faulted");
    expect(faulted.reason).toMatch(/3 times|given up/);
    // Its rows stay, and say why.
    const e = await api.refused(M("notes.list"));
    expect(e?.code).toBe(EXTENSION_UNAVAILABLE);
    expect(e?.data?.failure).toBe("faulted");
    await api.call("packages.disable", { id: SHOWCASE_ID });
    await api.call("packages.enable", { id: SHOWCASE_ID });
    expect(await runtime(api)).toMatchObject({ state: "running" });
    expect((await api.call(M("notes.list"))).notes).toHaveLength(1);
  });

  test("a process that is not the extension it was started as is refused", TAGS, async () => {
    const e = await api.refused("packages.installFile", { path: variants.wrong });
    expect(e?.code).toBe(REFUSED);
    expect(e?.message).toContain(`it says it is \`${SHOWCASE_ID}\``);
    expect(await entry(api, WRONG)).toBeUndefined();
  });

  test("a process that registers what was not installed is refused and ended", TAGS, async () => {
    const e = await api.refused("packages.installFile", { path: variants.tampered });
    expect(e?.code).toBe(REFUSED);
    expect(e?.message).toMatch(/digest|`more`/);
    expect(await entry(api, TAMPERED)).toBeUndefined();
    expect((await api.call("harnix.reachable")).methods).not.toContain(`${TAMPERED}.more`);
  });

  test("stopped, its process goes", TAGS, async () => {
    const pid = (await runtime(api)).process.pid;
    await api.call("packages.disable", { id: SHOWCASE_ID });
    expect(await runtime(api)).toMatchObject({ state: "stopped" });
    expect(alive(pid)).toBe(false);
    await api.call("packages.enable", { id: SHOWCASE_ID });
  });
});

test.describe("packages in processes of their own, in their sandbox", () => {
  test.describe.configure({ mode: "serial" });

  let backend: Backend;
  let api: TestClient;

  const ENGINE = `proc.${SHOWCASE_ID}`;
  /** This machine has the sandbox (the first test finds out). */
  let sandboxed = false;

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    backend = await startApps({ profile: `carrier = "process"\n` });
  });
  test.afterAll(async () => {
    await backend?.dispose();
  });
  test.beforeEach(async () => {
    api = await connect(backend, { topology: topology() });
  });
  test.afterEach(() => api?.close());

  test("the showcase runs and answers in its sandbox, and the list says so", TAGS, async () => {
    const sandbox = (await api.call("packages.list")).runtime.sandbox;
    test.skip(sandbox?.mode === "unavailable", `no sandbox on this machine: ${sandbox?.why}`);
    expect(sandbox).toMatchObject({ mode: "on", mechanism: expect.any(String) });
    sandboxed = true;
    await api.call("packages.installFile", { path: showcasePackage().path });
    expect(await until(api, "running", (rt) => rt.state === "running")).toMatchObject({ carrier: "process" });
    expect((await entry(api, SHOWCASE_ID)).sandbox).toBe("on");
    // Its stderr's last lines are kept for the list (none yet is fine: the list says so).
    expect((await runtime(api)).process.stderr).toEqual(expect.any(Array));
    expect(await api.call(M("notes.add"), { text: "from inside" })).toEqual({ count: 1 });
  });

  test("its own engine runs there too, its agent package in it, and the shell it starts is held to the same", TAGS, async () => {
    test.skip(!sandboxed, "no sandbox on this machine");
    test.setTimeout(180_000);
    const run = join(backend.state, "ext", SHOWCASE_ID, "run");
    const home = join(run, "home");
    // A command its engine runs as a session starts, as its shell tool would run one: git and a
    // shell's own tools from its own HOME, and a read of its consent record, which it may not.
    const settingsFile = join(run, "engine", "settings.json");
    const settings = existsSync(settingsFile) ? JSON.parse(readFileSync(settingsFile, "utf8")) : {};
    const record = join(run, "..", "install.json");
    const command = [
      `cd "$HOME"`,
      "git --version > hook-git.txt 2>&1",
      "/bin/ls /bin > hook-ls.txt 2>&1",
      `cat "${record}" > hook-record.txt 2>&1; echo "exit $?" >> hook-record.txt`,
      "echo done > hook-done.txt",
    ].join("; ");
    writeFileSync(settingsFile, JSON.stringify({ ...settings, hooks_config: { SessionStart: [{ type: "command", command }] } }));
    // Read when its engine starts: its process started again.
    await api.call("packages.disable", { id: SHOWCASE_ID });
    await api.call("packages.enable", { id: SHOWCASE_ID });
    await until(api, "running", (rt) => rt.state === "running");
    const deadline = Date.now() + 30_000;
    while ((await instanceRow(api, ENGINE))?.status?.state !== "ready") {
      if (Date.now() > deadline) throw new Error("its engine is not up in its sandbox");
      await sleep(100);
    }
    // Up means its agent package went into its own engine: a placement that fails refuses the start.
    // A session in its space is on its engine; a turn starts it (and fails at the model: there is
    // none here, which the hook does not need).
    const created = await api.call("harnix.session.create", { space: SHOWCASE_ID });
    const key = { instance: ENGINE, session_id: created.session_id, space: SHOWCASE_ID };
    await api.call("harnix.session.attach", key);
    await api.call("harnix.session.send", { ...key, message: "Say ready." }).catch(() => {});
    const done = join(home, "hook-done.txt");
    for (const until = Date.now() + 60_000; !existsSync(done); await sleep(200)) {
      if (Date.now() > until) throw new Error(`its engine's hook never ran in its sandbox: ${readdirSync(home).join(", ")}`);
    }
    // On macOS git is xcrun's shim, which says it cannot keep its cache in the user's temporary
    // directory (out of reach) and runs all the same.
    expect(readFileSync(join(home, "hook-git.txt"), "utf8")).toMatch(/^git version /m);
    expect(readFileSync(join(home, "hook-ls.txt"), "utf8")).toMatch(/^sh$/m);
    const read = readFileSync(join(home, "hook-record.txt"), "utf8");
    expect(read).toMatch(/exit [1-9]/);
    expect(read).toMatch(/Operation not permitted|Permission denied/);
    expect(read).not.toContain('"version"');
  });

  test("its engine's shell tool runs in its sandbox", TAGS, async () => {
    test.skip(modelMode() !== "live", "a turn on the extension's own engine calls the model: it records and replays nothing");
    test.skip(!sandboxed, "no sandbox on this machine");
    await provideModel(api);
    const created = await api.call("harnix.session.create", { space: SHOWCASE_ID });
    const key = { instance: ENGINE, session_id: created.session_id, space: SHOWCASE_ID };
    await api.call("harnix.session.attach", key);
    await api.call("harnix.session.send", {
      ...key,
      message: "Use the Bash tool once to run exactly `git --version && ls /bin/sh && pwd`, then reply with its output and nothing else.",
    });
    const ofIt = (kind: string) => (f: any) => f.method === "harnix.event" && f.params.event?.kind === kind && f.params.session_id === created.session_id;
    // Its engine asks before it runs a shell command, as any engine does; permitted here.
    const first = await api.waitFor((f) => ofIt("prompt")(f) || ofIt("tool_result")(f), { describe: "its shell tool asked for or run", timeout: 180_000 });
    if (first.params.event.kind === "prompt") {
      await api.call("harnix.session.respond", { ...key, prompt_id: first.params.event.prompt_id, decision: { type: "permit" } });
    }
    const result = await api.waitFor(ofIt("tool_result"), { describe: "its shell tool's result", timeout: 60_000 });
    const text = JSON.stringify(result.params.event);
    expect(text).toContain("git version");
    expect(text).toContain("/bin/sh");
    expect(result.params.event.is_error ?? false).toBe(false);
  });
});

/** What the test application idles an on-demand backend's process for. */
const IDLE_MS = 1500;

const instanceRow = async (api: TestClient, id: string) =>
  (await api.call("harnix.instances.list")).instances.find((i: any) => i.id === id);

test.describe("an extension process's own engine", () => {
  test.describe.configure({ mode: "serial" });

  const ENGINE = `proc.${SHOWCASE_ID}`;
  let backend: Backend;
  let api: TestClient;
  let scratch: string;

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    scratch = mkdtempSync(join(tmpdir(), "hx-proc-engine-"));
    // Idles in its process after a minute (a manifest's least), and the product lets the process
    // go once that has happened and its engine is unused.
    const onDemand = variant(join(scratch, "on-demand.hxext"), (e) => {
      const manifest = e.get("harnix-ext.toml")!.toString().replace("watch_sessions = true", "watch_sessions = false");
      e.set("harnix-ext.toml", Buffer.from(`${manifest}\n[backend]\nresident = "on-demand"\nidle_secs = 60\n`));
    });
    backend = await startApps({
      bin: appsTestApp(),
      env: { HARNIX_TESTAPP_APP_IDLE_MS: String(IDLE_MS) },
      profile: `carrier = "process"\nsandbox = "off"\n`,
    });
    const client = await connect(backend);
    try {
      await client.call("packages.installFile", { path: onDemand });
    } finally {
      client.close();
    }
  });
  test.afterAll(async () => {
    await backend?.dispose();
    if (scratch) rmSync(scratch, { recursive: true, force: true });
  });
  test.beforeEach(async () => {
    api = await connect(backend, { topology: topology() });
  });
  test.afterEach(() => api?.close());

  test("it is adopted as an extension's engine, and its spaces' sessions are there", TAGS, async () => {
    expect(await instanceRow(api, ENGINE)).toMatchObject({ carrier: "adopted", trust: "extension", owner: `module:${SHOWCASE_ID}`, status: { state: "ready" } });
    const hello = await api.call("harnix.hello");
    expect(hello.instances.find((i: any) => i.id === ENGINE)).toMatchObject({ carrier: "adopted", trust: "extension" });
    for (const space of [SHOWCASE_ID, SHOWCASE_CHATS]) {
      const created = await api.call("harnix.session.create", { space });
      const listed = (await api.call("harnix.sessions.list", { space })).sessions.find((s: any) => s.session_id === created.session_id);
      expect(listed, space).toMatchObject({ instance: ENGINE, space, trust: "extension" });
      const attached = await api.call("harnix.session.attach", { instance: ENGINE, session_id: created.session_id, space });
      expect(attached.trust).toBe("extension");
      await api.call("harnix.session.detach", { instance: ENGINE, session_id: created.session_id, space });
    }
    // Nobody reaches its settings or plugins, and nobody but its extension stops it.
    expect((await api.refused("engine.plugin.list", { instance: ENGINE }))?.code).toBe(REFUSED);
    expect((await api.refused("engine.config.get", { instance: ENGINE }))?.code).toBe(REFUSED);
    expect((await api.refused("harnix.instances.restart", { instance: ENGINE }))?.code).toBe(REFUSED);
  });

  test("a refresh catches up on a turn without losing a frame", TAGS, async () => {
    test.skip(modelMode() !== "live", "a turn on the extension's own engine calls the model: it records and replays nothing");
    await provideModel(api);
    const created = await api.call("harnix.session.create", { space: SHOWCASE_ID });
    const key = { instance: ENGINE, session_id: created.session_id, space: SHOWCASE_ID };
    await api.call("harnix.session.attach", key);
    await api.call("harnix.session.send", { ...key, message: "Say the word ready, and nothing else." });
    await api.waitFor((f) => f.method === "harnix.event" && f.params.event?.kind === "user_message", { describe: "the turn's first frame" });
    // The page reloads mid-turn: a new client attaches and gets the turn once, buffer or history.
    const late = await connect(backend, { topology: api.topology });
    try {
      const attached = await late.call("harnix.session.attach", key);
      expect(attached.trust).toBe("extension");
      const replayed = attached.replay.filter((f: any) => f.event?.kind === "user_message");
      expect(replayed.length > 0 !== attached.history_total > 0, "the turn exactly once").toBe(true);
      await late.waitFor((f) => f.method === "harnix.event" && f.params.event?.kind === "turn_complete", {
        describe: "the turn settling, seen by the client that came late",
        timeout: 120_000,
      });
    } finally {
      late.close();
    }
  });

  test("its process restarted, its sessions come back from its own store", TAGS, async () => {
    const created = await api.call("harnix.session.create", { space: SHOWCASE_ID });
    const pid = (await runtime(api)).process.pid;
    process.kill(pid, "SIGKILL");
    await until(api, "up again", (rt) => rt.state === "running" && rt.process.pid !== pid);
    const deadline = Date.now() + 30_000;
    while ((await instanceRow(api, ENGINE))?.status?.state !== "ready") {
      if (Date.now() > deadline) throw new Error("its engine is not back");
      await sleep(100);
    }
    const sessions = (await api.call("harnix.sessions.list", { instance: ENGINE })).sessions;
    expect(sessions.map((s: any) => s.session_id)).toContain(created.session_id);
  });

  test("idle, its process is let go; reading lists and opening the system space wake nothing", TAGS, async () => {
    test.setTimeout(180_000);
    const listedBefore = (await api.call("harnix.sessions.list", { instance: ENGINE })).sessions.length;
    const pid = (await runtime(api)).process.pid;
    // Its backend idles a minute in its process; the product lets the process go after that.
    const asleep = await until(api, "hibernating", (rt) => rt.state === "hibernating");
    expect(asleep.process.pid).toBeNull();
    expect(alive(pid)).toBe(false);
    expect((await instanceRow(api, ENGINE)).status.state).toBe("hibernating");
    // What only reads is answered without it: the lists, the hello, the system space's views.
    expect((await api.call("harnix.sessions.list", { instance: ENGINE })).sessions).toHaveLength(listedBefore);
    await api.call("harnix.sessions.list", { space: SHOWCASE_ID });
    await api.call("harnix.instances.list");
    await api.call("harnix.hello");
    await api.call("harnix.sessions.list", { space: "system" }).catch(() => {});
    await sleep(IDLE_MS);
    expect((await runtime(api)).state).toBe("hibernating");
    expect((await instanceRow(api, ENGINE)).status.state).toBe("hibernating");
  });

  test("let go while idle, a call wakes it and is answered", TAGS, async () => {
    const asleep = await until(api, "hibernating", (rt) => rt.state === "hibernating");
    const starts = asleep.process.starts;
    expect((await api.call(M("notes.list"))).notes).toEqual(expect.any(Array));
    const woken = await runtime(api);
    expect(woken).toMatchObject({ state: "running" });
    expect(woken.process.starts).toBe(starts + 1);
    expect((await instanceRow(api, ENGINE)).status.state).toBe("ready");
    // And a call on its engine wakes it too, once it has gone again.
    await until(api, "hibernating again", (rt) => rt.state === "hibernating");
    const created = await api.call("harnix.session.create", { space: SHOWCASE_ID });
    expect(created.session_id).toEqual(expect.any(String));
    expect((await runtime(api)).state).toBe("running");
  });
});
