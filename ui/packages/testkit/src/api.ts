/**
 * The API-level test client: `@harnix/core/client`, plus what a test needs on
 * top — every frame kept, waiting on a predicate, coverage counted.
 *
 * Built on the client under test rather than beside it, so the API suites
 * exercise the same handshake, topology fallback and error shapes the
 * interface does (design §13.2).
 */

import { HarnixClient } from "@harnix/core/client";
import type { Topology } from "@harnix/core/protocol";

import { FetchEventSource } from "./eventsource.ts";
import type { Backend } from "./launcher.ts";
import { sleep } from "./env.ts";

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

export async function connect(
  backend: Backend,
  { topology = "single_duplex" }: { topology?: Topology } = {},
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
    // Both faces attached before anything is sent.
    await Promise.all(sources.map((s) => s.opened));
    await sleep(50);
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
 * A test that could not reach what it was trying to reach — distinct from a
 * pass. A permission test whose model never reached for a tool has not shown
 * that permissions work; reporting it green would be the test lying, and red
 * would train people to ignore red. The suite adapter turns it into a skip
 * with the reason attached.
 */
export class Inconclusive extends Error {}
export const inconclusive = (reason: string): never => {
  throw new Inconclusive(reason);
};
