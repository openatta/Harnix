/**
 * 会话的状态与归约：把线上的东西变成对话流里的块。
 *
 * 来自 IPDrun nest-ui（D21），原本移植自旧界面的归约部分，改成纯函数、不可变更新：
 * 每个块有稳定的 `key`，变了的块换成新对象、没变的保持同一引用，React 只重画引用变了的
 * 那几个——对应原版的 `rev` 计数，一千个 text_delta 只碰一个节点。
 *
 * 两个来源产生同一张块表：`session.history`（transcript，到它自己的深度为止是权威的）
 * 与 `harnix.event`（正在跑的这一轮，加上 attach 时中枢重放的部分）。追赶顺序是先历史、
 * 后重放，不能反过来。两边的结构一致：一次模型回复在 transcript 里是一条 assistant 消息，
 * 实时是 step_start … step_end 之间的帧；正文段、思考、工具调用各成一块，次序相同。
 */

import type {
  AttachResult,
  ContextUsage,
  EngineEvent,
  HistoryMessage,
  PromptEvent,
  QueueItem,
  SessionInfo,
  ToolResultEvent,
  ToolUseEvent,
  ToolUseStartEvent,
  TurnCompleteEvent,
  Usage,
} from "../protocol/index.ts";

/**
 * 工具调用的状态。`interrupted`：这一轮结束时它还没有结果（被中断、失败或连接断开），
 * 不会再有结果了——不能让它永远显示成"运行中"。
 */
export type ToolStatus = "running" | "ok" | "error" | "interrupted";

/*
 * 时间都是客户端量的（毫秒时间戳）：`at` 是块出现的时刻，`startedAt` / `endedAt` 是实时看到的开始与结束。
 * 从历史读回来的块没有这些（transcript 不带时间），界面就不显示耗时。
 */
export type Block =
  /** `steered`：运行中送来、在这一轮的步骤边界进了对话（IPDrun N-108；历史里是 origin "steer"）。 */
  | { key: number; kind: "user"; text: string; itemId?: string; at?: number; steered?: boolean }
  | { key: number; kind: "assistant"; text: string; streaming: boolean; at?: number }
  | { key: number; kind: "think"; text: string; streaming?: boolean; startedAt?: number; endedAt?: number }
  | {
      key: number;
      kind: "tool";
      id: string;
      name: string;
      input?: unknown;
      result?: string;
      error?: boolean;
      status: ToolStatus;
      startedAt?: number;
      endedAt?: number;
    }
  | { key: number; kind: "ask"; prompt: PromptEvent; deadline: number; answered?: AskAnswer }
  | { key: number; kind: "note"; text: string; tone: NoteTone }
  | { key: number; kind: "context"; text: string }
  | { key: number; kind: "image"; src: string }
  | { key: number; kind: "agent"; id: string; label: string; state: "running" | "complete"; lines: AgentLine[] }
  /**
   * 引擎不产生、由界面自己放进流里的一块（例如请求信封的变化）：`type` 说它是什么，`flow.block` 的贡献按它认领。
   * 只活在这一次打开里——重新 attach 时由放它的一方再放一次。
   */
  | { key: number; kind: "custom"; type: string; data: unknown; at?: number };

export type NoteTone = "info" | "error" | "interrupted" | "stop";
/** 权限卡的结局：允许、拒绝、超时（引擎超时即按拒绝处理）。 */
export type AskAnswer = "allow" | "deny" | "timeout";
export interface AgentLine {
  text: string;
  streaming?: boolean;
}

/** 块的种类，供组件按 kind 取到窄化后的类型。 */
export type BlockOf<K extends Block["kind"]> = Extract<Block, { kind: K }>;

/**
 * 交给正在跑的那一轮、还没进对话的话（IPDrun N-108，on_busy: "steer"）：画在流的末尾"待送达"，
 * 进了对话（user_message_delivered）变成那个位置上的一条人的话；本轮在那之前结束（user_messages_returned）
 * 就从这里去掉——中枢把它排回队首，队列快照里会有它。
 */
export interface SteerItem {
  itemId: string;
  text: string;
}

/** 一个打开的会话的全部状态。 */
export interface SessionState {
  sessionId: string;
  /** attach 还没回来。 */
  loading: boolean;
  /** attach 失败的理由。 */
  failed: string | null;
  session: SessionInfo | null;
  blocks: Block[];
  /** 已见到的最大 seq：追赶锚。 */
  seq: number;
  /** 正在跑的一轮。 */
  turn: { turnId: string } | null;
  /** 队列：整值快照，不做增量合并。 */
  queue: QueueItem[];
  /** 待送达：交给了正在跑的一轮、还没进对话的话（IPDrun N-108）。 */
  steering: SteerItem[];
  usage: Usage | null;
  /** 上下文占用：最近一次调过模型的那一轮的 `turn_complete.context`（attach 时取 `session.get` 的）。 */
  context: ContextUsage | null;
  /** 回放被截断等需要提示的事。 */
  banner: string | null;
  /** 正在流式输出的助手块。 */
  stream: number | null;
  /** 正在流式输出的思考块。 */
  thinking: number | null;
  /** 工具 id → 块 key，用来把 tool_result 配到它的 tool_use 上（按 id，不按下标）。 */
  tools: Record<string, number>;
  /** 子 agent 会话 id → 块 key。 */
  agents: Record<string, number>;
  nextKey: number;
}

export function emptySession(sessionId: string): SessionState {
  return {
    sessionId,
    loading: true,
    failed: null,
    session: null,
    blocks: [],
    seq: 0,
    turn: null,
    queue: [],
    steering: [],
    usage: null,
    context: null,
    banner: null,
    stream: null,
    thinking: null,
    tools: {},
    agents: {},
    nextKey: 1,
  };
}

/* ── 小工具 ───────────────────────────────────────────────────────────── */

type NewBlock = Block extends infer B ? (B extends Block ? Omit<B, "key"> : never) : never;

function push(state: SessionState, block: NewBlock): [SessionState, number] {
  const key = state.nextKey;
  const next = { ...state, nextKey: key + 1, blocks: [...state.blocks, { ...block, key } as Block] };
  return [next, key];
}

function update(state: SessionState, key: number, change: (block: Block) => Block): SessionState {
  const index = findIndex(state.blocks, key);
  if (index < 0) return state;
  const blocks = state.blocks.slice();
  blocks[index] = change(blocks[index]!);
  return { ...state, blocks };
}

/** 从后往前找：被更新的几乎总是最近的块。 */
function findIndex(blocks: Block[], key: number): number {
  for (let i = blocks.length - 1; i >= 0; i -= 1) if (blocks[i]!.key === key) return i;
  return -1;
}

export function blockByKey(state: SessionState, key: number): Block | undefined {
  const index = findIndex(state.blocks, key);
  return index < 0 ? undefined : state.blocks[index];
}

function endStream(state: SessionState, now?: number): SessionState {
  let next = endThinking(state, now);
  if (next.stream == null) return next;
  const key = next.stream;
  next = update(next, key, (block) => (block.kind === "assistant" ? { ...block, streaming: false } : block));
  return { ...next, stream: null };
}

/** 思考结束：任何别的输出（正文、工具、提问、一轮结束）都意味着它想完了。 */
function endThinking(state: SessionState, now?: number): SessionState {
  if (state.thinking == null) return state;
  const key = state.thinking;
  const next = update(state, key, (block) => (block.kind === "think" ? { ...block, streaming: false, endedAt: now ?? block.endedAt } : block));
  return { ...next, thinking: null };
}

/** transcript 里一条消息的时间（不同版本的字段名不同；没有就不显示）。 */
function messageTime(message: HistoryMessage): number | undefined {
  for (const key of ["timestamp", "created_at", "ts", "time"]) {
    const value = message[key];
    if (typeof value === "number" && Number.isFinite(value)) return value < 1e12 ? value * 1000 : value;
    if (typeof value === "string") {
      const parsed = Date.parse(value);
      if (!Number.isNaN(parsed)) return parsed;
    }
  }
  return undefined;
}

export function addCustom(state: SessionState, type: string, data: unknown, at?: number): SessionState {
  return push(state, { kind: "custom", type, data, at })[0];
}

export function note(state: SessionState, text: string, tone: NoteTone = "info"): SessionState {
  return push(state, { kind: "note", text, tone })[0];
}

/**
 * 被取消的工具调用的结果：引擎取消工具时回一个以 "cancelled" 开头的错误结果。是人（或别的客户端、服务端代人）
 * 中断了这一轮，不是工具失败——标"已中断"（界面设计 §8.2）。中断可能不经这个页面（工位的中断走服务端），
 * 刷新后从历史读回来也一样，所以按结果本身认，不只看 interrupting。
 */
export function isCancelledResult(text: string): boolean {
  return text.length < 200 && /^\W*(?:cancell?ed|interrupted)\b/i.test(text);
}

/** 工具结果的文本：字符串原样；内容块数组取文本；其余转 JSON。 */
export function toolResultText(content: unknown, imageLabel = "[图片]"): string {
  if (content == null) return "";
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((part: { type?: string; text?: string }) =>
        part?.type === "text" ? (part.text ?? "") : part?.type === "image" ? imageLabel : JSON.stringify(part),
      )
      .join("\n");
  }
  return JSON.stringify(content, null, 2);
}

function imageSource(source: { type?: string; url?: string; media_type?: string; data?: string } | undefined): string {
  if (!source) return "";
  if (source.type === "url") return source.url ?? "";
  return `data:${source.media_type || "image/png"};base64,${source.data || ""}`;
}

/* ── attach 与历史 ────────────────────────────────────────────────────── */

/** attach 的应答落进状态：会话、seq、队列、用量、正在跑的一轮。 */
export function applyAttach(state: SessionState, attached: AttachResult, truncatedText: string): SessionState {
  return {
    ...state,
    session: attached.session ?? null,
    seq: attached.seq || 0,
    queue: attached.queue || [],
    usage: attached.session?.usage ?? null,
    context: attached.session?.context ?? null,
    turn: attached.running_turn ? { turnId: attached.running_turn } : null,
    banner: attached.truncated ? truncatedText : state.banner,
  };
}

/** 中枢缓冲区的重放与挂起的提问：在历史之后应用。 */
export function applyReplay(state: SessionState, attached: AttachResult, promptTimeoutSecs: number, now: number): SessionState {
  let next = state;
  for (const frame of attached.replay || []) {
    if (frame.seq > next.seq) next = { ...next, seq: frame.seq };
    next = applyEvent(next, frame.event, { promptTimeoutSecs, now, live: false });
  }
  for (const prompt of attached.pending_prompts || []) {
    next = applyEvent(next, prompt, { promptTimeoutSecs, now });
  }
  return { ...next, loading: false };
}

/** 人说的话的来源（daemon_rpc_protocol §4）；别的 user 消息（注入、压缩、扩展）不给气泡。 */
const SPOKEN = new Set(["user", "steer", "command"]);

/** 一条 user 消息里的这段文字是不是引擎或工具加的。旧日志没有 origin，只能认 `<system-reminder>`。 */
function isInjected(message: HistoryMessage, text: string): boolean {
  const origin = message.origin;
  if (typeof origin === "string" && origin !== "tool_result") return !SPOKEN.has(origin);
  return text.trimStart().startsWith("<system-reminder>");
}

/** transcript 里的一条消息变成块。 */
export function applyHistoryMessage(state: SessionState, message: HistoryMessage, labels: { result: string; image: string }): SessionState {
  const content = Array.isArray(message.content)
    ? message.content
    : [{ type: "text", text: String(message.content || "") }];
  const at = messageTime(message);
  let next = state;
  for (const raw of content) {
    const part = raw as Record<string, unknown> & { type: string };
    if (part.type === "text") {
      const text = String(part.text ?? "");
      if (!text.trim()) continue;
      // 注入的上下文（提醒、召回的记忆、skill 的指令、压缩的摘要）记成 user 消息。它不是用户说的，不给气泡；
      // 但丢掉会歪曲模型是根据什么回答的，所以变成一行可展开的。
      if (message.role === "user" && isInjected(message, text)) {
        next = push(next, { kind: "context", text })[0];
        continue;
      }
      next = push(
        next,
        message.role === "user"
          ? { kind: "user", text, at, ...(message.origin === "steer" ? { steered: true } : {}) }
          : { kind: "assistant", text, streaming: false, at },
      )[0];
    } else if (part.type === "thinking") {
      next = push(next, { kind: "think", text: String(part.thinking ?? "") })[0];
    } else if (part.type === "tool_use") {
      const id = String(part.id);
      const [pushed, key] = push(next, { kind: "tool", id, name: String(part.name ?? ""), input: part.input, status: "ok" });
      next = { ...pushed, tools: { ...pushed.tools, [id]: key } };
    } else if (part.type === "tool_result") {
      const id = String(part.tool_use_id);
      const isError = Boolean(part.is_error);
      const text = toolResultText(part.content, labels.image);
      const status: ToolStatus = isError ? (isCancelledResult(text) ? "interrupted" : "error") : "ok";
      const key = next.tools[id];
      if (key != null) {
        next = update(next, key, (block) => (block.kind === "tool" ? { ...block, result: text, error: isError, status } : block));
      } else {
        next = push(next, { kind: "tool", id, name: labels.result, result: text, error: isError, status })[0];
      }
    } else if (part.type === "image") {
      next = push(next, { kind: "image", src: imageSource(part.source as never) })[0];
    }
  }
  return next;
}

/* ── 归约 ─────────────────────────────────────────────────────────────── */

export interface ApplyOptions {
  promptTimeoutSecs: number;
  now: number;
  /**
   * 是不是正在发生的事件。重放（attach 时中枢缓冲区里的帧）不是：它们发生在过去，`now` 不是它们的时间，
   * 不记时间、不量耗时。缺省 true。
   */
  live?: boolean;
  /**
   * 我们已经要求中断这一轮：这时回来的失败结果（引擎取消工具时回 "cancelled"）是中断造成的，
   * 标成"已中断"而不是"失败"。
   */
  interrupting?: boolean;
  /** 没有名字的工具结果用的标签。 */
  resultLabel?: string;
  /** 别处中断的一轮（停止原因 cancelled）那一行的话 */
  interruptedText?: string;
  stopReasonText?: (reason: string) => string;
}

/** 一个引擎事件。没见过的 kind 一律忽略（daemon_rpc_protocol §7）。 */
export function applyEvent(state: SessionState, event: EngineEvent, options: ApplyOptions): SessionState {
  // 只有实时的事件才记时间；重放的不记（见 ApplyOptions.live）
  const now = options.live === false ? undefined : options.now;
  switch (event?.kind) {
    case "user_message": {
      const message = event as { text?: string; item_id?: string; delivery?: string };
      const text = String(message.text ?? "");
      // 交给正在跑的一轮（IPDrun N-108）：不是新的一轮，也还不在对话里——先画在末尾"待送达"
      if (message.delivery === "steer") {
        if (!message.item_id || state.steering.some((s) => s.itemId === message.item_id)) return state;
        return { ...state, steering: [...state.steering, { itemId: message.item_id, text }] };
      }
      // 同一条话作为一轮送来（退回后排队、或引擎没能送进那一轮）：不再待送达
      const base = message.item_id ? dropSteering(state, [message.item_id]) : state;
      return push(endStream(base, now), { kind: "user", text, itemId: message.item_id, at: now })[0];
    }

    // 待送达的话在步骤边界进了对话：画在此刻的位置（transcript 里它也在这里）
    case "user_message_delivered": {
      const delivered = event as { message_id?: string; item_id?: string; content?: string };
      const itemId = delivered.item_id ?? delivered.message_id;
      const pending = state.steering.find((s) => s.itemId === itemId);
      const text = delivered.content ?? pending?.text ?? "";
      const next = endStream(itemId ? dropSteering(state, [itemId]) : state, now);
      if (!text.trim()) return next;
      return push(next, { kind: "user", text, itemId, at: now, steered: true })[0];
    }

    // 本轮在读到它们之前结束：不在对话里；中枢把它们排回队首（队列快照另到）
    case "user_messages_returned": {
      const returned = event as { messages?: { message_id?: string }[] };
      const ids = (returned.messages ?? []).map((m) => m.message_id).filter((id): id is string => Boolean(id));
      return dropSteering(state, ids);
    }

    case "text_delta": {
      const text = String((event as { text?: string }).text ?? "");
      if (!text) return state;
      const base = endThinking(state, now);
      if (base.stream == null) {
        const [next, key] = push(base, { kind: "assistant", text, streaming: true, at: now });
        return { ...next, stream: key };
      }
      return update(base, base.stream, (block) => (block.kind === "assistant" ? { ...block, text: block.text + text } : block));
    }

    case "thinking_delta": {
      const text = String((event as { text?: string }).text ?? "");
      if (state.thinking == null) {
        const [next, key] = push(endStream(state, now), { kind: "think", text, streaming: true, startedAt: now });
        return { ...next, thinking: key };
      }
      return update(state, state.thinking, (block) => (block.kind === "think" ? { ...block, text: block.text + text } : block));
    }

    // 模型开始写一次工具调用：工具行先出来（运行中），参数随后的 tool_use 补上。大文件的 Write 要写好一阵参数，
    // 这时不该什么都看不见。参数的片段（tool_args_delta）不逐个画：拼到一半的 JSON 读不出东西。
    case "tool_use_start": {
      const start = event as ToolUseStartEvent;
      if (state.tools[start.id] != null) return state;
      const [next, key] = push(endStream(state, now), { kind: "tool", id: start.id, name: start.name, status: "running", startedAt: now });
      return { ...next, tools: { ...next.tools, [start.id]: key } };
    }

    case "tool_use": {
      const use = event as ToolUseEvent;
      const started = state.tools[use.id];
      if (started != null) {
        const next = endStream(state, now);
        return update(next, started, (block) => (block.kind === "tool" ? { ...block, name: use.name || block.name, input: use.input } : block));
      }
      const [next, key] = push(endStream(state, now), {
        kind: "tool",
        id: use.id,
        name: use.name,
        input: use.input,
        status: "running",
        startedAt: now,
      });
      return { ...next, tools: { ...next.tools, [use.id]: key } };
    }

    case "tool_result": {
      const result = event as ToolResultEvent;
      const text = toolResultText(result.content);
      const isError = Boolean(result.is_error);
      const status: ToolStatus = isError ? (options.interrupting || isCancelledResult(text) ? "interrupted" : "error") : "ok";
      const key = state.tools[result.id];
      if (key != null) {
        return update(state, key, (block) =>
          block.kind === "tool" ? { ...block, result: text, error: isError, status, endedAt: block.startedAt != null ? now : undefined } : block,
        );
      }
      return push(state, {
        kind: "tool",
        id: result.id,
        name: result.name || options.resultLabel || "result",
        result: text,
        error: isError,
        status: isError ? (isCancelledResult(text) ? "interrupted" : "error") : "ok",
      })[0];
    }

    case "prompt": {
      const prompt = event as PromptEvent;
      // 同一个提问可能既在重放里又在 pending_prompts 里：按 prompt_id 去重。
      if (state.blocks.some((b) => b.kind === "ask" && b.prompt.prompt_id === prompt.prompt_id)) return state;
      return push(endStream(state, now), {
        kind: "ask",
        prompt,
        deadline: options.now + options.promptTimeoutSecs * 1000,
      })[0];
    }

    // 一次模型调用结束：它的正文到此为止。下一次调用的正文另起一块——transcript 里那是另一条 assistant 消息。
    case "step_end":
      return endStream(state, now);

    // 本轮出错结束。中枢是发起者，收不到这一帧；万一收到，只收尾，失败由 harnix.turn_settled 说。
    case "error":
      return endStream(state, now);

    // turn_start / step_start 不成块；一轮的生命周期取自 harnix.turn_settled。
    case "turn_complete": {
      const done = event as TurnCompleteEvent;
      let next = endStream(state, now);
      next = { ...next, usage: done.usage ?? next.usage, context: done.context ?? next.context };
      // 我们要求的中断：停止原因（cancelled）不另说，一轮结束时统一说"已中断"
      if (done.stop_reason && done.stop_reason !== "end_turn" && !options.interrupting) {
        // 别处（服务端代人）中断的：同样是"已中断"，不是"停止原因：cancelled"
        if (/cancel/i.test(done.stop_reason)) next = note(next, options.interruptedText ?? done.stop_reason, "interrupted");
        else next = note(next, options.stopReasonText ? options.stopReasonText(done.stop_reason) : done.stop_reason, "stop");
      }
      return next;
    }

    case "subagent_progress":
      return subagentEvent(state, event as never);

    default:
      return state;
  }
}

function dropSteering(state: SessionState, ids: readonly string[]): SessionState {
  if (!state.steering.some((s) => ids.includes(s.itemId))) return state;
  return { ...state, steering: state.steering.filter((s) => !ids.includes(s.itemId)) };
}

function subagentEvent(
  state: SessionState,
  event: { agent_session_id: string; agent_label?: string; agent_type?: string; event?: EngineEvent },
): SessionState {
  let next = state;
  let key = next.agents[event.agent_session_id];
  if (key == null) {
    next = endStream(next);
    const [pushed, pushedKey] = push(next, {
      kind: "agent",
      id: event.agent_session_id,
      label: event.agent_label || event.agent_type || "subagent",
      state: "running",
      lines: [],
    });
    key = pushedKey;
    next = { ...pushed, agents: { ...pushed.agents, [event.agent_session_id]: key } };
  }
  const inner = (event.event ?? {}) as EngineEvent & { text?: string; name?: string; is_error?: boolean };
  return update(next, key, (block) => {
    if (block.kind !== "agent") return block;
    let lines = block.lines.slice();
    let agentState = block.state;
    if (inner.kind === "text_delta") {
      const last = lines[lines.length - 1];
      if (last?.streaming) lines[lines.length - 1] = { ...last, text: last.text + (inner.text ?? "") };
      else lines.push({ streaming: true, text: inner.text ?? "" });
    } else if (inner.kind === "tool_use") {
      lines.push({ text: `→ ${inner.name ?? ""}` });
    } else if (inner.kind === "tool_result") {
      lines.push({ text: `← ${inner.name ?? ""}${inner.is_error ? " ✕" : ""}` });
    } else if (inner.kind === "turn_complete") {
      // 子 agent 的流与父的是同一张事件表，它自己的 turn_complete 就是结束。
      agentState = "complete";
    }
    if (lines.length > 200) lines = lines.slice(lines.length - 200);
    return { ...block, lines, state: agentState };
  });
}

/* ── 一轮的开始与结束 ─────────────────────────────────────────────────── */

export interface SettleTexts {
  interrupted: string;
  failed: (error: string) => string;
}

/**
 * `harnix.turn_settled`：一轮结束。被我们中断的不算失败，用它自己的话说。
 * 这时还没有结果的工具调用不会再有结果了：标成"已中断"（引擎被取消时不补 tool_result）。
 */
export function applySettled(
  state: SessionState,
  params: { error?: { message?: string }; result?: { usage?: Usage; name?: string | null } },
  wasInterrupted: boolean,
  texts: SettleTexts,
  now?: number,
): SessionState {
  // 待送达的话不会再进这一轮了（退回的另有 user_messages_returned，它们在队列里）
  let next: SessionState = { ...endStream(state, now), turn: null, steering: [] };
  if (next.blocks.some((b) => b.kind === "tool" && b.status === "running")) {
    next = {
      ...next,
      blocks: next.blocks.map((b) =>
        b.kind === "tool" && b.status === "running" ? { ...b, status: "interrupted", endedAt: b.startedAt != null ? now : undefined } : b,
      ),
    };
  }
  if (wasInterrupted) next = note(next, texts.interrupted, "interrupted");
  if (params.error) {
    if (!wasInterrupted) next = note(next, texts.failed(params.error.message ?? ""), "error");
  } else if (params.result) {
    if (params.result.usage) next = { ...next, usage: params.result.usage };
    if (params.result.name && next.session) next = { ...next, session: { ...next.session, name: params.result.name } };
  }
  return next;
}

/** 一张权限卡被回答（或超时）。 */
export function answerAsk(state: SessionState, key: number, answer: AskAnswer): SessionState {
  return update(state, key, (block) => (block.kind === "ask" && !block.answered ? { ...block, answered: answer } : block));
}

/** 用量的合计：四项互不重叠，总量是四项之和；缓存一项是总量的一部分。 */
export function usageTotals(usage: Usage | null): { total: number; cached: number } | null {
  if (!usage) return null;
  const n = (v: number | undefined) => v || 0;
  const cached = n(usage.cache_read_input_tokens) + n(usage.cache_creation_input_tokens);
  return { total: n(usage.input_tokens) + n(usage.output_tokens) + cached, cached };
}
