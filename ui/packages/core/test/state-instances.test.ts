/**
 * Design §8.5, point 2: a session is `(instance, session)`. Two instances may each have a session
 * `S1`; a controller for one never takes the other's events, and its calls name its own instance
 * and space.
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

/** Just enough of `HarnixClient` for a controller: calls are recorded, notifications pushed. */
class FakeClient {
  status = "open";
  calls: { method: string; params: any }[] = [];
  private listeners = new Set<(m: string, p: Record<string, unknown>) => void>();
  onNotify(fn: (m: string, p: Record<string, unknown>) => void) {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
  onStatus() {
    return () => {};
  }
  async call(method: string, params: any) {
    this.calls.push({ method, params });
    if (method === "harnix.attach") {
      return { instance: params.instance, session: { session_id: params.session_id }, history_total: 0, replay: [], truncated: false, pending_prompts: [], running_turn: null, queue: [], seq: 0 };
    }
    if (method === "harnix.send") return { turn_id: "t" };
    return {};
  }
  notify() {}
  push(method: string, params: Record<string, unknown>) {
    for (const fn of [...this.listeners]) fn(method, params);
  }
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("the same session id on two instances", () => {
  it("routes each instance's events to its own controller, and calls name the instance and the space", async () => {
    const client = new FakeClient() as any;
    const a = new SessionController({ client, instance: "alpha", space: "greeter", sessionId: "S1", strings });
    const b = new SessionController({ client, instance: "beta", space: "notes", sessionId: "S1", strings });
    a.start();
    b.start();
    await flush();
    await flush();

    client.push("harnix.event", { instance: "beta", session_id: "S1", seq: 1, turn_id: "t", event: { kind: "user_message", text: "for beta" } });
    client.push("harnix.event", { instance: "alpha", session_id: "S1", seq: 1, turn_id: "t", event: { kind: "user_message", text: "for alpha" } });
    const text = (c: SessionController) =>
      c.getSnapshot().blocks.filter((x) => x.kind === "user").map((x) => (x as { text: string }).text);
    expect(text(a)).toEqual(["for alpha"]);
    expect(text(b)).toEqual(["for beta"]);

    await a.send("hello");
    const sent = client.calls.filter((c: any) => c.method === "harnix.send");
    expect(sent.at(-1).params).toMatchObject({ instance: "alpha", session_id: "S1", space: "greeter" });
    const attached = client.calls.filter((c: any) => c.method === "harnix.attach").map((c: any) => c.params);
    expect(attached).toEqual(
      expect.arrayContaining([
        { instance: "alpha", session_id: "S1", space: "greeter" },
        { instance: "beta", session_id: "S1", space: "notes" },
      ]),
    );
    a.stop();
    b.stop();
  });
});

describe("send while a turn runs", () => {
  it("queues by default, and steers when asked to", async () => {
    const client = new FakeClient() as any;
    const c = new SessionController({ client, instance: "alpha", space: "greeter", sessionId: "S1", strings });
    c.start();
    await flush();
    await flush();
    await c.send("first");
    await c.send("into this turn", [], { onBusy: "steer" });
    const sent = client.calls.filter((x: any) => x.method === "harnix.send").map((x: any) => x.params.on_busy);
    expect(sent).toEqual(["queue", "steer"]);
    c.stop();
  });
});
