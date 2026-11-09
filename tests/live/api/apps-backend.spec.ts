/**
 * Runtime applications, stage four: a WebAssembly backend. The showcase (`examples/apps/showcase`)
 * is a `backend-app`: one resident instance in this process, its methods `app.example.showcase.*`
 * in the authorization table, its topics pushed like any host topic, its space listed — and what
 * it does as itself (`module:example.showcase`) admitted against its manifest's `hub` and audited.
 *
 * The round trip a page makes (a method, the backend, its topic, back to the page) is here as a
 * client makes it; `ui/apps.spec.ts` makes it from the showcase's own interface module. A
 * timeout, a crash and a withdrawal are each provoked on purpose and answered the way the design
 * says: the instance recycled, rebuilt once and then set aside, the names gone.
 *
 * `fetch` reaches a server this suite starts on loopback, the one host the showcase's `net` lists.
 */

import { type AddressInfo } from "node:net";
import { type Server, createServer } from "node:http";

import { INVALID_PARAMS, INTERNAL_ERROR, METHOD_NOT_FOUND } from "@harnix/core/protocol";
import { type Backend, type TestClient, connect, expect, runTurn, sleep, test, topology } from "@harnix/testkit";

import { SHOWCASE_CHATS, SHOWCASE_ID, showcasePackage, startApps } from "./apps-host";

const TAGS = { tag: ["@feature:apps", "@layer:api"] };

const M = (name: string) => `app.${SHOWCASE_ID}.${name}`;
const ME = `module:${SHOWCASE_ID}`;

type Session = { instance: string; session_id: string };

const audit = async (api: TestClient) => (await api.call("harnix.audit", { limit: 1000 })).entries as any[];
const runtime = async (api: TestClient) => (await api.call("harnix.apps.list")).apps.find((a: any) => a.id === SHOWCASE_ID).runtime;
const topicValues = (api: TestClient, topic: string) =>
  api.events.filter((f) => f.method === "harnix.topic" && f.params.topic === topic).map((f) => f.params.value);

test.describe("an application with a backend", () => {
  test.describe.configure({ mode: "serial" });

  let backend: Backend;
  let api: TestClient;
  let server: Server;
  let served = "";

  test.beforeAll(async () => {
    test.setTimeout(600_000);
    showcasePackage();
    server = createServer((req, res) => {
      res.writeHead(200, { "content-type": "text/plain" });
      res.end(`hello from the test server, ${req.url}`);
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    served = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
    backend = await startApps({ env: { SHOWCASE_TOKEN: "set-for-the-suite" } });
    const client = await connect(backend);
    try {
      await client.call("harnix.apps.installFile", { path: showcasePackage().path });
    } finally {
      client.close();
    }
  });
  test.afterAll(async () => {
    await backend?.dispose();
    await new Promise((resolve) => server?.close(resolve));
  });
  test.beforeEach(async () => {
    api = await connect(backend, { topology: topology() });
  });
  test.afterEach(() => api?.close());

  test("its methods, topics and space are the process's, under app.<id>.*", TAGS, async () => {
    const rt = await runtime(api);
    expect(rt).toMatchObject({ state: "running", world: "backend" });
    expect(rt.methods).toEqual(expect.arrayContaining([M("notes.add"), M("notes.list"), M("state"), M("session.start")]));
    expect(rt.topics).toEqual([M("notes"), M("heartbeat")]);
    expect(rt.spaces).toEqual([SHOWCASE_ID, SHOWCASE_CHATS]);
    const registered = (await api.call("harnix.apps.list")).registered.find((r: any) => r.id === SHOWCASE_ID);
    expect(registered).toMatchObject({ by: "apps", hub: ["session.create", "harnix.send", "topic:harnix.sessions.status", "harnix.entries.append"] });
    // Reachable to this page like any method, and listed among the spaces.
    const reachable = await api.call("harnix.reachable");
    expect(reachable.methods).toContain(M("notes.add"));
    expect(reachable.topics).toContain(M("notes"));
    expect(JSON.stringify(await api.call("harnix.spaces.list"))).toContain(`"${SHOWCASE_ID}"`);
    // On the record, by the module that hosts it.
    const declared = (await audit(api)).filter((e) => e.decision === "declare" && e.subject === "module:apps");
    expect(declared.map((e) => e.method)).toEqual(expect.arrayContaining([M("notes.add"), `topic:${M("notes")}`, `space:${SHOWCASE_ID}`]));
    // `init` subscribed to the sessions' status, as its `hub` allows.
    expect(rt.health.subscriptions).toBe(1);
  });

  test("the round trip: a method, the backend, its topic, back", TAGS, async () => {
    const subscribed = await api.call("harnix.subscribe", { topic: M("notes") });
    expect(subscribed.topic).toBe(M("notes"));
    const added = await api.call(M("notes.add"), { text: "first note" });
    expect(added).toEqual({ count: 1 });
    const pushed = await api.waitFor(
      (f) => f.method === "harnix.topic" && f.params.topic === M("notes") && f.params.value?.count === 1,
      { describe: "the board, pushed" },
    );
    const note = pushed.params.value.notes[0];
    expect(note.text).toBe("first note");
    // The backend was told who called: this page, a device.
    expect(note.by).toMatch(/^device:/);
    expect((await api.call(M("notes.list"))).notes.map((n: any) => n.text)).toEqual(["first note"]);

    // Params that do not match the declared schema never reach the backend.
    for (const params of [{}, { text: "" }, { text: 7 }, { text: "x", extra: true }]) {
      const e = await api.refused(M("notes.add"), params);
      expect(e?.code, JSON.stringify(params)).toBe(INVALID_PARAMS);
      expect(e?.message).toContain("schema");
    }
    expect((await api.call(M("notes.list"))).notes).toHaveLength(1);
    // Calls into it are counted, errors among them.
    const calls = (await runtime(api)).calls;
    expect(calls.calls).toBeGreaterThanOrEqual(2);
    expect(calls.methods.find((m: any) => m.name === "notes.add")).toMatchObject({ calls: 5, errors: 4 });
  });

  test("its timer fires into the backend, which publishes its heartbeat", TAGS, async () => {
    await api.call("harnix.subscribe", { topic: M("heartbeat") });
    expect(await api.call(M("timer.start"), { ms: 100, times: 3 })).toMatchObject({ armed: true });
    await api.waitFor((f) => f.method === "harnix.topic" && f.params.topic === M("heartbeat") && f.params.value?.ticks === 3, {
      describe: "the third heartbeat",
    });
    expect(topicValues(api, M("heartbeat")).filter(Boolean).map((v) => v.ticks)).toEqual([1, 2, 3]);
  });

  test("a tool reads the backend's state, which its own instance cannot see, through host.call", TAGS, async () => {
    expect(await api.call(M("counter.bump"))).toEqual({ counter: 1 });
    const created = await api.call("session.create", { instance: "system", project_root: api.project, options: { recorder: { name: "app-board" } } });
    const s = { instance: "system", session_id: created.session_id };
    await api.call("harnix.attach", s);
    const settled = (await runTurn(api, s, "What is on the showcase's board?")).settled;
    expect(settled.params.error).toBeUndefined();
    const results = JSON.stringify(api.eventsOfKind("tool_result", s.session_id));
    expect(results).toContain("the board has 1 notes; the counter is at 1");
    // Its tool asked as the application itself, and was let in as one of its own methods.
    const asked = (await audit(api)).find((e) => e.subject === ME && e.method === M("state"));
    expect(asked?.decision).toBe("allow");
    // What the backend could read through `host.secret`: a name its `env` lists.
    expect(await api.call(M("state"))).toMatchObject({ secret_set: true, counter: 1, notes: 1 });
  });

  test("it reaches the network at the hosts its net lists, and nowhere else", TAGS, async () => {
    const fetched = await api.call(M("fetch"), { url: `${served}/ping` });
    expect(fetched).toEqual({ status: 200, body: "hello from the test server, /ping" });
    // The backend's own error, passed through as it gave it.
    const elsewhere = await api.refused(M("fetch"), { url: served.replace("127.0.0.1", "localhost") });
    expect(elsewhere?.code).toBe(4);
    expect(elsewhere?.message).toContain("`localhost` is not in this application's `net` capability");
  });

  test("the backend drives a session: creating one in its space, and sending it a prompt", TAGS, async () => {
    const started = await api.call(M("session.start"), {
      message: "Started by the showcase",
      project_root: api.project,
      space: SHOWCASE_ID,
      options: { recorder: { name: "app-backend-session" } },
    });
    expect(started.session_id).toBeTruthy();
    const s = { instance: "system", session_id: started.session_id };
    await expect
      .poll(async () => JSON.stringify(await api.call("session.history", s).catch(() => "")), { timeout: 30_000, intervals: [200] })
      .toContain("The showcase's backend started this conversation.");
    // It landed in the application's space.
    const listed = await api.call("harnix.sessions", { space: SHOWCASE_ID });
    expect(JSON.stringify(listed)).toContain(started.session_id);
    // As the application, through the admission point, on the record.
    const entries = await audit(api);
    for (const method of ["session.create", "harnix.send"]) {
      expect(entries.find((e) => e.subject === ME && e.method === method)?.decision, method).toBe("allow");
    }
    // And what it subscribed to (`harnix.sessions.status`) told it about the run.
    await expect.poll(async () => (await api.call(M("state"))).status_seen, { timeout: 10_000 }).toBeGreaterThan(0);
  });

  test("a call beyond its hub capability is refused, and on the record", TAGS, async () => {
    await api.call("harnix.subscribe", { topic: "harnix.apps" });
    const answer = await api.call(M("overreach"));
    expect(answer.refused).toBe(true);
    expect(answer.message).toContain("not in the application's `hub` capability");
    const entry = (await audit(api)).find((e) => e.subject === ME && e.method === "harnix.apps.uninstall");
    expect(entry).toMatchObject({ decision: "refuse", reason: "not in the application's `hub` capability" });
    // Nothing happened, and the system space has it among the refusals.
    const rt = await runtime(api);
    expect(rt.state).toBe("running");
    expect(rt.calls.refused[0]).toMatchObject({ method: "harnix.apps.uninstall" });
    // Pushed on `harnix.apps` too, without being asked.
    await expect
      .poll(() => {
        const last = topicValues(api, "harnix.apps").at(-1);
        return JSON.stringify(last?.apps?.find((a: any) => a.id === SHOWCASE_ID)?.runtime?.calls?.refused ?? []);
      }, { timeout: 10_000 })
      .toContain("harnix.apps.uninstall");
  });

  test("a call that outlives its timeout recycles the backend: init again, memory gone, storage kept", TAGS, async () => {
    expect(await api.call(M("counter.bump"))).toEqual({ counter: 2 });
    const before = (await api.call(M("state"))).started_ms;
    const began = Date.now();
    const e = await api.refused(M("spin"));
    expect(e?.code).toBe(INTERNAL_ERROR);
    expect(e?.data).toMatchObject({ app: SHOWCASE_ID, failure: "timed_out" });
    expect(Date.now() - began).toBeGreaterThanOrEqual(4_500);
    const health = (await runtime(api)).health;
    expect(health).toMatchObject({ state: "running", recycles: 1, timeouts: 1 });
    expect(await api.call(M("counter.bump")), "the instance's memory went with it").toEqual({ counter: 1 });
    expect((await api.call(M("state"))).started_ms).toBeGreaterThan(before);
    expect((await api.call(M("notes.list"))).notes, "the key-value store stayed").toHaveLength(1);
    expect((await runtime(api)).health.subscriptions, "init subscribed again").toBe(1);
  });

  test("a crash is caught: rebuilt once, and a second one sets the application aside", TAGS, async () => {
    const first = await api.refused(M("crash"));
    expect(first?.data).toMatchObject({ failure: "faulted" });
    expect(first?.message).toMatch(/wasm trap|unreachable/);
    expect((await runtime(api)).health).toMatchObject({ state: "running", rebuilds: 1 });
    expect((await api.call(M("notes.list"))).notes).toHaveLength(1);

    const second = await api.refused(M("crash"));
    expect(second?.data).toMatchObject({ failure: "faulted" });
    const rt = await runtime(api);
    expect(rt.state).toBe("faulted");
    expect(rt.reason).toContain("crashed again after being rebuilt");
    const after = await api.refused(M("notes.list"));
    expect(after?.data).toMatchObject({ failure: "unavailable" });
    expect(after?.message).toContain("disabled");

    // Enabling it again starts it again, from nothing but its storage.
    const on = await api.call("harnix.apps.enable", { id: SHOWCASE_ID });
    expect(on.runtime).toMatchObject({ state: "running" });
    expect((await runtime(api)).health).toMatchObject({ rebuilds: 0, recycles: 0 });
    expect((await api.call(M("notes.list"))).notes).toHaveLength(1);
  });

  test("disabled: its names are gone, a queued call is told why, its subscribers are told it ended", TAGS, async () => {
    await api.call("harnix.subscribe", { topic: M("notes") });
    // One call holding the backend, one queued behind it, then the switch.
    const spinning = api.refused(M("spin"));
    await sleep(300);
    const queued = api.refused(M("notes.list"));
    await sleep(200);
    await api.call("harnix.apps.disable", { id: SHOWCASE_ID });
    const waiting = await queued;
    expect(waiting?.code).toBe(INTERNAL_ERROR);
    expect(waiting?.message).toContain("stopped");
    expect((await spinning)?.data?.failure).toBe("timed_out");

    const ended = await api.waitFor((f) => f.method === "harnix.topic_ended" && f.params.topic === M("notes"), { describe: "the topic ended" });
    expect(ended.params.reason).toContain("withdrawn");
    for (const method of ["notes.list", "notes.add", "state"]) {
      expect((await api.refused(M(method), { text: "x" }))?.code, method).toBe(METHOD_NOT_FOUND);
    }
    expect((await api.call("harnix.reachable")).methods).not.toContain(M("notes.list"));
    expect(JSON.stringify(await api.call("harnix.spaces.list"))).not.toContain(`"${SHOWCASE_ID}"`);
    expect((await api.call("harnix.apps.list")).registered).toEqual([]);
    expect(await api.refused("harnix.subscribe", { topic: M("notes") }), "its topic is unknown now").not.toBeNull();

    // And back.
    await api.call("harnix.apps.enable", { id: SHOWCASE_ID });
    expect((await api.call(M("notes.list"))).notes).toHaveLength(1);
  });

  test("uninstalled: its names and its storage go with it", TAGS, async () => {
    await api.call("harnix.apps.uninstall", { id: SHOWCASE_ID });
    expect((await api.refused(M("notes.list")))?.code).toBe(METHOD_NOT_FOUND);
    await api.call("harnix.apps.installFile", { path: showcasePackage().path });
    expect((await api.call(M("notes.list"))).notes, "a fresh install starts with an empty board").toEqual([]);
    await api.call("harnix.apps.uninstall", { id: SHOWCASE_ID });
  });
});
