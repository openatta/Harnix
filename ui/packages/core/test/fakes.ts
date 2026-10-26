/**
 * 测试用的假连接：测试就是服务端，每种帧都能按需造出来。
 * 思路同 Harnix tests/reducer-smoke.mjs 的 ScriptedSocket。
 */

import { CONTRIB_API_VERSION, type EventSourceLike, type WebSocketLike } from "../src/index.ts";

export interface SentFrame {
  jsonrpc: string;
  method: string;
  params: Record<string, unknown>;
  id?: number;
}

type Answer = (method: string, params: Record<string, unknown>) => unknown;

/** 一台假服务端：记录收到的帧，按 `answer` 回应；返回 `undefined` 表示不回（测试手动回）。 */
export class FakeServer {
  sockets: FakeSocket[] = [];
  sent: SentFrame[] = [];
  /** 新建的 socket 是否自动打开。设为 false 可模拟"连接从未打开"。 */
  autoOpen = true;
  answer: Answer = (method) => (method === "harnix.handshake" ? handshakeOk() : {});

  get last(): FakeSocket {
    const socket = this.sockets[this.sockets.length - 1];
    if (!socket) throw new Error("no socket was opened");
    return socket;
  }

  /** 交给 HarnixClient 的 WebSocket 构造器。 */
  readonly WebSocket = (() => {
    // eslint-disable-next-line @typescript-eslint/no-this-alias
    const server = this;
    return class implements WebSocketLike {
      readyState = 0;
      onopen: ((event: unknown) => void) | null = null;
      onmessage: ((event: { data: unknown }) => void) | null = null;
      onclose: ((event: unknown) => void) | null = null;
      onerror: ((event: unknown) => void) | null = null;
      constructor(public url: string) {
        const socket = new FakeSocket(this, server);
        server.sockets.push(socket);
        if (server.autoOpen) queueMicrotask(() => socket.open());
      }
      send(data: string) {
        server.receive(this, JSON.parse(data) as SentFrame);
      }
      close() {
        if (this.readyState === 3) return;
        this.readyState = 3;
        queueMicrotask(() => this.onclose?.({}));
      }
    };
  })();

  receive(ws: WebSocketLike, frame: SentFrame): void {
    this.sent.push(frame);
    const result = this.answer(frame.method, frame.params ?? {});
    if (frame.id == null || result === undefined) return;
    const reply =
      result instanceof RpcFailure
        ? { jsonrpc: "2.0", id: frame.id, error: result.error }
        : { jsonrpc: "2.0", id: frame.id, result };
    queueMicrotask(() => ws.onmessage?.({ data: JSON.stringify(reply) }));
  }
}

export class FakeSocket {
  constructor(
    readonly ws: WebSocketLike & { url: string },
    readonly server: FakeServer,
  ) {}
  get url() {
    return this.ws.url;
  }
  open() {
    this.ws.readyState = 1;
    this.ws.onopen?.({});
  }
  /** 服务端推一条通知。 */
  push(method: string, params: Record<string, unknown>) {
    this.ws.onmessage?.({ data: JSON.stringify({ jsonrpc: "2.0", method, params }) });
  }
  /** 服务端回一条应答。 */
  reply(id: number, result: unknown) {
    this.ws.onmessage?.({ data: JSON.stringify({ jsonrpc: "2.0", id, result }) });
  }
  /** 服务端掉线。 */
  drop() {
    this.ws.readyState = 3;
    this.ws.onclose?.({});
  }
}

/** 让 answer 回一个错误。 */
export class RpcFailure {
  constructor(readonly error: { code: number; message: string }) {}
}

export function handshakeOk(topology = "single_duplex") {
  return {
    protocol_version: 4,
    contrib_api_version: CONTRIB_API_VERSION,
    topology,
    topologies: ["single_duplex", "split_streams"],
    subject: { kind: "device", id: "local" },
    client: 1,
  };
}

/** 分离流的假 HTTP 与假 SSE。 */
export class FakeSplit {
  requests: { url: string; body: Record<string, unknown> | null; keepalive?: boolean }[] = [];
  sources: FakeEventSource[] = [];
  handshake: unknown = { ...handshakeOk("split_streams"), credential: "cred-1" };
  rpc: Answer = () => ({});

  readonly fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const body = init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : null;
    this.requests.push({ url: input, body, keepalive: init?.keepalive });
    const path = new URL(input).pathname;
    if (path === "/handshake") return json(this.handshake);
    const method = String(body?.method ?? "");
    const result = this.rpc(method, (body?.params ?? {}) as Record<string, unknown>);
    if (result instanceof RpcFailure) return json({ jsonrpc: "2.0", id: body?.id, error: result.error });
    return json({ jsonrpc: "2.0", id: body?.id, result });
  };

  readonly EventSource = (() => {
    // eslint-disable-next-line @typescript-eslint/no-this-alias
    const split = this;
    return class implements EventSourceLike {
      onmessage: ((event: { data: string }) => void) | null = null;
      onerror: ((event: unknown) => void) | null = null;
      closed = false;
      constructor(public url: string) {
        split.sources.push(new FakeEventSource(this));
      }
      close() {
        this.closed = true;
      }
    };
  })();
}

export class FakeEventSource {
  constructor(readonly es: EventSourceLike & { url: string; closed: boolean }) {}
  get url() {
    return this.es.url;
  }
  push(method: string, params: Record<string, unknown>) {
    this.es.onmessage?.({ data: JSON.stringify({ jsonrpc: "2.0", method, params }) });
  }
  fail() {
    this.es.onerror?.({});
  }
}

function json(value: unknown): Response {
  return new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
}

/** 等所有已排队的微任务与一轮宏任务跑完。 */
export async function settle(times = 3): Promise<void> {
  for (let i = 0; i < times; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
}
