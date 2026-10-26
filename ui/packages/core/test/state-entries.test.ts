/**
 * Entries (design §7.6) go back between the messages they were written between, and each is one
 * block however it arrives — read with the history, or pushed while the page watches.
 */

import { describe, expect, it } from "vitest";
import { SessionController, type SessionStrings } from "../src/state/index.ts";

const entry = (id: string, at: number) => ({ id, ns: "app.greeter", event: "job_done", payload: { id }, at });

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
      return { instance: params.instance, session: { session_id: params.session_id }, history_total: 2, replay: [], truncated: false, pending_prompts: [], running_turn: null, queue: [], seq: 0 };
    }
    if (method === "session.history") {
      return {
        total: 2,
        messages: [
          { seq: 0, role: "user", origin: "user", content: [{ type: "text", text: "count, please" }] },
          { seq: 1, role: "assistant", origin: "assistant", content: [{ type: "text", text: "counting" }] },
        ].slice(params.offset, params.offset + params.limit),
      };
    }
    if (method === "session.entries") {
      return { entries: [entry("e1", 1), entry("e2", 2)] };
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

describe("entries", () => {
  it("are placed by `at` among the history's messages, and a pushed one already there is not drawn twice", async () => {
    const client = new FakeClient() as any;
    const c = new SessionController({ client, instance: "space-greeter", space: "greeter", sessionId: "S1", strings });
    c.start();
    for (let i = 0; i < 5; i++) await flush();
    const order = () =>
      c.getSnapshot().blocks.map((b: any) => (b.kind === "custom" ? `entry:${b.data.id}` : b.kind));
    expect(order()).toEqual(["user", "entry:e1", "assistant", "entry:e2"]);

    client.push("harnix.event", { instance: "space-greeter", session_id: "S1", seq: 1, turn_id: "", event: { kind: "extension_entry", entry: entry("e2", 2) } });
    client.push("harnix.event", { instance: "space-greeter", session_id: "S1", seq: 2, turn_id: "", event: { kind: "extension_entry", entry: entry("e3", 2) } });
    expect(order()).toEqual(["user", "entry:e1", "assistant", "entry:e2", "entry:e3"]);
    c.stop();
  });
});
