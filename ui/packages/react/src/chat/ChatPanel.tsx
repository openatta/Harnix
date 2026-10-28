/**
 * ChatPanel：唯一的会话组件（界面设计 §4.4 D:213——类目会话、条目会话、工位、集成页、⌘K 用同一个组件，
 * 只是配置不同）。DOM 与类名照原型 docs/ui/prototype/ipdrun-prototype.html 的 chatPane / cliPane / composer
 * （加 hx- 前缀），样式是原型同名规则的移植（styles.css）：
 *
 *   .chat
 *     .chead       .ctx（← 面包屑 · 头像 角色 · 状态 · …  操作）· .scope · 已选 [chip…] [清除]
 *     .pinned      固定区（决策卡、"需要你"）
 *     .msgs        conversation：.cv-u 你的话 · .cv-a（.cv-ah 头像 名字 时间 · .cv-at 正文 · 卡片 · .cv-act）· .cv-sys
 *     .cli         transcript：.cc-seg 任务分段 · .cc-row ● 段落 · .cc-tool 工具行 + ⎿ 摘要 + diff · .cc-sys · .cc-work
 *     .cmp         提醒条 · 快捷回复 · .cmp-box（输入区 · 底栏 ＋ ⚙ [上下文环] … 模型 🎤 发送/停止）· 小字
 *
 * 无头的核心是 useSession / SessionController / session/model（attach 追赶、事件归约、发送、中断）；
 * 这里只管样子与配置。sessionId 为 null 时只画框架（抬头、占位、禁用的输入框），会话打开之前用。
 */

import { useCallback, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { useConnection, useHello, useSession, useStrings } from "../context";
import { usageTotals, type Block, type SessionState } from "@harnix/core/state";
import type { SessionController } from "@harnix/core/state";
import { Agent, AskCard, ContextRow, ImageBlock } from "../components/blocks";
import { ConnectionBadge } from "../components/ConnectionBadge";
import { ChatComposer, type Attached, type ComposerApi, type ContextRingConfig, type ModelOption } from "./ChatComposer";
import { TOOL_NAMES } from "./format";
import { Icon } from "./icons";
import {
  AssistantHead,
  AssistantText,
  Avatar,
  MessageActions,
  QueuedBubble,
  SystemLine,
  ThinkRow,
  TranscriptText,
  TranscriptToolRow,
  Typing,
  UserBubble,
} from "./messages";
import {
  ChatContext,
  type ChatCommand,
  type ChatContextValue,
  type ChatFeatures,
  type ChatHeaderConfig,
  type ChatIds,
  type ChatRenderers,
  type ChatRole,
  type ChatVariant,
  type MentionItem,
  type QuickReply,
} from "./types";

export interface ChatPanelProps {
  /** 会话；null ⇒ 还没有会话（正在打开、打不开）：只画框架 */
  sessionId: string | null;
  /** 会话所在的实例；缺省 `system` */
  instance?: string;
  /** 调用方所在的 space，每个调用都带上（设计 §8.5） */
  space?: string;
  /** 发送时加进 `harnix.send` 的参数（例如 `@session` 引用）。要稳定的引用，否则会话会重开 */
  sendExtras?: (text: string) => Record<string, unknown>;
  /** 拿到控制器时告诉调用方（应用往流里放自己的块、外部触发发送时用） */
  onController?: (controller: SessionController | null) => void;
  /** conversation：对话（类目、条目）；transcript：工位转录（编程助手的样子） */
  variant?: ChatVariant;
  /** 回答者（头像、名字） */
  assistant?: ChatRole;
  /** 抬头；false ⇒ 不要抬头 */
  header?: ChatHeaderConfig | false;
  /** 已选的条目（类目会话）：chip + 清除 */
  selection?: { items: readonly string[]; onClear: () => void; label?: ReactNode };
  /** 固定在消息流顶上的内容（决策卡） */
  pinned?: ReactNode;
  /** 没有消息时的说明（缺省：头像 + 名字 + 一句"说点什么开始吧"） */
  emptyState?: ReactNode;
  /** sessionId 为 null 时消息流里显示的东西（正在打开、打不开的理由） */
  unavailable?: ReactNode;
  /** 消息流末尾、"正在回复"之前的内容（例如"已中断"一行） */
  flowTail?: ReactNode;
  /**
   * 排队中的话（中枢的队列，原型"排队中 · 当前步骤结束后送达"的气泡）怎么画：说明、data-testid、能不能撤回；
   * extra = 应用自己知道还没送达的话（例如工位插话：服务端排进同一个队列），中枢的队列里已有同样的话就不重复画
   */
  queue?: { note?: ReactNode; testId?: string; removable?: boolean; extra?: readonly { key: string | number; text: string }[] };
  /** 快捷回复（最多 4 个，经与打字相同的发送路径发出） */
  quickReplies?: readonly QuickReply[];
  /** 输入框上方的提醒条（例如测试工位的"不要透露实现细节"） */
  notice?: ReactNode;
  /** 输入框上方的警示 / 确认（例如入站检查拦下时的"仍然发送"） */
  guard?: ReactNode;
  /** 输入框下方的一行小字 */
  hint?: ReactNode;
  placeholder?: string;
  /** 只读（归档）：输入框换成这句说明；true 用缺省的话 */
  readOnly?: ReactNode | boolean;
  /** 暂时不能发送（例如服务连接断开）：输入框与发送键不可用，已输入的内容保留 */
  disabled?: boolean;
  /**
   * 草稿的键（例如会话或锚点）：每个键各自保留未发送的内容，输入框卸载、换到别的会话再回来仍在（页面内存）。
   * 键变了输入框重新挂上、换成那个键的草稿
   */
  draftKey?: string;
  /**
   * 代替 harnix.send 的发送（例如经应用自己的接口发，好先做检查）。true ⇒ 已发，清空输入框；
   * false ⇒ 没发，输入框保留原文。等待期间输入框只读。快捷回复同样经它。
   */
  send?: (text: string, attachments: unknown[]) => Promise<boolean>;
  /** 输入框的句柄（见 ChatComposer 的 `bind`） */
  composer?: (api: ComposerApi | null) => void;
  /** 上传一个文件作附件；给了输入框才能加附件 */
  upload?: (file: File) => Promise<Attached>;
  /** 代替 session.interrupt 的中断 */
  interrupt?: () => Promise<void> | void;
  /**
   * 在跑（应用知道得更准时给，例如工位的状态 running；原型 cliPane 的 w.busy）：覆盖按 Nest 会话判断的结果。
   * 决定"工作中 / 正在回复"一行与发送键是否变成停止（中断）
   */
  busy?: boolean;
  /** stop：运行中且没有输入时发送键变成"停止"（缺省）；separate：中断与发送是两个按钮 */
  sendMode?: "stop" | "separate";
  /** 底栏的模型（原型 .cmp-model）；缺省取 harnix.hello 的 engine.model */
  model?: string;
  /** 模型菜单里可选的模型（见 ChatComposer 的 `models`） */
  models?: readonly ModelOption[];
  /** 换模型；给了模型菜单才可选 */
  onModel?: (model: ModelOption) => Promise<void> | void;
  /** 底栏右侧、模型前面的额外控件（例如工位的"立即送达"） */
  composerTools?: ReactNode;
  /** 有一轮在跑时发送的话怎么办：queue（缺省）排到这一轮结束；steer 在下一个步骤边界送进这一轮。给了 `send` 时不用它 */
  onBusy?: "queue" | "steer";
  features?: ChatFeatures;
  /** 斜杠命令；缺省取 harnix.hello 的 commands */
  commands?: readonly ChatCommand[];
  /** "@" 的搜索 */
  mention?: (query: string) => Promise<MentionItem[]> | MentionItem[];
  /** 正文里的编号变 chip，可点 */
  ids?: ChatIds;
  /** 工具的友好名字（覆盖或补充内建的表） */
  toolNames?: Record<string, string>;
  renderers?: ChatRenderers;
  className?: string;
}

export function ChatPanel(props: ChatPanelProps) {
  if (!props.sessionId) return <ChatFrame {...props} state={null} controller={null} />;
  return <LiveChat {...props} sessionId={props.sessionId} />;
}

function LiveChat(props: ChatPanelProps & { sessionId: string }) {
  const { state, controller } = useSession(props.sessionId, props.instance, props.space, props.sendExtras);
  const { onController } = props;
  useLayoutEffect(() => {
    onController?.(controller);
    return () => onController?.(null);
  }, [controller, onController]);
  return <ChatFrame {...props} state={state} controller={controller} />;
}

/** 流块贡献认领一块：后注册的先认领，`match` 抛错就当没认领。 */
function claimFlow(renderers: ChatRenderers | undefined, block: Block): ReactNode | undefined {
  const rows = renderers?.flowBlocks ?? [];
  for (let i = rows.length - 1; i >= 0; i -= 1) {
    const contribution = rows[i]!;
    let matched = false;
    try {
      matched = contribution.match(block);
    } catch {
      matched = false;
    }
    if (matched) return contribution.render(block);
  }
  return undefined;
}

const NO_QUICK: readonly QuickReply[] = [];

function ChatFrame(props: ChatPanelProps & { state: SessionState | null; controller: SessionController | null }) {
  const { state, controller } = props;
  const strings = useStrings();
  const hello = useHello();
  const variant = props.variant ?? "conversation";
  const transcript = variant === "transcript";
  const features = props.features ?? {};
  const role = props.assistant ?? { name: strings.assistant, avatar: "AI" };
  const [diffs, setDiffs] = useState({ collapsed: false, generation: 0 });

  const toolNames = useMemo(() => ({ ...TOOL_NAMES, ...props.toolNames }), [props.toolNames]);
  const context = useMemo<ChatContextValue>(
    () => ({
      variant,
      toolNames,
      ids: props.ids,
      diffsCollapsed: diffs.collapsed,
      diffGeneration: diffs.generation,
      toolSummary: props.renderers?.toolSummary,
    }),
    [variant, toolNames, props.ids, diffs, props.renderers?.toolSummary],
  );

  // 应用自己发送（send / interrupt 给了）：一轮由服务端发起，客户端看到的是"别处在跑"，照样算在跑、能停
  const managed = Boolean(props.send || props.interrupt);
  const elsewhere = state?.turn == null && state?.session?.turn_state === "running";
  const running = props.busy ?? (state?.turn != null || (managed && elsewhere));
  // "这个会话正在别处运行"只在组件自己说了算的时候讲（应用给了 busy 或自己发送时，别处在跑就是在跑）
  const otherClient = props.busy === undefined && !managed && elsewhere;
  const viewState = !state ? (props.unavailable ? "unavailable" : "loading") : state.failed ? "failed" : state.loading ? "loading" : running ? "running" : "idle";

  const { send: sendProp, interrupt: interruptProp, onBusy } = props;
  const send = useCallback(
    async (text: string, attachments: unknown[]) => {
      if (sendProp) return sendProp(text, attachments);
      if (!controller) return false;
      if (onBusy) await controller.send(text, attachments, { onBusy });
      else await controller.send(text, attachments);
      return true;
    },
    [sendProp, controller, onBusy],
  );
  const interrupt = useCallback(() => {
    if (interruptProp) {
      controller?.markInterrupting();
      void interruptProp();
    } else void controller?.interrupt();
  }, [interruptProp, controller]);

  const commands = useMemo<readonly ChatCommand[] | null>(() => {
    if (features.slash === false) return null;
    if (props.commands) return props.commands;
    const list = (hello?.commands ?? []) as { name?: unknown; description?: unknown }[];
    return list
      .filter((c) => c && typeof c.name === "string")
      .map((c) => ({ name: String(c.name), description: typeof c.description === "string" ? c.description : undefined }));
  }, [features.slash, props.commands, hello]);

  // 上下文环：原型只在工位里有；会话可以经 features.contextRing 打开
  const ringOn = features.contextRing ?? transcript;
  const ring: ContextRingConfig | null = ringOn === false ? null : typeof ringOn === "object" ? ringOn : {};

  // ⚙ 选项（原型 tools 弹层）：转录里"收起所有 diff"是真的（纯前端）；其余要后端，画成不可用
  const optionsOn = features.options ?? true;
  const options = !optionsOn ? null : transcript ? (
    <>
      <div className="hx-pop-h">{strings.stationOptions}</div>
      <button type="button" className="hx-pop-it" disabled title={strings.needsBackend}>
        <span className="hx-pop-pi">
          <Icon.gauge />
        </span>
        <span className="hx-pop-pt">
          <b>{strings.autoCompact}</b>
          <small>{strings.needsBackend}</small>
        </span>
        <span className="hx-sw" aria-hidden />
      </button>
      <button
        type="button"
        role="switch"
        aria-checked={diffs.collapsed}
        className="hx-pop-it"
        data-testid="conv-diff-collapse"
        data-state={diffs.collapsed ? "on" : "off"}
        onClick={() => setDiffs((d) => ({ collapsed: !d.collapsed, generation: d.generation + 1 }))}
      >
        <span className="hx-pop-pi">
          <Icon.fold />
        </span>
        <span className="hx-pop-pt">
          <b>{strings.collapseDiffs}</b>
          <small>{strings.collapseDiffsHint}</small>
        </span>
        <span className={`hx-sw${diffs.collapsed ? " on" : ""}`} aria-hidden />
      </button>
    </>
  ) : (
    <>
      <div className="hx-pop-h">{strings.sessionOptions}</div>
      <button type="button" className="hx-pop-it" disabled title={strings.needsBackend}>
        <span className="hx-pop-pi">
          <Icon.think />
        </span>
        <span className="hx-pop-pt">
          <b>{strings.deepThinking}</b>
          <small>{strings.needsBackend}</small>
        </span>
        <span className="hx-sw" aria-hidden />
      </button>
      <button type="button" className="hx-pop-it" disabled title={strings.needsBackend}>
        <span className="hx-pop-pi">
          <Icon.globe />
        </span>
        <span className="hx-pop-pt">
          <b>{strings.webSearch}</b>
          <small>{strings.needsBackend}</small>
        </span>
        <span />
      </button>
    </>
  );

  const disabled = props.disabled === true || !state || state.loading || state.failed != null;
  const placeholder = props.placeholder ?? (running ? strings.inputPlaceholderRunning : strings.inputPlaceholder);
  const readOnly = props.readOnly === true ? strings.readOnly : props.readOnly || null;
  const header = props.header === false ? null : (props.header ?? {});
  const model = features.modelPicker === false ? null : (props.model ?? (typeof hello?.engine?.model === "string" ? hello.engine.model : null));

  return (
    <ChatContext.Provider value={context}>
      <section
        className={`hx-chat hx-chat-${variant}${props.className ? ` ${props.className}` : ""}`}
        data-testid="conv"
        data-id={props.sessionId ?? undefined}
        data-state={viewState}
        data-variant={variant}
        aria-busy={!state || state.loading}
      >
        {header ? <ChatHeader config={header} role={role} state={state} selection={props.selection} /> : null}
        {props.pinned ? (
          <div className="hx-pinned" data-testid="conv-pinned">
            {props.pinned}
          </div>
        ) : null}
        {state?.banner ? (
          <div className="hx-banner" role="alert" data-testid="conv-banner">
            <span>{state.banner}</span>
            <button type="button" className="hx-btn hx-btn-ghost hx-btn-xs" data-testid="conv-banner-dismiss" onClick={() => controller?.dismissBanner()}>
              {strings.dismiss}
            </button>
          </div>
        ) : null}

        <ChatFlow
          state={state}
          controller={controller}
          variant={variant}
          role={role}
          running={running}
          elsewhere={otherClient}
          renderers={props.renderers}
          copy={features.copy !== false}
          emptyState={props.emptyState}
          unavailable={props.unavailable}
          tail={props.flowTail}
          queueConfig={props.queue}
          ids={props.ids}
        />

        {readOnly ? (
          <div className="hx-cmp" data-testid="conv-readonly">
            <div className="hx-cmp-note">
              <Icon.book />
              <span>{readOnly}</span>
            </div>
          </div>
        ) : (
          <ChatComposer
            key={props.draftKey}
            draftKey={props.draftKey}
            state={state}
            running={running}
            elsewhere={otherClient}
            send={send}
            upload={props.upload}
            bind={props.composer}
            interrupt={interrupt}
            sendMode={props.sendMode ?? "stop"}
            transcript={transcript}
            disabled={disabled}
            placeholder={placeholder}
            quickReplies={props.quickReplies ?? NO_QUICK}
            notice={props.notice}
            guard={props.guard}
            hint={props.hint}
            commands={commands}
            mention={features.mention === false ? undefined : props.mention}
            contextRing={ring}
            options={options}
            attachments={features.attachments !== false}
            voice={features.voice !== false}
            model={model}
            models={props.models}
            onModel={props.onModel}
            tools={props.composerTools}
          />
        )}
      </section>
    </ChatContext.Provider>
  );
}

/* ── 抬头（原型 .chead）──────────────────────────────────────────── */

function ChatHeader({
  config,
  role,
  state,
  selection,
}: {
  config: ChatHeaderConfig;
  role: ChatRole;
  state: SessionState | null;
  selection?: ChatPanelProps["selection"];
}) {
  const strings = useStrings();
  const totals = usageTotals(state?.usage ?? null);
  const shownRole = config.role === false ? null : (config.role ?? role);
  const data: Record<string, string> = {};
  for (const [k, v] of Object.entries(config.data ?? {})) if (v != null) data[`data-${k}`] = v;
  return (
    <header className="hx-chead" data-testid="conv-header" {...data}>
      <div className="hx-chead-ctx">
        {config.onBack ? (
          <button
            type="button"
            className="hx-hbtn"
            data-testid="conv-back"
            title={config.backLabel ?? strings.back}
            aria-label={config.backLabel ?? strings.back}
            onClick={config.onBack}
          >
            ←
          </button>
        ) : null}
        {config.crumb ? (
          <b className="hx-chead-crumb" data-testid="conv-crumb">
            {config.crumb}
          </b>
        ) : null}
        {config.title ? (
          <b className="hx-chead-crumb hx-truncate" data-testid="conv-title">
            {state?.session?.name || strings.untitled}
          </b>
        ) : null}
        {shownRole ? (
          <span className="hx-chead-role" data-testid="conv-role">
            <Avatar role={shownRole} size="sm" />
            {shownRole.name}
          </span>
        ) : null}
        {config.status ? (
          <span className={`hx-tag hx-tag-${config.status.tone ?? "idle"}`} data-testid="conv-status-tag" data-state={config.status.tone ?? "idle"}>
            {config.status.text}
          </span>
        ) : null}
        {config.model ? (
          <span className="hx-chead-model hx-mono" data-testid="conv-model-name">
            {config.model}
          </span>
        ) : null}
        {config.usage && totals ? (
          <span className="hx-pill" data-testid="conv-usage" data-total={totals.total} data-cached={totals.cached}>
            {strings.tokens(totals.total)}
            {totals.cached ? ` · ${strings.cached(totals.cached)}` : ""}
          </span>
        ) : null}
        {config.meta}
        {config.actions || config.connection ? <span className="hx-spacer" /> : null}
        {config.actions}
        {config.connection ? <ConnectionBadge /> : null}
      </div>
      {config.scope ? (
        <div className="hx-chead-scope" data-testid="conv-scope">
          {config.scope}
        </div>
      ) : null}
      {config.extra}
      {selection && selection.items.length ? (
        <div className="hx-chead-sel" data-testid="conv-selection">
          <span className="hx-muted">{selection.label ?? strings.selected}</span>
          {selection.items.map((id) => (
            <span key={id} className="hx-chip" data-testid="conv-selection-item" data-id={id}>
              {id}
            </span>
          ))}
          <button type="button" className="hx-btn hx-btn-xs hx-btn-ghost" data-testid="conv-selection-clear" onClick={selection.onClear}>
            {strings.clearSelection}
          </button>
        </div>
      ) : null}
    </header>
  );
}

/* ── 消息流（原型 .msgs / .cli）───────────────────────────────────── */

type Speaker = "assistant" | "user" | "sys";

function speakerOf(block: Block): Speaker {
  if (block.kind === "user") return "user";
  if (block.kind === "note" || block.kind === "context") return "sys";
  return "assistant";
}

function timeOf(block: Block): number | undefined {
  if (block.kind === "assistant" || block.kind === "user") return block.at;
  if (block.kind === "tool" || block.kind === "think") return block.startedAt;
  return undefined;
}

/** 消息流的一项：一个块，或（会话的样子）同一角色连续发言的一组 */
type FlowEntry = { kind: "block"; block: Block } | { kind: "group"; key: number; blocks: Block[] };

function ChatFlow({
  state,
  controller,
  variant,
  role,
  running,
  elsewhere,
  renderers,
  copy,
  emptyState,
  unavailable,
  tail,
  queueConfig,
  ids,
}: {
  state: SessionState | null;
  controller: SessionController | null;
  variant: ChatVariant;
  role: ChatRole;
  running: boolean;
  elsewhere: boolean;
  renderers?: ChatRenderers;
  copy: boolean;
  emptyState?: ReactNode;
  unavailable?: ReactNode;
  tail?: ReactNode;
  queueConfig?: ChatPanelProps["queue"];
  ids?: ChatIds;
}) {
  const strings = useStrings();
  const { status } = useConnection();
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const transcript = variant === "transcript";

  const onScroll = useCallback(() => {
    const node = scroller.current;
    if (node) pinned.current = node.scrollHeight - node.scrollTop - node.clientHeight < 140;
  }, []);

  useLayoutEffect(() => {
    const node = scroller.current;
    if (node && pinned.current) node.scrollTop = node.scrollHeight;
  }, [state?.blocks, state?.queue, state?.steering, state?.turn, tail]);

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

  const blocks = state?.blocks ?? [];
  const queue = state?.queue ?? [];
  const inQueue = new Set(queue.map((q) => q.message.trim()));
  const extra = (queueConfig?.extra ?? []).filter((q) => !inQueue.has(q.text.trim()));

  // 会话的样子：同一角色连续发言（含工具行、卡片）合成一组，抬头只出一次（原型 msgItem 的 same）
  const entries: FlowEntry[] = [];
  for (const block of blocks) {
    if (!transcript && speakerOf(block) === "assistant") {
      const last = entries[entries.length - 1];
      if (last?.kind === "group") last.blocks.push(block);
      else entries.push({ kind: "group", key: block.key, blocks: [block] });
    } else entries.push({ kind: "block", block });
  }

  const last = blocks[blocks.length - 1];
  const streamingText = last?.kind === "assistant" && last.streaming;
  const streamingThink = last?.kind === "think" && last.streaming;
  let typing: ReactNode = null;
  if (running || elsewhere) {
    const text = running ? (transcript ? strings.working : strings.replying) : strings.runningElsewhere;
    if (transcript) typing = <Typing transcript text={text} hint={running ? strings.escToInterrupt : undefined} />;
    else if (!streamingText && !streamingThink)
      typing = <Typing head={last && speakerOf(last) === "assistant" ? undefined : <AssistantHead role={role} />} text={text} />;
  }

  const loading = !state ? !unavailable && status === "open" : state.loading && status === "open";
  const empty =
    state != null && !state.loading && blocks.length === 0 && queue.length === 0 && !state.steering.length && !extra.length && !running && !tail;

  return (
    <div
      ref={scroller}
      onScroll={onScroll}
      className={transcript ? "hx-cli" : "hx-msgs"}
      data-testid="conv-flow"
      role="log"
      aria-live="polite"
      aria-relevant="additions"
      onClick={onClick}
      onKeyDown={onKeyDown}
    >
      {!state && unavailable ? unavailable : null}
      {loading ? (
        <div className="hx-placeholder" data-testid="conv-loading">
          {strings.loading}
        </div>
      ) : empty ? (
        <div data-testid="conv-empty" className="hx-blk">
          {emptyState ?? (
            <div className="hx-cv-empty">
              <Avatar role={role} />
              <div>
                <b>{role.name}</b>
                <p>{strings.empty}</p>
              </div>
            </div>
          )}
        </div>
      ) : null}
      {entries.map((entry) =>
        entry.kind === "group" ? (
          <AssistantGroup key={entry.key} blocks={entry.blocks} role={role} controller={controller} renderers={renderers} copy={copy} />
        ) : (
          <FlowItem key={entry.block.key} block={entry.block} controller={controller} transcript={transcript} renderers={renderers} />
        ),
      )}
      {/* 交给正在跑的一轮、还没进对话（IPDrun N-108）：待送达，送达后变成流里的一条人的话 */}
      {(state?.steering ?? []).map((item) => (
        <QueuedBubble
          key={`steer-${item.itemId}`}
          testId="conv-steer-item"
          id={item.itemId}
          text={item.text}
          note={strings.steerPending}
          state="steering"
        />
      ))}
      {queue.map((item) => (
        <QueuedBubble
          key={item.item_id}
          testId={queueConfig?.testId ?? "conv-queue-item"}
          id={item.item_id}
          text={item.message}
          note={queueConfig?.note}
          action={
            queueConfig?.removable === false ? undefined : (
              <button type="button" data-testid="conv-queue-remove" onClick={() => void controller?.removeQueued(item.item_id)}>
                {strings.queueRemove}
              </button>
            )
          }
        />
      ))}
      {extra.map((item) => (
        <QueuedBubble key={`extra-${item.key}`} testId={queueConfig?.testId ?? "conv-queue-item"} text={item.text} note={queueConfig?.note} />
      ))}
      {tail}
      {typing}
    </div>
  );
}

/** 会话的样子：一组助手发言（原型 .cv-a）——抬头一次，正文、思考、工具行、卡片依次排，最后是复制 */
function AssistantGroup({
  blocks,
  role,
  controller,
  renderers,
  copy,
}: {
  blocks: Block[];
  role: ChatRole;
  controller: SessionController | null;
  renderers?: ChatRenderers;
  copy: boolean;
}) {
  const first = blocks[0]!;
  const texts = blocks.filter((b): b is Extract<Block, { kind: "assistant" }> => b.kind === "assistant");
  const streaming = texts.some((b) => b.streaming);
  return (
    <div className="hx-blk hx-cv-a" data-testid="conv-group">
      <AssistantHead role={role} at={timeOf(first)} />
      {blocks.map((block) => {
        const claimed = claimFlow(renderers, block);
        if (claimed !== undefined) return <div key={block.key}>{claimed}</div>;
        switch (block.kind) {
          case "assistant":
            return <AssistantText key={block.key} block={block} extras={renderers?.assistantExtras?.(block)} />;
          case "think":
            return <ThinkRow key={block.key} block={block} />;
          case "tool": {
            const rows = renderers?.toolRows ?? [];
            for (let i = rows.length - 1; i >= 0; i -= 1) {
              const contribution = rows[i]!;
              if (contribution.match(block))
                return (
                  <div key={block.key} className="hx-card">
                    {contribution.render(block)}
                  </div>
                );
            }
            return <TranscriptToolRow key={block.key} block={block} />;
          }
          case "ask":
            return controller ? (
              <div key={block.key} className="hx-card">
                <AskCard block={block} controller={controller} />
              </div>
            ) : null;
          case "image":
            return <ImageBlock key={block.key} block={block} />;
          case "agent":
            return <Agent key={block.key} block={block} />;
          default:
            return null;
        }
      })}
      {copy && texts.length && !streaming ? <MessageActions text={texts.map((t) => t.text).join("\n\n")} /> : null}
    </div>
  );
}

/** 不分组的一项：人的话、系统事件；转录里的每一块 */
function FlowItem({
  block,
  controller,
  transcript,
  renderers,
}: {
  block: Block;
  controller: SessionController | null;
  transcript: boolean;
  renderers?: ChatRenderers;
}) {
  const claimed = claimFlow(renderers, block);
  if (claimed !== undefined) return <>{claimed}</>;
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
    case "ask":
      return controller ? <AskCard block={block} controller={controller} /> : null;
    case "note":
      return <SystemLine block={block} transcript={transcript} />;
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
