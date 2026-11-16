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
  SessionsList,
  SpacesList,
  Topology,
  UiView,
} from "./generated/contract.ts";
import type { EventFrame, QueueFrame, SessionResync, TopicEnded, TopicFrame, TurnSettled } from "./generated/contract.ts";
import type { Decision, HistoryPage, TaskRow } from "./engine.ts";

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
