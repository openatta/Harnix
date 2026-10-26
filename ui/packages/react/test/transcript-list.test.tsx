/**
 * TranscriptList 的组件测试：块表来自 props，行组件与 ChatPanel 的转录相同（工具行的状态、摘要、展开；系统行；
 * 人的消息的贡献点；编号 chip；工作中一行；空）。
 */

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Block } from "@harnix/core/state";
import { HarnixUiProvider, TranscriptList } from "../src";
import { FakeClient } from "./fake-client";

afterEach(() => cleanup());

function mount(node: React.ReactNode) {
  return render(<HarnixUiProvider client={new FakeClient().asClient()}>{node}</HarnixUiProvider>);
}

const BLOCKS: Block[] = [
  { key: 1, kind: "user", text: "【简报】集成 SR-001 · 第 1 轮" },
  { key: 2, kind: "tool", id: "t1", name: "Bash", input: "git merge --no-ff ipdrun/SR-001/dev", result: "Merge made by the 'ort' strategy", status: "ok", startedAt: 0, endedAt: 600 },
  { key: 3, kind: "tool", id: "t2", name: "Exec", input: "执行器（内部调用）V-0003", result: "2 passed; 1 failed", status: "error", startedAt: 0, endedAt: 1350 },
  { key: 4, kind: "assistant", text: "第 1 轮有 1 个失败，交给 attributor（见 SR-001）。", streaming: false },
  { key: 5, kind: "note", text: "已合入 main。", tone: "info" },
  { key: 6, kind: "tool", id: "t3", name: "Exec", input: "执行器（内部调用）V-0004", status: "running" },
];

describe("TranscriptList", () => {
  it("同一套行：工具行（状态、名字、参数、耗时、摘要）、正文、系统行；在跑 ⇒ 工作中", () => {
    mount(<TranscriptList blocks={BLOCKS} running workingText="集成中…" toolNames={{ Exec: "Exec" }} />);
    const flow = screen.getByTestId("conv-flow");
    expect(flow.className).toContain("hx-cli");
    expect(flow.dataset.state).toBe("running");
    const tools = within(flow).getAllByTestId("conv-tool");
    expect(tools.map((t) => t.dataset.state)).toEqual(["ok", "error", "running"]);
    expect(within(tools[0]!).getByTestId("conv-tool-name").textContent).toBe("Bash");
    expect(tools[0]!.textContent).toContain("git merge --no-ff ipdrun/SR-001/dev");
    expect(tools[0]!.textContent).toContain("0.6s");
    expect(within(tools[1]!).getByTestId("conv-tool-summary").textContent).toContain("2 passed; 1 failed");
    expect(flow.querySelector('[data-testid="conv-message"][data-role="assistant"]')!.textContent).toContain("第 1 轮有 1 个失败");
    expect(within(flow).getByTestId("conv-note").textContent).toBe("已合入 main。");
    expect(within(flow).getByTestId("conv-typing").textContent).toContain("集成中…");
  });

  it("人的消息走贡献点；编号 chip 可点；工具行点开看输入与结果", () => {
    const open = vi.fn();
    mount(
      <TranscriptList
        blocks={BLOCKS.slice(0, 4)}
        ids={{ pattern: /\bSR-\d+\b/g, onOpen: open }}
        renderers={{ userRows: [{ id: "seg", match: (b) => b.text.startsWith("【简报】"), render: (b) => <div data-testid="seg">{b.text.slice(4)}</div> }] }}
      />,
    );
    expect(screen.getByTestId("seg").textContent).toBe("集成 SR-001 · 第 1 轮");
    const chip = screen.getAllByTestId("conv-chip").find((c) => c.dataset.id === "SR-001")!;
    fireEvent.click(chip);
    expect(open).toHaveBeenCalledWith("SR-001");
    const tool = screen.getAllByTestId("conv-tool")[0]!;
    expect(tool.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(tool);
    expect(tool.getAttribute("aria-expanded")).toBe("true");
    expect(screen.queryByTestId("conv-typing")).toBeNull();
  });

  it("没有块 ⇒ 空的说明", () => {
    mount(<TranscriptList blocks={[]} emptyState={<p data-testid="nothing">还没有转录。</p>} />);
    expect(screen.getByTestId("conv-empty")).toBeTruthy();
    expect(screen.getByTestId("nothing").textContent).toBe("还没有转录。");
  });
});
