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
 * and hands TAMPERED a package outside its sandbox. The last describe runs the showcase in its
 * sandbox, started as this binary.
 *
 * Without the engine part: no `agent/`, no `[[space]]` sessions on `proc.<id>`. A product
 * of another build (another Harnix, Agix or VM engine) is refused too; that takes another build,
 * and `crates/carrier/proc/tests/process.rs` makes one by telling the carrier another version.
 */

import { chmodSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { EXTENSION_UNAVAILABLE, REFUSED } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, sleep, test, topology } from "@harnix/testkit";

import { SHOWCASE_ID, appsTools, readZip, showcasePackage, startApps, variant } from "./packages-host";

const TAGS = { tag: ["@feature:apps", "@layer:api"] };

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
    // `ext-proc --ext <id> --pkg <dir> --run <dir> --wire-fd 3`, as the host starts it: the
    // process of WRONG says it is the showcase, the process of TAMPERED reads another manifest.
    const program = join(scratch, "proc-program.sh");
    writeFileSync(
      program,
      [
        "#!/bin/sh",
        'ext="$3"; pkg="$5"',
        `if [ "$3" = "${WRONG}" ]; then ext="${SHOWCASE_ID}"; fi`,
        `if [ "$3" = "${TAMPERED}" ]; then pkg="${other}"; fi`,
        `exec "${appsTools().bin}" "$1" "$2" "$ext" "$4" "$pkg" "$6" "$7" "$8" "$9"`,
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
    expect(answer.message).toContain("reentrant");
    // Refused in the process within 100 ms of the call coming back (`ext_proc.rs` times that); the
    // whole round trip here is a page's call, the product, the process and back, twice.
    expect(took).toBeLessThan(1_000);
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
    await api.call("packages.installFile", { path: showcasePackage().path });
    expect(await until(api, "running", (rt) => rt.state === "running")).toMatchObject({ carrier: "process" });
    expect((await entry(api, SHOWCASE_ID)).sandbox).toBe("on");
    expect(await api.call(M("notes.add"), { text: "from inside" })).toEqual({ count: 1 });
  });
});
