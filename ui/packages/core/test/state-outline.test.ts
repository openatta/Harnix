/**
 * What the turn rail stands on: the person's message that starts a turn knows its turn (live and read back),
 * older history is read in on demand and put before what is shown, the outline is read again as a turn
 * settles, and merged with the turns shown.
 */

import { describe, expect, it } from "vitest";
import { OutlineController, SessionController, mergeTurns, previewText, shownTurns, type Block, type SessionStrings } from "../src/state/index.ts";

const strings: SessionStrings = {
  openFailed: (r) => `failed: ${r}`,
  replayTruncated: "",
  olderHidden: (n) => `hidden ${n}`,
  result: "",
  image: "",
  stopReason: (r) => r,
  interrupted: "interrupted",
  turnFailed: (r) => r,
  sendFailed: (r) => r,
  interruptFailed: (r) => r,
  answerFailed: (r) => r,
};

/** A transcript of `turns` turns, two messages each: turn `tN` asks `qN` and is answered `aN`. */
function transcript(turns: number) {
  return Array.from({ length: turns * 2 }, (_, seq) => {
    const n = Math.floor(seq / 2);
    return seq % 2 === 0
      ? { seq, role: "user", origin: "user", turn_id: `t${n}`, content: [{ type: "text", text: `q${n}` }] }
      : { seq, role: "assistant", origin: "assistant", turn_id: `t${n}`, content: [{ type: "text", text: `a${n}` }] };
  });
}

class FakeClient {
  status = "open";
  calls: { method: string; params: any }[] = [];
  outline: unknown = { turns: [] };
  private listeners = new Set<(m: string, p: Record<string, unknown>) => void>();
  constructor(private readonly log: ReturnType<typeof transcript>) {}
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
      return { instance: "main", session: { session_id: params.session_id }, history_total: this.log.length, replay: [], truncated: false, pending_prompts: [], running_turn: null, queue: [], seq: 0 };
    }
    if (method === "engine.session.history") return { total: this.log.length, messages: this.log.slice(params.offset, params.offset + params.limit) };
    if (method === "engine.session.entries") return { entries: [{ id: "e-old", ns: "x", event: "y", payload: null, at: 1 }] };
    if (method === "harnix.session.outline") return this.outline;
    return {};
  }
  notify() {}
  push(method: string, params: Record<string, unknown>) {
    for (const fn of [...this.listeners]) fn(method, params);
  }
}

const flush = async () => {
  for (let i = 0; i < 10; i += 1) await new Promise((r) => setTimeout(r, 0));
};

function open(log: ReturnType<typeof transcript>) {
  const client = new FakeClient(log);
  const controller = new SessionController({ client: client as never, sessionId: "s", strings });
  controller.start();
  return { client, controller };
}

const users = (blocks: readonly Block[]) => blocks.filter((b): b is Extract<Block, { kind: "user" }> => b.kind === "user");

describe("the turns a session shows", () => {
  it("knows the turn and place of each person's message, read back or live", async () => {
    const { client, controller } = open(transcript(2));
    await flush();
    expect(users(controller.getSnapshot().blocks).map((b) => [b.text, b.turnId, b.seq])).toEqual([
      ["q0", "t0", 0],
      ["q1", "t1", 2],
    ]);
    client.push("harnix.event", { instance: "main", session_id: "s", seq: 1, turn_id: "live", event: { kind: "user_message", text: "now" } });
    const live = users(controller.getSnapshot().blocks).at(-1)!;
    expect([live.text, live.turnId, live.seq]).toEqual(["now", "live", undefined]);
  });

  it("reads older history in before what is shown, back to a turn, with its entries", async () => {
    // Five pages of two hundred are read at open: 1200 messages leave 200 unread.
    const { client, controller } = open(transcript(600));
    await flush();
    let state = controller.getSnapshot();
    expect(state.historyFrom).toBe(200);
    expect(state.blocks[0]).toMatchObject({ kind: "note", text: "hidden 200" });
    const before = state.blocks.length;

    expect(await controller.loadOlder(150)).toBe(true);
    state = controller.getSnapshot();
    expect(state.historyFrom).toBe(150);
    expect(state.blocks[0]).toMatchObject({ kind: "note", text: "hidden 150" });
    expect(state.blocks.filter((b) => b.kind === "note")).toHaveLength(1);
    expect(users(state.blocks)[0]).toMatchObject({ seq: 150, turnId: "t75" });
    expect(state.blocks.length).toBe(before + 50);
    expect(new Set(state.blocks.map((b) => b.key)).size).toBe(state.blocks.length);
    const reads = client.calls.filter((c) => c.method === "engine.session.history").at(-1)!.params;
    expect([reads.offset, reads.limit]).toEqual([150, 50]);

    // Back to the start: no note, and the entry written after the first message is in its place.
    expect(await controller.loadOlder(0)).toBe(true);
    state = controller.getSnapshot();
    expect(state.historyFrom).toBe(0);
    expect(state.blocks.some((b) => b.kind === "note")).toBe(false);
    expect(state.blocks.slice(0, 3).map((b) => b.kind)).toEqual(["user", "custom", "assistant"]);
    // What is shown already is not read again.
    const count = client.calls.length;
    expect(await controller.loadOlder(10)).toBe(true);
    expect(client.calls.length).toBe(count);
  });
});

describe("the outline", () => {
  it("is read at start and again when a turn of its session settles", async () => {
    const client = new FakeClient([]);
    client.outline = { instance: "main", session_id: "s", total: 2, turns: [{ turn_id: "t0", seq: 0, at: null, prompt: "q0", reply: "a0" }, { seq: "bad" }] };
    const outline = new OutlineController({ client: client as never, sessionId: "s", space: "acme" });
    outline.start();
    await flush();
    expect(outline.getSnapshot().turns.map((t) => t.turn_id)).toEqual(["t0"]);
    expect(client.calls.find((c) => c.method === "harnix.session.outline")!.params).toEqual({ instance: "main", session_id: "s", space: "acme" });
    const reads = () => client.calls.filter((c) => c.method === "harnix.session.outline").length;
    client.push("harnix.turn_settled", { instance: "main", session_id: "other", turn_id: "x" });
    client.push("harnix.turn_settled", { instance: "main", session_id: "s", turn_id: "t1" });
    await flush();
    expect(reads()).toBe(2);
    outline.stop();
    client.push("harnix.turn_settled", { instance: "main", session_id: "s", turn_id: "t2" });
    await flush();
    expect(reads()).toBe(2);
  });

  it("merges with the turns shown: the outline's, then the ones it does not have yet", () => {
    let key = 0;
    const block = (b: Record<string, unknown>) => ({ key: ++key, ...b }) as Block;
    const blocks = [
      block({ kind: "user", text: "q1", turnId: "t1", seq: 2 }),
      block({ kind: "assistant", text: "a1", streaming: false }),
      block({ kind: "user", text: "steered", turnId: "t1", steered: true }),
      block({ kind: "user", text: "now\nplease", turnId: "t2" }),
      block({ kind: "assistant", text: "work", streaming: true }),
    ];
    expect(shownTurns(blocks).map((t) => [t.block.text, t.reply])).toEqual([
      ["q1", "a1"],
      ["now\nplease", "work"],
    ]);
    const outline = [
      { turn_id: "t0", seq: 0, at: null, prompt: "q0", reply: "a0" },
      { turn_id: "t1", seq: 2, at: null, prompt: "q1", reply: "a1", marks: [{ kind: "station", label: "build" }] },
    ];
    expect(mergeTurns(outline, blocks).map((t) => [t.key, t.seq, t.prompt, t.reply, t.marks?.[0]?.label])).toEqual([
      ["t0", 0, "q0", "a0", undefined],
      ["t1", 2, "q1", "a1", "build"],
      ["t2", null, "now please", "work", undefined],
    ]);
    // No outline: the turns shown.
    expect(mergeTurns([], blocks).map((t) => t.key)).toEqual(["t1", "t2"]);
    expect(previewText("一二三四五", 3)).toBe("一二…");
  });
});
