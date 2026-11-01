/**
 * Finding one entry in an open session's conversation (`host.session.open(ref, {reveal})`,
 * `host.session.reveal(key)`): wait until that session's transcript has caught up, find the entry,
 * scroll it to the centre and mark it `data-revealed` for a moment (the shell's styles flash it).
 *
 * The entry is an element with `data-reveal="<key>"` — a `flow.block` renderer puts it on its block —
 * else the block whose `data-id` is the key.
 */

import type { App, SessionKey } from "./app.ts";

/** How long the mark stays (the flash in the styles runs as long). */
export const REVEAL_MS = 2400;
/** The longest wait for the session to be open and caught up. */
export const REVEAL_WAIT_MS = 10_000;
/** After the transcript has caught up, how long the entry may still take to be drawn. */
const DRAW_GRACE_MS = 1500;
const POLL_MS = 50;

const marks = new WeakMap<Element, ReturnType<typeof setTimeout>>();

function quoted(value: string): string {
  return `"${value.replace(/["\\]/g, "\\$&")}"`;
}

/** The entry in this session's conversation on the page, if it is drawn. */
export function findEntry(ref: SessionKey, key: string, doc: Document = document): Element | null {
  const conv = doc.querySelector(`#main [data-testid="conv"][data-id=${quoted(ref.sessionId)}]`) ?? doc.querySelector("#main");
  if (!conv) return null;
  return conv.querySelector(`[data-reveal=${quoted(key)}]`) ?? conv.querySelector(`.hx-blk[data-id=${quoted(key)}]`);
}

/** Scroll to it and mark it; marking again starts the flash over. */
export function markRevealed(el: Element): void {
  (el as HTMLElement).scrollIntoView?.({ block: "center", behavior: "smooth" });
  const before = marks.get(el);
  if (before !== undefined) clearTimeout(before);
  el.removeAttribute("data-revealed");
  void (el as HTMLElement).offsetWidth; // restart the animation
  el.setAttribute("data-revealed", "");
  marks.set(
    el,
    setTimeout(() => {
      el.removeAttribute("data-revealed");
      marks.delete(el);
    }, REVEAL_MS),
  );
}

/** Whether `ref` is the front space's open session and its controller has caught up. Null: it never
 *  will (another session is open now, or opening it failed). */
function caughtUp(app: App, ref: SessionKey): boolean | null {
  const current = app.snapshot.current;
  if (!current || current.instance !== ref.instance || current.sessionId !== ref.sessionId) return null;
  const controller = app.controller;
  if (!controller || controller.instance !== ref.instance || controller.sessionId !== ref.sessionId) return false;
  const state = controller.getSnapshot();
  if (state.failed != null) return null;
  return !state.loading;
}

/**
 * Wait for `ref` to be open and caught up, then find and reveal the entry. False when it is not
 * found within the bounds, or the person moved to another session meanwhile.
 */
export async function revealEntry(app: App, ref: SessionKey, key: string, wait = REVEAL_WAIT_MS): Promise<boolean> {
  const deadline = Date.now() + wait;
  let readyAt: number | null = null;
  for (;;) {
    const ready = caughtUp(app, ref);
    if (ready === null) return false;
    if (ready) {
      readyAt ??= Date.now();
      const el = findEntry(ref, key);
      if (el) {
        markRevealed(el);
        return true;
      }
      if (Date.now() - readyAt > DRAW_GRACE_MS) return false;
    }
    if (Date.now() > deadline) return false;
    await new Promise((resolve) => setTimeout(resolve, POLL_MS));
  }
}
