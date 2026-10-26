/**
 * 对话流里的各种块。每种块一个 memo 组件：块对象引用不变就不重画。
 *
 * 对应 旧界面 ui/builtin/flow-blocks.js 与 tool-rows.js。Nest 的 `flow.block` /
 * `tool.row` 贡献点在这里是 `toolRows` 注册表（后注册的先匹配），内建的工具行是兜底。
 *
 * 可测性（IPDrun XD64、nest-sync R-09）：可交互元素与断言区域带 data-testid，带编号的
 * 带 data-id，有状态的带 data-state。
 */

import { memo, useEffect, useState, type ReactNode } from "react";
import type { Decision } from "@harnix/core";
import { useStrings } from "../context";
import { duration, toolArgs, toolDisplayName } from "../chat/format";
import { useChat } from "../chat/types";
import type { AskAnswer, BlockOf } from "@harnix/core/state";
import type { SessionController } from "@harnix/core/state";

/** 截断，并说明截掉了多少。 */
export function clip(text: string, n: number): string {
  return text.length > n ? `${text.slice(0, n)} … (+${text.length - n})` : text;
}

/** 一行里最值得显示的那个参数。 */
export function argumentSummary(input: unknown): string {
  if (input == null) return "";
  if (typeof input === "string") return clip(input, 160);
  if (typeof input === "object") {
    const record = input as Record<string, unknown>;
    for (const key of ["command", "file_path", "path", "pattern", "query", "url", "description", "prompt"]) {
      if (record[key]) return clip(String(record[key]).replace(/\s+/g, " "), 160);
    }
  }
  return clip(JSON.stringify(input), 160);
}

/* ── 工具行的注册表（tool.row） ───────────────────────────────────────── */

export interface ToolRowContribution {
  id: string;
  match(block: BlockOf<"tool">): boolean;
  render(block: BlockOf<"tool">): ReactNode;
}

/**
 * 人的消息的注册表（Nest 的 `flow.block` 贡献点的一部分）：后注册的先匹配，没人认领用内建的气泡。
 * 例如把服务端代发的长篇任务简报折成一行。
 */
export interface UserRowContribution {
  id: string;
  match(block: BlockOf<"user">): boolean;
  render(block: BlockOf<"user">): ReactNode;
}

/* ── 各种块 ───────────────────────────────────────────────────────────── */

export const ContextRow = memo(function ContextRow({ block }: { block: BlockOf<"context"> }) {
  const strings = useStrings();
  const summary = clip(block.text.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim(), 120);
  return (
    <Disclosure testId="conv-context" summary={`${strings.injectedContext} · ${summary}`} state="done">
      <pre className="hx-pre">{block.text}</pre>
    </Disclosure>
  );
});

export const ImageBlock = memo(function ImageBlock({ block }: { block: BlockOf<"image"> }) {
  return (
    <div className="hx-blk" data-testid="conv-image">
      <img className="hx-image" src={block.src} alt="" />
    </div>
  );
});

export const Agent = memo(function Agent({ block }: { block: BlockOf<"agent"> }) {
  const strings = useStrings();
  return (
    <div className="hx-blk hx-agent" data-testid="conv-agent" data-id={block.id} data-state={block.state}>
      <div className="hx-agent-head">
        <span className="hx-tag">{strings.subagent}</span>
        <span className="hx-truncate">{block.label}</span>
        {block.state === "complete" ? <span className="hx-pill">{strings.subagentDone}</span> : <span className="hx-spinner" aria-hidden />}
      </div>
      <div className="hx-agent-lines">
        {block.lines.slice(-40).map((line, index) => (
          <div key={index} className="hx-truncate">
            {clip(line.text, 200)}
          </div>
        ))}
      </div>
    </div>
  );
});

/** 一行可展开的行：一行高，点开看内容。 */
export function Disclosure({
  testId,
  id,
  state,
  summary,
  tail,
  children,
}: {
  testId: string;
  id?: string;
  state: string;
  summary: ReactNode;
  tail?: ReactNode;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="hx-blk hx-row" data-testid={testId} data-id={id} data-state={state}>
      <button
        type="button"
        className="hx-row-head"
        data-testid={`${testId}-toggle`}
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <span className="hx-chevron" aria-hidden>
          {open ? "▾" : "▸"}
        </span>
        <span className="hx-row-summary hx-truncate">{summary}</span>
        {tail ? <span className="hx-row-tail">{tail}</span> : null}
      </button>
      {open ? <div className="hx-row-body">{children}</div> : null}
    </div>
  );
}

/** 工具调用的状态符号（界面设计 §8.2：✓ 完成、✕ 失败、● 运行中、⏸ 已中断） */
export const STATUS_GLYPH: Record<string, string> = { running: "●", ok: "✓", error: "✕", interrupted: "⏸" };

/** 实时量到的耗时（从历史读回的没有） */
export function toolDuration(block: BlockOf<"tool">): string {
  return block.startedAt != null && block.endedAt != null ? duration(block.endedAt - block.startedAt) : "";
}

/** 内建的工具行（会话的样子：一行高，点开看输入与结果）：认领一切工具，排在注册表最后。转录用 TranscriptToolRow。 */
export const BuiltinToolRow = memo(function BuiltinToolRow({ block }: { block: BlockOf<"tool"> }) {
  const strings = useStrings();
  const { toolNames } = useChat();
  const tail =
    block.status === "running"
      ? strings.toolRunning
      : block.status === "error"
        ? strings.toolFailed
        : block.status === "interrupted"
          ? strings.toolInterrupted
          : toolDuration(block);
  return (
    <Disclosure
      testId="conv-tool"
      id={block.id}
      state={block.status}
      summary={
        <>
          <span className={`hx-status hx-status-${block.status}`} aria-hidden>
            {STATUS_GLYPH[block.status]}
          </span>
          <span className="hx-tool-name" data-testid="conv-tool-name" title={block.name}>
            {toolDisplayName(block.name, toolNames)}
          </span>
          <span className="hx-tool-arg">{toolArgs(block.input)}</span>
        </>
      }
      tail={tail}
    >
      <ToolDetails block={block} />
    </Disclosure>
  );
});

/** 展开后的输入与结果 */
export function ToolDetails({ block }: { block: BlockOf<"tool"> }) {
  const strings = useStrings();
  return (
    <>
      {block.input !== undefined ? (
        <Labelled testId="conv-tool-input" label={strings.toolInput}>
          {typeof block.input === "string" ? block.input : JSON.stringify(block.input, null, 2)}
        </Labelled>
      ) : null}
      {block.result != null ? (
        <Labelled testId="conv-tool-result" label={block.error ? strings.toolError : strings.toolResult}>
          {clip(block.result, 4000)}
        </Labelled>
      ) : null}
    </>
  );
}

function Labelled({ label, testId, children }: { label: string; testId: string; children: string }) {
  return (
    <div className="hx-labelled" data-testid={testId}>
      <div className="hx-label">{label}</div>
      <pre className="hx-pre">{children}</pre>
    </div>
  );
}

/* ── 权限卡 ───────────────────────────────────────────────────────────── */

export const AskCard = memo(function AskCard({ block, controller }: { block: BlockOf<"ask">; controller: SessionController }) {
  const strings = useStrings();
  const left = useCountdown(block.deadline, !block.answered);
  useEffect(() => {
    if (!block.answered && left <= 0) controller.expire(block.key);
  }, [block.answered, block.key, controller, left]);

  const prompt = block.prompt;
  const tool = prompt.tool_name ?? "";
  const give = (decision: Decision) => void controller.answer(block.key, decision);
  const answered: Record<AskAnswer, string> = {
    allow: strings.permDecisionAllow,
    deny: strings.permDecisionDeny,
    timeout: strings.permDecisionTimeout,
  };
  const state = block.answered ? (block.answered === "timeout" ? "expired" : "answered") : "pending";

  // 任一观看者都能答、先答者生效、第二个回答静默成功：所以按钮在每个打开的页面上都可用。
  return (
    <div
      className={`hx-blk hx-ask${block.answered ? " hx-ask-done" : ""}`}
      data-testid="conv-ask"
      data-id={prompt.prompt_id}
      data-state={state}
      data-answer={block.answered}
      role="group"
      aria-label={strings.permRequest(tool)}
    >
      <div className="hx-ask-head">
        {block.answered ? strings.permAnswered(answered[block.answered], tool) : strings.permRequest(tool)}
      </div>
      {prompt.message ? <div className="hx-ask-msg">{prompt.message}</div> : null}
      {prompt.paths?.length ? <pre className="hx-pre">{prompt.paths.join("\n")}</pre> : null}
      {block.answered ? null : (
        <div className="hx-ask-actions">
          <button type="button" className="hx-btn hx-btn-primary" data-testid="conv-ask-allow" onClick={() => give({ type: "permit" })}>
            {strings.permAllowOnce}
          </button>
          <button
            type="button"
            className="hx-btn"
            data-testid="conv-ask-allow-session"
            onClick={() => give({ type: "permit_always", scope: "session" })}
          >
            {strings.permAllowSession}
          </button>
          <button
            type="button"
            className="hx-btn"
            data-testid="conv-ask-allow-local"
            title={strings.permAllowLocalHint}
            onClick={() => give({ type: "permit_always", scope: "local" })}
          >
            {strings.permAllowLocal}
          </button>
          <button
            type="button"
            className="hx-btn hx-btn-danger"
            data-testid="conv-ask-deny"
            onClick={() => give({ type: "deny", reason: "user denied" })}
          >
            {strings.permDeny}
          </button>
          {/* 倒计时不是装饰：没有它，等待未答提问的会话看起来像卡住了，其实正走向一次静默的拒绝。 */}
          <span className="hx-countdown" data-testid="conv-ask-countdown" aria-live="off">
            {strings.permCountdown(Math.max(0, left))}
          </span>
        </div>
      )}
    </div>
  );
});

function useCountdown(deadline: number, running: boolean): number {
  const compute = () => Math.ceil((deadline - Date.now()) / 1000);
  const [left, setLeft] = useState(compute);
  useEffect(() => {
    if (!running) return;
    setLeft(compute());
    const timer = setInterval(() => setLeft(compute()), 1000);
    return () => clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deadline, running]);
  return left;
}
