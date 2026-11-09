/**
 * The page's layers: the shell's dialogs, a dialog's own dialog, the shell's question on an
 * application's behalf, and the menus. One stack for all of them, so which one is on top is never a
 * matter of which listener happened to be registered first.
 *
 * - **Escape is the top layer's.** One pressed inside it goes to the layer's own handler (what is
 *   in it can take Escape first); one from anywhere else — focus left on the page, a key a frame
 *   handed over — closes it here, and nothing else on the page hears it.
 * - **A modal layer holds the focus.** It moves in when the layer opens, Tab goes round inside
 *   it, and it goes back where it was when the layer closes. Everything under it — the frame, the
 *   dialogs below — is inert meanwhile, and the shell's shortcuts do nothing (`modalUp()`).
 * - **A menu is a layer, not a modal one.** Escape closes it first, and the page stays live; a
 *   dialog opening closes it.
 * - **The shell's question ranks over everything** (the shell's `surfaces/confirm.tsx`), whenever it came: while
 *   it is up the page hears no key at all.
 */

import { useCallback, useLayoutEffect, useRef, type KeyboardEvent as ReactKeyboardEvent, type RefObject } from "react";

export interface LayerSpec {
  /** The layer's box: what "inside" means, and what goes inert under a modal above it. */
  element: () => HTMLElement | null;
  /** Escape, a click on the veil, a dialog opening over a menu. */
  close: () => void;
  /** Holds the focus and makes what is under it inert (default). A menu is not modal. */
  modal?: boolean;
  /** Above every layer of a lower rank, whenever it opened. The shell's question is 1. */
  rank?: number;
  /** Where the focus goes when it opens; default the first field or button inside it. */
  initial?: (box: HTMLElement) => HTMLElement | null;
  /** Where the focus goes back to when it closes; default what had it when it opened. */
  returnTo?: HTMLElement | null;
}

export interface Layer {
  readonly id: number;
  readonly modal: boolean;
  readonly rank: number;
  element(): HTMLElement | null;
  close(): void;
  /** Closed without giving the focus back: a click elsewhere put it where the person wanted it. */
  quiet: boolean;
  returnTo: HTMLElement | null;
}

let stack: Layer[] = [];
let next = 1;
let version = 0;
const listeners = new Set<() => void>();

export function subscribeLayers(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function layersVersion(): number {
  return version;
}

/** The layer on top: the highest rank, the last opened in it. */
export function topLayer(): Layer | null {
  return stack[stack.length - 1] ?? null;
}

/** A modal layer is up: the page under it does not answer, and neither do the shell's shortcuts. */
export function modalUp(): boolean {
  return stack.some((l) => l.modal);
}

export function isTop(layer: Layer | null): boolean {
  return layer != null && topLayer() === layer;
}

/** Open a layer over the others; returns its close-down, which hands the focus back. */
export function pushLayer(spec: LayerSpec): Layer {
  const modal = spec.modal ?? true;
  const layer: Layer = {
    id: next++,
    modal,
    rank: spec.rank ?? 0,
    element: spec.element,
    close: spec.close,
    quiet: false,
    returnTo: spec.returnTo ?? (document.activeElement instanceof HTMLElement && document.activeElement !== document.body ? document.activeElement : null),
  };
  // A dialog closes the menus: they belong to the page it covers.
  if (modal) for (const open of stack.filter((l) => !l.modal)) open.close();
  const at = stack.findIndex((l) => l.rank > layer.rank);
  stack = at < 0 ? [...stack, layer] : [...stack.slice(0, at), layer, ...stack.slice(at)];
  settle();
  if (isTop(layer)) {
    const box = layer.element();
    if (box) (spec.initial?.(box) ?? firstIn(box))?.focus();
  }
  return layer;
}

/** Close a layer down: off the stack, the page under it live again, the focus back where it was. */
export function popLayer(layer: Layer): void {
  if (!stack.includes(layer)) return;
  const wasTop = isTop(layer);
  stack = stack.filter((l) => l !== layer);
  settle();
  if (!wasTop || layer.quiet) return;
  const back = layer.returnTo;
  if (back && back.isConnected && !back.closest("[inert]")) {
    back.focus();
    return;
  }
  // Where it was is gone (a row deleted from the menu that opened this): into what is now on top.
  const under = topLayer()?.element();
  if (under) firstIn(under)?.focus();
}

/** Everything under the top modal layer is inert: the dialogs below it, and the frame. */
function settle(): void {
  let topModal = -1;
  stack.forEach((l, i) => {
    if (l.modal) topModal = i;
  });
  stack.forEach((l, i) => {
    const box = l.element();
    if (box && l.modal) box.inert = i < topModal;
  });
  const frame = typeof document !== "undefined" ? document.getElementById("frame") : null;
  if (frame) frame.inert = topModal >= 0;
  version += 1;
  for (const fn of [...listeners]) fn();
}

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), iframe, [tabindex]:not([tabindex="-1"]), [contenteditable="true"]';

/** What Tab stops at inside `box`, in order: shown, enabled, not inert. */
export function focusables(box: HTMLElement): HTMLElement[] {
  return [...box.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (el) => !el.closest("[inert]") && (typeof el.checkVisibility !== "function" || el.checkVisibility()),
  );
}

/** Where focus lands in a layer that opened: `[autofocus]`, else the first stop not marked to skip. */
function firstIn(box: HTMLElement): HTMLElement | null {
  const asked = box.querySelector<HTMLElement>("[autofocus], [data-autofocus]");
  if (asked) return asked;
  const all = focusables(box);
  return all.find((el) => !el.closest("[data-initial-skip]")) ?? all[0] ?? box;
}

/** Tab and Shift+Tab go round inside `box`; from outside it they go in. */
function trapTab(event: KeyboardEvent, box: HTMLElement): void {
  const all = focusables(box);
  const active = document.activeElement;
  if (!all.length) {
    event.preventDefault();
    box.focus();
    return;
  }
  const first = all[0]!;
  const last = all[all.length - 1]!;
  if (!(active instanceof Node) || !box.contains(active) || active === box) {
    event.preventDefault();
    (event.shiftKey ? last : first).focus();
  } else if (event.shiftKey && active === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && active === last) {
    event.preventDefault();
    first.focus();
  }
}

/**
 * The page's key listener for the layers, in the capture phase on `window` — ahead of every space's
 * listener, whatever order they were added in. Returns its removal.
 */
export function listenForLayers(): () => void {
  const onKey = (event: KeyboardEvent) => {
    const layer = topLayer();
    if (!layer) return;
    const box = layer.element();
    const inside = !!box && event.target instanceof Node && box.contains(event.target);
    if (event.key === "Escape") {
      // From inside, the layer's own handler (`useLayer`) has it: what is in the layer goes first.
      if (inside && layer.rank === 0) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      layer.close();
      return;
    }
    if (!layer.modal || !box) return;
    if (event.key === "Tab") trapTab(event, box);
    if (layer.rank > 0) {
      // The question: the page hears nothing while it is up, and a key from outside goes back in.
      if (!inside && event.key !== "Tab") {
        event.preventDefault();
        firstIn(box)?.focus();
      }
      event.stopImmediatePropagation();
    }
  };
  window.addEventListener("keydown", onKey, true);
  return () => window.removeEventListener("keydown", onKey, true);
}

export interface UseLayer extends Omit<LayerSpec, "element"> {
  /** False: not open (the hook is there, the layer is not). */
  open?: boolean;
}

/**
 * The layer `ref` is the box of, while it is on the page. Returns the box's `onKeyDown`: Escape
 * that nothing inside took closes it, and goes no further. `quiet()` closes it next without giving
 * the focus back.
 */
export function useLayer(ref: RefObject<HTMLElement | null>, spec: UseLayer): { onKeyDown: (event: ReactKeyboardEvent) => void; quiet: () => void } {
  const latest = useRef(spec);
  latest.current = spec;
  const layer = useRef<Layer | null>(null);
  const open = spec.open ?? true;
  useLayoutEffect(() => {
    if (!open) return;
    const pushed = pushLayer({
      element: () => ref.current,
      close: () => latest.current.close(),
      modal: latest.current.modal,
      rank: latest.current.rank,
      initial: (box) => latest.current.initial?.(box) ?? null,
      returnTo: latest.current.returnTo,
    });
    layer.current = pushed;
    return () => {
      layer.current = null;
      popLayer(pushed);
    };
  }, [open]);
  const onKeyDown = useCallback((event: ReactKeyboardEvent) => {
    if (event.key !== "Escape" || event.defaultPrevented || !isTop(layer.current)) return;
    event.preventDefault();
    event.stopPropagation();
    latest.current.close();
  }, []);
  const quiet = useCallback(() => {
    if (layer.current) layer.current.quiet = true;
  }, []);
  return { onKeyDown, quiet };
}
