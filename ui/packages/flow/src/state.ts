/**
 * What the flow space shows, shared by its parts, and the conversation's current flow (xops-02 §4,
 * §7): the flow selected in the right column is the open conversation's `focus`.
 *
 * - Selecting a flow calls `focus {instance, session_id, flow}` for the open session of this space.
 * - Another session opened here: the selection, if there is one, is sent for it; with nothing
 *   selected, the session's own remembered flow becomes the selection.
 * - The `focus {instance, session_id}` topic is followed for the open session: a flow the Agent
 *   created becomes the selection, a deleted one clears it. A value that arrives before this page's
 *   own `focus` call for that session has been answered is not followed (it is the old one).
 *
 * The wiring needs the host; it starts with `attach(host)` — `registerFlowSpace` does that when it
 * is given one, and every part of the space does it when it is drawn.
 */

import type { HostSessionRef as SessionRef, NestHost } from "@harnix/react";

import { focusTopic, methodName } from "./model";
import type { Focus } from "./types";

export interface FlowSelection {
  flow: string | null;
  run: string | null;
  station: string | null;
}

export class FlowSpaceState {
  private value: FlowSelection = { flow: null, run: null, station: null };
  private readonly listeners = new Set<() => void>();
  private host: NestHost | null = null;
  /** `instance/session_id` of the session the topic is followed for. */
  private followed: string | null = null;
  private unfollow: (() => void) | null = null;
  /** Bumped on every session change and every `focus` call: answers to an older one are ignored. */
  private generation = 0;
  /** The page's own `focus` call for the followed session has been answered. */
  private settled = true;
  /** The followed session's focus, as the topic last said. */
  private remembered: string | null | undefined = undefined;

  /**
   * @param space the interface space this state belongs to: sessions of other spaces are not ours.
   * @param module the flow module's id (`app.<module>.*`).
   */
  constructor(
    readonly space = "flow",
    readonly module = "flow",
  ) {}

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly get = (): FlowSelection => this.value;

  /** The open session's focus as the module last said, undefined before it has. */
  focusOf(): string | null | undefined {
    return this.remembered;
  }

  /** Start following the open session's focus; the first host wins, later calls do nothing. */
  attach(host: NestHost): void {
    if (this.host) return;
    this.host = host;
    host.session.onChange(() => this.follow());
    this.follow();
  }

  /** A flow chosen in the right column: shown, and made the open conversation's focus. */
  selectFlow(flow: string | null): void {
    const same = this.value.flow === flow;
    this.set({ flow, run: same ? this.value.run : null, station: same && !this.value.run ? this.value.station : null });
    this.sendFocus(flow);
  }

  showRun(run: string | null): void {
    // The station stays picked: the same station of another run of the flow.
    this.set({ ...this.value, run });
  }

  pick(station: string | null): void {
    this.set({ ...this.value, station });
  }

  /** The session of this space now open, if any. */
  session(): SessionRef | null {
    const host = this.host;
    if (!host) return null;
    if (host.spaces.current() !== this.space) return null;
    return host.session.current();
  }

  private follow(): void {
    const session = this.session();
    const key = session ? `${session.instance}/${session.sessionId}` : null;
    if (key === this.followed) return;
    this.unfollow?.();
    this.unfollow = null;
    this.followed = key;
    this.remembered = undefined;
    this.generation++;
    this.settled = true;
    if (!session || !this.host) return this.changed();
    // The selection goes to the new session; with none, the session's own is taken up.
    if (this.value.flow !== null) this.sendFocus(this.value.flow);
    this.unfollow = this.host.subscribe(focusTopic(this.module), { instance: session.instance, session_id: session.sessionId }, (v) => {
      if (this.followed !== key) return;
      const flow = (v as Focus | null)?.flow ?? null;
      this.remembered = flow;
      if (this.settled && flow !== this.value.flow) {
        this.set({ flow, run: null, station: null });
      } else {
        this.changed();
      }
    });
  }

  private sendFocus(flow: string | null): void {
    const session = this.session();
    if (!session || !this.host) return;
    const generation = ++this.generation;
    this.settled = false;
    this.host
      .call<Focus>(methodName(this.module, "focus"), { instance: session.instance, session_id: session.sessionId, flow })
      .then((answer) => {
        if (generation !== this.generation) return;
        this.remembered = answer?.flow ?? flow;
      })
      .catch(() => {
        // A flow that is gone (deleted elsewhere): the module keeps what it had.
      })
      .finally(() => {
        if (generation !== this.generation) return;
        this.settled = true;
        this.changed();
      });
  }

  private set(next: FlowSelection): void {
    this.value = next;
    this.changed();
  }

  private changed(): void {
    // A new object each time, so a reader of `get` draws again when only the focus changed.
    this.value = { ...this.value };
    for (const fn of [...this.listeners]) fn();
    this.host?.redraw?.();
  }
}
