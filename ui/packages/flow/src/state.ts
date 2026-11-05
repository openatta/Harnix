/**
 * What the flow space shows, shared by its parts, and the conversation in the middle (xops-03 §6):
 * **what is selected on the right is what the middle talks to.**
 *
 * - The flows tab with a flow selected: that flow's own conversation (`session {flow}`, made by
 *   the module the first time). The runs tab with a run selected: the run's main session (its
 *   `@main` session key). Anything else — nothing selected, the reference tab: the general
 *   conversation (`session {}`), where flows are created and questions asked.
 * - The middle is switched when the selection or the tab changes, and when this space comes to the
 *   front; an answer to an older switch is dropped.
 * - The `focus {instance, session_id}` topic is followed for the open session, for the line over
 *   the composer: a flow's conversation works on that flow. When the general conversation creates
 *   a flow it becomes that flow's, and with nothing selected the flow is selected; a flow deleted
 *   under its own conversation clears the selection.
 *
 * The wiring needs the host; it starts with `attach(host)` — `registerFlowSpace` does that when it
 * is given one, and every part of the space does it when it is drawn.
 */

import type { HostSessionRef as SessionRef, NestHost } from "@harnix/react";

import { focusTopic, isDryRun, mainSessionOf, methodName } from "./model";
import type { FlowConversation, Focus, RunView } from "./types";

export interface FlowSelection {
  flow: string | null;
  run: string | null;
  station: string | null;
}

/** The right column's tab ids, as the state needs them. */
export interface StateTabs {
  flows: string;
  runs: string;
  reference: string;
}

export class FlowSpaceState {
  private value: FlowSelection = { flow: null, run: null, station: null };
  private readonly listeners = new Set<() => void>();
  private host: NestHost | null = null;
  /** `instance/session_id` of the session the topic is followed for. */
  private followed: string | null = null;
  private unfollow: (() => void) | null = null;
  /** The followed session's focus, as the topic last said. */
  private remembered: string | null | undefined = undefined;
  /** The right column's tab, as last seen. */
  private tab: string | null = null;
  /** Bumped on every switch of the middle: answers to an older one are dropped. */
  private switching = 0;
  /** This space was in front when last looked. */
  private front = false;
  readonly tabs: StateTabs;

  /**
   * @param space the interface space this state belongs to: sessions of other spaces are not ours.
   * @param module the flow module's id (`app.<module>.*`).
   */
  constructor(
    readonly space = "flow",
    readonly module = "flow",
    tabs?: StateTabs,
  ) {
    this.tabs = tabs ?? { flows: `${space}.flows`, runs: `${space}.runs`, reference: `${space}.reference` };
  }

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly get = (): FlowSelection => this.value;

  /** The open session's focus as the module last said, undefined before it has. */
  focusOf(): string | null | undefined {
    return this.remembered;
  }

  /** Start following; the first host wins, later calls do nothing. */
  attach(host: NestHost): void {
    if (this.host) return;
    this.host = host;
    host.session.onChange(() => {
      // Brought to the front: the middle is what the selection says.
      const front = host.spaces.current() === this.space;
      const arrived = front && !this.front;
      this.front = front;
      if (arrived) void this.centre();
      this.follow();
    });
    this.front = host.spaces.current() === this.space;
    this.tab = host.details?.tab?.() ?? null;
    host.details?.onTab?.((tab) => {
      if (tab === this.tab) return;
      this.tab = tab;
      void this.centre();
    });
    this.follow();
  }

  /** A flow chosen in the flows tab: shown, and its conversation opened in the middle. */
  selectFlow(flow: string | null): void {
    const same = this.value.flow === flow;
    this.set({ flow, run: null, station: same ? this.value.station : null });
    void this.centre(this.tabs.flows);
  }

  /** A run chosen in the runs tab: shown, and its main session opened in the middle. */
  showRun(run: string | null): void {
    // The station stays picked: the same station of another run.
    this.set({ ...this.value, run });
    void this.centre(this.tabs.runs);
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

  /** The general conversation, opened in the middle; null when it could not be. */
  async openGeneral(): Promise<SessionRef | null> {
    const generation = ++this.switching;
    const ref = await this.conversation(null);
    if (!ref || generation !== this.switching) return ref;
    await this.open(ref);
    return ref;
  }

  /**
   * Put the middle on what the selection says, for `tab` (the one in front by default). Nothing
   * while another space is in front or before the host is known. Resolves to what was opened.
   */
  async centre(tab: string | null = this.tab): Promise<SessionRef | null> {
    const host = this.host;
    if (!host || host.spaces.current() !== this.space) return null;
    const generation = ++this.switching;
    const { flow, run } = this.value;
    let ref: SessionRef | null = null;
    if (tab === this.tabs.runs && run) {
      ref = isDryRun(run) ? await this.conversation(await this.flowOfRun(run)) : await this.mainOf(run);
    } else if (tab === this.tabs.flows && flow) {
      ref = await this.conversation(flow);
    } else if (tab === this.tabs.flows || tab === this.tabs.reference || tab === this.tabs.runs) {
      ref = await this.conversation(null);
    }
    if (!ref || generation !== this.switching) return null;
    await this.open(ref);
    return ref;
  }

  private async open(ref: SessionRef): Promise<void> {
    const host = this.host;
    if (!host) return;
    const now = host.session.current();
    if (now && now.instance === ref.instance && now.sessionId === ref.sessionId) return;
    try {
      await host.session.open(ref);
    } catch {
      // Gone between the answer and the open: the middle stays as it was.
    }
  }

  /** A flow's conversation (null: the general one), made by the module when it has none. */
  private async conversation(flow: string | null): Promise<SessionRef | null> {
    const host = this.host;
    if (!host) return null;
    try {
      const c = await host.call<FlowConversation>(methodName(this.module, "session"), flow ? { flow } : {});
      return c && typeof c.instance === "string" && typeof c.session_id === "string" ? { instance: c.instance, sessionId: c.session_id } : null;
    } catch {
      return null;
    }
  }

  private async mainOf(run: string): Promise<SessionRef | null> {
    const host = this.host;
    if (!host) return null;
    try {
      return mainSessionOf((await host.call<RunView>(methodName(this.module, "get"), { run }))?.record);
    } catch {
      return null;
    }
  }

  private async flowOfRun(run: string): Promise<string | null> {
    try {
      return (await this.host?.call<RunView>(methodName(this.module, "get"), { run }))?.record?.flow ?? null;
    } catch {
      return null;
    }
  }

  private follow(): void {
    const session = this.session();
    const key = session ? `${session.instance}/${session.sessionId}` : null;
    if (key === this.followed) return;
    this.unfollow?.();
    this.unfollow = null;
    this.followed = key;
    this.remembered = undefined;
    if (!session || !this.host) {
      this.changed();
      return;
    }
    this.unfollow = this.host.subscribe(focusTopic(this.module), { instance: session.instance, session_id: session.sessionId }, (v) => {
      if (this.followed !== key) return;
      const before = this.remembered;
      const flow = (v as Focus | null)?.flow ?? null;
      this.remembered = flow;
      if (flow !== null && this.value.flow === null && this.value.run === null) {
        // The general conversation created a flow and became its own.
        this.set({ flow, run: null, station: null });
      } else if (flow === null && before && before === this.value.flow && this.value.run === null) {
        // The flow this conversation was for is gone.
        this.set({ flow: null, run: null, station: null });
      } else {
        this.changed();
      }
    });
  }

  private set(next: FlowSelection): void {
    this.value = next;
    this.changed();
  }

  /** Whether a redraw is already asked for. */
  private redrawing = false;

  private changed(): void {
    // A new object each time, so a reader of `get` draws again when only the focus changed.
    this.value = { ...this.value };
    for (const fn of [...this.listeners]) fn();
    // The space's `notice()` reads the selection: the shell reads it again — only while this space
    // is in front, and never from inside the shell's own notification (a session change calls
    // `follow` while the shell is still telling its listeners).
    const host = this.host;
    if (!host?.redraw || this.redrawing || host.spaces?.current?.() !== this.space) return;
    this.redrawing = true;
    queueMicrotask(() => {
      this.redrawing = false;
      if (host.spaces.current() === this.space) host.redraw();
    });
  }
}
