/**
 * 分离流：unary 走 HTTP，两个订阅面（会话、宿主）各一条只下行的流（SSE）。
 *
 * 移植自 Harnix ui/runtime/topology.js。服务端只提供这一种、不提供单双向流时
 * （典型是反向代理后面），客户端库选它。方法名、参数、错误码、事件形状都不变，
 * 调用方看不到连接。
 *
 * 握手换来一个凭据，之后**每条通道都带这个凭据**，不再各自协商一遍。
 */

import type { Negotiated, CallError } from "../protocol/frames.ts";

/** 浏览器 `EventSource` 的最小子集，测试可以换成假的。 */
export interface EventSourceLike {
  onmessage: ((event: { data: string }) => void) | null;
  onerror: ((event: unknown) => void) | null;
  close(): void;
}
export type EventSourceCtor = new (url: string) => EventSourceLike;

export type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

/** 从流上收到的一帧（JSON-RPC 通知）。 */
export interface IncomingFrame {
  method?: string;
  params?: Record<string, unknown>;
  [key: string]: unknown;
}

export interface SplitCarrier {
  topology: "split_streams";
  negotiated: Negotiated;
  call(method: string, params?: unknown): Promise<unknown>;
  notify(method: string, params?: unknown): void;
  close(): void;
}

export interface SplitOptions {
  /** 页面的基址，如 `http://127.0.0.1:4287`。 */
  baseUrl: string;
  token: string;
  protocolVersion: number;
  contribApiVersion: number;
  fetch: FetchLike;
  EventSource: EventSourceCtor;
  onFrame(frame: IncomingFrame): void;
  onStatus(state: "degraded", face: string): void;
}

/** 读一个应答体；不是 JSON 的（比如 401 的纯文本理由）当作错误消息。 */
async function readFrame(response: Response): Promise<Record<string, unknown>> {
  const text = await response.text();
  try {
    return JSON.parse(text) as Record<string, unknown>;
  } catch {
    throw { code: response.status, message: text || response.statusText } satisfies CallError;
  }
}

/** 建立分离流：先握手，再打开两个订阅面。 */
export async function connectSplit(options: SplitOptions): Promise<SplitCarrier> {
  const { baseUrl, token, fetch, EventSource, onFrame, onStatus } = options;
  const url = (path: string) => new URL(path, baseUrl).toString();

  const handshake = await readFrame(
    await fetch(url("/handshake"), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        token,
        protocol_version: options.protocolVersion,
        contrib_api_version: options.contribApiVersion,
        topology: "split_streams",
      }),
    }),
  );
  if (handshake.error) throw handshake.error;

  const negotiated = handshake as unknown as Negotiated;
  const credential = String(negotiated.credential ?? "");
  const readers: EventSourceLike[] = [];

  // 两个面分开开：它们是两个订阅，看侧栏的客户端不该收所有会话的正文。
  for (const face of ["session", "host"]) {
    const source = new EventSource(url(`/events/${face}?token=${encodeURIComponent(credential)}`));
    source.onmessage = (event) => {
      try {
        onFrame(JSON.parse(event.data) as IncomingFrame);
      } catch {
        // 保活注释，或本构建不认识的帧。
      }
    };
    // 一个面断了不等于连接断了：另一个照常，这一个由 EventSource 自己重连。
    source.onerror = () => onStatus("degraded", face);
    readers.push(source);
  }

  let nextId = 1;

  return {
    topology: "split_streams",
    negotiated,

    async call(method, params) {
      // 带外应答有自己的端点：只下行的流没有回程。
      const endpoint = method === "session.respondToPrompt" ? "respond" : "rpc";
      const frame = await readFrame(
        await fetch(url(`/${endpoint}?token=${encodeURIComponent(credential)}`), {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ jsonrpc: "2.0", method, params: params ?? {}, id: nextId++ }),
        }),
      );
      if (frame.error) throw frame.error;
      return frame.result;
    },

    notify(method, params) {
      // 不要应答；keepalive 让卸载中的页面也能发出去——notify 只用在那里。
      fetch(url(`/rpc?token=${encodeURIComponent(credential)}`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", method, params: params ?? {} }),
        keepalive: true,
      }).catch(() => {});
    },

    close() {
      for (const source of readers) source.close();
    },
  };
}
