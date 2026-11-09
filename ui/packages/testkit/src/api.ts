/**
 * The API-level test client: `@harnix/core/client`, plus what a test needs on
 * top — every frame kept, waiting on a predicate, coverage counted.
 *
 * Built on the client under test rather than beside it, so the API suites
 * exercise the same handshake, topology fallback and error shapes the
 * interface does.
 */

import { HarnixClient } from "@harnix/core/client";
import type { Topology } from "@harnix/core/protocol";

import { FetchEventSource } from "./eventsource.ts";
import type { Backend } from "./launcher.ts";
import { sleep, topology as topologyOfRun } from "./env.ts";

/** One server-initiated frame, as a suite sees it. */
export interface Frame {
  method: string;
  params: any;
}

/* ── coverage ─────────────────────────────────────────────────────────── */
//
// Against what the backend says a client may call (`harnix.reachable`), not a
// list kept by hand: a hand-kept list drifts the moment a method is added, and
// it drifts in the direction that makes coverage look better.

const exercised = new Set<string>();
const refuted = new Set<string>();
const codesSeen = new Map<number, string>();

export function coverage() {
  return {
    called: [...exercised].sort(),
    errored: [...refuted].sort(),
    codes: [...codesSeen.entries()].sort((a, b) => b[0] - a[0]),
  };
}

function noteError(method: string, error: unknown): void {
  refuted.add(method);
  const code = (error as { code?: unknown } | null)?.code;
  if (typeof code === "number" && !codesSeen.has(code)) codesSeen.set(code, method);
}

/* ── the client ───────────────────────────────────────────────────────── */

export interface TestClient {
  topology: Topology;
  /** What the handshake agreed. */
  handshake: any;
  /** Every server-initiated frame, in arrival order. */
  events: Frame[];
  backend: Backend;
  /** The project this backend's sessions run in. */
  project: string;
  call(method: string, params?: unknown): Promise<any>;
  /** A call that is expected to be refused: the error, or null if it answered. */
  refused(method: string, params?: unknown): Promise<any>;
  /** Wait for a frame, or fail saying what was waited for. */
  waitFor(match: (frame: Frame) => boolean, options?: { timeout?: number; describe?: string }): Promise<Frame>;
  /** Session events of one kind, in arrival order. */
  eventsOfKind(kind: string, sessionId?: string): any[];
  /** Everything the model streamed, joined. */
  text(sessionId?: string): string;
  close(): void;
}

/** Connect to `backend`, speaking `topology` — this run's (`HARNIX_TOPOLOGY`) unless told otherwise. */
export async function connect(
  backend: Backend,
  { topology = topologyOfRun() }: { topology?: Topology } = {},
): Promise<TestClient> {
  const sources: FetchEventSource[] = [];
  const client = new HarnixClient({
    token: backend.token,
    baseUrl: backend.url,
    speaks: [topology],
    EventSource: class extends FetchEventSource {
      constructor(url: string) {
        super(url);
        sources.push(this);
      }
    },
  });

  const events: Frame[] = [];
  const waiters: { match: (f: Frame) => boolean; resolve: (f: Frame) => void }[] = [];
  client.onNotify((method, params) => {
    const frame = { method, params };
    events.push(frame);
    for (let i = waiters.length - 1; i >= 0; i -= 1) {
      if (waiters[i]!.match(frame)) waiters.splice(i, 1)[0]!.resolve(frame);
    }
  });

  client.connect();
  exercised.add("harnix.handshake");
  const handshake = await client.ready();
  if (sources.length) {
    // Both faces attached before anything is sent: the server answers a stream's request only
    // once it has registered the reader, so there is nothing to wait out after that.
    await Promise.all(sources.map((s) => s.opened));
    const refused = sources.filter((s) => !s.attached);
    if (refused.length) {
      client.close();
      throw new Error(`the server did not take ${refused.length} of the event streams: ${refused.map((s) => s.refusal).join(", ")}`);
    }
  }

  const call = async (method: string, params?: unknown) => {
    exercised.add(method);
    try {
      return await client.call(method, params ?? {});
    } catch (e) {
      noteError(method, e);
      throw e;
    }
  };

  const self: TestClient = {
    topology,
    handshake,
    events,
    backend,
    project: backend.project,
    call,
    async refused(method, params) {
      try {
        await call(method, params);
        return null;
      } catch (e) {
        return e;
      }
    },
    async waitFor(match, { timeout = 20_000, describe = "a frame" } = {}) {
      const already = events.find(match);
      if (already) return already;
      return new Promise((resolve, reject) => {
        const waiter = { match, resolve };
        waiters.push(waiter);
        setTimeout(() => {
          const at = waiters.indexOf(waiter);
          if (at !== -1) {
            waiters.splice(at, 1);
            reject(new Error(`timed out waiting for ${describe}`));
          }
        }, timeout);
      });
    },
    eventsOfKind(kind, sessionId) {
      return events
        .filter((f) => f.method === "harnix.event")
        .filter((f) => !sessionId || f.params.session_id === sessionId)
        .filter((f) => f.params.event?.kind === kind)
        .map((f) => f.params.event);
    },
    text(sessionId) {
      return self.eventsOfKind("text_delta", sessionId)
        .map((e) => e.text)
        .join("");
    },
    close() {
      client.close();
    },
  };
  return self;
}

/**
 * Stop whatever this session is doing, then remove it.
 *
 * Deleting a session with a turn still running leaves the engine finishing
 * work for something that no longer exists, and the next test pays for it.
 * So: interrupt, **wait for the settlement**, then delete. Never throws —
 * cleanup that can fail a test hides the failure that mattered.
 */
export async function finish(client: TestClient, sessionId: string): Promise<void> {
  try {
    const info = await client.call("session.get", { session_id: sessionId }).catch(() => null);
    const running = info?.turn_state && info.turn_state !== "idle";
    if (running) {
      await client.call("session.interrupt", { session_id: sessionId }).catch(() => {});
      await client
        .waitFor((f) => f.method === "harnix.turn_settled" && f.params.session_id === sessionId, {
          timeout: 20_000,
          describe: "the interrupted turn to settle during cleanup",
        })
        .catch(() => {});
    }
    await client.call("session.delete", { session_id: sessionId }).catch(() => {});
  } catch {
    /* cleanup never fails a test */
  }
}

/**
 * Every session on `client`'s backend, of every instance and space, stopped and removed — as
 * `finish` does for one. For tests that share a backend: what one test leaves behind (a turn
 * still on an ask, a list one row longer) is not the next test's to count. Never throws.
 */
export async function dropSessions(client: TestClient): Promise<void> {
  try {
    const { instances } = await client.call("harnix.instances.list");
    for (const { id: instance } of instances ?? []) {
      const { sessions } = await client.call("harnix.instances.sessions", { instance }).catch(() => ({ sessions: [] }));
      for (const { session_id, running } of sessions ?? []) {
        const ref = { instance, session_id };
        if (running) {
          // This client watches none of them, so it is asked rather than told when the turn settles.
          await client.call("session.interrupt", ref).catch(() => {});
          const still = async () =>
            ((await client.call("harnix.instances.sessions", { instance }).catch(() => ({ sessions: [] }))).sessions ?? [])
              .some((s: any) => s.session_id === session_id && s.running);
          for (const deadline = Date.now() + 20_000; Date.now() < deadline && (await still()); ) await sleep(100);
        }
        await client.call("session.delete", ref).catch(() => {});
      }
    }
  } catch {
    /* cleanup never fails a test */
  }
}

/** Asks some turn already answered, per client: a later turn does not answer one again. */
const answeredBy = new WeakMap<TestClient, Set<string>>();

/**
 * One turn: send `message`, answer every permission ask it raises with `decision`, and wait for
 * the turn to settle.
 *
 * The asks answered are the ones this client is shown after the send, a sub-agent's included —
 * those come in under the sub-agent's own session. What it returns: the settlement frame and the
 * `harnix.event` params of every ask it answered, in order.
 */
export async function runTurn(
  api: TestClient,
  session: { session_id: string; instance?: string },
  message: string,
  { decision = "permit", timeout = 90_000 }: { decision?: string; timeout?: number } = {},
): Promise<{ settled: Frame; asks: any[] }> {
  const answered = answeredBy.get(api) ?? new Set<string>();
  answeredBy.set(api, answered);
  const from = api.events.length;
  const sent = await api.call("harnix.send", { ...session, message });
  const asks: any[] = [];
  const deadline = Date.now() + timeout;
  for (;;) {
    const settled = api.events.find(
      (f) => f.method === "harnix.turn_settled" && f.params.session_id === session.session_id
        && (!sent?.turn_id || f.params.turn_id === sent.turn_id),
    );
    if (settled) return { settled, asks };
    for (const f of api.events.slice(from)) {
      if (f.method !== "harnix.event" || f.params.event?.kind !== "prompt") continue;
      const id = f.params.event.prompt_id;
      if (answered.has(id)) continue;
      answered.add(id);
      asks.push(f.params);
      await api.call("session.respondToPrompt", {
        instance: f.params.instance ?? session.instance,
        session_id: f.params.session_id,
        prompt_id: id,
        decision: decision === "deny" ? { type: "deny", reason: "not now" } : { type: decision },
      });
    }
    if (Date.now() > deadline) {
      throw new Error(`the turn "${message}" did not settle in ${timeout / 1000}s; asks so far: ${JSON.stringify(asks)}`);
    }
    await sleep(50);
  }
}

/** A suite's own check: fails with `message` unless `cond` holds, and narrows `cond` after it. */
export function assert(cond: unknown, message: string): asserts cond {
  if (!cond) throw new Error(message);
}

/**
 * A test that could not reach what it was trying to reach — distinct from a
 * pass. A permission test whose model never reached for a tool has not shown
 * that permissions work; reporting it green would be the test lying, and red
 * would train people to ignore red. The suite adapter turns it into a skip
 * with the reason attached.
 */
export class Inconclusive extends Error {}
export function inconclusive(reason: string): never {
  throw new Inconclusive(reason);
}
