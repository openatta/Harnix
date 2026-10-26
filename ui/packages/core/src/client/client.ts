/**
 * 客户端库：五种通道语义，映射到一种拓扑上。
 *
 * 移植自 Harnix ui/runtime/client.js，语义逐条保留：
 *
 * - 握手之前什么都不发生。协议版本、贡献点 API 版本、拓扑在握手里一次谈完；不匹配就
 *   拒绝并带一句说明哪边旧了，不做静默降级。
 * - 先试单双向流（`/ws`）；服务端说"这个拓扑不提供"时换分离流——这不是被禁止的降级，
 *   拓扑只是话怎么传，每种拓扑传的是同样的话。连接从未打开（浏览器拿不到升级被拒的
 *   状态码）时也去问另一个端点。
 * - 被拒的握手不是断线：不重连，理由保留。断线才按退避重连。
 * - `answer` 是独立语义（带外应答）：单双向流上沿同一条连接回去，分离流上走自己的端点。
 *
 * 与原版的差别：原版是模块级单例（读 `document` 里的 meta 与 `location`），这里是一个类，
 * 令牌、基址、WebSocket / fetch / EventSource 都由构造参数注入，所以能在 node 与测试里跑，
 * 一个页面也可以有多个实例。另外，原版在握手说"拓扑不提供"后主动关 socket，onclose 会
 * 再走一次分离流回退（连两次）；这里关之前先摘掉处理器。
 */

import type {
  MethodName,
  Negotiated,
  NotificationName,
  Notifications,
  ParamsOf,
  ResultOf,
  CallError,
} from "../protocol/frames.ts";
import { CONTRIB_API_VERSION, PROTOCOL_VERSION, type Topology } from "../protocol/generated/contract.ts";
import {
  connectSplit,
  type EventSourceCtor,
  type FetchLike,
  type IncomingFrame,
  type SplitCarrier,
} from "./split.ts";

/** 浏览器 `WebSocket` 的最小子集，测试可以换成假的。 */
export interface WebSocketLike {
  readyState: number;
  onopen: ((event: unknown) => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onclose: ((event: unknown) => void) | null;
  onerror: ((event: unknown) => void) | null;
  send(data: string): void;
  close(): void;
}
export type WebSocketCtor = new (url: string) => WebSocketLike;

/**
 * 连接状态。`incompatible` 是终态：版本在页面打开期间不会变，重连只会再被拒一次。
 * `degraded` 只在分离流上出现：一个订阅面断了，另一个还在。
 */
export type ConnectionStatus = "connecting" | "open" | "closed" | "degraded" | "error" | "incompatible";

export type StatusListener = (status: ConnectionStatus, detail?: string) => void;
export type NotifyListener = (method: string, params: Record<string, unknown>) => void;

export interface HarnixClientOptions {
  /** 每进程令牌；页面里由服务端注入到 `<meta name="harnix-token">`。 */
  token: string;
  /** 服务端基址，如 `http://127.0.0.1:4287`。缺省取 `globalThis.location.origin`。 */
  baseUrl?: string;
  /** 本客户端会说的拓扑，按偏好排序。 */
  speaks?: Topology[];
  WebSocket?: WebSocketCtor;
  fetch?: FetchLike;
  EventSource?: EventSourceCtor;
  /** 第 n 次重连前等多久（毫秒）。缺省 300·n，n 封顶 6。 */
  backoff?: (attempt: number) => number;
  /** 本地产生的错误文案（连接已关、未连接）。 */
  messages?: { connectionClosed?: string; notConnected?: string };
}

/** 本客户端会说的拓扑，最好的在前。 */
export const SPEAKS: Topology[] = ["single_duplex", "split_streams"];

const OPEN = 1;

interface Pending {
  resolve(value: unknown): void;
  reject(error: unknown): void;
}

export class HarnixClient {
  private readonly token: string;
  private readonly baseUrl: string;
  private readonly speaks: Topology[];
  private readonly WebSocketImpl: WebSocketCtor | undefined;
  private readonly fetchImpl: FetchLike | undefined;
  private readonly EventSourceImpl: EventSourceCtor | undefined;
  private readonly backoff: (attempt: number) => number;
  private readonly messages: { connectionClosed: string; notConnected: string };

  private socket: WebSocketLike | null = null;
  private carrier: SplitCarrier | null = null;
  private nextId = 1;
  private attempt = 0;
  private refused = false;
  private closedByUser = false;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly pending = new Map<number, Pending>();
  private readonly notifyListeners = new Set<NotifyListener>();
  private readonly statusListeners = new Set<StatusListener>();

  private negotiatedValue: Negotiated | null = null;
  private statusValue: ConnectionStatus = "connecting";
  private statusDetail: string | undefined;

  constructor(options: HarnixClientOptions) {
    this.token = options.token;
    const origin = (globalThis as { location?: { origin?: string } }).location?.origin;
    this.baseUrl = options.baseUrl ?? origin ?? "http://127.0.0.1";
    this.speaks = options.speaks ?? SPEAKS;
    this.WebSocketImpl = options.WebSocket ?? (globalThis as { WebSocket?: WebSocketCtor }).WebSocket;
    this.fetchImpl = options.fetch ?? (globalThis.fetch ? globalThis.fetch.bind(globalThis) : undefined);
    this.EventSourceImpl =
      options.EventSource ?? (globalThis as { EventSource?: EventSourceCtor }).EventSource;
    this.backoff = options.backoff ?? ((attempt) => 300 * attempt);
    this.messages = {
      connectionClosed: options.messages?.connectionClosed ?? "connection closed",
      notConnected: options.messages?.notConnected ?? "not connected",
    };
  }

  /* ── 观察 ─────────────────────────────────────────────────────────── */

  /** 握手谈定的东西：拓扑、版本、主体。未谈定时为 null。 */
  get negotiated(): Negotiated | null {
    return this.negotiatedValue;
  }

  get status(): ConnectionStatus {
    return this.statusValue;
  }

  /** 最近一次状态附带的说明（例如 incompatible 的理由）。 */
  get detail(): string | undefined {
    return this.statusDetail;
  }

  /** 当前所在的拓扑。 */
  get topology(): Topology | null {
    return this.negotiatedValue?.topology ?? null;
  }

  /** 所有服务端通知。返回取消订阅的函数。 */
  onNotify(fn: NotifyListener): () => void {
    this.notifyListeners.add(fn);
    return () => this.notifyListeners.delete(fn);
  }

  /** 某一种通知，带类型。 */
  on<M extends NotificationName>(method: M, fn: (params: Notifications[M]) => void): () => void {
    return this.onNotify((name, params) => {
      if (name === method) fn(params as unknown as Notifications[M]);
    });
  }

  onStatus(fn: StatusListener): () => void {
    this.statusListeners.add(fn);
    return () => this.statusListeners.delete(fn);
  }

  /** 等到连接谈定。被拒（incompatible）时 reject。 */
  ready(): Promise<Negotiated> {
    if (this.statusValue === "open" && this.negotiatedValue) return Promise.resolve(this.negotiatedValue);
    return new Promise((resolve, reject) => {
      const off = this.onStatus((status, detail) => {
        if (status === "open" && this.negotiatedValue) {
          off();
          resolve(this.negotiatedValue);
        } else if (status === "incompatible") {
          off();
          reject({ code: undefined, message: detail ?? "incompatible" } satisfies CallError);
        }
      });
    });
  }

  private setStatus(status: ConnectionStatus, detail?: string): void {
    this.statusValue = status;
    this.statusDetail = detail;
    for (const fn of [...this.statusListeners]) fn(status, detail);
  }

  private dispatch(frame: IncomingFrame): void {
    if (typeof frame.method !== "string") return;
    const params = (frame.params ?? {}) as Record<string, unknown>;
    for (const fn of [...this.notifyListeners]) fn(frame.method, params);
  }

  /* ── 连接 ─────────────────────────────────────────────────────────── */

  connect(): void {
    if (this.refused || this.closedByUser) return;
    this.setStatus("connecting");
    if (this.speaks[0] === "split_streams") {
      void this.connectSplitStreams("split_streams preferred");
      return;
    }
    this.connectDuplex();
  }

  /** 关闭并停止重连（页面卸载、组件卸载、测试收尾）。 */
  close(): void {
    this.closedByUser = true;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
    if (this.socket) {
      const socket = this.socket;
      this.socket = null;
      socket.onclose = null;
      socket.onmessage = null;
      socket.onerror = null;
      socket.close();
    }
    this.carrier?.close();
    this.carrier = null;
    this.rejectPending();
    this.setStatus("closed");
  }

  private rejectPending(): void {
    for (const [, slot] of this.pending) slot.reject({ message: this.messages.connectionClosed });
    this.pending.clear();
  }

  /* ── 模块自有链接 ───────────────────────────────────────────────── */

  /**
   * 模块 `module` 自己的 HTTP 路由（设计 §7.7）：同源的 `/apps/<module>/<path>`，凭据由这里带上。
   * `path` 走不出这个模块的前缀——`..`、别的来源一律拒绝，模块代码拼不出别的地址。
   */
  async appFetch(module: string, path: string, init: RequestInit = {}): Promise<Response> {
    if (!this.fetchImpl) throw new Error(this.messages.notConnected);
    const url = this.appUrl(module, path);
    const headers = new Headers(init.headers);
    if (this.token) headers.set("Authorization", `Bearer ${this.token}`);
    return this.fetchImpl(url.toString(), { ...init, headers, credentials: "same-origin" });
  }

  /** 模块 `module` 自己的 WebSocket；凭据放在查询串里（浏览器的 WebSocket 设不了头）。 */
  appSocket(module: string, path: string): WebSocketLike {
    if (!this.WebSocketImpl) throw new Error(this.messages.notConnected);
    const url = this.appUrl(module, path);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    if (this.token) url.searchParams.set("token", this.token);
    return new this.WebSocketImpl(url.toString());
  }

  private appUrl(module: string, path: string): URL {
    if (!/^[a-z][a-z0-9_-]*$/.test(module)) throw new Error(`not a module id: ${module}`);
    const prefix = `/apps/${module}/`;
    if (/^[a-z][a-z0-9+.-]*:|^[\\/]{2}|\\/.test(path)) throw new Error(`not a path of module ${module}: ${path}`);
    const relative = path.replace(/^\/+/, "");
    const base = new URL(this.baseUrl);
    const url = new URL(prefix + relative, base);
    if (url.origin !== base.origin || !url.pathname.startsWith(prefix)) throw new Error(`not a path of module ${module}: ${path}`);
    return url;
  }

  private wsUrl(): string {
    const url = new URL("/ws", this.baseUrl);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.searchParams.set("token", this.token);
    return url.toString();
  }

  /** 拓扑二：服务端不提供单双向流时。只从一个点名了拓扑的拒绝（或从未打开的连接）走到这里。 */
  private async connectSplitStreams(reason: string): Promise<void> {
    if (!this.speaks.includes("split_streams") || !this.fetchImpl || !this.EventSourceImpl) {
      this.refused = true;
      this.negotiatedValue = null;
      this.setStatus("incompatible", reason);
      return;
    }
    try {
      const carrier = await connectSplit({
        baseUrl: this.baseUrl,
        token: this.token,
        protocolVersion: PROTOCOL_VERSION,
        contribApiVersion: CONTRIB_API_VERSION,
        fetch: this.fetchImpl,
        EventSource: this.EventSourceImpl,
        onFrame: (frame) => this.dispatch(frame),
        onStatus: (state, face) => this.setStatus(state, face),
      });
      if (this.closedByUser) {
        carrier.close();
        return;
      }
      this.carrier = carrier;
      this.negotiatedValue = carrier.negotiated;
      this.setStatus("open");
    } catch (e) {
      this.refused = true;
      this.negotiatedValue = null;
      this.setStatus("incompatible", errorText(e) || reason);
    }
  }

  private connectDuplex(): void {
    const WebSocketImpl = this.WebSocketImpl;
    if (!WebSocketImpl) {
      void this.connectSplitStreams("WebSocket is not available here");
      return;
    }
    const socket = new WebSocketImpl(this.wsUrl());
    this.socket = socket;

    socket.onopen = async () => {
      this.attempt = 0;
      try {
        const negotiated = (await this.call("harnix.handshake", {
          protocol_version: PROTOCOL_VERSION,
          contrib_api_version: CONTRIB_API_VERSION,
          topology: "single_duplex",
        })) as Negotiated;
        this.negotiatedValue = negotiated;
        this.carrier = null;
        this.setStatus("open");
      } catch (e) {
        const message = errorText(e);
        // "这里不提供这个拓扑"不是不匹配，是一个回答：试本客户端会说的下一个。
        if (message.includes("topology") && this.speaks.includes("split_streams")) {
          this.drop(socket);
          void this.connectSplitStreams(message);
          return;
        }
        // 有意的拒绝，带理由。重连只会再被拒，所以停在这里，理由保留。
        this.negotiatedValue = null;
        this.refused = true;
        this.drop(socket);
        this.setStatus("incompatible", message);
      }
    };

    socket.onmessage = (event) => {
      let frame: IncomingFrame & { id?: number | null; result?: unknown; error?: unknown };
      try {
        frame = JSON.parse(String(event.data)) as typeof frame;
      } catch {
        return;
      }
      if (frame.id != null && this.pending.has(frame.id)) {
        const slot = this.pending.get(frame.id)!;
        this.pending.delete(frame.id);
        if (frame.error) slot.reject(frame.error);
        else slot.resolve(frame.result);
        return;
      }
      this.dispatch(frame);
    };

    socket.onclose = () => {
      if (this.socket === socket) this.socket = null;
      this.rejectPending();
      if (this.closedByUser) return;
      // 连接从未打开，也就没有握手告诉我们什么。浏览器的 WebSocket API 不给升级被拒的
      // 状态码，所以想知道提供什么，只能去问另一个端点。
      if (!this.negotiatedValue && !this.refused && this.speaks.includes("split_streams") && this.attempt === 0) {
        this.attempt = 1;
        void this.connectSplitStreams("the bidirectional topology is not served here");
        return;
      }
      // 被拒的握手不是掉线：落到下面会用"已断开"盖掉理由，再重连进同一个拒绝。
      if (this.refused) return;
      this.setStatus("closed");
      this.attempt = Math.min(this.attempt + 1, 6);
      this.reconnectTimer = setTimeout(() => {
        this.reconnectTimer = null;
        if (!this.closedByUser && !this.refused) {
          this.setStatus("connecting");
          this.connectDuplex();
        }
      }, this.backoff(this.attempt));
    };

    socket.onerror = () => this.setStatus("error");
  }

  /** 主动关一条我们不再要的 socket：先摘处理器，免得 onclose 再走一次回退或重连。 */
  private drop(socket: WebSocketLike): void {
    socket.onclose = null;
    socket.onmessage = null;
    socket.onerror = null;
    if (this.socket === socket) this.socket = null;
    this.rejectPending();
    socket.close();
  }

  /* ── 五种语义里客户端发起的三种 ───────────────────────────────────────── */

  /** 一次 unary 调用，走握手谈定的拓扑。错误以 `{code, message}` reject。 */
  call<M extends MethodName>(method: M, params?: ParamsOf<M>): Promise<ResultOf<M>>;
  call(method: string, params?: unknown): Promise<unknown>;
  call(method: string, params?: unknown): Promise<unknown> {
    if (this.carrier) return this.carrier.call(method, params);
    return new Promise((resolve, reject) => {
      const socket = this.socket;
      if (!socket || socket.readyState !== OPEN) {
        reject({ message: this.messages.notConnected } satisfies CallError);
        return;
      }
      const id = this.nextId++;
      this.pending.set(id, { resolve, reject });
      socket.send(JSON.stringify({ jsonrpc: "2.0", method, params: params ?? {}, id }));
    });
  }

  /** 不要应答；用在卸载时，那时应答也到不了。 */
  notify<M extends MethodName>(method: M, params?: ParamsOf<M>): void;
  notify(method: string, params?: unknown): void;
  notify(method: string, params?: unknown): void {
    if (this.carrier) {
      this.carrier.notify(method, params);
      return;
    }
    const socket = this.socket;
    if (socket && socket.readyState === OPEN) {
      socket.send(JSON.stringify({ jsonrpc: "2.0", method, params: params ?? {} }));
    }
  }

  /**
   * 带外应答：回答服务端发起的提问（权限等）。单列成一种语义，因为只下行的事件流没有
   * 回程；单双向流上它恰好沿同一条连接回去，分离流上走自己的端点。调用方不必知道是哪种。
   */
  answer<M extends MethodName>(method: M, params?: ParamsOf<M>): Promise<ResultOf<M>>;
  answer(method: string, params?: unknown): Promise<unknown>;
  answer(method: string, params?: unknown): Promise<unknown> {
    return this.call(method, params);
  }
}

/** 错误对象的可读文本。 */
export function errorText(e: unknown, fallback = "unknown error"): string {
  if (e && typeof e === "object") {
    const record = e as { message?: unknown; msg?: unknown };
    if (typeof record.message === "string" && record.message) return record.message;
    if (typeof record.msg === "string" && record.msg) return record.msg;
  }
  if (typeof e === "string" && e) return e;
  return e == null ? fallback : String(e);
}

/** 页面里的令牌：服务端在发 index.html 时把 `__HARNIX_TOKEN__` 换成真值。 */
export function tokenFromDocument(doc: { querySelector(selector: string): Element | null } | undefined =
  (globalThis as { document?: Document }).document): string {
  const meta = doc?.querySelector('meta[name="harnix-token"]');
  return meta?.getAttribute("content") ?? "";
}
