/**
 * 一个由测试驱动的假客户端：测试就是服务端。实现 SessionController 与组件用到的那部分
 * HarnixClient 表面（status、onStatus、onNotify、call、answer、notify）。
 */

import type { ConnectionStatus, HarnixClient, NotifyListener, StatusListener } from "@harnix/core";

type Handler = (params: Record<string, unknown>) => unknown | Promise<unknown>;

export class FakeClient {
  status: ConnectionStatus = "open";
  detail: string | undefined;
  calls: { method: string; params: Record<string, unknown> }[] = [];
  notified: { method: string; params: Record<string, unknown> }[] = [];
  handlers: Record<string, Handler> = {};
  private notifyListeners = new Set<NotifyListener>();
  private statusListeners = new Set<StatusListener>();
  private seq = 0;

  asClient(): HarnixClient {
    return this as unknown as HarnixClient;
  }

  onNotify(fn: NotifyListener) {
    this.notifyListeners.add(fn);
    return () => this.notifyListeners.delete(fn);
  }

  onStatus(fn: StatusListener) {
    this.statusListeners.add(fn);
    return () => this.statusListeners.delete(fn);
  }

  setStatus(status: ConnectionStatus, detail?: string) {
    this.status = status;
    this.detail = detail;
    for (const fn of [...this.statusListeners]) fn(status, detail);
  }

  async call(method: string, params: Record<string, unknown> = {}) {
    this.calls.push({ method, params });
    const handler = this.handlers[method];
    if (!handler) return {};
    return handler(params);
  }

  answer(method: string, params: Record<string, unknown> = {}) {
    return this.call(method, params);
  }

  notify(method: string, params: Record<string, unknown> = {}) {
    this.notified.push({ method, params });
  }

  /** 服务端推一条通知。 */
  push(method: string, params: Record<string, unknown>) {
    for (const fn of [...this.notifyListeners]) fn(method, params);
  }

  /** 推一条 harnix.event，seq 自动递增（也可指定）。 */
  event(sessionId: string, event: Record<string, unknown>, seq?: number) {
    this.seq = seq ?? this.seq + 1;
    this.push("harnix.event", { session_id: sessionId, seq: this.seq, turn_id: "t1", event });
  }

  setSeq(seq: number) {
    this.seq = seq;
  }

  called(method: string) {
    return this.calls.filter((c) => c.method === method);
  }
}

/** 一个空会话的 attach 应答。 */
export function attached(sessionId: string, extra: Record<string, unknown> = {}) {
  return {
    session: { session_id: sessionId, name: "测试会话", scene: "coding", turn_state: "idle" },
    history_total: 0,
    replay: [],
    truncated: false,
    pending_prompts: [],
    running_turn: null,
    queue: [],
    seq: 0,
    ...extra,
  };
}

export async function flush(times = 5) {
  for (let i = 0; i < times; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
}
