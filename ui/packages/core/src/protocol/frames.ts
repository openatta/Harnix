/**
 * 会话视图用到的方法与事件的类型。
 *
 * 形状以 Harnix crates/hub（harnix.* 方法）与 Agix 的 daemon_rpc_protocol
 * （session.* 方法、§7 事件白名单）为准。只写会话视图读到的字段；其余字段以索引签名
 * 放行，因为两边的表都只会往下长，客户端必须容忍不认识的东西。
 */

import type { Topology } from "./generated/contract.ts";

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

/**
 * 调用失败时 reject 的东西：服务端的 JSON-RPC 错误（生成的 `RpcError`，带 `code`），
 * 或客户端本地产生的（未连接、连接已关），没有 `code`。
 */
export interface CallError {
  code?: number;
  message: string;
  data?: unknown;
}

/* ── 握手 ─────────────────────────────────────────────────────────────── */

export interface HandshakeParams {
  protocol_version: number;
  contrib_api_version: number;
  topology: Topology;
}

/** 握手谈定的结果：拓扑、版本、我们是哪个主体。分离流还带 `credential`。 */
export interface Negotiated {
  protocol_version: number;
  contrib_api_version: number;
  topology: Topology;
  topologies?: Topology[];
  subject?: { kind: string; id?: string };
  channels?: string[];
  client?: number;
  credential?: string;
  [key: string]: unknown;
}

/* ── harnix.hello ───────────────────────────────────────────────────────── */

export interface Scene {
  scene: string;
  name?: string;
  description?: string;
  active?: boolean;
  requires_project?: boolean;
  [key: string]: unknown;
}

export interface Hello {
  protocol_version: number;
  contrib_api_version: number;
  engine: {
    model?: string;
    active_scenes?: string[];
    has_credentials?: boolean;
    [key: string]: unknown;
  };
  scenes: Scene[];
  commands: unknown[];
  contributions: unknown[];
  /** What this product is and keeps (design §10.5): its brand, the built-in spaces it keeps of
   *  `sessions` / `projects` / `system`, whether packages' interface modules load. */
  shell?: {
    spaces?: string[];
    brand?: { name?: string | null; icon?: string | null; product_id?: string };
    runtime_modules?: boolean;
    state_dir?: string;
    projects_dir?: string;
    /** Who runs this process, for the avatar. */
    user?: string;
  };
  limits?: {
    max_frame_bytes?: number;
    max_upload_bytes?: number;
    replay_max_frames?: number;
    prompt_timeout_secs?: number;
    /** 单双向流上同时在途的请求上限；超额的回 `BUSY`（-31005）。客户端照它在本地排队。 */
    max_in_flight?: number;
  };
}

/* ── 会话 ─────────────────────────────────────────────────────────────── */

/** `harnix.sessions` 的一行：引擎的 SessionInfo 加上 hub 折叠进来的 scene / project_root / running。 */
export interface SessionRow {
  session_id: string;
  name?: string | null;
  preview?: string | null;
  scene?: string | null;
  project_root?: string | null;
  status?: string;
  running?: boolean;
  message_count?: number;
  created_at?: string;
  last_active?: string;
  [key: string]: unknown;
}

/** `session.get` 的结果（attach 原样带回）。 */
export interface SessionInfo extends SessionRow {
  turn_state?: "idle" | "running" | string;
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

/** 队列里的一项。队列一律整值推送（概念与架构 §3.2）。 */
export interface QueueItem {
  item_id: string;
  message: string;
  attachments?: unknown;
}

/** hub 缓冲区里的一帧：`seq` 是唯一的追赶锚（§3.3.3）。 */
export interface ReplayFrame {
  seq: number;
  turn_id: string;
  event: EngineEvent;
}

export interface AttachResult {
  instance: string;
  session: SessionInfo;
  /**
   * 读历史读到这里为止，其后由 `replay` 接上（IPDrun N-120）。一轮在跑时是这一轮开始前的消息数：引擎每一步开始前
   * 落盘，正在跑的一轮已有一部分在 transcript 里，而缓冲区里是它的全部。
   */
  history_total: number;
  replay: ReplayFrame[];
  truncated: boolean;
  pending_prompts: PromptEvent[];
  running_turn: string | null;
  queue: QueueItem[];
  seq: number;
}

/**
 * 模型眼里同是 user 的几种消息（daemon_rpc_protocol §4 session.history）。界面只把 user / steer / command
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

/** transcript 里的一条消息（session.history）。位置、来源、所属的轮与用量和实时帧对得上；旧日志没有的为 null。 */
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

export type SendResult =
  | { turn_id: string; item_id?: string; queued?: false }
  | { queued: true; item: QueueItem }
  /** on_busy: "steer" 且本轮在接收：交给了正在跑的那一轮（IPDrun N-108），送达看 user_message_delivered。 */
  | { steered: true; item: QueueItem; into_turn_id: string };

/** 权限提问的回答（daemon_rpc_protocol §4.7）。 */
export type Decision =
  | { type: "permit" }
  | { type: "permit_always"; scope: "session" | "local" | string }
  | { type: "deny"; reason?: string };

/* ── 事件（harnix.event 里的 event，白名单见 daemon_rpc_protocol §7） ──────── */

export interface UserMessageEvent {
  kind: "user_message";
  text: string;
  /** 这条消息在中枢队列里的编号（与 harnix.send 应答、harnix.queue 里的 item_id 对应；IPDrun N-103）。 */
  item_id?: string;
  /**
   * "steer"：交给了正在跑的这一轮（IPDrun N-108），还没进对话——它不开新的一轮；进了对话时来一帧
   * user_message_delivered，本轮在那之前结束则来 user_messages_returned（中枢把它排到队首）。
   */
  delivery?: "steer";
}
/** 运行中送来的消息（on_busy: "steer"）在步骤边界加进了对话，模型下一次调用会读到；`item_id` 是中枢补的（IPDrun N-108）。 */
export interface UserMessageDeliveredEvent {
  kind: "user_message_delivered";
  message_id: string;
  content?: string;
  item_id?: string;
}
/** 运行中送来、但本轮结束前没读到的消息：不在 transcript 里；中枢把它们排回队首（IPDrun N-108）。 */
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
  code?: number;
  message?: string;
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
  | TurnCompleteEvent
  | ErrorEvent
  | SubagentProgressEvent
  | UnknownEvent;

/* ── 方法表 ───────────────────────────────────────────────────────────── */

/** 一个会话的地址：`instance` 省略即 `system`（设计 §5.6）。 */
export interface SessionKey {
  instance?: string;
  session_id: string;
  /** 调用方声称的 space；中枢对着账本核对（`SPACE_MISMATCH`）。 */
  space?: string;
}

/** 一台实例，`harnix.instances` 里的一行。 */
export interface InstanceRow {
  id: string;
  carrier: "in_process" | "spawn" | "attach";
  template: string | null;
  params: Record<string, string>;
  owner: string;
  status: { state: "starting" | "ready" } | { state: "degraded"; since_ms: number; reason: string } | { state: "down"; reason: string };
  version?: string | null;
  protocol_version?: number | null;
  data_root: string;
  scenes: string[];
  pid?: number | null;
  created_ms: number;
}

/** 一件任务（Agix `session.tasks`）。 */
export interface TaskRow {
  task_id: string;
  kind: "agent" | "teammate" | "shell" | "monitor" | "cron";
  status: "running" | "done" | "failed" | "stopped" | "waiting";
  started_ms: number;
  ended_ms: number | null;
  detail: Record<string, unknown>;
  error?: string;
}

export interface Methods {
  "harnix.handshake": { params: HandshakeParams; result: Negotiated };
  "harnix.hello": { params: Record<string, never>; result: Hello };
  "harnix.sessions": { params: { instance?: string; space?: string }; result: { sessions: SessionRow[] } };
  "harnix.spaces.list": {
    params: Record<string, never>;
    result: { spaces: { id: string; instance: string | null; built: string | null; scene: string | null }[] };
  };
  "harnix.attach": { params: SessionKey; result: AttachResult };
  "harnix.detach": { params: SessionKey; result: { detached: boolean } };
  "harnix.send": {
    params: SessionKey & {
      message: string;
      attachments?: unknown[];
      /** queue：排队到本轮结束；reject：忙就拒绝；interrupt：中断本轮、丢掉排着的再跑这条；steer：当前步骤结束后送进正在跑的这一轮（N-108）。 */
      on_busy?: "queue" | "reject" | "interrupt" | "steer";
      /** `@session` 引用：中枢把每个会话说过的话作为一段文本附上。 */
      references?: { instance?: string; session_id: string }[];
    };
    result: SendResult;
  };
  "harnix.queue.remove": { params: SessionKey & { item_id: string }; result: { items: QueueItem[] } };
  "harnix.subscribe": { params: { topic: string; params?: Record<string, unknown> }; result: { topic: string; value: unknown } };
  "harnix.unsubscribe": { params: { topic: string; params?: Record<string, unknown> }; result: { unsubscribed: boolean } };
  "harnix.instances.list": { params: Record<string, never>; result: { instances: InstanceRow[] } };
  "harnix.instances.sessions": { params: { instance: string }; result: { instance: string; sessions: SessionRow[] } };
  "harnix.instances.templates": {
    params: Record<string, never>;
    result: { templates: { id: string; carrier: string; params: string[]; ui: boolean; max: number | null }[] };
  };
  "harnix.instances.create": { params: { template: string; name: string; params?: Record<string, string> }; result: InstanceRow };
  "harnix.instances.stop": { params: { instance: string }; result: { stopped: string } };
  "harnix.sessions.tasks": { params: SessionKey; result: { active: boolean; tasks: TaskRow[] } };
  "session.create": {
    params: {
      instance?: string;
      scene?: string;
      project_root?: string | null;
      options?: Record<string, unknown>;
    };
    result: { instance: string; session_id: string; scene?: string; project_root?: string | null; [key: string]: unknown };
  };
  "session.history": {
    params: SessionKey & { offset?: number; limit?: number };
    result: HistoryPage;
  };
  "session.interrupt": { params: SessionKey; result: unknown };
  "session.respondToPrompt": {
    params: SessionKey & { prompt_id: string; decision: Decision };
    result: { prompt_id: string };
  };
}

export type MethodName = keyof Methods;
export type ParamsOf<M extends MethodName> = Methods[M]["params"];
export type ResultOf<M extends MethodName> = Methods[M]["result"];

/* ── 通知 ─────────────────────────────────────────────────────────────── */

export interface Notifications {
  /** 会话事件：中枢分配的 seq，追赶以它为锚。 */
  "harnix.event": { instance: string; session_id: string; seq: number; turn_id: string; event: EngineEvent };
  /** 一轮结束（成功、失败或被中断）。 */
  "harnix.turn_settled": {
    instance: string;
    session_id: string;
    turn_id: string;
    result?: { name?: string | null; usage?: Usage; [key: string]: unknown };
    error?: CallError;
    /** 这一轮怎么结束的：它的 `turn_complete` 里的几项（没有 `turn_complete` 的轮没有）。 */
    outcome?: { stop_reason?: string | null; usage?: Usage | null; structured_output?: unknown; message?: string; [key: string]: unknown };
  };
  /** 队列的整值快照。 */
  "harnix.queue": { instance: string; session_id: string; items: QueueItem[] };
  /** 中枢重建了这个会话的缓冲（实例重连、订阅被引擎丢掉）：重新 attach。 */
  "harnix.session_resync": { instance: string; session_id: string };
  /** 宿主事件：与具体会话无关。 */
  "harnix.host_event": { kind: string; instance?: string; [key: string]: unknown };
  /** 宿主主题的整值快照。 */
  "harnix.topic": { topic: string; params: Record<string, unknown>; value: unknown };
}

export type NotificationName = keyof Notifications;
