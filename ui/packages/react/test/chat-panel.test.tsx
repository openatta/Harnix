/**
 * ChatPanel 的组件测试：真组件、假服务端。只用 data-testid / data-id / data-state 定位。
 * 覆盖配置面：抬头、快捷回复（与打字同一条发送路径）、停止键、工具行（转录：摘要、⏸、diff、收起所有 diff）、
 * 编号 chip、"/" 与 "@" 菜单、只读、头像分组。
 */

import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatPanel, HarnixUiProvider, type ChatPanelProps } from "../src";
import { FakeClient, attached, flush } from "./fake-client";

let server: FakeClient;

function mount(props: Partial<ChatPanelProps> = {}) {
  return render(
    <HarnixUiProvider client={server.asClient()}>
      <ChatPanel sessionId="S1" {...props} />
    </HarnixUiProvider>,
  );
}

async function settle() {
  await act(async () => {
    await flush();
  });
}

const input = () => screen.getByTestId("conv-input") as HTMLTextAreaElement;

function type(value: string) {
  const node = input();
  fireEvent.change(node, { target: { value, selectionStart: value.length } });
}

beforeEach(() => {
  server = new FakeClient();
  server.handlers["harnix.attach"] = (p) => attached(String(p.session_id));
});

afterEach(() => cleanup());

describe("抬头与框架", () => {
  it("一个抬头：← 面包屑、角色、范围、状态、模型、已选与清除、固定区；没有会话时只画框架", async () => {
    const back = vi.fn();
    const clear = vi.fn();
    const view = render(
      <HarnixUiProvider client={server.asClient()}>
        <ChatPanel
          sessionId={null}
          assistant={{ name: "architect", avatar: "构" }}
          header={{ crumb: "特性 › SR-002", onBack: back, scope: "条目上下文", status: { text: "运行中", tone: "run" }, model: "ds-flash", data: { anchor: "SR-002" } }}
          selection={{ items: ["SR-009", "SR-010"], onClear: clear }}
          pinned={<div data-testid="decision" />}
          unavailable={<div data-testid="opening">正在打开</div>}
        />
      </HarnixUiProvider>,
    );
    await settle();
    expect(screen.getAllByTestId("conv-header")).toHaveLength(1);
    const header = screen.getByTestId("conv-header");
    expect(header.dataset.anchor).toBe("SR-002");
    expect(within(header).getByTestId("conv-crumb").textContent).toBe("特性 › SR-002");
    expect(within(header).getByTestId("conv-role").textContent).toContain("architect");
    expect(within(header).getByTestId("conv-status-tag").dataset.state).toBe("run");
    expect(within(header).getByTestId("conv-model-name").textContent).toBe("ds-flash");
    fireEvent.click(within(header).getByTestId("conv-back"));
    expect(back).toHaveBeenCalled();
    expect(screen.getAllByTestId("conv-selection-item").map((n) => n.dataset.id)).toEqual(["SR-009", "SR-010"]);
    fireEvent.click(screen.getByTestId("conv-selection-clear"));
    expect(clear).toHaveBeenCalled();
    expect(screen.getByTestId("decision")).toBeTruthy();
    expect(screen.getByTestId("opening")).toBeTruthy();
    expect(screen.getByTestId("conv").dataset.state).toBe("unavailable");
    expect(input().disabled).toBe(true);
    view.unmount();
  });

  it("空状态按上下文给；只读时输入框换成说明", async () => {
    mount({ emptyState: <p data-testid="custom-empty">这一条的对话会长期保留</p>, readOnly: "这个工位已归档" });
    await settle();
    expect(screen.getByTestId("conv-empty").textContent).toContain("长期保留");
    expect(screen.getByTestId("conv-readonly").textContent).toContain("已归档");
    expect(screen.queryByTestId("conv-input")).toBeNull();
  });
});

describe("发送", () => {
  it("快捷回复（最多 4 个）与打字走同一条发送路径", async () => {
    const sent: string[] = [];
    mount({
      quickReplies: [
        { label: "条款可测吗？", text: "检查一下条款是否都可测" },
        { label: "b" },
        { label: "c" },
        { label: "d" },
        { label: "e" },
      ],
      send: async (text) => {
        sent.push(text);
        return true;
      },
    });
    await settle();
    expect(screen.getAllByTestId("conv-quick-reply")).toHaveLength(4);
    fireEvent.click(screen.getAllByTestId("conv-quick-reply")[0]!);
    await settle();
    type("打字的话");
    fireEvent.keyDown(input(), { key: "Enter" });
    await settle();
    expect(sent).toEqual(["检查一下条款是否都可测", "打字的话"]);
    expect(server.called("harnix.send")).toHaveLength(0);
  });

  it("没有给 send：快捷回复经 harnix.send 发出（on_busy: queue）", async () => {
    server.handlers["harnix.send"] = () => ({ turn_id: "t1" });
    mount({ quickReplies: [{ label: "还缺哪些边界条款？" }] });
    await settle();
    fireEvent.click(screen.getByTestId("conv-quick-reply"));
    await settle();
    expect(server.called("harnix.send")[0]!.params).toMatchObject({ message: "还缺哪些边界条款？", on_busy: "queue" });
  });

  it("运行中、没有输入：发送键是停止（Esc 同效）；有输入时是发送（插话进队列）", async () => {
    server.handlers["harnix.attach"] = () => attached("S1", { running_turn: "t1" });
    const interrupt = vi.fn();
    mount({ interrupt });
    await settle();
    expect(screen.queryByTestId("conv-send")).toBeNull();
    fireEvent.click(screen.getByTestId("conv-interrupt"));
    expect(interrupt).toHaveBeenCalledTimes(1);
    type("插一句");
    expect(screen.queryByTestId("conv-interrupt")).toBeNull();
    expect((screen.getByTestId("conv-send") as HTMLButtonElement).disabled).toBe(false);
    fireEvent.keyDown(input(), { key: "Escape" });
    expect(interrupt).toHaveBeenCalledTimes(2);
  });

  it("草稿按 draftKey 各自保留：卸载、换键再回来仍在；发出去即清掉", async () => {
    const send = async () => true;
    const view = mount({ draftKey: "A", send });
    await settle();
    type("A 的草稿");
    view.unmount();
    const b = mount({ draftKey: "B", send });
    await settle();
    expect(input().value).toBe("");
    type("B 的草稿");
    b.rerender(
      <HarnixUiProvider client={server.asClient()}>
        <ChatPanel sessionId="S1" draftKey="A" send={send} />
      </HarnixUiProvider>,
    );
    await settle();
    expect(input().value).toBe("A 的草稿");
    fireEvent.keyDown(input(), { key: "Enter" });
    await settle();
    expect(input().value).toBe("");
    b.unmount();
    mount({ draftKey: "B", send });
    await settle();
    expect(input().value).toBe("B 的草稿");
    cleanup();
    mount({ draftKey: "A", send });
    await settle();
    expect(input().value).toBe("");
  });

  it("disabled（如断线）：发送不可用，已输入的内容保留", async () => {
    const view = mount({ draftKey: "D" });
    await settle();
    type("断线前写的");
    view.rerender(
      <HarnixUiProvider client={server.asClient()}>
        <ChatPanel sessionId="S1" draftKey="D" disabled />
      </HarnixUiProvider>,
    );
    expect(input().value).toBe("断线前写的");
    expect((screen.getByTestId("conv-send") as HTMLButtonElement).disabled).toBe(true);
    fireEvent.keyDown(input(), { key: "Enter" });
    await settle();
    expect(server.called("harnix.send")).toHaveLength(0);
    expect(input().value).toBe("断线前写的");
  });

  it("sendMode separate：中断与发送是两个按钮", async () => {
    mount({ sendMode: "separate" });
    await settle();
    expect((screen.getByTestId("conv-interrupt") as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByTestId("conv-send")).toBeTruthy();
  });
});

describe("消息流", () => {
  it("助手：头像 + 名字一组只出一次；人的话打断分组；编号变 chip，点了交给应用", async () => {
    const open = vi.fn();
    server.handlers["session.history"] = () => ({
      messages: [
        { role: "user", content: "看下 SR-002" },
        { role: "assistant", content: [{ type: "text", text: "SL-002-1 缺一条" }, { type: "text", text: "`SR-003` 不变" }] },
        { role: "user", content: "好" },
        { role: "assistant", content: [{ type: "text", text: "收到" }] },
      ],
    });
    server.handlers["harnix.attach"] = () => attached("S1", { history_total: 4 });
    mount({ assistant: { name: "architect", avatar: "构" }, ids: { pattern: /\b(?:SR|SL)-\d+(?:-\d+)?\b/g, onOpen: open } });
    await settle();
    expect(screen.getAllByTestId("conv-speaker")).toHaveLength(2);
    const messages = screen.getAllByTestId("conv-message");
    // chip 不改变正文：textContent 原样
    expect(messages.map((m) => m.textContent)).toEqual(["看下 SR-002", "SL-002-1 缺一条", "SR-003 不变", "好", "收到"]);
    const chips = screen.getAllByTestId("conv-chip").map((c) => c.dataset.id);
    // 代码里的编号不变 chip
    expect(chips).toEqual(["SR-002", "SL-002-1"]);
    fireEvent.click(screen.getAllByTestId("conv-chip")[1]!);
    expect(open).toHaveBeenCalledWith("SL-002-1");
  });

  it("排队中的插话在流里（带撤回）；正在回复在流里", async () => {
    server.handlers["harnix.attach"] = () => attached("S1", { running_turn: "t1", queue: [{ item_id: "q1", message: "关键分支加上日志" }] });
    mount();
    await settle();
    const queued = screen.getByTestId("conv-queue-item");
    expect(queued.dataset.state).toBe("queued");
    expect(queued.textContent).toContain("排队中");
    expect(screen.getByTestId("conv-typing").textContent).toContain("正在回复");
  });
});

describe("立即送达（IPDrun N-108）", () => {
  it("交给正在跑的一轮：流的末尾待送达 → 送达后成为已送达的人的话；退回则去掉，转为队列里的排队", async () => {
    server.handlers["harnix.attach"] = () => attached("S1", { running_turn: "t1" });
    mount({ variant: "transcript" });
    await settle();
    act(() => {
      server.event("S1", { kind: "tool_use", id: "b1", name: "Bash", input: { command: "sleep 6" } });
      server.event("S1", { kind: "user_message", text: "顺手看下日志", item_id: "q-1", delivery: "steer" });
    });
    const pending = screen.getByTestId("conv-steer-item");
    expect(pending.dataset.state).toBe("steering");
    expect(pending.dataset.id).toBe("q-1");
    expect(pending.textContent).toContain("待送达");
    expect(screen.queryAllByTestId("conv-message").filter((m) => m.dataset.role === "user")).toHaveLength(0);
    act(() => {
      server.event("S1", { kind: "tool_result", id: "b1", content: "ok" });
      server.event("S1", { kind: "user_message_delivered", message_id: "q-1", item_id: "q-1", content: "顺手看下日志" });
    });
    expect(screen.queryByTestId("conv-steer-item")).toBeNull();
    const said = screen.getAllByTestId("conv-message").find((m) => m.dataset.role === "user")!;
    expect(said.dataset.state).toBe("steered");
    expect(said.dataset.item).toBe("q-1");
    expect(within(said).getByTestId("conv-steered").textContent).toContain("已送达");
    expect(screen.getByTestId("conv-tool").dataset.state).toBe("ok");

    // 第二句没赶上：本轮结束前退回，中枢把它排回队首
    act(() => {
      server.event("S1", { kind: "user_message", text: "再补一句", item_id: "q-2", delivery: "steer" });
      server.event("S1", { kind: "user_messages_returned", messages: [{ message_id: "q-2", content: "再补一句" }] });
      server.push("harnix.queue", { session_id: "S1", items: [{ item_id: "q-2", message: "再补一句" }] });
    });
    expect(screen.queryByTestId("conv-steer-item")).toBeNull();
    expect(screen.getByTestId("conv-queue-item").dataset.id).toBe("q-2");
  });
});

describe("转录", () => {
  it("工具行：友好名字 + 参数 + 实时耗时 + ⎿ 摘要；编辑类带行号的 diff 缺省展开，收起所有 diff 后收起", async () => {
    mount({ variant: "transcript" });
    await settle();
    act(() => {
      server.event("S1", { kind: "user_message", text: "开始" });
      server.event("S1", { kind: "tool_use", id: "e1", name: "Edit", input: { file_path: "src/a.rs", old_string: "a\nb\nc", new_string: "a\nB\nc" } });
      server.event("S1", { kind: "tool_result", id: "e1", content: "ok" });
      server.event("S1", { kind: "tool_use", id: "b1", name: "Bash", input: { command: "cargo test" } });
      server.event("S1", { kind: "tool_result", id: "b1", content: "running 3 tests\ntest result: ok. 3 passed" });
      server.event("S1", { kind: "tool_use", id: "m1", name: "mcp__ipdrun__xflow_state", input: { orient: true } });
    });
    const rows = screen.getAllByTestId("conv-tool");
    expect(rows.map((r) => within(r).getByTestId("conv-tool-name").textContent)).toEqual(["Update", "Bash", "xflow_state"]);
    expect(within(rows[0]!).getByTestId("conv-tool-summary").textContent).toContain("增加 1 行，删除 1 行");
    expect(within(rows[1]!).getByTestId("conv-tool-summary").textContent).toContain("test result: ok. 3 passed");
    expect(within(rows[2]!).getByTestId("conv-tool-summary").textContent).toContain("运行中");
    expect(rows[0]!.textContent).toMatch(/\d\.\ds/);
    const diff = within(rows[0]!).getByTestId("conv-tool-diff");
    expect(within(diff).getAllByTestId("conv-diff-line").map((l) => l.dataset.op)).toEqual(["same", "del", "add", "same"]);

    // 收起所有 diff（⚙ 选项里，纯前端）
    fireEvent.click(screen.getByTestId("conv-options"));
    fireEvent.click(screen.getByTestId("conv-diff-collapse"));
    expect(within(screen.getAllByTestId("conv-tool")[0]!).queryByTestId("conv-tool-diff")).toBeNull();
    // 单行仍然可以点开
    fireEvent.click(within(screen.getAllByTestId("conv-tool")[0]!).getByTestId("conv-tool-toggle"));
    expect(within(screen.getAllByTestId("conv-tool")[0]!).getByTestId("conv-tool-diff")).toBeTruthy();
  });

  it("中断：还在跑的工具变成 ⏸ 已中断", async () => {
    server.handlers["harnix.attach"] = () => attached("S1", { running_turn: "t1" });
    mount({ variant: "transcript" });
    await settle();
    act(() => server.event("S1", { kind: "tool_use", id: "s1", name: "Bash", input: { command: "sleep 30" } }));
    expect(screen.getByTestId("conv-tool").dataset.state).toBe("running");
    expect(screen.getByTestId("conv-typing").textContent).toContain("Esc");
    act(() => server.event("S1", { kind: "tool_use", id: "s2", name: "Bash", input: { command: "sleep 40" } }));
    fireEvent.click(screen.getByTestId("conv-interrupt"));
    await settle();
    // 引擎取消工具时回一条失败的结果 "cancelled"：这是中断造成的，不是失败
    act(() => server.event("S1", { kind: "tool_result", id: "s2", content: "cancelled", is_error: true }));
    act(() => server.push("harnix.turn_settled", { session_id: "S1", turn_id: "t1", error: { message: "cancelled" } }));
    expect(screen.getAllByTestId("conv-tool").map((r) => r.dataset.state)).toEqual(["interrupted", "interrupted"]);
    const row = screen.getAllByTestId("conv-tool")[0]!;
    expect(row.dataset.state).toBe("interrupted");
    expect(row.textContent).toContain("⏸");
    expect(within(row).getByTestId("conv-tool-summary").textContent).toContain("已中断");
  });

  it("别处中断（BUG-UI-211：工位的中断走服务端）：取消的工具标已中断，不是失败；停止原因不另起一行", async () => {
    server.handlers["harnix.attach"] = () => attached("S1", { running_turn: "t1" });
    mount({ variant: "transcript" });
    await settle();
    act(() => server.event("S1", { kind: "tool_use", id: "s1", name: "Bash", input: { command: "sleep 6" } }));
    act(() => server.event("S1", { kind: "tool_result", id: "s1", content: "cancelled", is_error: true }));
    act(() => server.event("S1", { kind: "turn_complete", stop_reason: "cancelled" }));
    const row = screen.getByTestId("conv-tool");
    expect(row.dataset.state).toBe("interrupted");
    expect(within(row).getByTestId("conv-tool-summary").textContent).toContain("⏸ 已中断");
    expect(screen.getAllByTestId("conv-note").map((n) => [n.dataset.state, n.textContent])).toEqual([["interrupted", "已中断"]]);
    // 真的失败照旧是失败
    act(() => server.event("S1", { kind: "tool_use", id: "s2", name: "Bash", input: { command: "false" } }));
    act(() => server.event("S1", { kind: "tool_result", id: "s2", content: "exit status 1", is_error: true }));
    expect(screen.getAllByTestId("conv-tool")[1]!.dataset.state).toBe("error");
  });
});

describe("上下文环", () => {
  it("取 turn_complete.context 的 tokens / window，不取本轮用量的合计；阈值来自 compact_threshold", async () => {
    mount({ variant: "transcript" });
    await settle();
    // 还没有调过模型：没有环
    expect(screen.queryByTestId("conv-context-ring")).toBeNull();
    act(() => {
      server.event("S1", { kind: "user_message", text: "开始" });
      server.event("S1", {
        kind: "turn_complete",
        stop_reason: "end_turn",
        api_calls: 3,
        // 三次调用的合计：拿它画环会远超窗口
        usage: { input_tokens: 150_000, cache_read_input_tokens: 120_000, output_tokens: 900 },
        last_call_usage: { input_tokens: 20_000, cache_read_input_tokens: 30_000, output_tokens: 300 },
        context: { tokens: 50_000, compact_threshold: 140_000, window: 200_000 },
      });
    });
    const ring = screen.getByTestId("conv-context-ring");
    expect(ring.dataset.tokens).toBe("50000");
    expect(ring.dataset.max).toBe("200000");
    expect(ring.dataset.value).toBe("0.25");
    expect(ring.dataset.threshold).toBe("0.7");
    expect(ring.textContent).toBe("25%");
    expect(ring.getAttribute("title")).not.toContain("约");
    fireEvent.click(ring);
    expect(screen.getByTestId("conv-context-note").textContent).toBe("超过 70% 自动压缩");
  });

  it("attach 时取 session.get 的 context；会话还没有 context 时才用应用给的比例", async () => {
    server.handlers["harnix.attach"] = () =>
      attached("S1", { session: { session_id: "S1", turn_state: "idle", context: { tokens: 30_000, compact_threshold: 150_000, window: 200_000 } } });
    mount({ variant: "transcript", features: { contextRing: { ratio: 0.9, note: "压缩与清空都以摘要重建上下文" } } });
    await settle();
    const ring = screen.getByTestId("conv-context-ring");
    expect(ring.dataset.value).toBe("0.15");
    fireEvent.click(ring);
    expect(screen.getByTestId("conv-context-note").textContent).toBe("超过 75% 自动压缩 · 压缩与清空都以摘要重建上下文");
    cleanup();

    server.handlers["harnix.attach"] = (p) => attached(String(p.session_id));
    mount({ variant: "transcript", features: { contextRing: { ratio: 0.4 } } });
    await settle();
    expect(screen.getByTestId("conv-context-ring").dataset.value).toBe("0.4");
  });
});

describe("菜单", () => {
  it("/ 列出 harnix.hello 里引擎支持的命令；回车选中", async () => {
    server.handlers["harnix.hello"] = () => ({
      protocol_version: 1,
      contrib_api_version: 1,
      engine: {},
      scenes: [],
      commands: [
        { name: "compact", description: "压缩上下文" },
        { name: "clear", description: "清空" },
      ],
      contributions: [],
    });
    mount();
    await settle();
    type("/co");
    const menu = screen.getByTestId("conv-menu");
    expect(menu.dataset.kind).toBe("/");
    expect(within(menu).getAllByTestId("conv-menu-item").map((n) => n.dataset.id)).toEqual(["compact"]);
    fireEvent.keyDown(input(), { key: "Enter" });
    expect(input().value).toBe("/compact ");
    expect(screen.queryByTestId("conv-menu")).toBeNull();
  });

  it("@ 经应用给的搜索列出条目；点一项插入编号", async () => {
    const search = vi.fn(async (q: string) => [{ id: "SR-002", label: "部署记录" }].filter((x) => x.id.includes(q.toUpperCase())));
    mount({ mention: search });
    await settle();
    type("参考 @sr");
    await settle();
    expect(search).toHaveBeenCalledWith("sr");
    const item = screen.getByTestId("conv-menu-item");
    expect(item.dataset.id).toBe("SR-002");
    fireEvent.click(item);
    expect(input().value).toBe("参考 SR-002 ");
  });
});
