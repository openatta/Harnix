/**
 * TranscriptList：只读的转录（原型 cliPane 的 .cli）。行组件与 ChatPanel 的 transcript 样子是同一套——
 * .cc-seg 任务分段（应用经 userRows 认领）· .cc-row ● 段落 · .cc-tool 工具行 + ⎿ 摘要 + diff · .cc-sys 系统行 · .cc-work 工作中；
 * 不同的是数据来自 props（块表由应用给，例如从自己的转录接口映射过来），不挂 Harnix 会话、不归约事件流。
 *
 * 用于没有实时流、靠重新拉取刷新的转录（例如确定性步骤记成工具行的工位）。外面的框（抬头、固定区、输入框）由应用自己摆，
 * 这里只是消息流那一块：贴着底部时新内容到了自动滚到底。
 */

import { useCallback, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { useStrings } from "../context";
import { Agent, ContextRow, ImageBlock } from "../components/blocks";
import type { Block } from "@harnix/core/state";
import { TOOL_NAMES } from "./format";
import { SystemLine, ThinkRow, TranscriptText, TranscriptToolRow, Typing, UserBubble } from "./messages";
import { ChatContext, type ChatContextValue, type ChatIds, type ChatRenderers } from "./types";

export interface TranscriptListProps {
  /** 块表（自上而下）；key 在表内唯一且稳定 */
  blocks: readonly Block[];
  /** 在跑：末尾一行"工作中…" */
  running?: boolean;
  /** "工作中"一行的文字（缺省 strings.working） */
  workingText?: string;
  /** "工作中"一行右侧的小字 */
  workingHint?: string;
  /** 末尾、"工作中"之前的内容 */
  tail?: ReactNode;
  /** 没有块时显示的东西 */
  emptyState?: ReactNode;
  /** 正文里的编号变 chip，可点 */
  ids?: ChatIds;
  /** 工具的友好名字（覆盖或补充内建的表） */
  toolNames?: Record<string, string>;
  /** 工具行、人的消息的贡献点与工具摘要（同 ChatPanel） */
  renderers?: ChatRenderers;
  /** "收起所有 diff"（应用的开关） */
  diffsCollapsed?: boolean;
  testId?: string;
  className?: string;
}

export function TranscriptList(props: TranscriptListProps) {
  const strings = useStrings();
  const { blocks, renderers, ids } = props;
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const collapsed = props.diffsCollapsed ?? false;
  // 开关每切一次，各行自己点过的开合作废（与 ChatPanel 的 diffGeneration 同一规则）
  const [generation, setGeneration] = useState({ collapsed, n: 0 });
  if (generation.collapsed !== collapsed) setGeneration({ collapsed, n: generation.n + 1 });

  const toolNames = useMemo(() => ({ ...TOOL_NAMES, ...props.toolNames }), [props.toolNames]);
  const context = useMemo<ChatContextValue>(
    () => ({
      variant: "transcript",
      toolNames,
      ids,
      diffsCollapsed: collapsed,
      diffGeneration: generation.n,
      toolSummary: renderers?.toolSummary,
    }),
    [toolNames, ids, collapsed, generation.n, renderers?.toolSummary],
  );

  const onScroll = useCallback(() => {
    const node = scroller.current;
    if (node) pinned.current = node.scrollHeight - node.scrollTop - node.clientHeight < 140;
  }, []);

  useLayoutEffect(() => {
    const node = scroller.current;
    if (node && pinned.current) node.scrollTop = node.scrollHeight;
  }, [blocks, props.running, props.tail]);

  // 编号 chip：点或回车 ⇒ 应用去打开它
  const openChip = (target: EventTarget | null) => {
    if (!ids?.onOpen || !(target instanceof Element)) return false;
    const chip = target.closest<HTMLElement>('[data-testid="conv-chip"]');
    if (!chip?.dataset.id || (ids.canOpen && !ids.canOpen(chip.dataset.id))) return false;
    ids.onOpen(chip.dataset.id);
    return true;
  };
  const onClick = (e: MouseEvent) => {
    if (openChip(e.target)) {
      e.preventDefault();
      e.stopPropagation();
    }
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && openChip(e.target)) e.preventDefault();
  };

  const empty = blocks.length === 0 && !props.running && !props.tail;
  return (
    <ChatContext.Provider value={context}>
      <div
        ref={scroller}
        onScroll={onScroll}
        className={`hx-cli${props.className ? ` ${props.className}` : ""}`}
        data-testid={props.testId ?? "conv-flow"}
        data-state={props.running ? "running" : "idle"}
        data-count={blocks.length}
        role="log"
        aria-live="polite"
        aria-relevant="additions"
        onClick={onClick}
        onKeyDown={onKeyDown}
      >
        {empty ? (
          <div data-testid="conv-empty" className="hx-blk">
            {props.emptyState ?? <div className="hx-placeholder">{strings.empty}</div>}
          </div>
        ) : null}
        {blocks.map((block) => (
          <Row key={block.key} block={block} renderers={renderers} />
        ))}
        {props.tail}
        {props.running ? <Typing transcript text={props.workingText ?? strings.working} hint={props.workingHint} /> : null}
      </div>
    </ChatContext.Provider>
  );
}

/** 转录里的一块（与 ChatPanel 转录的 FlowItem 同样的分派；没有会话，权限卡不画） */
function Row({ block, renderers }: { block: Block; renderers?: ChatRenderers }) {
  switch (block.kind) {
    case "user": {
      const rows = renderers?.userRows ?? [];
      for (let i = rows.length - 1; i >= 0; i -= 1) {
        const contribution = rows[i]!;
        if (contribution.match(block)) return <>{contribution.render(block)}</>;
      }
      return <UserBubble block={block} />;
    }
    case "assistant":
      return <TranscriptText block={block} extras={renderers?.assistantExtras?.(block)} />;
    case "think":
      return (
        <div className="hx-blk hx-cc-row">
          <span className="hx-cc-b" aria-hidden />
          <div className="hx-cc-text hx-think">
            <ThinkRow block={block} />
          </div>
        </div>
      );
    case "tool": {
      const rows = renderers?.toolRows ?? [];
      for (let i = rows.length - 1; i >= 0; i -= 1) {
        const contribution = rows[i]!;
        if (contribution.match(block)) return <>{contribution.render(block)}</>;
      }
      return <TranscriptToolRow block={block} />;
    }
    case "note":
      return <SystemLine block={block} transcript />;
    case "context":
      return <ContextRow block={block} />;
    case "image":
      return <ImageBlock block={block} />;
    case "agent":
      return <Agent block={block} />;
    default:
      return null;
  }
}
