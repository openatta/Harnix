/**
 * Runtime applications, stage three: a package installed from a file, and everything it brings —
 * its WebAssembly tools in every new session (asking first, every time), its event hook on every
 * tool call, its agent package (a QuickJS prompt block, an agent type), its skill, its interface
 * module — then switched off, on, and taken out. On the apps-enabled binary (`apps-host.ts`), with
 * the showcase application (`examples/apps/showcase`).
 *
 * The model's tool calls are hand-made recordings (`fixtures/recordings/app-*`). A daemon this
 * process spawns runs no VM: it reaches the same tools over the application's MCP endpoint, and
 * the call runs here.
 */

import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { REFUSED } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, modelMode, sleep, test, topology } from "@harnix/testkit";

import { SHOWCASE_ID, showcasePackage, startApps, variant } from "./apps-host";

const TAGS = { tag: ["@feature:apps", "@layer:api"] };

const TOOL = (name: string) => `app__example_showcase__${name}`;
const TOOLS = ["echo", "count", "config", "read_project", "slow", "vetoed", "board"].map(TOOL);
const MCP_SERVER = "app-example_showcase";
const BLOCK_MARKER = "showcase-block-7d21";

type Session = { instance: string; session_id: string };

/** Asks answered by any turn so far, so a later turn does not answer an old one again. */
const handled = new Set<string>();

async function session(api: TestClient, recording: string, instance = "system"): Promise<Session> {
  const created = await api.call("session.create", {
    instance,
    project_root: api.project,
    options: { recorder: { name: recording } },
  });
  const s = { instance, session_id: created.session_id };
  await api.call("harnix.attach", s);
  return s;
}

/** Send, answer every permission ask with `decision`, and wait for the turn to settle. */
async function turn(api: TestClient, s: Session, message: string, decision = "permit"): Promise<{ settled: any; asks: any[] }> {
  const asks: any[] = [];
  const sent = await api.call("harnix.send", { ...s, message });
  const answered = new Set<string>();
  const deadline = Date.now() + 90_000;
  for (;;) {
    const settled = api.events.find(
      (f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id
        && (!sent.turn_id || f.params.turn_id === sent.turn_id),
    );
    if (settled) return { settled, asks };
    for (const f of api.events) {
      if (f.method !== "harnix.event" || f.params.event?.kind !== "prompt") continue;
      const prompt = f.params.event;
      // Every ask this client sees while the turn runs: a sub-agent's comes in under its own id.
      if (answered.has(prompt.prompt_id) || handled.has(prompt.prompt_id)) continue;
      answered.add(prompt.prompt_id);
      handled.add(prompt.prompt_id);
      asks.push(f.params);
      await api.call("session.respondToPrompt", {
        instance: f.params.instance ?? s.instance,
        session_id: f.params.session_id,
        prompt_id: prompt.prompt_id,
        decision: decision === "deny" ? { type: "deny", reason: "not now" } : { type: decision },
      });
    }
    if (Date.now() > deadline) throw new Error(`the turn "${message}" did not settle; asks so far: ${JSON.stringify(asks)}`);
    await sleep(50);
  }
}

const results = (api: TestClient, s: Session) => JSON.stringify(api.eventsOfKind("tool_result", s.session_id));

/** What a new session in `instance` would send the model: its system prompt's text and its tools. */
async function context(api: TestClient, instance = "system"): Promise<{ text: string; tools: string[]; s: Session }> {
  const s = await session(api, "says-ready", instance);
  const context = await api.call("harnix.system.context", s);
  const text = JSON.stringify(context);
  const tools: string[] = (context.tools ?? context.context?.tools ?? []).map((t: any) => t.name ?? t);
  return { text, tools, s };
}

test.describe("an application, installed from a file", () => {
  test.describe.configure({ mode: "serial" });

  let backend: Backend;
  let api: TestClient;
  let scratch: string;

  test.beforeAll(async () => {
    // Before the backend: building and packing is the slow part, and has nothing to do with it.
    showcasePackage();
    scratch = mkdtempSync(join(tmpdir(), "hx-apps-"));
    backend = await startApps({
      profile: `
[apps.config."${SHOWCASE_ID}"]
greeting = "hello from the profile"

[[instances]]
id = "far"
carrier = "spawn"
`,
    });
  });
  test.afterAll(async () => {
    await backend?.dispose();
    if (scratch) rmSync(scratch, { recursive: true, force: true });
  });
  test.beforeEach(async () => {
    api = await connect(backend, { topology: topology() });
  });
  test.afterEach(() => api?.close());

  test("a package that is wrong is refused with why, and nothing of it stays", TAGS, async () => {
    const { fingerprint } = showcasePackage();
    const cases: [string, string, (e: Map<string, Buffer>) => void][] = [
      ["bad-toml", "not valid TOML", (e) => e.set("harnix-app.toml", Buffer.from("[app\nid = 1"))],
      ["unknown-capability", "unknown capability `gpu`", (e) => {
        e.set("harnix-app.toml", Buffer.from(e.get("harnix-app.toml")!.toString().replace("storage_mb = 1", "storage_mb = 1\ngpu = true")));
      }],
      ["unknown-section", "unknown section `daemon`", (e) => {
        e.set("harnix-app.toml", Buffer.from(`${e.get("harnix-app.toml")!.toString()}\n[daemon]\nrun = "x"\n`));
      }],
      ["methods-without-a-backend", "are a backend's", (e) => {
        e.set("harnix-app.toml", Buffer.from(e.get("harnix-app.toml")!.toString().replace("backend-app@0.1.0", "tools-app@0.1.0")));
      }],
      ["other-engine", "engine-0.0.0+elsewhere+000000000000", (e) => {
        const cwasm = `wasm/tools.${fingerprint}.cwasm`;
        e.set("wasm/tools.engine-0.0.0+elsewhere+000000000000.cwasm", e.get(cwasm)!);
        e.delete(cwasm);
      }],
      ["escape", "leaves the package", (e) => e.set("../escape.txt", Buffer.from("x"))],
      ["no-instance", "no such instance", (e) => {
        e.set("harnix-app.toml", Buffer.from(e.get("harnix-app.toml")!.toString().replace('install_into = ["system"]', 'install_into = ["nowhere"]')));
      }],
    ];
    for (const [name, says, edit] of cases) {
      const path = variant(join(scratch, `${name}.hxapp`), edit);
      const error = await api.refused("harnix.apps.installFile", { path });
      expect(error?.code, name).toBe(REFUSED);
      expect(error?.message, name).toContain(says);
      if (name === "other-engine") expect(error?.message, name).toContain(fingerprint);
    }
    expect((await api.refused("harnix.apps.installFile", { path: join(scratch, "missing.hxapp") }))?.code).toBe(REFUSED);
    expect((await api.refused("harnix.apps.installFile", {}))?.message).toContain("missing path");
    const listed = await api.call("harnix.apps.list");
    expect(listed.apps).toEqual([]);
    expect(listed.runtime).toMatchObject({ fingerprint, dev: true });
    const plugins = (await api.call("plugin.list")).plugins.map((p: any) => p.name);
    expect(plugins).not.toContain("example-showcase");
    expect(existsSync(join(backend.state, "apps", SHOWCASE_ID))).toBe(false);
  });

  test("installed: its agent package, its skill, its tools and its interface module, at once", TAGS, async () => {
    const { path, fingerprint } = showcasePackage();
    const installed = await api.call("harnix.apps.installFile", { path });
    expect(installed).toMatchObject({ id: SHOWCASE_ID, version: "0.1.0", title: "Showcase", enabled: true });
    expect(installed.installed).toMatchObject({
      fingerprint,
      agent: { package: "example-showcase", instances: ["system"] },
      skills: { system: ["showcase-tour"] },
    });
    const listed = (await api.call("harnix.apps.list")).apps;
    expect(listed).toHaveLength(1);
    expect(listed[0]).toMatchObject({ id: SHOWCASE_ID, enabled: true, runtime: { state: "running" } });
    expect(listed[0].runtime.tools.map((t: any) => t.name).sort()).toEqual([...TOOLS].sort());
    expect(listed[0].runtime.health).toMatchObject({ world: "backend", state: "running" });

    // What the engine says the agent package puts in front of the model, as the installer shows it.
    const disclosure = JSON.stringify(installed.installed.agent.disclosure);
    expect(disclosure).toContain("prompt.block");
    expect(disclosure).toContain("showcase-guide");
    const plugin = (await api.call("plugin.list")).plugins.find((p: any) => p.name === "example-showcase");
    expect(plugin, "the agent package is installed in system").toBeTruthy();
    expect(plugin.enabled).not.toBe(false);
    expect((await api.refused("harnix.apps.installFile", { path }))?.message).toContain("installed already");

    // The interface module is served beside the page, and named in what a client is told.
    const module = await fetch(`${backend.url.replace(/\/$/, "")}/apps/${SHOWCASE_ID}/ui/showcase.js`);
    expect(module.status).toBe(200);
    expect(module.headers.get("content-type")).toContain("javascript");
    expect(await module.text()).toContain("example.showcase.card");
    expect((await fetch(`${backend.url.replace(/\/$/, "")}/apps/${SHOWCASE_ID}/ui/../harnix-app.toml`)).status).toBe(404);
    await expect
      .poll(async () => JSON.stringify((await api.call("harnix.hello")).contributions ?? []), { timeout: 10_000 })
      .toContain(`/apps/${SHOWCASE_ID}/ui/showcase.js`);
    // What a store has to compile for, for this process to run it.
    expect(JSON.stringify(await api.call("harnix.process"))).toContain(fingerprint);
  });

  test("a new session has its tools, its QuickJS block and its skill", TAGS, async () => {
    const { text, tools } = await context(api);
    for (const tool of TOOLS) expect(tools.length ? tools : text, tool).toContain(tool);
    expect(text, "the QuickJS prompt.block").toContain(BLOCK_MARKER);
    const commands = JSON.stringify(await api.call("commands.list", { instance: "system" }));
    expect(commands, "the skill").toContain("showcase-tour");
  });

  test("each tool asks first, and answers once permitted", TAGS, async () => {
    const cases: [string, string, string][] = [
      ["app-echo", "echo", "showcase-echo-42"],
      ["app-count", "count", "count = 1"],
      ["app-config", "config", "hello from the profile"],
      ["app-read-project", "read_project", "A project for the tests to run in."],
    ];
    for (const [recording, tool, answer] of cases) {
      const s = await session(api, recording);
      const { settled, asks } = await turn(api, s, `Use the showcase's ${tool} tool`);
      expect(settled.params.error, `${tool}: ${JSON.stringify(settled.params.error)}`).toBeUndefined();
      expect(asks.length, `${tool} asked`).toBeGreaterThanOrEqual(1);
      expect(JSON.stringify(asks[0]), tool).toContain(TOOL(tool));
      expect(api.eventsOfKind("tool_use", s.session_id).map((u: any) => u.name)).toContain(TOOL(tool));
      expect(results(api, s), tool).toContain(answer);
    }
    // The configuration is the manifest's, with the profile's set over it.
    const config = await session(api, "app-config");
    await turn(api, config, "again");
    expect(results(api, config)).toContain("app__example_showcase__vetoed");
    // The count is kept by the application, not by the call.
    const again = await session(api, "app-count");
    await turn(api, again, "count again");
    expect(results(api, again)).toContain("count = 2");
  });

  test("refused when asked: the tool does not run", TAGS, async () => {
    const s = await session(api, "app-count");
    const { asks } = await turn(api, s, "count, and be refused", "deny");
    expect(asks.length).toBeGreaterThanOrEqual(1);
    expect(results(api, s)).not.toContain("count = ");
    const counted = await session(api, "app-count");
    await turn(api, counted, "count");
    expect(results(api, counted), "the refused call counted").toContain("count = 3");
  });

  test("its progress is pushed while it works", TAGS, async () => {
    await api.call("harnix.subscribe", { topic: "harnix.apps.progress" });
    const s = await session(api, "app-slow");
    const { settled } = await turn(api, s, "Use the slow tool");
    expect(settled.params.error).toBeUndefined();
    const steps = api.events
      .filter((f) => f.method === "harnix.topic" && f.params.topic === "harnix.apps.progress"
        && f.params.value?.session_id === s.session_id)
      .map((f) => f.params.value);
    expect(steps.map((v) => v.text)).toEqual(["step 1 of 3", "step 2 of 3", "step 3 of 3"]);
    expect(steps[0]).toMatchObject({ app: SHOWCASE_ID, tool: TOOL("slow") });
    expect(results(api, s)).toContain("worked through 3 steps");
  });

  test("cancelled while it works, it stops, and the application goes on", TAGS, async () => {
    await api.call("harnix.subscribe", { topic: "harnix.apps.progress" });
    const s = await session(api, "app-slow-long");
    await api.call("harnix.send", { ...s, message: "Use the slow tool for a long while" });
    const ask = await api.waitFor(
      (f) => f.method === "harnix.event" && f.params.session_id === s.session_id && f.params.event.kind === "prompt",
      { timeout: 30_000, describe: "the ask for the slow tool" },
    );
    await api.call("session.respondToPrompt", { ...s, prompt_id: ask.params.event.prompt_id, decision: { type: "permit" } });
    await api.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === "harnix.apps.progress"
        && f.params.value?.session_id === s.session_id && f.params.value?.text === "step 2 of 100",
      { timeout: 30_000, describe: "the slow tool under way" },
    );
    const interrupted = Date.now();
    await api.call("session.interrupt", s);
    await api.waitFor(
      (f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id,
      { timeout: 15_000, describe: "the interrupted turn to settle" },
    );
    expect(Date.now() - interrupted).toBeLessThan(10_000);
    expect(results(api, s)).not.toContain("worked through");
    const seen = api.events.filter((f) => f.method === "harnix.topic" && f.params.value?.session_id === s.session_id).length;
    await sleep(1000);
    const after = api.events.filter((f) => f.method === "harnix.topic" && f.params.value?.session_id === s.session_id).length;
    expect(after, "the call went on after the turn was cancelled").toBe(seen);
    const runtime = (await api.call("harnix.apps.list")).apps[0].runtime;
    expect(runtime.state).toBe("running");
    expect(runtime.health.consecutive_faults).toBe(0);
  });

  test("its event hook refuses the tool its configuration names", TAGS, async () => {
    const s = await session(api, "app-vetoed");
    await turn(api, s, "Use the vetoed tool");
    const out = results(api, s);
    expect(out).toContain(`refused by the application \`${SHOWCASE_ID}\``);
    expect(out).toContain("blocked_tool");
    expect(out).not.toContain("should not have");
  });

  test("a sub-agent of the application's own agent type uses its tool", TAGS, async () => {
    const s = await session(api, "app-subagent");
    const { settled, asks } = await turn(api, s, "Have a sub-agent call the showcase's echo tool");
    expect(settled.params.error, JSON.stringify(settled.params.error)).toBeUndefined();
    expect(JSON.stringify(asks), "the sub-agent's call asked").toContain(TOOL("echo"));
    expect(api.text(s.session_id)).toContain("The sub-agent used the showcase's tool.");
    const subagents = JSON.stringify(api.events.filter((f) => f.method === "harnix.event" && f.params.session_id === s.session_id));
    expect(subagents).toContain("showcase-sub-7");
  });

  test("a spawned instance reaches the tools over the application's MCP endpoint", TAGS, async () => {
    // The daemon is told of the endpoint once this process listens; it connects in the background.
    await expect
      .poll(async () => JSON.stringify(await api.call("mcp.status", { instance: "far" }).catch((e) => e)), { timeout: 60_000, intervals: [500] })
      .toContain(`"name":"${MCP_SERVER}","tool_count":7`);
    const s = await session(api, "app-mcp-echo", "far");
    const { settled, asks } = await turn(api, s, "Use the showcase's echo tool over MCP");
    expect(settled.params.error, JSON.stringify(settled.params.error)).toBeUndefined();
    expect(asks.length, "the daemon asked first").toBeGreaterThanOrEqual(1);
    expect(results(api, s)).toContain("showcase-over-mcp");

    // The endpoint is the application's token's alone.
    const url = `${backend.url.replace(/\/$/, "")}/apps/${SHOWCASE_ID}/mcp`;
    const anonymous = await fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${backend.token}` },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }),
    });
    expect(anonymous.status, "the page's token opened the MCP endpoint").toBe(401);
    expect((await fetch(url)).status).toBe(405);
  });

  test("disabled: a new session has none of it, and enabled it is back", TAGS, async () => {
    const off = await api.call("harnix.apps.disable", { id: SHOWCASE_ID });
    expect(off).toMatchObject({ id: SHOWCASE_ID, enabled: false, runtime: { state: "stopped" } });
    const { text, tools } = await context(api);
    for (const tool of TOOLS) expect(tools.length ? tools : [text], tool).not.toContain(tool);
    expect(text).not.toContain(BLOCK_MARKER);
    expect(JSON.stringify(await api.call("commands.list", { instance: "system" }))).not.toContain("showcase-tour");
    expect((await fetch(`${backend.url.replace(/\/$/, "")}/apps/${SHOWCASE_ID}/ui/showcase.js`)).status).toBe(404);
    const plugin = (await api.call("plugin.list")).plugins.find((p: any) => p.name === "example-showcase");
    expect(plugin?.enabled).toBe(false);
    await expect
      .poll(async () => JSON.stringify(await api.call("mcp.status", { instance: "far" }).catch(() => "")), { timeout: 60_000, intervals: [500] })
      .not.toContain(MCP_SERVER);

    const on = await api.call("harnix.apps.enable", { id: SHOWCASE_ID });
    expect(on).toMatchObject({ enabled: true, runtime: { state: "running" } });
    const back = await context(api);
    expect(back.tools.length ? back.tools : back.text).toContain(TOOL("echo"));
    expect(back.text).toContain(BLOCK_MARKER);
    expect((await fetch(`${backend.url.replace(/\/$/, "")}/apps/${SHOWCASE_ID}/ui/showcase.js`)).status).toBe(200);
  });

  test("uninstalled: everything it brought goes, its data with it", TAGS, async () => {
    const removed = await api.call("harnix.apps.uninstall", { id: SHOWCASE_ID });
    expect(removed).toMatchObject({ id: SHOWCASE_ID, uninstalled: true, removed: { agent: { package: "example-showcase" } } });
    expect((await api.call("harnix.apps.list")).apps).toEqual([]);
    expect((await api.call("plugin.list")).plugins.map((p: any) => p.name)).not.toContain("example-showcase");
    expect(JSON.stringify(await api.call("commands.list", { instance: "system" }))).not.toContain("showcase-tour");
    const { text, tools } = await context(api);
    expect(tools.length ? tools : [text]).not.toContain(TOOL("echo"));
    expect(text).not.toContain(BLOCK_MARKER);
    expect(existsSync(join(backend.state, "apps", SHOWCASE_ID)), "its directory, data and all").toBe(false);
    expect((await fetch(`${backend.url.replace(/\/$/, "")}/apps/${SHOWCASE_ID}/ui/showcase.js`)).status).toBe(404);
    expect((await api.refused("harnix.apps.uninstall", { id: SHOWCASE_ID }))?.message).toContain("no application");

    // And it installs again, from nothing: the count starts over.
    await api.call("harnix.apps.installFile", { path: showcasePackage().path });
    const s = await session(api, "app-count");
    await turn(api, s, "count");
    expect(results(api, s)).toContain("count = 1");
    await api.call("harnix.apps.uninstall", { id: SHOWCASE_ID });
  });
});

test.describe("an application, against the model", () => {
  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    if (modelMode() !== "live") return;
    backend = await startApps({ mode: "live" });
    const client = await connect(backend);
    try {
      await client.call("harnix.apps.installFile", { path: showcasePackage().path });
    } finally {
      client.close();
    }
  });
  test.afterAll(async () => { await backend?.dispose(); });

  test("the QuickJS block and the tools are in the request envelope", TAGS, async () => {
    test.skip(modelMode() !== "live", "a replayed turn writes no envelope; the context check above covers replay");
    api = await connect(backend, { topology: topology() });
    try {
      const created = await api.call("session.create", { project_root: api.project });
      const s = { instance: "system", session_id: created.session_id };
      await api.call("harnix.attach", s);
      await turn(api, s, "Say exactly: ready");
      const { headers } = await api.call("harnix.requestHeaders", s);
      const envelope = JSON.stringify(headers);
      expect(envelope).toContain(BLOCK_MARKER);
      expect(envelope).toContain(TOOL("echo"));
    } finally {
      api.close();
    }
  });
});

test.describe("an application, with [apps] dev off", () => {
  let backend: Backend;
  let api: TestClient;

  test.beforeAll(async () => {
    showcasePackage();
    backend = await startApps({ dev: false });
  });
  test.afterAll(async () => { await backend?.dispose(); });
  test.beforeEach(async () => {
    api = await connect(backend, { topology: topology() });
  });
  test.afterEach(() => api?.close());

  test("an unsigned package is not installed from a file", TAGS, async () => {
    const error = await api.refused("harnix.apps.installFile", { path: showcasePackage().path });
    expect(error?.code).toBe(REFUSED);
    expect(error?.message).toContain("[apps] dev = true");
    expect((await api.call("harnix.apps.list")).apps).toEqual([]);
    expect((await api.call("harnix.apps.list")).runtime).toMatchObject({ dev: false });
  });
});
