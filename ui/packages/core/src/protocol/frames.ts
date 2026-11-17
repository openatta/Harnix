/**
 * 方法表与通知表：每个方法的参数与答、每种通知的参数，都指向它们的类型。
 *
 * Harnix 自己的形状从 Rust 生成（`generated/contract.ts`，源头是 `harnix_contract::wire`，
 * `scripts/check.sh` 比对漂移）；引擎（Agix）的形状手写在 `engine.ts`。这里只剩把两者接到方法名与
 * 通知名上的表，和几个只在客户端这一侧存在的类型（本地产生的错误、握手谈定的结果）。
 */

import type {
  AttachResult,
  EngineTrust,
  FeedbackCategory,
  FeedbackFiled,
  Handshake,
  Hello,
  HelloEngine,
  InstanceRow,
  InstancesList,
  QueueItem,
  Rating,
  SessionOutline,
  Inbox,
  RewindParams,
  RewindResult,
  SessionHooks,
  SessionSkills,
  SessionsList,
  SpacesList,
  Topology,
  UiView,
} from "./generated/contract.ts";
import type { EventFrame, QueueFrame, RespondParams, SessionResync, TopicEnded, TopicFrame, TurnSettled } from "./generated/contract.ts";
import type { Decision, HistoryPage, TaskRow } from "./engine.ts";
import type { McpAuthDone, McpAuthStarted, McpContext, McpList } from "./generated/settings.ts";
import type {
  CreateParams,
  Deleted,
  History,
  HistoryParams,
  IdParams,
  ListParams,
  ModelDeleteParams,
  Receipt,
  Schedule,
  ScheduleList,
  UpdateParams,
} from "./generated/schedule.ts";

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

/** 客户端在握手里发的：就是生成的 `Handshake`。 */
export type HandshakeParams = Handshake;

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

/** 一台实例的引擎受信到哪一步（生成的 `EngineTrust`）：`extension` 的会话按不受信内容渲染。 */
export type InstanceTrust = EngineTrust;

/**
 * Whether a turn has a model to go to, as hello tells it: a provider, or the `ANTHROPIC_*` key the
 * engine started with. Unknown (hello could not read the providers) counts as yes, so a page does
 * not cry wolf.
 */
export function modelConfigured(engine: Partial<HelloEngine> | null | undefined): boolean {
  const providers = engine?.providers;
  if (!providers) return true;
  return Object.keys(providers.providers ?? {}).length > 0 || engine?.has_credentials === true;
}

/* ── 方法表 ───────────────────────────────────────────────────────────── */

/** 一个会话的地址：`instance` 省略即 `main`。 */
export interface SessionKey {
  instance?: string;
  session_id: string;
  /** 调用方声称的 space；中枢对着账本核对（`SPACE_MISMATCH`）。 */
  space?: string;
}

/** `harnix.session.send` 的答。 */
export type SendResult =
  | { turn_id: string; item_id?: string; queued?: false; delivery?: "queued_by_engine" }
  | { queued: true; item: QueueItem }
  /** on_busy: "steer" 且本轮在接收：交给了正在跑的那一轮，送达看 user_message_delivered。 */
  | { steered: true; item: QueueItem; into_turn_id: string };

export interface Methods {
  "harnix.handshake": { params: HandshakeParams; result: Negotiated };
  "harnix.hello": { params: Record<string, never>; result: Hello };
  "harnix.ui": { params: Record<string, never>; result: UiView };
  "harnix.sessions.list": { params: { instance?: string; space?: string }; result: SessionsList };
  "harnix.spaces.list": { params: Record<string, never>; result: SpacesList };
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
  /** 对一轮回答的反馈（设备的）：记进会话日志，后来的盖过先前的；`rating: null` 撤回。 */
  "harnix.session.feedback": {
    params: SessionKey & { turn_id: string; call_id?: string; rating: Rating | null; category?: FeedbackCategory; note?: string };
    result: FeedbackFiled;
  };
  "harnix.topics.subscribe": { params: { topic: string; params?: Record<string, unknown> }; result: { topic: string; value: unknown } };
  "harnix.topics.unsubscribe": { params: { topic: string; params?: Record<string, unknown> }; result: { unsubscribed: boolean } };
  "harnix.instances.list": { params: Record<string, never>; result: InstancesList };
  "harnix.instances.templates": {
    params: Record<string, never>;
    result: { templates: { id: string; carrier: string; params: string[]; ui: boolean; max: number | null }[] };
  };
  "harnix.instances.create": { params: { template: string; name: string; params?: Record<string, string> }; result: InstanceRow };
  "harnix.instances.stop": { params: { instance: string }; result: { stopped: string } };
  /** 引擎的答（`engine.session.tasks`），中枢加上 `instance`、`session_id` 与 `teams`。 */
  "harnix.session.tasks": { params: SessionKey; result: { active: boolean; tasks: TaskRow[]; teams?: unknown[]; [key: string]: unknown } };
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
  /** 会话的各轮：每轮从历史的哪里开始（`seq`），问了什么、最后答了什么（都截短）。见 `SessionOutline`。 */
  "harnix.session.outline": { params: SessionKey; result: SessionOutline };
  /** 等人处理的：各会话正在等的权限、计划审阅、提问与询问，带会话所属的 space、名字和项目；声称一个 space 时只答它的。同名主题推同样的值。 */
  "harnix.inbox": { params: { space?: string }; result: Inbox };
  "harnix.session.interrupt": { params: SessionKey; result: unknown };
  /** 回答一个提问：权限给 `decision`，模型的提问和 MCP server 的询问给 `answer`；先答者生效。 */
  "harnix.session.respond": {
    params: RespondParams;
    result: { prompt_id: string };
  };
  "settings.mcp.list": { params: McpAt; result: McpList };
  "settings.mcp.add": { params: McpTarget & { config: McpServerConfig }; result: McpList };
  "settings.mcp.update": { params: McpTarget & { config: McpServerConfig }; result: McpList };
  "settings.mcp.remove": { params: McpTarget; result: McpList };
  "settings.mcp.enable": { params: McpTarget & { enabled: boolean }; result: McpList };
  "settings.mcp.reconnect": { params: McpTarget; result: McpList };
  /** `origin` 是页面的 `location.origin`：浏览器登录完回到这个源上扩展的路由。 */
  "settings.mcp.authorize": { params: McpAt & { name: string; origin?: string; scopes?: string[] }; result: McpAuthStarted };
  "settings.mcp.authComplete": { params: { callback_url: string }; result: McpAuthDone };
  "settings.mcp.authCancel": { params: { flow_id: string }; result: { server: string } };
  "settings.mcp.logout": { params: McpAt & { name: string }; result: McpList };
  /** 会话下一次调模型时带的 MCP 部分：每个服务器的工具与指令。 */
  "settings.mcp.context": { params: SessionKey; result: McpContext };
  /** 文件、对话或两者回到之前一点；先 `dry_run` 看计划，人确认后再做（docs/harnix-app.md §5.1）。 */
  "harnix.session.rewind": { params: RewindParams; result: RewindResult };
  /** 会话看得到的 skill，各自的来源层与目录；未受信项目里被扣下一部分的标 `withheld`。只开给设备。 */
  "harnix.session.skills": { params: SessionKey; result: SessionSkills };
  /** 会话的轮次跑的 hook：设置三层的与本进程登记的，只有事件、种类、匹配，不带命令、地址、请求头。只开给设备。 */
  "harnix.session.hooks": { params: SessionKey; result: SessionHooks };
  /** 定时（docs/harnix-app.md §9）：带 `space` 的调用只够得着那个 space 的定时；`model` 是模型在引擎里建的，只能删。 */
  "schedule.list": { params: ListParams; result: ScheduleList };
  "schedule.get": { params: IdParams; result: Schedule };
  "schedule.create": { params: CreateParams; result: Schedule };
  /** `revision` 是读到的那一版；有人改过就是 `CONFLICT`，`data.revision` 是现在的。 */
  "schedule.update": { params: UpdateParams; result: Schedule };
  "schedule.delete": { params: IdParams; result: Deleted };
  "schedule.enable": { params: IdParams; result: Schedule };
  "schedule.disable": { params: IdParams; result: Schedule };
  /** 现在投一次（开着关着都行），下一次的时间不变；离上次投递不到一分钟时回执是 `skipped`、`min_interval`。 */
  "schedule.runNow": { params: IdParams; result: Receipt };
  "schedule.history": { params: HistoryParams; result: History };
  "schedule.model.delete": { params: ModelDeleteParams; result: Deleted };
}

/** `settings.mcp.*` 定位到哪一对 (项目, 场景)：省略即引擎缺省的场景、无项目。 */
export interface McpAt {
  scene?: string;
  project_root?: string | null;
}

/** 一个环境变量或请求头的值：取自引擎的环境变量、新给的值，或保留已写的（页面从来拿不到它）。 */
export type McpGiven = { var: string } | { secret: string } | { keep: true };

/** 表单给的一个 MCP 服务器。 */
export interface McpServerConfig {
  transport: "stdio" | "http" | "sse";
  command?: string;
  args?: string[];
  env?: Record<string, McpGiven>;
  url?: string;
  headers?: Record<string, McpGiven>;
}

/** 一层里的一个服务器。 */
export type McpTarget = McpAt & { tier: "global" | "scene" | "project"; name: string };

export type MethodName = keyof Methods;
export type ParamsOf<M extends MethodName> = Methods[M]["params"];
export type ResultOf<M extends MethodName> = Methods[M]["result"];

/* ── 通知 ─────────────────────────────────────────────────────────────── */

export interface Notifications {
  /** 会话事件：中枢分配的 seq，追赶以它为锚。 */
  "harnix.event": EventFrame;
  /** 一轮结束（成功、失败或被中断）。 */
  "harnix.turn_settled": TurnSettled;
  /** 队列的整值快照。 */
  "harnix.queue": QueueFrame;
  /** 中枢重建了这个会话的缓冲（实例重连、订阅被引擎丢掉）：重新 attach。 */
  "harnix.session_resync": SessionResync;
  /** 宿主事件：与具体会话无关。 */
  "harnix.host_event": { kind: string; instance?: string; [key: string]: unknown };
  /** 宿主主题的整值快照。 */
  "harnix.topic": TopicFrame;
  /** 订阅的主题没有了（扩展包撤下了）：每个参数组发一次，之后不再推；要再订，得等它重新登记。 */
  "harnix.topic_ended": TopicEnded;
}

export type NotificationName = keyof Notifications;
