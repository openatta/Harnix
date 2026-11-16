/**
 * The turn rail: a vertical strip on the right edge of the conversation, one tick per turn (by the person's
 * message that started it). Pointing at a tick or focusing it previews the turn — its prompt on one line, its
 * reply on three — and choosing it scrolls there, reading older history in first when the turn is not shown
 * yet. ⌥↑ / ⌥↓ go to the turn before or after the one being read (`turnNav.ts`).
 *
 * The turns are the session's outline (`harnix.session.outline`, read again as each turn settles) merged
 * with the turns shown (`mergeTurns`): the running turn is in no outline yet, and without one the rail lists
 * what is shown. The rail itself only mounts the ticks in view, so a long session costs a few dozen.
 * Under a narrow width it is not drawn (`styles.css`); the keys still work.
 */

import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type RefObject } from "react";
import { mergeTurns, type NavTurn, type SessionController, type SessionState } from "@harnix/core/state";
import { useTurnOutline } from "../context";
import { useT } from "../i18n/index";
import { addTurnRail } from "./turnNav";

/** The distance between two ticks. */
export const TICK_SPACING = 12;
/** Ticks mounted beyond each end of the rail's view. */
const OVERSCAN = 8;
/** The rail's height before it is measured (and where nothing measures, as in a test). */
const UNMEASURED = 480;
/** A turn whose start is this far above the top of the view is being read, not passed. */
const SLACK = 8;

/** The block that starts each turn shown carries this, with `data-turn-key`: what the rail scrolls to. */
export const TURN_ANCHOR = "data-turn-anchor";

/** The key a turn is listed under: its turn id, else its place in the transcript (as `mergeTurns`). */
export function turnKey(turnId: string | null | undefined, seq: number | null | undefined): string {
  return turnId ?? `seq:${seq}`;
}

/** Where the anchor's first box is, from the top of the scroller's view. */
function topOf(anchor: Element, scroller: HTMLElement): number {
  const box = anchor.firstElementChild ?? anchor;
  return box.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
}

export interface TurnRailProps {
  controller: SessionController;
  state: SessionState;
  /** The transcript's scroller. */
  scroller: RefObject<HTMLDivElement | null>;
  /** Whether the transcript follows its end: a jump lets go of it. */
  pinned: RefObject<boolean>;
}

export function TurnRail({ controller, state, scroller, pinned }: TurnRailProps) {
  const t = useT();
  const outline = useTurnOutline(controller);
  const turns = useMemo(() => mergeTurns(outline.turns, state.blocks), [outline.turns, state.blocks]);
  const [active, setActive] = useState(-1);
  const [focus, setFocus] = useState<number | null>(null);
  const [preview, setPreview] = useState<number | null>(null);
  const [view, setView] = useState({ top: 0, height: UNMEASURED });
  const track = useRef<HTMLDivElement>(null);
  const hovering = useRef(false);
  const wantFocus = useRef(false);
  /** The turn a jump waits on older history for. */
  const pending = useRef<string | null>(null);
  const previewId = useId();

  const indexOf = useCallback((key: string | undefined) => (key == null ? -1 : turns.findIndex((turn) => turn.key === key)), [turns]);

  /** The turn being read: the last whose start is at or above the top of the view (the last one at the end). */
  const reading = useCallback((): { index: number; passed: boolean } => {
    const node = scroller.current;
    if (!node) return { index: -1, passed: false };
    const anchors = [...node.querySelectorAll<HTMLElement>(`[${TURN_ANCHOR}]`)];
    if (!anchors.length) return { index: -1, passed: false };
    let at = 0;
    let above = false;
    for (let i = 0; i < anchors.length; i += 1) {
      if (topOf(anchors[i]!, node) <= SLACK) {
        at = i;
        above = true;
      } else break;
    }
    if (node.scrollHeight - node.scrollTop - node.clientHeight < 4) {
      at = anchors.length - 1;
      above = true;
    }
    const anchor = anchors[at]!;
    return { index: indexOf(anchor.dataset.turnKey), passed: above && topOf(anchor, node) < -SLACK };
  }, [scroller, indexOf]);

  const scrollTo = useCallback(
    (key: string): boolean => {
      const node = scroller.current;
      const anchor = node ? [...node.querySelectorAll<HTMLElement>(`[${TURN_ANCHOR}]`)].find((a) => a.dataset.turnKey === key) : undefined;
      if (!node || !anchor) return false;
      pinned.current = false;
      node.scrollTop += topOf(anchor, node) - SLACK;
      return true;
    },
    [scroller, pinned],
  );

  const go = useCallback(
    (index: number) => {
      const turn = turns[index];
      if (!turn) return;
      setActive(index);
      pinned.current = false;
      if (scrollTo(turn.key)) return;
      // Not shown: read the history back to it first, then land on it (the effect below).
      if (turn.seq != null && turn.seq < (state.historyFrom ?? 0)) {
        pending.current = turn.key;
        void controller.loadOlder(turn.seq).then((shown) => {
          if (!shown && pending.current === turn.key) pending.current = null;
        });
      }
    },
    [turns, scrollTo, controller, state.historyFrom, pinned],
  );

  useLayoutEffect(() => {
    if (pending.current && scrollTo(pending.current)) pending.current = null;
  }, [state.blocks, scrollTo]);

  // Follow the reading position as the transcript scrolls, or changes under it.
  useEffect(() => {
    const node = scroller.current;
    if (!node) return;
    let frame = 0;
    const update = () => {
      frame = 0;
      const { index } = reading();
      if (index >= 0) setActive(index);
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(update);
    };
    update();
    node.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      node.removeEventListener("scroll", onScroll);
      if (frame) cancelAnimationFrame(frame);
    };
  }, [scroller, reading, state.blocks]);

  // The keys: the turn before or after the one being read.
  const step = useRef<(delta: -1 | 1) => void>(() => {});
  step.current = (delta) => {
    if (!turns.length) return;
    const { index, passed } = reading();
    const target = index < 0 ? (delta < 0 ? turns.length - 1 : 0) : delta < 0 ? (passed ? index : index - 1) : index + 1;
    if (target >= 0 && target < turns.length) go(target);
  };
  const count = useRef(0);
  count.current = turns.length;
  useEffect(
    () =>
      addTurnRail({
        root: () => scroller.current?.closest(".hx-chat") ?? scroller.current ?? null,
        count: () => count.current,
        step: (delta) => step.current(delta),
      }),
    [scroller],
  );

  // The rail's own view: what part of it shows, for which ticks to mount.
  useLayoutEffect(() => {
    const el = track.current;
    if (!el) return;
    const measure = () => setView({ top: el.scrollTop, height: el.clientHeight || UNMEASURED });
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [turns.length > 1]);

  const length = turns.length * TICK_SPACING;
  // Few turns sit in the middle of the rail; more than it holds scroll in it.
  const offset = Math.max(0, Math.floor((view.height - length) / 2));

  // Keep the turn being read in the rail's view, unless the pointer is on the rail.
  useLayoutEffect(() => {
    const el = track.current;
    if (!el || active < 0 || hovering.current) return;
    const y = offset + active * TICK_SPACING;
    if (y < el.scrollTop || y > el.scrollTop + (el.clientHeight || UNMEASURED) - TICK_SPACING) el.scrollTop = Math.max(0, y - (el.clientHeight || UNMEASURED) / 2);
  }, [active, offset]);

  // A tick reached by the arrows takes the focus once it is mounted.
  useLayoutEffect(() => {
    if (!wantFocus.current || focus == null) return;
    wantFocus.current = false;
    track.current?.querySelector<HTMLButtonElement>(`[data-index="${focus}"]`)?.focus();
  });

  if (turns.length < 2) return null;

  const stop = focus ?? (active >= 0 ? active : turns.length - 1);
  const first = Math.max(0, Math.floor((view.top - offset) / TICK_SPACING) - OVERSCAN);
  const last = Math.min(turns.length - 1, Math.ceil((view.top - offset + view.height) / TICK_SPACING) + OVERSCAN);
  const mounted = new Set<number>();
  for (let i = first; i <= last; i += 1) mounted.add(i);
  for (const i of [active, stop - 1, stop, stop + 1, preview ?? -1]) if (i >= 0 && i < turns.length) mounted.add(i);
  const indices = [...mounted].sort((a, b) => a - b);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const to =
      event.key === "ArrowUp" ? stop - 1 : event.key === "ArrowDown" ? stop + 1 : event.key === "Home" ? 0 : event.key === "End" ? turns.length - 1 : null;
    if (to == null) return;
    event.preventDefault();
    const next = Math.max(0, Math.min(turns.length - 1, to));
    wantFocus.current = true;
    setFocus(next);
    setPreview(next);
  };

  const shown = preview != null ? turns[preview] : undefined;
  const previewTop = preview != null ? offset + preview * TICK_SPACING - view.top : 0;

  return (
    <nav className="hx-turnrail" aria-label={t("chat.turnRail.label")} data-testid="conv-turnrail" data-count={turns.length}>
      <div
        ref={track}
        className="hx-turnrail-track"
        onScroll={(e) => setView({ top: e.currentTarget.scrollTop, height: e.currentTarget.clientHeight || UNMEASURED })}
        onKeyDown={onKeyDown}
        onPointerEnter={() => (hovering.current = true)}
        onPointerLeave={() => {
          hovering.current = false;
          setPreview(null);
        }}
      >
        <div role="list" className="hx-turnrail-list" style={{ height: offset + length }}>
          {indices.map((i) => {
            const turn = turns[i]!;
            return (
              <div key={turn.key} role="listitem" aria-setsize={turns.length} aria-posinset={i + 1} className="hx-turnrail-item" style={{ top: offset + i * TICK_SPACING }}>
                <button
                  type="button"
                  className="hx-turnrail-tick"
                  data-testid="conv-turnrail-tick"
                  data-index={i}
                  data-key={turn.key}
                  data-state={i === active ? "active" : undefined}
                  data-mark={turn.marks?.length ? turn.marks[0]!.kind : undefined}
                  tabIndex={i === stop ? 0 : -1}
                  aria-current={i === active ? "true" : undefined}
                  aria-label={t("chat.turnRail.tick", { n: i + 1, prompt: turn.prompt || t("chat.turnRail.noText") })}
                  aria-describedby={preview === i ? previewId : undefined}
                  onClick={() => go(i)}
                  onFocus={() => {
                    setFocus(i);
                    setPreview(i);
                  }}
                  onBlur={() => setPreview((p) => (p === i ? null : p))}
                  onPointerEnter={() => setPreview(i)}
                >
                  <span className="hx-turnrail-mark" aria-hidden />
                </button>
              </div>
            );
          })}
        </div>
      </div>
      {shown ? <TurnPreview id={previewId} turn={shown} n={preview! + 1} top={previewTop} /> : null}
    </nav>
  );
}

function TurnPreview({ id, turn, n, top }: { id: string; turn: NavTurn; n: number; top: number }) {
  const t = useT();
  const at = turn.at ? new Date(turn.at) : null;
  return (
    <div id={id} role="tooltip" className="hx-turnrail-preview" data-testid="conv-turnrail-preview" style={{ top: Math.max(0, top) }}>
      <div className="hx-turnrail-head">
        <span>{t("chat.turnRail.turn", { n })}</span>
        {at && !Number.isNaN(at.getTime()) ? <time dateTime={turn.at!}>{at.toLocaleString()}</time> : null}
        {(turn.marks ?? []).map((m, i) => (
          <span key={i} className="hx-turnrail-tag" data-kind={m.kind}>
            {m.label}
          </span>
        ))}
      </div>
      <div className="hx-turnrail-prompt" data-testid="conv-turnrail-prompt">
        {turn.prompt || t("chat.turnRail.noText")}
      </div>
      {turn.reply ? (
        <div className="hx-turnrail-reply" data-testid="conv-turnrail-reply">
          {turn.reply}
        </div>
      ) : null}
    </div>
  );
}
