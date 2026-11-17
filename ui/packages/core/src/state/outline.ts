/**
 * A session's turns, for a turn rail and for a trajectory: `harnix.session.outline` (every turn of the
 * session, read off its transcript by the hub), read again each time a turn of it settles, and merged with
 * the turns already shown — the one running is in no transcript yet, and an outline that cannot be read
 * leaves those shown.
 *
 * Framework-free like the session controller: `subscribe` / `getSnapshot` for `useSyncExternalStore`.
 */

import { errorText, type HarnixClient } from "../client/index.ts";
import type { Notifications, OutlineMark, OutlineTurn } from "../protocol/index.ts";
import type { Block } from "./model.ts";
import { MAIN_INSTANCE } from "./controller.ts";

/** How much of a turn's words a row shows, as the hub cuts them: about a line, about three lines. */
export const OUTLINE_PROMPT_CHARS = 100;
export const OUTLINE_REPLY_CHARS = 300;

export interface OutlineSnapshot {
  /** The outline's turns, oldest first; empty until it is read, or when it cannot be. */
  turns: readonly OutlineTurn[];
  /** It was read at least once. */
  loaded: boolean;
  /** Why the last read failed. */
  failed: string | null;
}

export interface OutlineControllerOptions {
  client: HarnixClient;
  instance?: string;
  space?: string;
  sessionId: string;
}

const EMPTY: OutlineSnapshot = { turns: [], loaded: false, failed: null };

export class OutlineController {
  private readonly client: HarnixClient;
  readonly instance: string;
  readonly space: string | undefined;
  readonly sessionId: string;
  private state: OutlineSnapshot = EMPTY;
  private readonly listeners = new Set<() => void>();
  private offs: (() => void)[] = [];
  /** Each read takes the next; one that comes back after a later one is dropped. */
  private generation = 0;

  constructor(options: OutlineControllerOptions) {
    this.client = options.client;
    this.instance = options.instance ?? MAIN_INSTANCE;
    this.space = options.space;
    this.sessionId = options.sessionId;
  }

  /** Read it, and again whenever a turn of the session settles or the connection comes back. */
  start(): void {
    if (this.offs.length) return;
    this.offs.push(
      this.client.onNotification((method, raw) => {
        if (method !== "harnix.turn_settled" && method !== "harnix.session_resync") return;
        const params = raw as unknown as Notifications["harnix.turn_settled"];
        if (params.session_id !== this.sessionId || (params.instance ?? MAIN_INSTANCE) !== this.instance) return;
        void this.refresh();
      }),
    );
    this.offs.push(
      this.client.onStatus((status) => {
        if (status === "open") void this.refresh();
      }),
    );
    void this.refresh();
  }

  stop(): void {
    this.generation += 1;
    for (const off of this.offs.splice(0)) off();
  }

  readonly subscribe = (fn: () => void): (() => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  readonly getSnapshot = (): OutlineSnapshot => this.state;

  async refresh(): Promise<void> {
    if (this.client.status !== "open") return;
    const generation = ++this.generation;
    const key: { instance: string; session_id: string; space?: string } = { instance: this.instance, session_id: this.sessionId };
    if (this.space) key.space = this.space;
    try {
      const outline = await this.client.call("harnix.session.outline", key);
      if (generation !== this.generation) return;
      this.set({ turns: Array.isArray(outline?.turns) ? outline.turns.filter(validTurn) : [], loaded: true, failed: null });
    } catch (e) {
      if (generation !== this.generation) return;
      this.set({ ...this.state, loaded: true, failed: errorText(e) });
    }
  }

  private set(next: OutlineSnapshot): void {
    this.state = next;
    for (const fn of [...this.listeners]) fn();
  }
}

/** A row a rail can go to: one with a place in the transcript. Its words are shown as they are or not at all. */
function validTurn(turn: OutlineTurn): boolean {
  return turn != null && typeof turn.seq === "number" && Number.isFinite(turn.seq);
}

/** One turn as a rail shows it: the outline's row, or a turn only the page has shown. */
export interface NavTurn {
  /** Stable while the turn is listed: its turn id, else its place in the transcript. */
  key: string;
  turnId: string | null;
  /** Where it starts in the transcript; null for a turn only seen live. */
  seq: number | null;
  at: string | null;
  prompt: string;
  reply: string;
  marks?: OutlineMark[];
}

/** `text` on one line, cut to `max` characters with `…`, as the hub cuts the outline's. */
export function previewText(text: string, max: number): string {
  const line = text.split(/\s+/).filter(Boolean).join(" ");
  const chars = [...line];
  return chars.length <= max ? line : `${chars.slice(0, Math.max(0, max - 1)).join("")}…`;
}

/** The user block that starts each turn shown, with its turn's last reply: the turns as the page has them. */
export function shownTurns(blocks: readonly Block[]): { block: Extract<Block, { kind: "user" }>; reply: string }[] {
  const turns: { block: Extract<Block, { kind: "user" }>; reply: string }[] = [];
  for (const block of blocks) {
    if (block.kind === "user") {
      if (block.steered || (block.turnId == null && block.seq == null)) continue;
      const last = turns[turns.length - 1];
      if (block.turnId != null && last?.block.turnId === block.turnId) continue;
      turns.push({ block, reply: "" });
    } else if (block.kind === "assistant" && block.text.trim()) {
      const last = turns[turns.length - 1];
      if (last) last.reply = block.text;
    }
  }
  return turns;
}

/**
 * The outline's turns, then the turns shown that it does not have yet (the one running, one that settled
 * since it was read) — or, with no outline, the turns shown.
 */
export function mergeTurns(outline: readonly OutlineTurn[], blocks: readonly Block[]): NavTurn[] {
  const merged: NavTurn[] = outline.map((t) => ({
    key: t.turn_id ?? `seq:${t.seq}`,
    turnId: t.turn_id,
    seq: t.seq,
    at: t.at,
    prompt: t.prompt,
    reply: t.reply,
    ...(t.marks?.length ? { marks: t.marks } : {}),
  }));
  const ids = new Set(outline.map((t) => t.turn_id).filter((id): id is string => id != null));
  const seqs = new Set(outline.map((t) => t.seq));
  const last = outline.length ? outline[outline.length - 1]!.seq : -1;
  for (const { block, reply } of shownTurns(blocks)) {
    if (block.turnId != null ? ids.has(block.turnId) : block.seq != null && seqs.has(block.seq)) continue;
    // Shown from history before the outline's last turn yet not in it: the outline is older than what is
    // shown, and will have it when read again.
    if (block.turnId == null && block.seq != null && block.seq < last) continue;
    merged.push({
      key: block.turnId ?? `seq:${block.seq}`,
      turnId: block.turnId ?? null,
      seq: block.seq ?? null,
      at: block.at != null ? new Date(block.at).toISOString() : null,
      prompt: previewText(block.text, OUTLINE_PROMPT_CHARS),
      reply: previewText(reply, OUTLINE_REPLY_CHARS),
    });
  }
  return merged;
}
