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
 * What is selected is shown beside the conversation, in the shell's second side panel (xops-04 §7):
 * the flows tab's flow, or the runs tab's run (`detail()`). Closing that panel lets go of the
 * selection there and leaves the middle as it is (`closeDetail()`).
 *
 * The wiring needs the host; it starts with `attach(host)` — `registerFlowSpace` does that when it
 * is given one, and every part of the space does it when it is drawn.
 *
 * Shown inside another space (`within`, `registerSystemFlows`): there is no right-column list to
 * follow. A flow's context is entered from the other space's page (`enter`) and left for it
 * (`leave`); in its left column the flow's conversation and its runs are picked, and the last pick
 * says what the middle and the panel beside it show, as the tab in front does otherwise. Coming to
 * the front by another way (a session opened there, the page loaded on it) keeps the session open.
 */

import type { HostSessionRef as SessionRef, NestHost } from "@harnix/react";

import { focusTopic, isDryRun, mainSessionOf, methodName } from "./model";
import type { FlowConversation, Focus, RunView } from "./types";

/** What the second side panel shows: the flows tab's flow, or the runs tab's run. */
export type FlowDetailOf = { kind: "flow"; flow: string } | { kind: "run"; run: string };

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
  /** Shown inside another space: the panel beside the conversation was closed, until the next pick. */
  private shut = false;
  /** The general conversation is being opened: the focus of the session it replaces says nothing. */
  private opening = false;
  readonly tabs: StateTabs;
  /** The space this one is shown inside, if it is (no right-column list then); null otherwise. */
  readonly within: string | null;

  /**
   * @param space the interface space this state belongs to: sessions of other spaces are not ours.
   * @param module the flow module's id (`app.<module>.*`).
   * @param options `within`: the space this one is shown inside.
   */
  constructor(
    readonly space = "flow",
    readonly module = "flow",
    tabs?: StateTabs,
    options: { within?: string } = {},
  ) {
    this.tabs = tabs ?? { flows: `${space}.flows`, runs: `${space}.runs`, reference: `${space}.reference` };
    this.within = options.within ?? null;
    // Inside another space, what was picked last stands for the tab: the flow's conversation first.
    if (this.within) this.tab = this.tabs.flows;
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
      this.lookFront();
      this.follow();
    });
    this.front = host.spaces.current() === this.space;
    if (this.within) {
      this.follow();
      return;
    }
    this.tab = host.details?.tab?.() ?? null;
    host.details?.onTab?.((tab) => {
      if (tab === this.tab) return;
      this.tab = tab;
      // The panel beside the conversation follows the tab.
      this.changed();
      void this.centre();
    });
    this.follow();
  }

  /**
   * Whether this space came to the front since last looked: then the right column's tab is read
   * again and the middle put on the selection. Looked at when the session changes and when a part
   * of the space is drawn — a space switch with no session to change tells nothing else.
   */
  lookFront(): void {
    const host = this.host;
    if (!host) return;
    const front = host.spaces.current() === this.space;
    const arrived = front && !this.front;
    this.front = front;
    if (!arrived) return;
    if (this.within) {
      // Not by `enter`: what is open stays; with nothing open, the selection's conversation.
      this.changed();
      if (!host.session.current()) void this.centre();
      return;
    }
    this.tab = host.details?.tab?.() ?? this.tab;
    this.changed();
    void this.centre();
  }

  /** A flow chosen in the flows tab: shown, and its conversation opened in the middle. */
  selectFlow(flow: string | null): void {
    const same = this.value.flow === flow;
    this.picked(this.tabs.flows);
    this.set({ flow, run: null, station: same ? this.value.station : null });
    void this.centre(this.tabs.flows);
  }

  /** A run chosen in the runs tab: shown, and its main session opened in the middle. */
  showRun(run: string | null): void {
    this.picked(this.tabs.runs);
    // The station stays picked: the same station of another run.
    this.set({ ...this.value, run });
    void this.centre(this.tabs.runs);
  }

  /** Inside another space a pick is the tab: what the middle and the panel follow. */
  private picked(tab: string): void {
    if (!this.within) return;
    this.tab = tab;
    this.shut = false;
  }

  /**
   * Into this space from the one it is shown inside: `flow`'s context (null: the general
   * conversation's), `run` shown if given, brought to the front with the middle on it.
   */
  async enter(flow: string | null, run: string | null = null): Promise<void> {
    const host = this.host;
    if (!host) return;
    const same = this.value.flow === flow;
    this.tab = run ? this.tabs.runs : this.tabs.flows;
    this.shut = false;
    // Selected before the switch: whatever draws on arrival already reads it.
    this.set({ flow, run, station: same ? this.value.station : null });
    if (host.spaces.current() !== this.space) await host.spaces.switch(this.space);
    this.front = host.spaces.current() === this.space;
    await this.centre();
  }

  /** Back to the space this one is shown inside; nothing when it is not shown inside one. */
  async leave(): Promise<void> {
    if (this.within && this.host) await this.host.spaces.switch(this.within);
  }

  /** Bring a list to the front after showing something there: the right column's tab, where there
   *  is one; inside another space the list is the left column's, always in view. */
  openList(tab: string): void {
    if (!this.within) this.host?.details?.open(tab);
  }

  pick(station: string | null): void {
    this.set({ ...this.value, station });
  }

  /** The right column's tab: the shell's, while this space is in front; else as last seen. */
  tabNow(): string | null {
    const host = this.host;
    if (host && !this.within && host.spaces.current() === this.space) {
      const now = host.details?.tab?.();
      if (typeof now === "string") this.tab = now;
    }
    return this.tab;
  }

  /** What the second side panel shows now: the selection of the tab in front; null for none. */
  detail(): FlowDetailOf | null {
    if (this.shut) return null;
    const tab = this.tabNow();
    if (tab === this.tabs.flows && this.value.flow) return { kind: "flow", flow: this.value.flow };
    if (tab === this.tabs.runs && this.value.run) return { kind: "run", run: this.value.run };
    return null;
  }

  /** The second side panel was closed: what it showed is no longer selected; the middle stays. */
  closeDetail(): void {
    if (this.within) {
      // The left column still says what the context is: only the panel goes.
      this.shut = true;
      this.changed();
      return;
    }
    const tab = this.tabNow();
    if (tab === this.tabs.runs) this.set({ ...this.value, run: null });
    else if (tab === this.tabs.flows) this.set({ flow: null, run: this.value.run, station: null });
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
    this.opening = true;
    try {
      if (this.within && this.host) {
        // The general conversation's context, entered if it is not in front.
        this.picked(this.tabs.flows);
        this.set({ flow: null, run: null, station: null });
        if (this.host.spaces.current() !== this.space) await this.host.spaces.switch(this.space);
      }
      const generation = ++this.switching;
      const ref = await this.conversation(null);
      if (!ref || generation !== this.switching) return ref;
      await this.open(ref);
      return ref;
    } finally {
      this.opening = false;
    }
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
    // A session opened by someone else while the answer is awaited wins over it.
    const before = host.session.current();
    let ref: SessionRef | null = null;
    if (tab === this.tabs.runs && run) {
      ref = isDryRun(run) ? await this.conversation(await this.flowOfRun(run)) : await this.mainOf(run);
    } else if (tab === this.tabs.flows && flow) {
      ref = await this.conversation(flow);
    } else if (tab === this.tabs.flows || tab === this.tabs.reference || tab === this.tabs.runs) {
      ref = await this.conversation(null);
    }
    if (!ref || generation !== this.switching) return null;
    if (!(await this.open(ref, before))) return null;
    return ref;
  }

  /** Open `ref` in the middle, unless the session there is no longer `before` (null: none was open;
   *  undefined: open it whatever is there — a person asked). Whether it is open now. */
  private async open(ref: SessionRef, before?: SessionRef | null): Promise<boolean> {
    const host = this.host;
    if (!host) return false;
    const now = host.session.current();
    if (now && now.instance === ref.instance && now.sessionId === ref.sessionId) return true;
    const same = (a: SessionRef | null, b: SessionRef | null) => (a === null ? b === null : b !== null && a.instance === b.instance && a.sessionId === b.sessionId);
    if (before !== undefined && !same(before, now)) return false;
    try {
      await host.session.open(ref);
      return true;
    } catch {
      // Gone between the answer and the open: the middle stays as it was.
      return false;
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
      if (flow !== null && this.value.flow === null && this.value.run === null && !this.opening) {
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
