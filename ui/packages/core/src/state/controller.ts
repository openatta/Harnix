/**
 * 一个打开的会话：attach、追赶、接事件、发送、中断、回答提问、撤回排队。
 *
 * 来自 IPDrun nest-ui（D21）。不依赖 React：状态是一份不可变快照，`subscribe` / `getSnapshot`
 * 正好对上 `useSyncExternalStore`，测试里也能直接驱动。
 *
 * 会话的全名是 `(instance, session)`：每个调用都带 `instance`，每条通知都按两者过滤（设计 §5.6）。
 *
 * 追赶的顺序（概念与架构 §3.2、§3.3.3）：attach 拿到 seq、队列与缓冲区快照 → 读历史 →
 * 应用重放与挂起的提问 → 之后只收 `seq` 大于已见值的 `harnix.event`。断线重连后再 attach 一次，
 * 就是"刷新不丢内容"；中枢推 `harnix.session_resync`（实例重连、订阅被引擎丢掉）时也一样。一轮的每一部分只从一边来：一轮在跑时 `history_total` 是它开始前的
 * 消息数（中枢 N-120），这一轮全部来自重放。
 */

import { errorText, type HarnixClient } from "../client/index.ts";
import type { Decision, Notifications } from "../protocol/index.ts";
import {
  addCustom,
  addEntry,
  answerAsk,
  applyAttach,
  applyEvent,
  applyHistoryMessage,
  applyReplay,
  applySettled,
  blockByKey,
  emptySession,
  note,
  type AskAnswer,
  type SessionEntry,
  type SessionState,
} from "./model.ts";

/** 控制器自己说的话：由界面层按语言给出。 */
export interface SessionStrings {
  openFailed(reason: string): string;
  replayTruncated: string;
  olderHidden(count: number): string;
  result: string;
  image: string;
  stopReason(reason: string): string;
  /** 人的话被拒收（ipdrun-13）：理由，及被拒的那句（steer 时）。缺省只显示理由。 */
  messageRefused?(reason: string, text?: string): string;
  interrupted: string;
  turnFailed(reason: string): string;
  sendFailed(reason: string): string;
  interruptFailed(reason: string): string;
  answerFailed(reason: string): string;
}

/** 没有 `instance` 的调用与通知都是 `system` 的（设计 §5.6）。 */
export const SYSTEM_INSTANCE = "system";

const HISTORY_PAGE = 200;
const HISTORY_PAGES_AT_OPEN = 5;
const DEFAULT_PROMPT_TIMEOUT_SECS = 300;

export interface SessionControllerOptions {
  client: HarnixClient;
  /** 缺省 `system`。 */
  instance?: string;
  /** 调用方所在的 space：每个调用都带上，中枢对着账本核对（设计 §8.5）。缺省不带。 */
  space?: string;
  /** 发送时加进 `harnix.send` 的参数（例如 `@session` 引用），按这条消息算。 */
  extras?: (text: string) => Record<string, unknown>;
  sessionId: string;
  strings: SessionStrings;
  /** 引擎对未答提问的超时（秒）；缺省 300，与 Agix 的 --permission-prompt-timeout 一致。 */
  promptTimeoutSecs?: number;
  now?: () => number;
}

export class SessionController {
  private readonly client: HarnixClient;
  private readonly strings: SessionStrings;
  readonly instance: string;
  readonly space: string | undefined;
  private readonly extras: ((text: string) => Record<string, unknown>) | undefined;
  private readonly promptTimeoutSecs: number;
  private readonly now: () => number;
  private state: SessionState;
  private readonly listeners = new Set<() => void>();
  private readonly offs: (() => void)[] = [];
  /** 每次 open 自增；旧的 open 回来时发现代数变了就作废（用户已经走开了）。 */
  private generation = 0;
  private stopped = true;
  /**
   * 我们要求中断、还在等它结束的那一轮。"被中断"不是引擎会报告的状态——被取消的一轮
   * 不写终止标记——所以只有发起中断的一方知道它是被截断而不是正常结束。
   */
  private interrupting = false;

  constructor(options: SessionControllerOptions) {
    this.client = options.client;
    this.instance = options.instance ?? SYSTEM_INSTANCE;
    this.space = options.space;
    this.extras = options.extras;
    this.strings = options.strings;
    this.promptTimeoutSecs = options.promptTimeoutSecs ?? DEFAULT_PROMPT_TIMEOUT_SECS;
    this.now = options.now ?? Date.now;
    this.state = emptySession(options.sessionId);
  }

  /** 开始观看：挂上通知与状态监听，attach。与 stop() 成对，可以反复（React 严格模式会这样做）。 */
  start(): void {
    if (this.offs.length) return;
    this.stopped = false;
    this.offs.push(this.client.onNotify((method, params) => this.handle(method, params)));
    // 重连后重新 attach：断线期间跑完或还在跑的那一轮，从中枢的缓冲区与历史补齐。
    this.offs.push(
      this.client.onStatus((status) => {
        if (status === "open") void this.open();
      }),
    );
    void this.open();
  }

  /** 停止观看：detach，摘掉监听，作废进行中的 open。 */
  stop(): void {
    if (!this.offs.length) return;
    this.stopped = true;
    this.generation += 1;
    for (const off of this.offs.splice(0)) off();
    // 推迟一拍：React 严格模式会立刻 stop 再 start，那时不该 detach——detach 与紧随的
    // attach 在服务端可能并发处理，先后不定，会把刚登记的观看者摘掉。
    queueMicrotask(() => {
      if (this.stopped && this.client.status === "open") {
        this.client.notify("harnix.detach", this.key());
      }
    });
  }

  /* ── 观察 ─────────────────────────────────────────────────────────── */

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly getSnapshot = (): SessionState => this.state;

  private set(next: SessionState): void {
    if (next === this.state) return;
    this.state = next;
    for (const fn of [...this.listeners]) fn();
  }

  get sessionId(): string {
    return this.state.sessionId;
  }

  /** 调用里的会话地址。 */
  private key(): { instance: string; session_id: string; space?: string } {
    const key: { instance: string; session_id: string; space?: string } = { instance: this.instance, session_id: this.state.sessionId };
    if (this.space) key.space = this.space;
    return key;
  }

  /* ── 打开 ─────────────────────────────────────────────────────────── */

  /** attach 并追赶。连接还没开时什么也不做，等 onStatus("open") 再来。 */
  async open(): Promise<void> {
    if (this.stopped || this.client.status !== "open") return;
    const generation = ++this.generation;
    const sessionId = this.state.sessionId;
    const stale = () => this.stopped || generation !== this.generation;

    let fresh = emptySession(sessionId);
    this.interrupting = false;
    this.held = [];
    this.set(fresh);

    let attached;
    try {
      attached = await this.client.call("harnix.attach", this.key());
    } catch (e) {
      if (stale()) return;
      this.set({ ...note(fresh, this.strings.openFailed(errorText(e)), "error"), loading: false, failed: errorText(e) });
      return;
    }
    if (stale()) return;
    fresh = applyAttach(fresh, attached, this.strings.replayTruncated);

    try {
      // 重放里有的轮不从历史读：中枢已把历史截在这一轮之前，但压缩会让消息数变小，那时这一轮已落盘的部分
      // 可能落进 history_total 之内——按 turn_id 再挡一次（旧日志没有 turn_id，不受影响）。
      const replayed = new Set((attached.replay || []).map((frame) => frame.turn_id).filter(Boolean));
      fresh = await this.loadHistory(fresh, attached.history_total || 0, stale, replayed);
    } catch (e) {
      fresh = note(fresh, this.strings.openFailed(errorText(e)), "error");
    }
    if (stale()) return;

    fresh = applyReplay(fresh, attached, this.promptTimeoutSecs, this.now());
    this.set(fresh);
    // attach 应答之前或读历史期间到达的通知：现在按到达顺序补上。缓冲区快照里没有它们
    // （中枢在同一把锁下登记观看者并取快照），harnix.event 再按 seq 去重一次。
    const held = this.held;
    this.held = [];
    for (const [method, params] of held) this.handle(method, params);
  }

  /** 追赶期间到达的本会话通知，open() 收尾时按序补上。 */
  private held: [string, Record<string, unknown>][] = [];

  private async loadHistory(state: SessionState, total: number, stale: () => boolean, replayed: Set<string>): Promise<SessionState> {
    let next = state;
    const start = Math.max(0, total - HISTORY_PAGE * HISTORY_PAGES_AT_OPEN);
    // The application's entries go back between the messages they were written between; the
    // ones before the first message shown are as hidden as those messages.
    const entries = (await this.entries()).filter((e) => e.at >= start);
    if (stale()) return next;
    let pending = 0;
    const placeUpTo = (seq: number) => {
      while (pending < entries.length && entries[pending]!.at <= seq) next = addEntry(next, entries[pending++]!);
    };
    if (start > 0) next = note(next, this.strings.olderHidden(start));
    for (let offset = start; offset < total; offset += HISTORY_PAGE) {
      const page = await this.client.call("session.history", {
        ...this.key(),
        session_id: next.sessionId,
        offset,
        // 读到 total 为止：其后由重放接上，多读的部分会画两遍
        limit: Math.min(HISTORY_PAGE, total - offset),
      });
      if (stale()) return next;
      for (const message of page.messages || []) {
        placeUpTo(message.seq ?? offset);
        if (message.turn_id && replayed.has(message.turn_id)) continue;
        next = applyHistoryMessage(next, message, { result: this.strings.result, image: this.strings.image });
      }
    }
    placeUpTo(Number.POSITIVE_INFINITY);
    return next;
  }

  /** The session's entries, oldest first. An engine without them (before Agix 0.2.8) has none. */
  private async entries(): Promise<SessionEntry[]> {
    try {
      const listed = (await this.client.call("session.entries", { ...this.key(), session_id: this.state.sessionId })) as { entries?: SessionEntry[] };
      return listed.entries ?? [];
    } catch {
      return [];
    }
  }

  private applyOptions() {
    return {
      promptTimeoutSecs: this.promptTimeoutSecs,
      now: this.now(),
      interrupting: this.interrupting,
      resultLabel: this.strings.result,
      stopReasonText: this.strings.stopReason,
      refusedText: this.strings.messageRefused?.bind(this.strings),
      interruptedText: this.strings.interrupted,
    };
  }

  /* ── 服务端通知 ───────────────────────────────────────────────────── */

  private handle(method: string, raw: Record<string, unknown>): void {
    if (this.stopped) return;
    const sessionId = this.state.sessionId;
    if (raw.session_id !== sessionId) return;
    if ((raw.instance ?? SYSTEM_INSTANCE) !== this.instance) return;
    if (method === "harnix.session_resync") {
      // 中枢重建了这个会话的缓冲：按刷新那条路重新 attach。
      void this.open();
      return;
    }
    if (this.state.loading) {
      // 还在追赶：先记下，open() 收尾时补上。
      this.held.push([method, raw]);
      return;
    }

    if (method === "harnix.event") {
      const params = raw as unknown as Notifications["harnix.event"];
      if (params.seq <= this.state.seq) return;
      const next = applyEvent({ ...this.state, seq: params.seq }, params.event, this.applyOptions());
      // 一条 user_message 是一轮开始的信号（中枢合成的那条）。
      const turn = params.event.kind === "user_message" ? { turnId: params.turn_id } : next.turn;
      this.set({ ...next, turn });
      return;
    }

    if (method === "harnix.turn_settled") {
      const params = raw as unknown as Notifications["harnix.turn_settled"];
      const wasInterrupted = this.interrupting;
      this.interrupting = false;
      this.set(
        applySettled(
          this.state,
          params,
          wasInterrupted,
          {
            interrupted: this.strings.interrupted,
            failed: this.strings.turnFailed,
          },
          this.now(),
        ),
      );
      return;
    }

    if (method === "harnix.queue") {
      const params = raw as unknown as Notifications["harnix.queue"];
      // 整值快照：直接替换，不合并。
      this.set({ ...this.state, queue: params.items || [] });
    }
  }

  /* ── 动作 ─────────────────────────────────────────────────────────── */

  /**
   * 发送；有一轮在跑时进中枢的队列（on_busy: queue），上一轮结束后自动发出。`onBusy: "steer"`：在下一个步骤边界
   * 送进正在跑的这一轮（带附件的照中枢的规则排队）。
   */
  async send(text: string, attachments: unknown[] = [], options?: { onBusy?: "queue" | "steer" }): Promise<void> {
    const message = text.trim();
    if (!message) return;
    const sessionId = this.state.sessionId;
    // 挂着的中断属于当时在跑的那一轮；若它没结束（连接断了），下一轮不该继承这条提示。
    this.interrupting = false;
    try {
      const result = await this.client.call("harnix.send", {
        ...this.extras?.(message),
        ...this.key(),
        message,
        attachments,
        on_busy: options?.onBusy ?? "queue",
      });
      if (this.state.sessionId !== sessionId) return;
      if ("queued" in result && result.queued) {
        const item = result.item;
        if (!this.state.queue.some((q) => q.item_id === item.item_id)) {
          this.set({ ...this.state, queue: [...this.state.queue, item] });
        }
      } else if ("turn_id" in result && !(options?.onBusy === "steer" && this.state.turn)) {
        // A steered message goes into the turn already running: that turn stays the one shown.
        this.set({ ...this.state, turn: { turnId: result.turn_id } });
      }
    } catch (e) {
      this.set(note(this.state, this.strings.sendFailed(errorText(e)), "error"));
    }
  }

  /** 中断正在跑的一轮；中枢同时清空队列（整值推送一个空队列）。 */
  async interrupt(): Promise<void> {
    // 先记下：取消的事件（工具的 "cancelled" 结果、turn_settled）可能比 session.interrupt 的应答先到
    this.interrupting = true;
    try {
      await this.client.call("session.interrupt", this.key());
    } catch (e) {
      this.interrupting = false;
      this.set({ ...this.state, banner: this.strings.interruptFailed(errorText(e)) });
    }
  }

  /**
   * 应用经自己的路径中断（例如服务端代为中断）时，先告诉控制器：这一轮接下来的失败是中断造成的。
   */
  markInterrupting(): void {
    if (this.state.turn != null) this.interrupting = true;
  }

  /**
   * 回答一张权限卡。任何一个观看者都能答，先答者生效，第二个回答是静默成功——所以每个
   * 打开的页面上按钮都保持可用，而不是只留一个。
   */
  async answer(key: number, decision: Decision): Promise<void> {
    const block = blockByKey(this.state, key);
    if (!block || block.kind !== "ask" || block.answered) return;
    const answer: AskAnswer = decision.type === "deny" ? "deny" : "allow";
    this.set(answerAsk(this.state, key, answer));
    try {
      await this.client.answer("session.respondToPrompt", {
        ...this.key(),
        prompt_id: block.prompt.prompt_id,
        // Who answered is kept by the hub for the tool's audit; the engine never sees it.
        ...(block.prompt.tool_use_id ? { tool_use_id: block.prompt.tool_use_id } : {}),
        decision,
      });
    } catch (e) {
      this.set({ ...this.state, banner: this.strings.answerFailed(errorText(e)) });
    }
  }

  /** 引擎超时即按拒绝处理；卡片自己数到零时标成超时。 */
  expire(key: number): void {
    this.set(answerAsk(this.state, key, "timeout"));
  }

  async removeQueued(itemId: string): Promise<void> {
    try {
      const result = await this.client.call("harnix.queue.remove", {
        ...this.key(),
        item_id: itemId,
      });
      this.set({ ...this.state, queue: result.items || [] });
    } catch (e) {
      this.set({ ...this.state, banner: errorText(e) });
    }
  }

  /** 放一块界面自己的内容进流（`custom` 块）。还在追赶时不放：追赶会整个重建状态。 */
  insert(type: string, data: unknown): boolean {
    if (this.state.loading) return false;
    this.set(addCustom(this.state, type, data, this.now()));
    return true;
  }

  dismissBanner(): void {
    this.set({ ...this.state, banner: null });
  }
}
