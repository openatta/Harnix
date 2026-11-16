/**
 * 引擎（Agix）的形状：`harnix.event` 里的 `event`、`engine.*` 方法的答、中枢原样带回的引擎值。
 *
 * 这些不在 Harnix 协议的加法承诺里（harnix-ext.md §16），照 `hello.engine_protocol` 读，手写，以 Agix 的
 * daemon_rpc_protocol 为准。只写客户端读到的字段；其余以索引签名放行，客户端必须容忍不认识的东西。
 * Harnix 自己的形状不在这里：它们从 Rust 生成（`generated/`）。
 */

import type { HubSessionRow } from "./generated/contract.ts";

/* ── 引擎的场景、会话 ─────────────────────────────────────────────────── */

/** `engine.scene.list` 的一行（`hello.scenes`）。 */
export interface Scene {
  scene: string;
  name?: string;
  description?: string;
  active?: boolean;
  requires_project?: boolean;
  /** 场景层的一行：它跑在引擎的哪个场景上。 */
  base?: string;
  scene_layer?: boolean;
  [key: string]: unknown;
}

/** 引擎的 SessionInfo：`engine.session.list` 的一行，中枢在上面折进自己的字段（`HubSessionRow`）。 */
export interface EngineSession {
  session_id: string;
  name?: string | null;
  preview?: string | null;
  /** 引擎跑的场景（它自己的）；会话建在场景层里时，`scene_layer` 是那一层，显示用它。 */
  scene?: string | null;
  project_root?: string | null;
  status?: string;
  message_count?: number;
  created_at?: string;
  last_active?: string;
  [key: string]: unknown;
}

/** `harnix.sessions.list` 与 `harnix.instances.sessions` 的一行：引擎的 SessionInfo 加上中枢折叠进来的字段。 */
export type SessionRow = EngineSession & HubSessionRow;

/** `engine.session.get` 的结果（attach 原样带回，中枢折进 `scene_layer` 与 `prompt_append`）。 */
export interface SessionInfo extends EngineSession, Pick<HubSessionRow, "scene_layer" | "prompt_append"> {
  /** `rebuilding`（原地重建）与 `operating`（对上下文的操作独占）时没有轮可打断。 */
  turn_state?: "idle" | "running" | "rebuilding" | "operating" | string;
  usage?: Usage | null;
  /** 最近一次调过模型的那一轮的上下文占用；会话在内存里、且至少有一轮调过模型时才有。 */
  context?: ContextUsage;
}

/**
 * 上下文用了多少（daemon_rpc_protocol §7.2）：`tokens` 是最后一次调用发出去的上下文（厂商计数，输入加两项缓存），
 * `compact_threshold` 是自动压缩开始的位置，`window` 是模型的上下文窗口。
 */
export interface ContextUsage {
  tokens: number;
  compact_threshold?: number;
  window?: number;
}

/** 四字段用量（daemon_rpc_protocol §3.5）。四项互不重叠，合计才是这一轮的总量。 */
export interface Usage {
  input_tokens?: number;
  output_tokens?: number;
  cache_read_input_tokens?: number;
  cache_creation_input_tokens?: number;
}

/** 一个会话的目标（`engine.session.goal`；`harnix.instances.sessions` 行里的 `goal`）：引擎为它不断开新轮的一个目的。 */
export interface Goal {
  objective: string;
  phase: "active" | "paused" | "blocked" | "complete";
  armed: boolean;
  max_rounds: number;
  rounds_started: number;
  blocked_reason?: { code?: string; message?: string } | string | null;
  [key: string]: unknown;
}

/* ── transcript ───────────────────────────────────────────────────────── */

/**
 * 模型眼里同是 user 的几种消息（daemon_rpc_protocol §4 engine.session.history）。界面只把 user / steer / command
 * 画成人说的话；`null` 是旧日志，不知道。
 */
export type MessageOrigin =
  | "user"
  | "steer"
  | "command"
  | "injected"
  | "tool_result"
  | "assistant"
  | "compact"
  | "extension"
  | null;

/** transcript 里的一条消息（engine.session.history）。位置、来源、所属的轮与用量和实时帧对得上；旧日志没有的为 null。 */
export interface HistoryMessage {
  role: "user" | "assistant" | string;
  content: string | ContentPart[];
  /** 在投影视图里的位置（offset 的单位），与 step_end / turn_complete 的 seq 同一套坐标。 */
  seq?: number;
  /** 写下它的那一行的 id；压缩后的替换消息是 "<压缩行 id>#<序号>"。 */
  id?: string;
  ts?: string;
  /** 哪一轮写的，与实时帧的 turn_id 相同。 */
  turn_id?: string | null;
  origin?: MessageOrigin | string;
  /** 产生这条回复的那次调用的用量；只有 assistant 有。 */
  usage?: Usage | null;
  [key: string]: unknown;
}

export type ContentPart =
  | { type: "text"; text: string }
  | { type: "thinking"; thinking?: string }
  | { type: "tool_use"; id: string; name: string; input?: unknown }
  | { type: "tool_result"; tool_use_id: string; content?: unknown; is_error?: boolean }
  | { type: "image"; source?: { type: string; url?: string; media_type?: string; data?: string } }
  | { type: string; [key: string]: unknown };

export interface HistoryPage {
  messages: HistoryMessage[];
  total?: number;
}

/** 权限提问的回答（daemon_rpc_protocol §4.7）。 */
export type Decision =
  | { type: "permit" }
  | { type: "permit_always"; scope: "session" | "local" | string }
  | { type: "deny"; reason?: string };

/** 一件任务（Agix `engine.session.tasks`）。 */
export interface TaskRow {
  task_id: string;
  kind: "agent" | "teammate" | "member" | "mcp_task" | "shell" | "monitor" | "cron" | "code" | (string & {});
  /** `idle`：只有常驻成员（`member`）有，活着、在两条消息之间。引擎加了新的种类也要容忍。 */
  status: "running" | "done" | "failed" | "stopped" | "waiting" | "idle" | (string & {});
  started_ms?: number | null;
  ended_ms?: number | null;
  detail?: Record<string, unknown>;
  error?: string;
  usage?: Usage;
  /** 它在等的任务。 */
  waiting_for?: string[];
  [key: string]: unknown;
}

/* ── 事件（harnix.event 里的 event，白名单见 daemon_rpc_protocol §7） ──────── */

export interface UserMessageEvent {
  kind: "user_message";
  text: string;
  /** 这条消息在中枢队列里的编号（与 harnix.session.send 应答、harnix.queue 里的 item_id 对应）。 */
  item_id?: string;
  /**
   * "steer"：交给了正在跑的这一轮，还没进对话——它不开新的一轮；进了对话时来一帧
   * user_message_delivered，本轮在那之前结束则来 user_messages_returned（话不在对话里，中枢不重发）。
   */
  delivery?: "steer";
}
/** 运行中送来的消息（on_busy: "steer"）在步骤边界加进了对话，模型下一次调用会读到；`item_id` 是中枢补的。 */
export interface UserMessageDeliveredEvent {
  kind: "user_message_delivered";
  message_id: string;
  content?: string;
  item_id?: string;
}
/** 运行中送来、但本轮结束前没读到的消息：不在 transcript 里，引擎交还给客户端；中枢不重发，要不要再说由人决定。 */
export interface UserMessagesReturnedEvent {
  kind: "user_messages_returned";
  messages: { message_id: string; content?: string }[];
}
/**
 * 运行中送来的消息（on_busy: "steer"）在步骤边界被拒收（模块的 prompt 中间件，ipdrun-13）：不进对话、不排回队列。
 * `message_id` 是那次 steer 的编号，`message` 是拒收的理由。没有模块拦人的消息时不会出现。
 */
export interface UserMessageRefusedEvent {
  kind: "user_message_refused";
  message_id: string;
  content: string;
  message?: string | null;
}
/** 本轮开始，是本轮的第一帧。 */
export interface TurnStartEvent {
  kind: "turn_start";
}
/** 本轮要调一次模型；`step` 从 0 计。 */
export interface StepStartEvent {
  kind: "step_start";
  step: number;
}
/** 这次调用结束，它的回复和它要的工具结果都已落盘；`usage` 只是这一次的，`seq` 是此刻 transcript 的消息数。 */
export interface StepEndEvent {
  kind: "step_end";
  step: number;
  stop_reason?: string;
  tool_calls?: number;
  usage?: Usage;
  seq?: number;
}
/** 思考的增量，与正文分开。 */
export interface ThinkingDeltaEvent {
  kind: "thinking_delta";
  text: string;
}
export interface TextDeltaEvent {
  kind: "text_delta";
  text: string;
}
/** 模型开始写一次工具调用，参数还在流；同 id 的 tool_use 随后到达。 */
export interface ToolUseStartEvent {
  kind: "tool_use_start";
  id: string;
  name: string;
}
/** 参数的原始片段，按到达顺序拼起来就是 tool_use.input 的 JSON 文本（高频）。 */
export interface ToolArgsDeltaEvent {
  kind: "tool_args_delta";
  id: string;
  partial_json: string;
}
export interface ToolUseEvent {
  kind: "tool_use";
  id: string;
  name: string;
  input?: unknown;
}
export interface ToolResultEvent {
  kind: "tool_result";
  id: string;
  name?: string;
  content?: unknown;
  is_error?: boolean;
}
export interface PromptEvent {
  kind: "prompt";
  prompt_type?: string;
  prompt_id: string;
  tool_name?: string;
  message?: string;
  paths?: string[];
  /** 被问的那次 tool_use。 */
  tool_use_id?: string;
  /** 宿主工具自己的问句（Harnix 的链路附上的）：卡片优先显示它。 */
  tool_question?: string;
}
/**
 * 本轮结束，transcript 已落盘。被 hook 拦下、被中断、被截停的轮同样以它结束；没调模型的轮（hook 拦下、本地命令）
 * 没有 `context`。
 */
export interface TurnCompleteEvent {
  kind: "turn_complete";
  stop_reason?: string;
  api_calls?: number;
  tool_calls?: number;
  /** 此刻 transcript 的消息数。 */
  seq?: number;
  /** 本轮所有调用的合计。 */
  usage?: Usage;
  /** 本轮最后一次调用的用量。 */
  last_call_usage?: Usage;
  context?: ContextUsage;
  /** 本轮模型最后一次成功交给 StructuredOutput 的内容。 */
  structured_output?: unknown;
  /** 截停这一轮的一方给的理由（例如 hook 的 message）。 */
  message?: string;
}
/** 本轮出错结束。只发给发起这一轮之外的订阅者；发起者（中枢）从 run_turn 的错误应答得知，经 harnix.turn_settled 转告。 */
export interface ErrorEvent {
  kind: "error";
  /** 引擎的失败种类（`rate_limited`、`model_unavailable`、`session_closed`……）；不认识的按 `retryable` 处理。 */
  code?: string;
  message?: string;
  retryable?: boolean;
  retry_after_ms?: number | null;
  /** false：这一轮没写进日志（会话中途关闭、引擎中途退出）。 */
  persisted?: boolean;
}
/** 一个提问不再悬着：被答了（在这里或别处），或被撤回（超时、取消）。 */
export interface PromptResolvedEvent {
  kind: "prompt_resolved";
  prompt_id: string;
  resolution: "answered" | "withdrawn" | string;
  reason?: "timed_out" | "cancelled" | string;
}
/** 引擎的提示：模型要用户回来看，或引擎自己的提醒（例如「总是允许」没能写盘）。 */
export interface NotificationEvent {
  kind: "notification";
  message?: string;
  status?: "proactive" | string | null;
}
export interface SubagentProgressEvent {
  kind: "subagent_progress";
  agent_session_id: string;
  agent_label?: string;
  agent_type?: string;
  event?: EngineEvent;
}
/** 没见过的 kind：必须忽略，不报错（daemon_rpc_protocol §7）。 */
export interface UnknownEvent {
  kind: string;
  [key: string]: unknown;
}

export type EngineEvent =
  | UserMessageEvent
  | UserMessageDeliveredEvent
  | UserMessagesReturnedEvent
  | UserMessageRefusedEvent
  | TurnStartEvent
  | StepStartEvent
  | StepEndEvent
  | TextDeltaEvent
  | ThinkingDeltaEvent
  | ToolUseStartEvent
  | ToolArgsDeltaEvent
  | ToolUseEvent
  | ToolResultEvent
  | PromptEvent
  | PromptResolvedEvent
  | NotificationEvent
  | TurnCompleteEvent
  | ErrorEvent
  | SubagentProgressEvent
  | UnknownEvent;
