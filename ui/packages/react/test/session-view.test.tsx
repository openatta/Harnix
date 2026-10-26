/**
 * 会话视图的组件测试：真组件、假服务端（概念与架构 §7 的建议——界面测试不需要浏览器
 * 这一条，用 React 的组件测试替代假 DOM）。定位只用 data-testid / data-id / data-state。
 */

import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { HarnixUiProvider, SessionView } from "../src";
import { FakeClient, attached, flush } from "./fake-client";

let server: FakeClient;

function mount(sessionId = "S1") {
  const view = render(
    <HarnixUiProvider client={server.asClient()}>
      <SessionView sessionId={sessionId} />
    </HarnixUiProvider>,
  );
  return {
    ...view,
    rerenderWith(id: string) {
      view.rerender(
        <HarnixUiProvider client={server.asClient()}>
          <SessionView sessionId={id} />
        </HarnixUiProvider>,
      );
    },
  };
}

const conv = () => screen.getByTestId("conv");
const messages = () => screen.queryAllByTestId("conv-message");

async function settle() {
  await act(async () => {
    await flush();
  });
}

beforeEach(() => {
  server = new FakeClient();
  server.handlers["harnix.attach"] = (p) => attached(String(p.session_id));
});

afterEach(() => cleanup());

describe("挂上与追赶", () => {
  it("先历史、后重放、再挂起的提问；正在跑的一轮可以中断", async () => {
    server.handlers["harnix.attach"] = () =>
      attached("S1", {
        history_total: 2,
        running_turn: "t2",
        seq: 3,
        replay: [
          { seq: 2, turn_id: "t2", event: { kind: "user_message", text: "第二问" } },
          { seq: 3, turn_id: "t2", event: { kind: "text_delta", text: "正在" } },
        ],
        pending_prompts: [{ kind: "prompt", prompt_id: "p1", tool_name: "Bash", message: "rm -rf x" }],
      });
    server.handlers["session.history"] = () => ({
      messages: [
        { role: "user", content: "第一问" },
        { role: "assistant", content: [{ type: "text", text: "**答**" }] },
      ],
    });
    mount();
    await settle();

    expect(conv().dataset.state).toBe("running");
    expect(conv().dataset.id).toBe("S1");
    const texts = messages().map((m) => `${m.dataset.role}:${m.textContent}`);
    expect(texts).toEqual(["user:第一问", "assistant:答", "user:第二问", "assistant:正在"]);
    // 权限卡截断了流式块（与 Nest 相同），之后的正文另起一块。
    expect(messages()[3]!.dataset.state).toBe("done");
    expect(screen.getByTestId("conv-ask").dataset.id).toBe("p1");
    expect((screen.getByTestId("conv-interrupt") as HTMLButtonElement).disabled).toBe(false);
    // 只读到 history_total：其后由重放接上
    expect(server.called("session.history")[0]!.params).toMatchObject({ session_id: "S1", offset: 0, limit: 2 });

    // 已在快照里的 seq 不再应用一次。
    act(() => server.event("S1", { kind: "text_delta", text: "重复" }, 3));
    expect(messages()).toHaveLength(4);
    act(() => server.event("S1", { kind: "text_delta", text: "中" }, 4));
    expect(messages()).toHaveLength(5);
    expect(messages()[4]!.textContent).toBe("中");
    expect(messages()[4]!.dataset.state).toBe("streaming");
  });

  /** 之前一轮两条，加上正在跑的 t2 已落盘的三条（引擎每一步开始前落盘）：用户消息、带工具调用的回复、工具结果。 */
  const midTurnTranscript = [
    { role: "user", content: "第一问", turn_id: "t1", origin: "user", seq: 0 },
    { role: "assistant", content: [{ type: "text", text: "答" }], turn_id: "t1", origin: "assistant", seq: 1 },
    { role: "user", content: "第二问", turn_id: "t2", origin: "user", seq: 2 },
    {
      role: "assistant",
      content: [
        { type: "text", text: "先读" },
        { type: "tool_use", id: "u1", name: "Read", input: { file_path: "a.txt" } },
      ],
      turn_id: "t2",
      origin: "assistant",
      seq: 3,
    },
    { role: "user", content: [{ type: "tool_result", tool_use_id: "u1", content: "x" }], turn_id: "t2", origin: "tool_result", seq: 4 },
  ];
  /** 中枢缓冲区里 t2 的全部帧（从它合成的 user_message 起）。 */
  const midTurnReplay = [
    { seq: 1, turn_id: "t2", event: { kind: "user_message", text: "第二问" } },
    { seq: 2, turn_id: "t2", event: { kind: "turn_start" } },
    { seq: 3, turn_id: "t2", event: { kind: "step_start", step: 0 } },
    { seq: 4, turn_id: "t2", event: { kind: "text_delta", text: "先读" } },
    { seq: 5, turn_id: "t2", event: { kind: "tool_use_start", id: "u1", name: "Read" } },
    { seq: 6, turn_id: "t2", event: { kind: "tool_use", id: "u1", name: "Read", input: { file_path: "a.txt" } } },
    { seq: 7, turn_id: "t2", event: { kind: "tool_result", id: "u1", content: "x" } },
    { seq: 8, turn_id: "t2", event: { kind: "step_end", step: 0, seq: 5 } },
    { seq: 9, turn_id: "t2", event: { kind: "step_start", step: 1 } },
    { seq: 10, turn_id: "t2", event: { kind: "text_delta", text: "读完了" } },
  ];

  function serveTranscript() {
    server.handlers["session.history"] = (p) => {
      const offset = Number(p.offset ?? 0);
      return { messages: midTurnTranscript.slice(offset, offset + Number(p.limit ?? 100)), total: midTurnTranscript.length };
    };
  }

  it("一轮跑到一半时挂上（刷新）：这一轮已落盘的步骤不画两遍", async () => {
    serveTranscript();
    // 中枢给的 history_total 是这一轮开始前的消息数（N-120）
    server.handlers["harnix.attach"] = () => attached("S1", { history_total: 2, running_turn: "t2", seq: 10, replay: midTurnReplay });
    mount();
    await settle();

    expect(conv().dataset.state).toBe("running");
    expect(messages().map((m) => `${m.dataset.role}:${m.textContent}`)).toEqual([
      "user:第一问",
      "assistant:答",
      "user:第二问",
      "assistant:先读",
      "assistant:读完了",
    ]);
    expect(screen.getAllByTestId("conv-tool")).toHaveLength(1);
    expect(screen.getByTestId("conv-tool").dataset.state).toBe("ok");
    expect(server.called("session.history").map((c) => c.params.limit)).toEqual([2]);
  });

  it("历史的深度里混进了正在跑的一轮（压缩之后）：按 turn_id 只取重放的那份", async () => {
    serveTranscript();
    server.handlers["harnix.attach"] = () => attached("S1", { history_total: 5, running_turn: "t2", seq: 10, replay: midTurnReplay });
    mount();
    await settle();
    expect(messages().map((m) => m.textContent)).toEqual(["第一问", "答", "第二问", "先读", "读完了"]);
    expect(screen.getAllByTestId("conv-tool")).toHaveLength(1);
  });

  it("追赶期间到达的事件不丢、不重", async () => {
    let release!: (value: unknown) => void;
    server.handlers["harnix.attach"] = () => new Promise((resolve) => (release = resolve));
    mount();
    await settle();
    expect(conv().dataset.state).toBe("loading");

    act(() => server.event("S1", { kind: "user_message", text: "早到" }, 1));
    await act(async () => {
      release(attached("S1", { seq: 0 }));
      await flush();
    });
    expect(messages().map((m) => m.textContent)).toEqual(["早到"]);
    expect(conv().dataset.state).toBe("running");
  });

  it("重连后重新 attach；换会话时 detach 旧的", async () => {
    const view = mount();
    await settle();
    expect(server.called("harnix.attach")).toHaveLength(1);

    act(() => server.setStatus("closed"));
    expect(screen.getByTestId("conn-status").dataset.state).toBe("closed");
    await act(async () => {
      server.setStatus("open");
      await flush();
    });
    expect(server.called("harnix.attach")).toHaveLength(2);

    view.rerenderWith("S2");
    await settle();
    expect(server.notified).toContainEqual({ method: "harnix.detach", params: { instance: "system", session_id: "S1" } });
    expect(server.called("harnix.attach").at(-1)!.params).toEqual({ instance: "system", session_id: "S2" });
    expect(conv().dataset.id).toBe("S2");
  });

  it("attach 失败：说出理由，输入框不可用", async () => {
    server.handlers["harnix.attach"] = () => Promise.reject({ code: -32000, message: "session not found" });
    mount();
    await settle();
    expect(conv().dataset.state).toBe("failed");
    expect(screen.getByTestId("conv-note").textContent).toContain("session not found");
    expect((screen.getByTestId("conv-input") as HTMLTextAreaElement).disabled).toBe(true);
  });
});

describe("一轮", () => {
  it("发送 → 用户消息 → 流式回复 → 用量 → 结束", async () => {
    server.handlers["harnix.send"] = () => {
      server.event("S1", { kind: "user_message", text: "你好" });
      return { turn_id: "t1" };
    };
    mount();
    await settle();
    expect(screen.getByTestId("conv-empty")).toBeTruthy();

    const input = screen.getByTestId("conv-input");
    fireEvent.change(input, { target: { value: "你好" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await settle();

    expect(server.called("harnix.send")[0]!.params).toMatchObject({ session_id: "S1", message: "你好", on_busy: "queue" });
    expect((input as HTMLTextAreaElement).value).toBe("");
    expect(conv().dataset.state).toBe("running");
    // 正在回复：流里的一行（头像 + 小方块）；没有输入时发送键是"停止"
    expect(screen.getByTestId("conv-typing")).toBeTruthy();
    expect(screen.getByTestId("conv-interrupt")).toBeTruthy();

    act(() => {
      server.event("S1", { kind: "text_delta", text: "你也" });
      server.event("S1", { kind: "text_delta", text: "好" });
    });
    const reply = messages()[1]!;
    expect(reply.dataset.role).toBe("assistant");
    expect(reply.dataset.state).toBe("streaming");

    act(() => {
      server.event("S1", {
        kind: "turn_complete",
        stop_reason: "end_turn",
        usage: { input_tokens: 600, output_tokens: 8, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 },
      });
      server.push("harnix.turn_settled", { session_id: "S1", turn_id: "t1", result: { name: "问候" } });
    });
    expect(messages()[1]!.dataset.state).toBe("done");
    expect(messages()[1]!.textContent).toBe("你也好");
    expect(screen.getByTestId("conv-usage").dataset.total).toBe("608");
    expect(screen.getByTestId("conv-title").textContent).toBe("问候");
    expect(conv().dataset.state).toBe("idle");
    expect(screen.queryByTestId("conv-interrupt")).toBeNull();
    expect(screen.queryByTestId("conv-typing")).toBeNull();
  });

  it("别的会话的事件不进这个视图", async () => {
    mount();
    await settle();
    act(() => server.event("OTHER", { kind: "user_message", text: "别处" }));
    expect(messages()).toHaveLength(0);
  });

  it("工具行：一行高，点开看输入与结果", async () => {
    mount();
    await settle();
    act(() => {
      server.event("S1", { kind: "tool_use", id: "tu1", name: "Bash", input: { command: "echo hx-fixture" } });
    });
    const row = screen.getByTestId("conv-tool");
    expect(row.dataset.id).toBe("tu1");
    expect(row.dataset.state).toBe("running");
    expect(within(row).getByTestId("conv-tool-name").textContent).toBe("Bash");
    expect(within(row).queryByTestId("conv-tool-result")).toBeNull();

    act(() => server.event("S1", { kind: "tool_result", id: "tu1", name: "Bash", content: "hx-fixture\n" }));
    expect(screen.getByTestId("conv-tool").dataset.state).toBe("ok");
    // 整行可点（原型 cliLine 的 data-act="toolOpen"）：开合状态在行上
    const toggle = within(screen.getByTestId("conv-tool")).getByTestId("conv-tool-toggle");
    expect(screen.getByTestId("conv-tool").getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(toggle);
    expect(screen.getByTestId("conv-tool").getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByTestId("conv-tool-input").textContent).toContain("echo hx-fixture");
    expect(screen.getByTestId("conv-tool-result").textContent).toContain("hx-fixture");
  });

  it("tool_use_start：参数写完之前工具行就出现（运行中）；tool_use 到了在同一行补上参数", async () => {
    mount();
    await settle();
    act(() => {
      server.event("S1", { kind: "user_message", text: "写个大文件" });
      server.event("S1", { kind: "tool_use_start", id: "w1", name: "Write" });
      server.event("S1", { kind: "tool_args_delta", id: "w1", partial_json: '{"file_path":"big.txt","content":"' });
    });
    const row = screen.getByTestId("conv-tool");
    expect(row.dataset.id).toBe("w1");
    expect(row.dataset.state).toBe("running");
    expect(within(row).getByTestId("conv-tool-name").textContent).toBe("Write");
    // 拼到一半的参数不画
    expect(row.textContent).not.toContain("big.txt");

    act(() => server.event("S1", { kind: "tool_use", id: "w1", name: "Write", input: { file_path: "big.txt", content: "..." } }));
    expect(screen.getAllByTestId("conv-tool")).toHaveLength(1);
    expect(screen.getByTestId("conv-tool").dataset.state).toBe("running");
    expect(screen.getByTestId("conv-tool").textContent).toContain("big.txt");
    act(() => server.event("S1", { kind: "tool_result", id: "w1", content: "written" }));
    expect(screen.getByTestId("conv-tool").dataset.state).toBe("ok");
  });

  it("权限卡：四个回答与倒计时；回答走 session.respondToPrompt", async () => {
    mount();
    await settle();
    act(() => server.event("S1", { kind: "prompt", prompt_type: "permission", prompt_id: "p9", tool_name: "Bash", paths: ["/tmp/x"] }));
    const card = screen.getByTestId("conv-ask");
    expect(card.dataset.state).toBe("pending");
    expect(within(card).getAllByRole("button")).toHaveLength(4);
    expect(within(card).getByTestId("conv-ask-countdown").textContent).toMatch(/\d+/);

    fireEvent.click(within(card).getByTestId("conv-ask-allow-session"));
    await settle();
    expect(server.called("session.respondToPrompt")[0]!.params).toEqual({
      instance: "system",
      session_id: "S1",
      prompt_id: "p9",
      decision: { type: "permit_always", scope: "session" },
    });
    expect(screen.getByTestId("conv-ask").dataset.state).toBe("answered");
    expect(screen.getByTestId("conv-ask").dataset.answer).toBe("allow");
    expect(within(screen.getByTestId("conv-ask")).queryAllByRole("button")).toHaveLength(0);
  });

  it("插话：运行中发送进队列；队列整值替换；可撤回", async () => {
    server.handlers["harnix.attach"] = () => attached("S1", { running_turn: "t1" });
    server.handlers["harnix.send"] = (p) => ({ queued: true, item: { item_id: "q1", message: p.message } });
    server.handlers["harnix.queue.remove"] = () => ({ items: [] });
    mount();
    await settle();

    const input = screen.getByTestId("conv-input");
    fireEvent.change(input, { target: { value: "关键分支加上日志" } });
    fireEvent.click(screen.getByTestId("conv-send"));
    await settle();
    const item = screen.getByTestId("conv-queue-item");
    expect(item.dataset.id).toBe("q1");
    expect(item.textContent).toContain("关键分支加上日志");

    act(() =>
      server.push("harnix.queue", {
        session_id: "S1",
        items: [
          { item_id: "q1", message: "关键分支加上日志" },
          { item_id: "q2", message: "另一台设备发的" },
        ],
      }),
    );
    expect(screen.getAllByTestId("conv-queue-item").map((n) => n.dataset.id)).toEqual(["q1", "q2"]);

    fireEvent.click(within(screen.getAllByTestId("conv-queue-item")[0]!).getByTestId("conv-queue-remove"));
    await settle();
    expect(server.called("harnix.queue.remove")[0]!.params).toEqual({ instance: "system", session_id: "S1", item_id: "q1" });
    expect(screen.queryByTestId("conv-queue")).toBeNull();
  });

  it("中断：调用 session.interrupt；这一轮结束时说'已中断'而不是失败", async () => {
    server.handlers["harnix.attach"] = () => attached("S1", { running_turn: "t1" });
    mount();
    await settle();
    fireEvent.click(screen.getByTestId("conv-interrupt"));
    await settle();
    expect(server.called("session.interrupt")[0]!.params).toEqual({ instance: "system", session_id: "S1" });

    act(() => server.push("harnix.turn_settled", { session_id: "S1", turn_id: "t1", error: { code: -32000, message: "cancelled" } }));
    const notes = screen.getAllByTestId("conv-note");
    expect(notes.map((n) => n.dataset.state)).toEqual(["interrupted"]);
    expect(conv().dataset.state).toBe("idle");
  });

  it("没要求中断的失败照常报错", async () => {
    server.handlers["harnix.attach"] = () => attached("S1", { running_turn: "t1" });
    mount();
    await settle();
    act(() => server.push("harnix.turn_settled", { session_id: "S1", turn_id: "t1", error: { message: "provider down" } }));
    const note = screen.getByTestId("conv-note");
    expect(note.dataset.state).toBe("error");
    expect(note.getAttribute("role")).toBe("alert");
  });
});

describe("应用的贡献", () => {
  it("onSend 代替 harnix.send：false 保留原文，true 清空；userRows 认领人的消息", async () => {
    const sent: string[] = [];
    let answer = false;
    const userRows = [
      {
        id: "brief",
        match: (b: { text: string }) => b.text.startsWith("# 任务"),
        render: (b: { text: string }) => <div data-testid="brief-row">{b.text.split("\n")[0]}</div>,
      },
    ];
    server.handlers["session.history"] = () => ({ messages: [{ role: "user", content: "# 任务 T-1\n很长的简报" }] });
    server.handlers["harnix.attach"] = () => attached("S1", { history_total: 1 });
    render(
      <HarnixUiProvider client={server.asClient()}>
        <SessionView
          sessionId="S1"
          userRows={userRows}
          composerTop={<div data-testid="composer-top" />}
          onSend={async (text) => {
            sent.push(text);
            return answer;
          }}
        />
      </HarnixUiProvider>,
    );
    await settle();
    expect(screen.getByTestId("brief-row").textContent).toBe("# 任务 T-1");
    expect(messages()).toHaveLength(0);
    expect(screen.getByTestId("composer-top")).toBeTruthy();

    const input = screen.getByTestId("conv-input") as HTMLTextAreaElement;
    fireEvent.change(input, { target: { value: "看下 src/x.js" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await settle();
    expect(sent).toEqual(["看下 src/x.js"]);
    expect(input.value).toBe("看下 src/x.js");
    expect(server.called("harnix.send")).toHaveLength(0);

    answer = true;
    fireEvent.keyDown(input, { key: "Enter" });
    await settle();
    expect(sent).toHaveLength(2);
    expect(input.value).toBe("");
  });
});
