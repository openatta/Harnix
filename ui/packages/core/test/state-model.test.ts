import { describe, expect, it } from "vitest";
import {
  applyEvent,
  applyHistoryMessage,
  applySettled,
  emptySession,
  toolResultText,
  usageTotals,
  type SessionState,
} from "../src/state/model.ts";

const opts = { promptTimeoutSecs: 300, now: 1_000 };
const texts = { interrupted: "已中断", failed: (e: string) => `失败：${e}` };

function run(events: Record<string, unknown>[], state: SessionState = { ...emptySession("S"), loading: false }) {
  return events.reduce((s, e) => applyEvent(s, e as never, opts), state);
}

describe("归约", () => {
  it("text_delta 拼进同一个流式块，其它块保持同一引用", () => {
    const s1 = run([{ kind: "user_message", text: "hi" }, { kind: "text_delta", text: "Hel" }]);
    const s2 = applyEvent(s1, { kind: "text_delta", text: "lo" }, opts);
    expect(s2.blocks).toHaveLength(2);
    expect(s2.blocks[1]).toMatchObject({ kind: "assistant", text: "Hello", streaming: true });
    expect(s2.blocks[0]).toBe(s1.blocks[0]);
    expect(s2.blocks[1]).not.toBe(s1.blocks[1]);
  });

  it("tool_use_start 先出工具行（运行中、还没有参数）；参数片段不画；tool_use 补上参数，不另起一行", () => {
    const s1 = run([
      { kind: "step_start", step: 0 },
      { kind: "text_delta", text: "写个文件" },
      { kind: "tool_use_start", id: "w1", name: "Write" },
      { kind: "tool_args_delta", id: "w1", partial_json: '{"file_path":"a.txt","con' },
    ]);
    expect(s1.blocks.map((b) => b.kind)).toEqual(["assistant", "tool"]);
    expect(s1.blocks[0]).toMatchObject({ streaming: false });
    expect(s1.blocks[1]).toMatchObject({ kind: "tool", id: "w1", name: "Write", status: "running", startedAt: 1_000 });
    expect((s1.blocks[1] as { input?: unknown }).input).toBeUndefined();
    const s2 = run([{ kind: "tool_use", id: "w1", name: "Write", input: { file_path: "a.txt", content: "x" } }], s1);
    expect(s2.blocks).toHaveLength(2);
    expect(s2.blocks[1]).toMatchObject({ id: "w1", status: "running", input: { file_path: "a.txt", content: "x" }, startedAt: 1_000 });
    const s3 = run([{ kind: "tool_result", id: "w1", content: "ok" }], s2);
    expect(s3.blocks[1]).toMatchObject({ status: "ok", result: "ok" });
  });

  it("step_end 结束这次调用的正文：下一次调用的正文另起一块（与 transcript 里两条 assistant 消息一致）", () => {
    const s = run([
      { kind: "turn_start" },
      { kind: "step_start", step: 0 },
      { kind: "text_delta", text: "一" },
      { kind: "step_end", step: 0, stop_reason: "max_tokens", seq: 3 },
      { kind: "step_start", step: 1 },
      { kind: "text_delta", text: "二" },
    ]);
    expect(s.blocks.map((b) => [b.kind, (b as { text?: string }).text])).toEqual([
      ["assistant", "一"],
      ["assistant", "二"],
    ]);
  });

  it("turn_complete 的 context 记下（上下文环取它，不取本轮用量的合计）；没调模型的轮不带 context，保留上一次的", () => {
    const s1 = run([
      {
        kind: "turn_complete",
        stop_reason: "end_turn",
        usage: { input_tokens: 90_000, output_tokens: 10 },
        last_call_usage: { input_tokens: 50_000 },
        context: { tokens: 50_000, compact_threshold: 150_000, window: 200_000 },
      },
    ]);
    expect(s1.context).toEqual({ tokens: 50_000, compact_threshold: 150_000, window: 200_000 });
    const s2 = run([{ kind: "turn_complete", stop_reason: "end_turn" }], s1);
    expect(s2.context).toBe(s1.context);
  });

  it("error 帧只收尾，不另报失败（失败由 nest.turn_settled 说）", () => {
    const s = run([{ kind: "text_delta", text: "半" }, { kind: "error", code: -32000, message: "boom" }]);
    expect(s.blocks).toHaveLength(1);
    expect(s.blocks[0]).toMatchObject({ streaming: false });
  });

  it("工具调用结束流式块；tool_result 按 id 配对，不按顺序", () => {
    const s = run([
      { kind: "text_delta", text: "先看看" },
      { kind: "tool_use", id: "a", name: "Read", input: { file_path: "x" } },
      { kind: "tool_use", id: "b", name: "Bash", input: { command: "ls" } },
      { kind: "tool_result", id: "b", name: "Bash", content: "out-b" },
      { kind: "tool_result", id: "a", name: "Read", content: [{ type: "text", text: "out-a" }], is_error: true },
    ]);
    expect(s.blocks[0]).toMatchObject({ kind: "assistant", streaming: false });
    expect(s.blocks[1]).toMatchObject({ kind: "tool", id: "a", status: "error", result: "out-a", error: true });
    expect(s.blocks[2]).toMatchObject({ kind: "tool", id: "b", status: "ok", result: "out-b" });
    expect(s.stream).toBeNull();
  });

  it("没有先行 tool_use 的结果自成一行", () => {
    const s = run([{ kind: "tool_result", id: "z", content: "orphan" }]);
    expect(s.blocks[0]).toMatchObject({ kind: "tool", id: "z", status: "ok", result: "orphan" });
  });

  it("权限提问成卡，按 prompt_id 去重，带截止时间", () => {
    const prompt = { kind: "prompt", prompt_id: "p1", tool_name: "Bash", message: "rm" };
    const s = run([prompt, prompt]);
    expect(s.blocks).toHaveLength(1);
    expect(s.blocks[0]).toMatchObject({ kind: "ask", deadline: 1_000 + 300_000 });
  });

  it("turn_complete 带用量；非 end_turn 的停止原因成一条提示", () => {
    const usage = { input_tokens: 10, output_tokens: 5, cache_read_input_tokens: 100, cache_creation_input_tokens: 1 };
    const s = run([{ kind: "text_delta", text: "x" }, { kind: "turn_complete", stop_reason: "max_tokens", usage }]);
    expect(s.usage).toEqual(usage);
    expect(s.blocks.at(-1)).toMatchObject({ kind: "note", tone: "stop" });
    expect(usageTotals(s.usage)).toEqual({ total: 116, cached: 101 });
  });

  it("不认识的 kind 一律忽略", () => {
    const s0 = run([]);
    expect(run([{ kind: "compact" }, { kind: "team_progress" }, { kind: "skills_changed" }], s0)).toBe(s0);
  });

  it("子 agent 按 agent_session_id 归组", () => {
    const s = run([
      { kind: "subagent_progress", agent_session_id: "A", agent_label: "探查", event: { kind: "text_delta", text: "a" } },
      { kind: "subagent_progress", agent_session_id: "A", event: { kind: "text_delta", text: "b" } },
      { kind: "subagent_progress", agent_session_id: "A", event: { kind: "tool_use", name: "Grep" } },
      { kind: "subagent_progress", agent_session_id: "A", event: { kind: "turn_complete" } },
    ]);
    expect(s.blocks).toHaveLength(1);
    expect(s.blocks[0]).toMatchObject({ kind: "agent", label: "探查", state: "complete", lines: [{ text: "ab" }, { text: "→ Grep" }] });
  });

  it("一轮结束：我们要求的中断说'已中断'，不报失败", () => {
    const running = { ...run([{ kind: "text_delta", text: "x" }]), turn: { turnId: "t" } };
    const s = applySettled(running, { error: { message: "cancelled" } }, true, texts);
    expect(s.turn).toBeNull();
    expect(s.blocks.filter((b) => b.kind === "note")).toEqual([expect.objectContaining({ text: "已中断", tone: "interrupted" })]);
    const failed = applySettled(running, { error: { message: "boom" } }, false, texts);
    expect(failed.blocks.at(-1)).toMatchObject({ kind: "note", tone: "error", text: "失败：boom" });
  });

  it("一轮结束时还没有结果的工具标成已中断（引擎被取消时不补 tool_result），有结果的不动", () => {
    let s: SessionState = { ...emptySession("S"), loading: false, turn: { turnId: "t" } };
    s = applyEvent(s, { kind: "tool_use", id: "a", name: "Read", input: {} }, { ...opts, now: 1_000 });
    s = applyEvent(s, { kind: "tool_result", id: "a", content: "x" }, { ...opts, now: 1_800 });
    s = applyEvent(s, { kind: "tool_use", id: "b", name: "Bash", input: { command: "sleep 9" } }, { ...opts, now: 2_000 });
    const settled = applySettled(s, { error: { message: "cancelled" } }, true, texts, 5_000);
    const tools = settled.blocks.filter((b) => b.kind === "tool");
    expect(tools).toEqual([
      expect.objectContaining({ id: "a", status: "ok", startedAt: 1_000, endedAt: 1_800 }),
      expect.objectContaining({ id: "b", status: "interrupted", startedAt: 2_000, endedAt: 5_000 }),
    ]);
  });

  it("时间只记实时的：user_message 带 item_id 与时间；思考量出秒数；重放的事件不记时间", () => {
    let s: SessionState = { ...emptySession("S"), loading: false };
    s = applyEvent(s, { kind: "user_message", text: "hi", item_id: "q-1" }, { ...opts, now: 500 });
    s = applyEvent(s, { kind: "thinking_delta", text: "想" }, { ...opts, now: 1_000 });
    s = applyEvent(s, { kind: "thinking_delta", text: "想" }, { ...opts, now: 2_000 });
    s = applyEvent(s, { kind: "text_delta", text: "答" }, { ...opts, now: 4_000 });
    expect(s.blocks).toEqual([
      expect.objectContaining({ kind: "user", itemId: "q-1", at: 500 }),
      expect.objectContaining({ kind: "think", text: "想想", streaming: false, startedAt: 1_000, endedAt: 4_000 }),
      expect.objectContaining({ kind: "assistant", text: "答", at: 4_000 }),
    ]);
    const replayed = applyEvent({ ...emptySession("S"), loading: false }, { kind: "tool_use", id: "c", name: "Bash" }, { ...opts, live: false });
    expect(replayed.blocks[0]).toMatchObject({ kind: "tool", status: "running" });
    expect((replayed.blocks[0] as { startedAt?: number }).startedAt).toBeUndefined();
  });
});

describe("立即送达：交给正在跑的一轮（IPDrun N-108）", () => {
  const steer = { kind: "user_message", text: "插一句", item_id: "q-2", delivery: "steer" };

  it("待送达不成块、不开新的一轮；user_message_delivered 时在此刻的位置成一条已送达的人的话", () => {
    let s = run([
      { kind: "user_message", text: "开始", item_id: "q-1" },
      { kind: "step_start", step: 0 },
      { kind: "tool_use", id: "a", name: "Bash", input: {} },
      steer,
    ]);
    expect(s.steering).toEqual([{ itemId: "q-2", text: "插一句" }]);
    expect(s.blocks.map((b) => b.kind)).toEqual(["user", "tool"]);
    // 重放里同一帧再来一次：不重复
    s = run([steer], s);
    expect(s.steering).toHaveLength(1);
    s = run(
      [
        { kind: "tool_result", id: "a", content: "ok" },
        { kind: "step_end", step: 0, tool_calls: 1 },
        { kind: "user_message_delivered", message_id: "q-2", item_id: "q-2", content: "插一句" },
        { kind: "step_start", step: 1 },
        { kind: "text_delta", text: "好的" },
      ],
      s,
    );
    expect(s.steering).toEqual([]);
    expect(s.blocks.map((b) => b.kind)).toEqual(["user", "tool", "user", "assistant"]);
    expect(s.blocks[2]).toMatchObject({ kind: "user", text: "插一句", itemId: "q-2", steered: true });
    expect(s.blocks[0]).not.toHaveProperty("steered");
  });

  it("本轮在送达前结束：user_messages_returned 去掉待送达（中枢把它排回队首），下一轮以普通 user_message 送来", () => {
    let s = run([{ kind: "user_message", text: "开始", item_id: "q-1" }, steer]);
    s = run([{ kind: "user_messages_returned", messages: [{ message_id: "q-2", content: "插一句" }] }], s);
    expect(s.steering).toEqual([]);
    expect(s.blocks).toHaveLength(1);
    s = run([steer], s);
    s = run([{ kind: "user_message", text: "插一句", item_id: "q-2" }], s);
    expect(s.steering).toEqual([]);
    expect(s.blocks.map((b) => [b.kind, (b as { itemId?: string }).itemId])).toEqual([
      ["user", "q-1"],
      ["user", "q-2"],
    ]);
    expect(s.blocks[1]).not.toHaveProperty("steered");
  });

  it("一轮结束时还待送达的不会再进这一轮：清掉", () => {
    const s = run([{ kind: "user_message", text: "开始", item_id: "q-1" }, steer]);
    expect(applySettled(s, { result: {} }, false, texts).steering).toEqual([]);
  });

  it("历史里 origin steer 的人的话标已送达", () => {
    const labels = { result: "结果", image: "[图片]" };
    const s = applyHistoryMessage(emptySession("S"), { role: "user", content: "插一句", origin: "steer", turn_id: "t1", seq: 2, id: "c" }, labels);
    expect(s.blocks[0]).toMatchObject({ kind: "user", text: "插一句", steered: true });
  });
});

describe("历史", () => {
  it("transcript 变成块：注入的上下文折成一行，工具结果配对", () => {
    let s = emptySession("S");
    const labels = { result: "结果", image: "[图片]" };
    s = applyHistoryMessage(s, { role: "user", content: "\n<system-reminder>记忆</system-reminder>" }, labels);
    s = applyHistoryMessage(s, { role: "user", content: [{ type: "text", text: "你好" }] }, labels);
    s = applyHistoryMessage(
      s,
      {
        role: "assistant",
        content: [
          { type: "thinking", thinking: "想想" },
          { type: "text", text: "我看看" },
          { type: "tool_use", id: "t1", name: "Bash", input: { command: "ls" } },
        ],
      },
      labels,
    );
    s = applyHistoryMessage(s, { role: "user", content: [{ type: "tool_result", tool_use_id: "t1", content: "a.txt" }] }, labels);
    expect(s.blocks.map((b) => b.kind)).toEqual(["context", "user", "think", "assistant", "tool"]);
    expect(s.blocks[4]).toMatchObject({ result: "a.txt", status: "ok" });
  });

  it("按 origin 分：user / steer / command 是人说的话，injected / compact / extension 折成一行", () => {
    let s = emptySession("S");
    const labels = { result: "结果", image: "[图片]" };
    s = applyHistoryMessage(s, { role: "user", content: "你好", origin: "user", turn_id: "t1", seq: 0, id: "a" }, labels);
    s = applyHistoryMessage(s, { role: "user", content: "召回的记忆", origin: "injected", turn_id: "t1", seq: 1, id: "b" }, labels);
    s = applyHistoryMessage(s, { role: "user", content: "插一句", origin: "steer", turn_id: "t1", seq: 2, id: "c" }, labels);
    s = applyHistoryMessage(s, { role: "user", content: "之前的摘要", origin: "compact", seq: 3, id: "d#0" }, labels);
    s = applyHistoryMessage(s, { role: "user", content: "/review 展开", origin: "command", seq: 4, id: "e" }, labels);
    expect(s.blocks.map((b) => b.kind)).toEqual(["user", "context", "user", "context", "user"]);
  });

  it("从历史读回被取消的工具（BUG-UI-211）：已中断，不是失败", () => {
    let s = emptySession("S");
    const labels = { result: "结果", image: "[图片]" };
    s = applyHistoryMessage(s, { role: "assistant", content: [{ type: "tool_use", id: "t1", name: "Bash", input: { command: "sleep 6" } }] }, labels);
    s = applyHistoryMessage(s, { role: "user", content: [{ type: "tool_result", tool_use_id: "t1", content: "cancelled", is_error: true }] }, labels);
    expect(s.blocks[0]).toMatchObject({ status: "interrupted", error: true });
  });

  it("工具结果文本", () => {
    expect(toolResultText(null)).toBe("");
    expect(toolResultText([{ type: "text", text: "a" }, { type: "image" }])).toBe("a\n[图片]");
    expect(toolResultText({ ok: 1 })).toBe('{\n  "ok": 1\n}');
  });
});
