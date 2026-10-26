/**
 * 对话流里的消息：逐个对应原型 docs/ui/prototype/ipdrun-prototype.html 的 msgItem / typingHTML / cliLine / diffHTML，
 * DOM 结构与类名照原型（加 hx- 前缀），样式在 styles.css 的同名规则里。
 *
 *   你的话     .cv-u > .cv-ub：右侧浅底圆角气泡；编号是 chip；排队中的在气泡里多一行 .q
 *   AI 的话    .cv-a：.cv-ah（头像 + 名字 + 时间）· .cv-think · .cv-at（正文直排）· .cv-act（悬停出现复制、重新生成）；
 *              同一角色连续发言（中间夹着工具行、提议卡）是一组，抬头只出一次
 *   系统事件   .cv-sys：两侧细线的居中小字
 *   正在回复   .cv-a > .cv-typing：旋转的小方块 + "正在回复"
 *
 * 转录（工位，原型 cliLine）：.cc-seg 任务分段；.cc-row（● + .cc-text）agent 的话；.cc-row.cc-tool 工具行
 * （.cc-call `Update(path)  1.3s` / .cc-out `⎿ 摘要`，编辑类可展开 .cc-diff）；.cc-sys 灰色系统行；.cc-work 工作中。
 * 圆点颜色表示状态：完成绿、运行中闪烁、失败红、中断黄。会话里的工具行用同一种样子。
 *
 * 可测性：消息正文是 data-testid=conv-message（只含正文，不含头像、名字、时间、按钮）。
 */

import { memo, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";
import { useStrings } from "../context";
import { markdown } from "../markdown";
import type { BlockOf, ToolStatus } from "@harnix/core/state";
import { ToolDetails, toolDuration } from "../components/blocks";
import { chipifyHtml, clockTime, editOf, numberedDiff, splitIds, toolArgs, toolDisplayName, toolSummary } from "./format";
import { Icon } from "./icons";
import { useChat, type ChatRole } from "./types";

/* ── 头像与助手的抬头 ─────────────────────────────────────────────── */

export function Avatar({ role, size }: { role: ChatRole; size?: "sm" }) {
  return (
    <span
      className={`hx-av${size === "sm" ? " hx-av-sm" : ""}`}
      style={role.color ? ({ "--hx-av-c": role.color } as CSSProperties) : undefined}
      aria-hidden
    >
      {role.avatar}
    </span>
  );
}

/** 一组助手发言的抬头（原型 .cv-ah）：头像 + 名字 + 时间 */
export function AssistantHead({ role, at }: { role: ChatRole; at?: number }) {
  return (
    <div className="hx-cv-ah" data-testid="conv-speaker">
      <Avatar role={role} />
      <b>{role.name}</b>
      {at != null ? <span className="hx-faint hx-num">{clockTime(at)}</span> : null}
    </div>
  );
}

/* ── 人的话 ───────────────────────────────────────────────────────── */

/** 纯文本里的编号变 chip（React 元素，文字不变） */
export function WithIds({ text }: { text: string }) {
  const { ids } = useChat();
  const parts = splitIds(text, ids?.pattern);
  if (parts.length === 1 && "text" in parts[0]!) return <>{text}</>;
  const clickableOf = (id: string) => Boolean(ids?.onOpen) && (ids?.canOpen ? ids.canOpen(id) : true);
  return (
    <>
      {parts.map((p, i) =>
        "text" in p ? (
          <span key={i}>{p.text}</span>
        ) : (
          <span
            key={i}
            className={`hx-chip${clickableOf(p.id) ? " hx-chip-link" : ""}`}
            data-testid="conv-chip"
            data-id={p.id}
            role={clickableOf(p.id) ? "link" : undefined}
            tabIndex={clickableOf(p.id) ? 0 : undefined}
          >
            {p.id}
          </span>
        ),
      )}
    </>
  );
}

export const UserBubble = memo(function UserBubble({ block }: { block: BlockOf<"user"> }) {
  const strings = useStrings();
  return (
    <div className="hx-blk hx-cv-u">
      <div
        className="hx-cv-ub"
        data-testid="conv-message"
        data-role="user"
        data-state={block.steered ? "steered" : "done"}
        data-item={block.itemId}
      >
        {/* 运行中送来、在这一轮的步骤边界进了对话（IPDrun N-108） */}
        {block.steered ? (
          <div className="hx-cv-q hx-cv-steered" data-testid="conv-steered">
            <span>{strings.steerDelivered}</span>
          </div>
        ) : null}
        <WithIds text={block.text} />
      </div>
    </div>
  );
});

/**
 * 还没送达的话（原型 cliLine 的 queued：气泡里多一行"排队中 · 当前步骤结束后送达"）。
 * 应用自己的排队（例如工位插话）也用它，经 ChatPanel 的 flowTail 放进流里。
 */
export function QueuedBubble({
  text,
  note,
  testId,
  id,
  action,
  state,
}: {
  text: string;
  /** 气泡上方那一行小字；缺省"排队中 · 当前步骤结束后送达" */
  note?: ReactNode;
  testId?: string;
  id?: string;
  /** 那一行小字后面的按钮（例如撤回） */
  action?: ReactNode;
  /** data-state：缺省 queued；交给正在跑的一轮、待送达的是 steering（IPDrun N-108） */
  state?: "queued" | "steering";
}) {
  const strings = useStrings();
  return (
    <div className="hx-blk hx-cv-u" data-testid={testId} data-id={id} data-state={state ?? "queued"}>
      <div className="hx-cv-ub">
        <div className="hx-cv-q">
          <span>{note ?? strings.queuedInFlow}</span>
          {action}
        </div>
        <WithIds text={text} />
      </div>
    </div>
  );
}

/* ── 助手的话 ─────────────────────────────────────────────────────── */

function useMessageHtml(text: string): string {
  const { ids } = useChat();
  const canOpen = ids?.canOpen;
  const onOpen = Boolean(ids?.onOpen);
  return chipifyHtml(markdown(text), ids?.pattern, (id) => onOpen && (canOpen ? canOpen(id) : true));
}

export function CopyButton({ text }: { text: string }) {
  const strings = useStrings();
  const [done, setDone] = useState(false);
  const copy = () => {
    const clip = typeof navigator !== "undefined" ? navigator.clipboard : undefined;
    if (!clip) return;
    clip.writeText(text).then(
      () => {
        setDone(true);
        setTimeout(() => setDone(false), 1200);
      },
      () => undefined,
    );
  };
  return (
    <button type="button" data-testid="conv-copy" title={done ? strings.copied : strings.copy} aria-label={strings.copy} onClick={copy}>
      {done ? <Icon.check /> : <Icon.copy />}
    </button>
  );
}

/** 一组发言下面的"复制""重新生成"（原型 .cv-act；重新生成要后端，画成不可用） */
export function MessageActions({ text }: { text: string }) {
  const strings = useStrings();
  return (
    <div className="hx-cv-act">
      <CopyButton text={text} />
      <button type="button" data-testid="conv-retry" disabled title={strings.retry} aria-label={strings.retry}>
        <Icon.retry />
      </button>
    </div>
  );
}

/** 会话的样子：一段正文（原型 .cv-at），不加气泡 */
export const AssistantText = memo(function AssistantText({ block, extras }: { block: BlockOf<"assistant">; extras?: ReactNode }) {
  const html = useMessageHtml(block.text);
  return (
    <>
      <div
        className="hx-cv-at hx-md"
        data-testid="conv-message"
        data-role="assistant"
        data-state={block.streaming ? "streaming" : "done"}
        aria-busy={block.streaming || undefined}
        dangerouslySetInnerHTML={{ __html: html }}
      />
      {extras}
    </>
  );
});

/** 转录的样子：前面一个圆点（原型 cliLine 的 agent 的话） */
export const TranscriptText = memo(function TranscriptText({ block, extras }: { block: BlockOf<"assistant">; extras?: ReactNode }) {
  const html = useMessageHtml(block.text);
  return (
    <div className="hx-blk hx-cc-row">
      <span className="hx-cc-b" aria-hidden>
        ●
      </span>
      <div className="hx-cc-text">
        <div
          className="hx-md"
          data-testid="conv-message"
          data-role="assistant"
          data-state={block.streaming ? "streaming" : "done"}
          aria-busy={block.streaming || undefined}
          dangerouslySetInnerHTML={{ __html: html }}
        />
        {extras}
      </div>
    </div>
  );
});

/* ── 思考 ─────────────────────────────────────────────────────────── */

/** "思考了 N 秒"一行（原型 .cv-think），点开看思考的内容（.cv-thtext） */
export const ThinkRow = memo(function ThinkRow({ block }: { block: BlockOf<"think"> }) {
  const strings = useStrings();
  const [open, setOpen] = useState(false);
  const seconds =
    block.startedAt != null && block.endedAt != null ? Math.max(1, Math.round((block.endedAt - block.startedAt) / 1000)) : null;
  const label = block.streaming ? strings.thinkingLive : seconds != null ? strings.thoughtFor(seconds) : strings.thinking;
  // 还在想：与"正在回复"同一个样子（原型 typingHTML 的"正在思考"）
  if (block.streaming)
    return (
      <div className="hx-cv-typing" data-testid="conv-think" data-state="streaming">
        <span className="hx-spark" aria-hidden />
        <span data-testid="conv-think-toggle">{label}</span>
      </div>
    );
  return (
    <>
      <button
        type="button"
        className="hx-cv-think"
        data-testid="conv-think"
        data-state={block.streaming ? "streaming" : "done"}
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <Icon.think />
        <span data-testid="conv-think-toggle">{label}</span>
        <span className={`hx-cv-chev${open ? " open" : ""}`}>
          <Icon.down />
        </span>
      </button>
      {open ? <div className="hx-cv-thtext hx-md" dangerouslySetInnerHTML={{ __html: markdown(block.text) }} /> : null}
    </>
  );
});

/* ── 系统事件 ─────────────────────────────────────────────────────── */

export const SystemLine = memo(function SystemLine({ block, transcript }: { block: BlockOf<"note">; transcript?: boolean }) {
  return (
    <div
      className={`hx-blk ${transcript ? "hx-cc-sys" : "hx-cv-sys"} hx-tone-${block.tone}`}
      data-testid="conv-note"
      data-state={block.tone}
      role={block.tone === "error" ? "alert" : undefined}
    >
      <span>{block.text}</span>
    </div>
  );
});

/* ── 正在回复 ─────────────────────────────────────────────────────── */

export function Typing({ head, text, transcript, hint }: { head?: ReactNode; text: string; transcript?: boolean; hint?: string }) {
  if (transcript) {
    return (
      <div className="hx-blk hx-cc-work" data-testid="conv-typing">
        <span className="hx-spark" aria-hidden />
        <span>{text}</span>
        {hint ? <span className="hx-faint">{hint}</span> : null}
      </div>
    );
  }
  return (
    <div className="hx-blk hx-cv-a" data-testid="conv-typing">
      {head}
      <div className="hx-cv-typing">
        <span className="hx-spark" aria-hidden />
        <span>{text}</span>
      </div>
    </div>
  );
}

/* ── 工具行 ───────────────────────────────────────────────────────── */

const DIFF_MAX_LINES = 400;

/** 带行号的 diff（原型 diffHTML：.cc-diff > .cc-dh 文件 · .cc-dw > .cc-dr > .dl 行） */
export function DiffView({ path, pairs }: { path: string; pairs: { old: string; new: string }[] }) {
  const lines = pairs.flatMap((p, i) => {
    const rows = numberedDiff(p.old, p.new);
    return i > 0 ? [{ op: "gap" as const, text: "…" }, ...rows] : rows;
  });
  const shown = lines.slice(0, DIFF_MAX_LINES);
  return (
    <div className="hx-cc-diff" data-testid="conv-tool-diff" onClick={(e) => e.stopPropagation()}>
      <div className="hx-cc-dh">
        <Icon.file />
        <span>{path}</span>
      </div>
      <div className="hx-cc-dw">
        <div className="hx-cc-dr">
          {shown.map((l, i) =>
            l.op === "gap" ? (
              <div key={i} className="hx-dl gap">
                <span className="ln" />
                <span className="ln" />
                <span className="sg" />
                <code>…</code>
              </div>
            ) : (
              <div key={i} className={`hx-dl ${l.op === "add" ? "a" : l.op === "del" ? "d" : "c"}`} data-testid="conv-diff-line" data-op={l.op}>
                <span className="ln">{"oldNo" in l && l.op !== "add" ? l.oldNo : ""}</span>
                <span className="ln">{"newNo" in l && l.op !== "del" ? l.newNo : ""}</span>
                <span className="sg">{l.op === "add" ? "+" : l.op === "del" ? "−" : ""}</span>
                <code>{l.text}</code>
              </div>
            ),
          )}
          {lines.length > shown.length ? (
            <div className="hx-dl gap">
              <span className="ln" />
              <span className="ln" />
              <span className="sg" />
              <code>… +{lines.length - shown.length}</code>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

/** 圆点的颜色类（原型 .cc-b.ok / .run / .x / .int） */
const BULLET: Record<ToolStatus, string> = { ok: "ok", running: "run", error: "x", interrupted: "int" };

/**
 * 工具行（原型 cliLine 的 tool）：`● Update(path)  1.3s` / `⎿ 增加 3 行，删除 1 行`。编辑类可展开 diff（缺省展开，
 * "收起所有 diff"后缺省收起）；其余展开看输入与结果。整行可点（原型 data-act="toolOpen"）。
 * 转录与会话共用这一种样子。
 */
export const TranscriptToolRow = memo(function TranscriptToolRow({ block }: { block: BlockOf<"tool"> }) {
  const strings = useStrings();
  const { toolNames, diffsCollapsed, diffGeneration, toolSummary: domainSummary } = useChat();
  const [local, setLocal] = useState<{ generation: number; open: boolean } | null>(null);
  const edit = editOf(block);
  const fallback = edit ? !diffsCollapsed : false;
  const open = local && local.generation === diffGeneration ? local.open : fallback;
  const can = block.status !== "running" && (edit != null || block.input !== undefined || block.result != null);
  const toggle = () => setLocal({ generation: diffGeneration, open: !open });
  const onKey = (e: KeyboardEvent) => {
    if (e.target !== e.currentTarget || (e.key !== "Enter" && e.key !== " ")) return;
    e.preventDefault();
    toggle();
  };
  const summary = (block.status === "ok" && domainSummary?.(block)) || toolSummary(block, {
    running: strings.toolRunning,
    interrupted: strings.toolInterrupted,
    done: strings.toolDone,
    readLines: strings.readLines,
    editLines: strings.editLines,
    wroteLines: strings.wroteLines,
    results: strings.results,
  });
  return (
    <div
      className="hx-blk hx-cc-row hx-cc-tool"
      data-testid="conv-tool"
      data-id={block.id}
      data-state={block.status}
      data-diff={edit ? "1" : undefined}
      data-act={can ? "toolOpen" : undefined}
      role={can ? "button" : undefined}
      tabIndex={can ? 0 : undefined}
      aria-expanded={can ? open : undefined}
      onClick={can ? toggle : undefined}
      onKeyDown={can ? onKey : undefined}
    >
      <span className={`hx-cc-b ${BULLET[block.status]}`} role="img" aria-label={block.status}>
        ●
      </span>
      <div className="hx-cc-text">
        <div className="hx-cc-call" data-testid="conv-tool-toggle">
          <b data-testid="conv-tool-name" title={block.name}>
            {toolDisplayName(block.name, toolNames)}
          </b>
          <span className="hx-cc-arg">({toolArgs(block.input)})</span>
          <span className="hx-cc-dur hx-num">{block.status === "running" ? "" : toolDuration(block)}</span>
        </div>
        <div className="hx-cc-out" data-testid="conv-tool-summary">
          <span className="hx-cc-l">⎿</span>
          <span className="hx-cc-o">{summary}</span>
          {can ? <span className="hx-cc-tg">{open ? strings.collapse : strings.expand}</span> : null}
        </div>
        {open ? (
          edit ? (
            <DiffView path={edit.path} pairs={edit.pairs} />
          ) : (
            <div className="hx-cc-more" onClick={(e) => e.stopPropagation()}>
              <ToolDetails block={block} />
            </div>
          )
        ) : null}
      </div>
    </div>
  );
});

/* ── 应用可用的小件 ───────────────────────────────────────────────── */

/**
 * 转录里的任务分段（原型 .cc-seg）：一条细线 + [任务] 粗体一行，可展开看全文。
 * 应用经 userRows 把服务端代发的简报认领成它。
 */
export function SegmentRow({
  tag,
  title,
  body,
  testId = "conv-segment",
  meta,
}: {
  tag: string;
  title: ReactNode;
  /** 展开后的全文（markdown） */
  body?: string;
  testId?: string;
  meta?: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="hx-blk hx-cc-seg-wrap" data-testid={testId} data-state={open ? "open" : "closed"}>
      <button
        type="button"
        className="hx-cc-seg"
        data-testid={`${testId}-toggle`}
        aria-expanded={open}
        disabled={!body}
        onClick={() => setOpen((v) => !v)}
      >
        <span className="hx-tag hx-tag-run">{tag}</span>
        <span>{title}</span>
        {meta ? <span className="hx-faint hx-cc-seg-meta">{meta}</span> : null}
        {body ? (
          <span className={`hx-cv-chev hx-faint${open ? " open" : ""}`}>
            <Icon.down />
          </span>
        ) : null}
      </button>
      {open && body ? <div className="hx-cc-seg-body hx-md" dangerouslySetInnerHTML={{ __html: markdown(body) }} /> : null}
    </div>
  );
}

/** 输入框上方的提醒条 / 警示条（原型 .cmp-note；notice、guard 用） */
export function ComposerNote({
  tone = "info",
  icon,
  children,
  testId,
  role,
  state,
}: {
  tone?: "info" | "warn" | "bad";
  icon?: "shield" | "book" | false;
  children: ReactNode;
  testId?: string;
  role?: string;
  state?: string;
}) {
  const I = icon === "shield" ? Icon.shield : icon === "book" ? Icon.book : null;
  return (
    <div className={`hx-cmp-note${tone !== "info" ? ` hx-cmp-note-${tone}` : ""}`} data-testid={testId} data-state={state} role={role}>
      {I ? <I /> : null}
      {children}
    </div>
  );
}
