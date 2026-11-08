/**
 * A send's answer and its turn's events can arrive in either order: over split streams the events
 * come on their own stream, and a short turn can be over before the answer to `harnix.send` is back.
 * A turn that has settled stays settled however late that answer is.
 */

import { describe, expect, it } from "vitest";
import { SessionController, type SessionStrings } from "../src/state/index.ts";

const strings: SessionStrings = {
  openFailed: (r) => r,
  replayTruncated: "",
  olderHidden: () => "",
  result: "",
  image: "",
  stopReason: (r) => r,
  interrupted: "interrupted",
  turnFailed: (r) => r,
  sendFailed: (r) => r,
  interruptFailed: (r) => r,
  answerFailed: (r) => r,
};

/** A client whose answer to `harnix.send` waits until the test lets it go. */
class FakeClient {
  status = "open";
  private listeners = new Set<(m: string, p: Record<string, unknown>) => void>();
  release: () => void = () => {};
  onNotify(fn: (m: string, p: Record<string, unknown>) => void) {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
  onStatus() {
    return () => {};
  }
  async call(method: string, params: any) {
    if (method === "harnix.attach") {
      return { instance: params.instance, session: { session_id: params.session_id }, history_total: 0, replay: [], truncated: false, pending_prompts: [], running_turn: null, queue: [], seq: 0 };
    }
    if (method === "session.history") return { total: 0, messages: [] };
    if (method === "session.entries") return { entries: [] };
    if (method === "harnix.send") return new Promise((resolve) => (this.release = () => resolve({ turn_id: "t1" })));
    return {};
  }
  notify() {}
  push(method: string, params: Record<string, unknown>) {
    for (const fn of [...this.listeners]) fn(method, params);
  }
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("send", () => {
  it("an answer that arrives after its turn settled does not start the turn again", async () => {
    const client = new FakeClient() as any;
    const c = new SessionController({ client, sessionId: "S1", strings });
    c.start();
    for (let i = 0; i < 5; i++) await flush();

    const sent = c.send("hello");
    const event = (seq: number, kind: string) =>
      client.push("harnix.event", { instance: "system", session_id: "S1", seq, turn_id: "t1", event: { kind, text: "hello" } });
    event(1, "user_message");
    expect(c.getSnapshot().turn).toEqual({ turnId: "t1" });
    event(2, "turn_complete");
    client.push("harnix.turn_settled", { instance: "system", session_id: "S1", turn_id: "t1", result: {} });
    expect(c.getSnapshot().turn).toBeNull();

    client.release();
    await sent;
    expect(c.getSnapshot().turn).toBeNull();
    c.stop();
  });

  it("an answer that arrives first marks its turn running", async () => {
    const client = new FakeClient() as any;
    const c = new SessionController({ client, sessionId: "S1", strings });
    c.start();
    for (let i = 0; i < 5; i++) await flush();
    const sent = c.send("hello");
    client.release();
    await sent;
    expect(c.getSnapshot().turn).toEqual({ turnId: "t1" });
    c.stop();
  });
});
