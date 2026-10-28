/**
 * What a module contributes natively, on either carrier (ipdrun-10), and the prompt middleware it
 * can give through it (ipdrun-13).
 *
 * `harnix-testapp`'s `fleet` has two templates with one `native` closure — a scene with a tool of
 * its own, a block at the end of the system prompt, a middleware that refuses `blocked-word` — one
 * in process, one spawned. The spawned daemon builds the closure again in its own process, from
 * `--template`. `probe` gives the same middleware to every session of `system`.
 */

import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { type Backend, type TestClient, binary, connect, expect, test } from "@harnix/testkit";

import { startTestApp } from "./testapp";

const TAGS = { tag: ["@feature:instances", "@layer:api"] };
const TURN_TAGS = { tag: ["@feature:sessions", "@layer:api"] };

/** What `crates/testapp`'s native contributions answer and mark. */
const NATIVE_ANSWER = "probe-native-answer-7f3a";
const NATIVE_MARKER = "probe-native-marker-51c2";

let backend: Backend;
let api: TestClient;

test.describe.configure({ mode: "serial" });

test.beforeAll(async () => {
  backend = await startTestApp();
});
test.afterAll(async () => {
  await backend?.dispose();
});
test.beforeEach(async () => {
  api = await connect(backend);
});
test.afterEach(() => api?.close());

type Session = { instance: string; session_id: string };

/** A session in the native scene, on an instance of `template` named `name`. */
async function nativeSession(template: string, name: string, recording: string): Promise<Session> {
  const s = await api.call("app.fleet.native_session", {
    template,
    name,
    scene: "probe-native",
    project_root: api.project,
    recording,
  });
  await api.call("harnix.attach", s);
  return s;
}

async function settle(s: Session, message: string): Promise<any> {
  const sent = await api.call("harnix.send", { ...s, message });
  return api.waitFor(
    (f) => f.method === "harnix.turn_settled" && f.params.session_id === s.session_id
      && (!sent.turn_id || f.params.turn_id === sent.turn_id),
    { timeout: 60_000, describe: `the turn "${message}" to settle` },
  );
}

for (const [carrier, template] of [["spawn", "fleet.native-spawn"], ["in_process", "fleet.native-local"]]) {
  test(`${carrier}: the template's native tool answers and its block is in the system prompt (ipdrun-10)`, TAGS, async () => {
    const s = await nativeSession(template, `native-${carrier}`, "calls-a-native-tool");
    const { instances } = await api.call("harnix.instances.list");
    expect(instances.find((i: any) => i.id === s.instance)?.carrier).toBe(carrier);

    const settled = await settle(s, "Call the ProbeFixed tool");
    expect(settled.params.error, JSON.stringify(settled.params.error)).toBeUndefined();
    const uses = api.eventsOfKind("tool_use", s.session_id);
    expect(uses.map((u: any) => u.name)).toContain("ProbeFixed");
    const results = JSON.stringify(api.eventsOfKind("tool_result", s.session_id));
    expect(results).toContain(NATIVE_ANSWER);

    const context = await api.call("app.fleet.context", s);
    expect(JSON.stringify(context)).toContain(NATIVE_MARKER);
  });

  test(`${carrier}: the template's prompt middleware refuses a turn before the model (ipdrun-13)`, TURN_TAGS, async () => {
    const s = await nativeSession(template, `native-${carrier}`, "says-ready");
    const refused = await settle(s, "a message with blocked-word in it");
    expect(refused.params.outcome?.stop_reason).toBe("stopped_by_hook");
    expect(refused.params.outcome?.message).toContain("blocked-word");
    const complete = api.eventsOfKind("turn_complete", s.session_id).at(-1);
    expect(complete?.api_calls, "the model was called").toBe(0);

    const next = await settle(s, "Say exactly: fixture ready");
    expect(next.params.outcome?.stop_reason).toBe("end_turn");
    expect(api.text(s.session_id)).toMatch(/fixture ready/i);
  });
}

test("system: a module's prompt middleware refuses a turn, and the next one runs (ipdrun-13)", TURN_TAGS, async () => {
  const created = await api.call("session.create", {
    project_root: api.project,
    options: { recorder: { name: "says-ready" } },
  });
  const s = { instance: "system", session_id: created.session_id };
  await api.call("harnix.attach", s);
  const refused = await settle(s, "blocked-word, please");
  expect(refused.params.outcome?.stop_reason).toBe("stopped_by_hook");
  expect(api.eventsOfKind("turn_complete", s.session_id).at(-1)?.api_calls).toBe(0);
  const next = await settle(s, "Say exactly: fixture ready");
  expect(next.params.outcome?.stop_reason).toBe("end_turn");
});

test("system: a steered message the middleware refuses arrives as user_message_refused, and stays out of the transcript (ipdrun-13)", TURN_TAGS, async () => {
  const created = await api.call("session.create", {
    project_root: api.project,
    options: { recorder: { name: "runs-a-while" } },
  });
  const s = { instance: "system", session_id: created.session_id };
  await api.call("harnix.attach", s);
  const sent = await api.call("harnix.send", { ...s, message: "Wait four seconds" });
  const ask = await api.waitFor(
    (f) => f.method === "harnix.event" && f.params.session_id === s.session_id && f.params.event.kind === "prompt",
    { timeout: 30_000, describe: "the permission ask for the Bash call" },
  );
  await api.call("session.respondToPrompt", { ...s, prompt_id: ask.params.event.prompt_id, decision: { type: "permit" } });
  const steered = await api.call("harnix.send", { ...s, message: "steered blocked-word", on_busy: "steer" });
  expect(steered.delivery).toBe("steer");
  const frame = await api.waitFor(
    (f) => f.method === "harnix.event" && f.params.session_id === s.session_id
      && f.params.event.kind === "user_message_refused",
    { timeout: 30_000, describe: "the steered message to be refused" },
  );
  expect(frame.params.event.content).toContain("blocked-word");
  expect(frame.params.event.message).toContain("blocked-word");
  expect(frame.params.event.message_id).toBeTruthy();
  const settled = await api.waitFor(
    (f) => f.method === "harnix.turn_settled" && f.params.turn_id === sent.turn_id,
    { timeout: 60_000, describe: "the turn to settle" },
  );
  expect(settled.params.error).toBeUndefined();
  const history = await api.call("session.history", s);
  expect(JSON.stringify(history.messages)).not.toContain("steered blocked-word");
});

test("a daemon asked for a template this binary does not declare refuses to start, saying so (ipdrun-10)", TAGS, () => {
  const dir = mkdtempSync(join(tmpdir(), "hx-tpl-"));
  try {
    const socket = join(dir, "d.sock");
    const run = spawnSync(
      binary("harnix-testapp"),
      ["daemon", "--data-root", join(dir, "data"), "--socket", socket, "--template", "fleet.nowhere"],
      { encoding: "utf8", timeout: 30_000 },
    );
    expect(run.status).toBe(78);
    expect(run.stderr).toContain("fleet.nowhere");
    expect(readFileSync(`${socket}.refused`, "utf8")).toContain("no template `fleet.nowhere`");
    expect(existsSync(socket), "it listened anyway").toBe(false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
