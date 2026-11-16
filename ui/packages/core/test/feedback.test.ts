/**
 * Feedback on an answer: read from the hub's entries in one place, the last word on a turn
 * standing; the answer a turn gave knows its turn, read back or live; the controller files it and
 * has it at once, and the push of the same entry is not a second one.
 */

import { describe, expect, it } from "vitest";
import { FEEDBACK_EVENT, FEEDBACK_NS, isFeedbackEntry, parseFeedback, standingFeedback } from "../src/protocol/index.ts";
import { SessionController, type SessionStrings } from "../src/state/index.ts";

const rated = (id: string, turn: string, rating: string | null, extra: Record<string, unknown> = {}) => ({
  id,
  ns: FEEDBACK_NS,
  event: FEEDBACK_EVENT,
  payload: { turn_id: turn, rating, at: 1, by: "device:d", ...extra },
  at: 2,
});

describe("feedback", () => {
  it("reads the hub's entry, and nothing else", () => {
    expect(parseFeedback(rated("e", "t1", "down", { category: "slow", note: "late" }))).toEqual({
      turn_id: "t1",
      rating: "down",
      category: "slow",
      note: "late",
      at: 1,
      by: "device:d",
    });
    // Taken back: what came with it does not stand either; a category it does not know is dropped.
    expect(parseFeedback(rated("e", "t1", null, { category: "slow" }))).toEqual({ turn_id: "t1", rating: null, at: 1, by: "device:d" });
    expect(parseFeedback(rated("e", "t1", "up", { category: "boring" }))?.category).toBeUndefined();
    expect(parseFeedback({ ns: "harnix.changes", event: "turn", payload: { turn_id: "t1" } })).toBeNull();
    expect(parseFeedback({ ns: FEEDBACK_NS, event: FEEDBACK_EVENT, payload: {} })).toBeNull();
    expect(isFeedbackEntry(rated("e", "t", "up"))).toBe(true);
    expect(isFeedbackEntry({ ns: "probe", event: "rating" })).toBe(false);
  });

  it("the last word on a turn stands; one taken back leaves none; one about a single call is not the turn's", () => {
    const standing = standingFeedback([
      rated("e1", "t1", "up"),
      rated("e2", "t2", "down"),
      rated("e3", "t1", "down", { note: "changed my mind" }),
      rated("e4", "t2", null),
      rated("e5", "t3", "up", { call_id: "c1" }),
    ]);
    expect([...standing.keys()]).toEqual(["t1"]);
    expect(standing.get("t1")).toMatchObject({ rating: "down", note: "changed my mind" });
  });
});

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

class FakeClient {
  status = "open";
  calls: { method: string; params: any }[] = [];
  private listeners = new Set<(m: string, p: Record<string, unknown>) => void>();
  onNotification(fn: (m: string, p: Record<string, unknown>) => void) {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
  onStatus() {
    return () => {};
  }
  async call(method: string, params: any) {
    this.calls.push({ method, params });
    if (method === "harnix.session.attach") {
      return { instance: "main", session: { session_id: params.session_id }, history_total: 2, replay: [], truncated: false, pending_prompts: [], running_turn: null, queue: [], seq: 0 };
    }
    if (method === "engine.session.history") {
      return {
        total: 2,
        messages: [
          { seq: 0, role: "user", origin: "user", turn_id: "t1", content: [{ type: "text", text: "hi" }] },
          { seq: 1, role: "assistant", origin: "assistant", turn_id: "t1", content: [{ type: "text", text: "hello" }] },
        ].slice(params.offset, params.offset + params.limit),
      };
    }
    if (method === "harnix.session.feedback") {
      const payload = { turn_id: params.turn_id, rating: params.rating, at: 1, by: "device:d" };
      return { feedback: payload, entry: { id: "f1", ns: FEEDBACK_NS, event: FEEDBACK_EVENT, payload, at: 2 } };
    }
    return {};
  }
  notify() {}
  push(method: string, params: Record<string, unknown>) {
    for (const fn of [...this.listeners]) fn(method, params);
  }
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("the controller", () => {
  it("knows each answer's turn, read back or live; files feedback and has it at once, once", async () => {
    const client = new FakeClient() as any;
    const c = new SessionController({ client, instance: "main", space: "sessions", sessionId: "S1", strings });
    c.start();
    for (let i = 0; i < 5; i++) await flush();
    const answers = () => c.getSnapshot().blocks.filter((b: any) => b.kind === "assistant").map((b: any) => b.turnId);
    expect(answers()).toEqual(["t1"]);

    client.push("harnix.event", { instance: "main", session_id: "S1", seq: 1, turn_id: "t2", event: { kind: "user_message", text: "again" } });
    client.push("harnix.event", { instance: "main", session_id: "S1", seq: 2, turn_id: "t2", event: { kind: "text_delta", text: "hello again" } });
    expect(answers()).toEqual(["t1", "t2"]);

    expect(await c.feedback("t1", "down", { category: "slow", note: "  " })).toBe(true);
    const call = client.calls.find((x: any) => x.method === "harnix.session.feedback");
    // A blank note is not sent.
    expect(call.params).toEqual({ instance: "main", session_id: "S1", space: "sessions", turn_id: "t1", rating: "down", category: "slow" });
    const entries = () => c.getSnapshot().blocks.filter((b: any) => b.kind === "custom" && b.type === "entry").map((b: any) => b.data.id);
    expect(entries()).toEqual(["f1"]);
    client.push("harnix.event", { instance: "main", session_id: "S1", seq: 3, turn_id: "", event: { kind: "extension_entry", entry: { id: "f1", ns: FEEDBACK_NS, event: FEEDBACK_EVENT, payload: {}, at: 2 } } });
    expect(entries()).toEqual(["f1"]);

    // Taking it back sends nothing but the turn.
    await c.feedback("t1", null, { category: "slow", note: "x" });
    expect(client.calls.filter((x: any) => x.method === "harnix.session.feedback")[1].params).toEqual({
      instance: "main",
      session_id: "S1",
      space: "sessions",
      turn_id: "t1",
      rating: null,
    });
    c.stop();
  });
});
