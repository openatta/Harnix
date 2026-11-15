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

/**
 * 一台实例的引擎受信到哪一步：`product` 是产品自己拉起或连上的；`extension` 是扩展进程里的引擎，由第三方代码控制。
 * 它的会话在列表、attach 与事件里标 `trust: "extension"`，外壳按不受信内容渲染。
 */
export type InstanceTrust = "product" | "extension";

/** `hello.instances` 的一行：每台实例是什么、受信到哪一步、说的哪版协议。 */
export interface HelloInstance {
  id: string;
  carrier: InstanceRow["carrier"];
  trust: InstanceTrust;
  status: InstanceRow["status"];
  version?: string | null;
  protocol_version?: number | null;
}

/** The engine's model providers as `hello` tells them: their models only, nothing of how each is reached. */
export interface EngineProviders {
  providers: Record<string, { default_model?: string | null; models?: string[] }>;
  default_provider?: string | null;
}

export interface Hello {
  protocol_version: number;
  contrib_api_version: number;
  /**
   * 引擎（Agix）的协议版本。`harnix.event` 里的 `event` 与引擎直通方法的答是引擎的形状，不在
   * `protocol_version` 的加法承诺里：它们照这个版本读。
   */
  engine_protocol?: number;
  instances?: HelloInstance[];
  engine: {
    model?: string;
    active_scenes?: string[];
    /** The scene a session that names none is created in. */
    scene?: string | null;
    has_credentials?: boolean;
    /** What a session can be switched to: each provider's default model and the others it allows,
     *  and the default provider. Null when the engine could not say. */
    providers?: EngineProviders | null;
    [key: string]: unknown;
  };
  scenes: Scene[];
  commands: unknown[];
  /** What a client fills in when it names none. `space` is null until the product names one. */
  defaults?: { instance: string; space: string | null };
  /** Every extension and whether it is enabled: the shell activates its compiled-in UI packages by
   *  this before its first draw. What changes later is `harnix.ui`'s. */
  extensions?: ExtensionEntry[];
  /** What this product is (concept §5.10): its brand, whether packages' interface modules load. */
  shell?: {
    brand?: { name?: string | null; icon?: string | null; product_id?: string };
    runtime_modules?: boolean;
    state_dir?: string;
    projects_dir?: string;
    /** Who runs this process, for the avatar. */
    user?: string;
    /** False (`[shell] new_session = false`, ipdrun-46): the shell draws no "New session" button and
     *  ⌘K starts none, in every space. Absent or true: as before. */
    new_session?: boolean;
    /** `[shell] model_switch`: the composer changes a session's model through `harnix.session.setModel`. */
    model_switch?: boolean;
    /** `[shell] narrow_details` (ipdrun-50): "beside" keeps three columns at ≤ 960px instead of
     *  maximizing an open right column over the conversation. Absent or "over": as before. */
    narrow_details?: "over" | "beside";
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

/** One extension, as `hello.extensions` and `harnix.ui` tell it. */
export interface ExtensionEntry {
  id: string;
  trust: "origin" | "consent" | string;
  carrier?: "native" | "wasm" | "process" | string;
  version?: string | null;
  enabled: boolean;
  /** In `hello.extensions`: the compiled-in UI package it is drawn by, when that is not its id. */
  bundle?: string;
  /** In `harnix.ui` only: what it declared of its interface (`bundle`, `surfaces`, `inline`,
   *  `modules`, `points`), null when nothing. */
  ui?: { bundle?: string; [key: string]: unknown } | null;
}

/** `harnix.ui`'s value: each extension's entry, and the interface modules to load. */
export interface UiView {
  extensions: ExtensionEntry[];
  contributions: unknown[];
}

/* ── 会话 ─────────────────────────────────────────────────────────────── */

/** `harnix.sessions.list` 的一行：引擎的 SessionInfo 加上 hub 折叠进来的 scene / project_root / running。 */
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
  /** 扩展进程里的引擎的会话：按不受信内容渲染（见 `InstanceTrust`）。产品自己的会话没有这个字段。 */
  trust?: "extension";
  [key: string]: unknown;
}

/** `engine.session.get` 的结果（attach 原样带回）。 */
export interface SessionInfo extends SessionRow {
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

/** 队列里的一项。队列一律整值推送（概念与架构 §3.2）。 */
export interface QueueItem {
  item_id: string;
  message: string;
  attachments?: unknown;
}

/** hub 缓冲区里的一帧：`seq` 是唯一的追赶锚（概念与架构 §3.3.3）。 */
export interface ReplayFrame {
  seq: number;
  turn_id: string;
  event: EngineEvent;
}

export interface AttachResult {
  instance: string;
  session: SessionInfo;
  /**
   * 读历史读到这里为止，其后由 `replay` 接上。一轮在跑时是这一轮开始前的消息数：引擎每一步开始前
   * 落盘，正在跑的一轮已有一部分在 transcript 里，而缓冲区里是它的全部。
   */
  history_total: number;
  replay: ReplayFrame[];
  truncated: boolean;
  pending_prompts: PromptEvent[];
  running_turn: string | null;
  queue: QueueItem[];
  seq: number;
  /** 扩展进程里的引擎的会话（见 `InstanceTrust`）。 */
  trust?: "extension";
}

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

export type SendResult =
  | { turn_id: string; item_id?: string; queued?: false }
  | { queued: true; item: QueueItem }
  /** on_busy: "steer" 且本轮在接收：交给了正在跑的那一轮，送达看 user_message_delivered。 */
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

/* ── 方法表 ───────────────────────────────────────────────────────────── */

/** 一个会话的地址：`instance` 省略即 `main`。 */
export interface SessionKey {
  instance?: string;
  session_id: string;
  /** 调用方声称的 space；中枢对着账本核对（`SPACE_MISMATCH`）。 */
  space?: string;
}

/** 一台实例，`harnix.instances` 里的一行。 */
export interface InstanceRow {
  id: string;
  carrier: "in_process" | "spawn" | "attach" | "adopted";
  template: string | null;
  params: Record<string, string>;
  owner: string;
  status:
    | { state: "starting" | "ready" }
    | { state: "degraded"; since_ms: number; reason: string }
    | { state: "down"; reason: string }
    | { state: "hibernating"; since_ms: number };
  version?: string | null;
  protocol_version?: number | null;
  data_root: string;
  scenes: string[];
  pid?: number | null;
  created_ms: number;
  /** 只在扩展进程里的引擎上有（见 `InstanceTrust`）。 */
  trust?: "extension";
}

/** 一件任务（Agix `engine.session.tasks`）。 */
export interface TaskRow {
  task_id: string;
  kind: "agent" | "teammate" | "member" | "mcp_task" | "shell" | "monitor" | "cron" | "code";
  /** `idle`：只有常驻成员（`member`）有，活着、在两条消息之间。 */
  status: "running" | "done" | "failed" | "stopped" | "waiting" | "idle";
  started_ms: number;
  ended_ms: number | null;
  detail: Record<string, unknown>;
  error?: string;
}

export interface Methods {
  "harnix.handshake": { params: HandshakeParams; result: Negotiated };
  "harnix.hello": { params: Record<string, never>; result: Hello };
  "harnix.ui": { params: Record<string, never>; result: UiView };
  "harnix.sessions.list": { params: { instance?: string; space?: string }; result: { instance?: string; sessions: SessionRow[] } };
  "harnix.spaces.list": {
    params: Record<string, never>;
    result: { spaces: { id: string; instance: string | null; built: string | null; scene: string | null }[] };
  };
  "harnix.session.attach": { params: SessionKey; result: AttachResult };
  "harnix.session.detach": { params: SessionKey; result: { detached: boolean } };
  "harnix.session.send": {
    params: SessionKey & {
      message: string;
      attachments?: unknown[];
      /** queue：排队到本轮结束；reject：忙就拒绝；interrupt：中断本轮、丢掉排着的再跑这条；steer：当前步骤结束后送进正在跑的这一轮。 */
      on_busy?: "queue" | "reject" | "interrupt" | "steer";
      /** `@session` 引用：中枢把每个会话说过的话作为一段文本附上。 */
      references?: { instance?: string; session_id: string }[];
    };
    result: SendResult;
  };
  "harnix.session.queue.remove": { params: SessionKey & { item_id: string }; result: { items: QueueItem[] } };
  "harnix.session.queue.edit": { params: SessionKey & { item_id: string; message: string }; result: { items: QueueItem[] } };
  "harnix.topics.subscribe": { params: { topic: string; params?: Record<string, unknown> }; result: { topic: string; value: unknown } };
  "harnix.topics.unsubscribe": { params: { topic: string; params?: Record<string, unknown> }; result: { unsubscribed: boolean } };
  "harnix.instances.list": { params: Record<string, never>; result: { instances: InstanceRow[] } };
  "harnix.instances.templates": {
    params: Record<string, never>;
    result: { templates: { id: string; carrier: string; params: string[]; ui: boolean; max: number | null }[] };
  };
  "harnix.instances.create": { params: { template: string; name: string; params?: Record<string, string> }; result: InstanceRow };
  "harnix.instances.stop": { params: { instance: string }; result: { stopped: string } };
  "harnix.session.tasks": { params: SessionKey; result: { active: boolean; tasks: TaskRow[] } };
  "harnix.session.create": {
    params: {
      instance?: string;
      scene?: string;
      project_root?: string | null;
      options?: Record<string, unknown>;
    };
    result: { instance: string; session_id: string; scene?: string; project_root?: string | null; [key: string]: unknown };
  };
  "engine.session.history": {
    params: SessionKey & { offset?: number; limit?: number };
    result: HistoryPage;
  };
  "harnix.session.interrupt": { params: SessionKey; result: unknown };
  "harnix.session.respond": {
    params: SessionKey & { prompt_id: string; decision: Decision; tool_use_id?: string };
    result: { prompt_id: string };
  };
}

export type MethodName = keyof Methods;
export type ParamsOf<M extends MethodName> = Methods[M]["params"];
export type ResultOf<M extends MethodName> = Methods[M]["result"];

/* ── 通知 ─────────────────────────────────────────────────────────────── */

export interface Notifications {
  /** 会话事件：中枢分配的 seq，追赶以它为锚。 */
  "harnix.event": {
    instance: string;
    session_id: string;
    seq: number;
    turn_id: string;
    event: EngineEvent;
    /** 扩展进程里的引擎的会话（见 `InstanceTrust`）。 */
    trust?: "extension";
  };
  /** 一轮结束（成功、失败或被中断）。 */
  "harnix.turn_settled": {
    instance: string;
    session_id: string;
    turn_id: string;
    result?: { name?: string | null; usage?: Usage; [key: string]: unknown };
    error?: CallError;
    /** 这一轮怎么结束的：它的 `turn_complete` 里的几项（没有 `turn_complete` 的轮没有）。 */
    outcome?: {
      stop_reason?: string | null;
      usage?: Usage | null;
      structured_output?: unknown;
      message?: string;
      /** 这一轮的回答文本，最多 64 KiB，超了截尾并 `text_truncated`；工具调用带 `is_error`（ipdrun-41）。 */
      text?: string;
      text_truncated?: boolean;
      tool_calls?: { id: string; name: string; is_error?: boolean }[];
      [key: string]: unknown;
    };
  };
  /** 队列的整值快照。 */
  "harnix.queue": { instance: string; session_id: string; items: QueueItem[] };
  /** 中枢重建了这个会话的缓冲（实例重连、订阅被引擎丢掉）：重新 attach。 */
  "harnix.session_resync": { instance: string; session_id: string };
  /** 宿主事件：与具体会话无关。 */
  "harnix.host_event": { kind: string; instance?: string; [key: string]: unknown };
  /** 宿主主题的整值快照。 */
  "harnix.topic": { topic: string; params: Record<string, unknown>; value: unknown };
  /** 订阅的主题没有了（扩展包撤下了）：每个参数组发一次，之后不再推；要再订，得等它重新登记。 */
  "harnix.topic_ended": { topic: string; params: Record<string, unknown>; reason: string };
}

export type NotificationName = keyof Notifications;
