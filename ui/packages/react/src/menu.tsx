/**
 * A menu's box: what every menu on the page shares — the shell's (its `Menu.tsx`: the avatar's, a
 * session's, a project's) and the composer's (`+`, the model). Anchored to what opened it — a button,
 * a row, the pointer of a right click — and not a dialog: it is a layer (`layers.ts`), so Escape
 * closes it first and a dialog opening closes it, but the page under it stays live.
 *
 * `role="menu"`; the focus goes to its first item, ↑/↓ (and Home/End) move between items, Enter
 * or a click runs one, Escape or Tab closes it, and the focus goes back to what opened it. A click
 * outside closes it and leaves the focus where the click put it. What is in it is the caller's:
 * its items are the `[role="menuitem"]` buttons in it, and a text field marked `data-menu-field`
 * (the model menu's own name) is a stop of ↑/↓ too.
 */

import { useLayoutEffect, useRef, type KeyboardEvent as ReactKeyboardEvent, type ReactNode, type RefObject } from "react";

import { useLayer } from "./layers";

/** What ↑/↓ stop at, in order. */
const STOPS = '[role="menuitem"]:not(:disabled), [data-menu-field]:not(:disabled)';

export interface MenuBoxProps {
  /** What a reader hears it as. */
  label: string;
  /** It wants to close: Escape, Tab, a click outside. An item closes it itself. */
  onClose: () => void;
  /** Where it opens, in the window (the caller puts it over the page). Without it, where its CSS puts it. */
  at?: { x: number; y: number };
  className?: string;
  testId?: string;
  /** A click in here is not "outside" (the button that toggles it). */
  within?: RefObject<HTMLElement | null>;
  /** Where the focus goes back to; default what had it when the menu opened. */
  returnTo?: HTMLElement | null;
  /** Something it started is still running (`aria-busy`). */
  busy?: boolean;
  children: ReactNode;
}

export function MenuBox({ label, onClose, at, className, testId, within, returnTo, busy, children }: MenuBoxProps) {
  const box = useRef<HTMLDivElement>(null);
  // Opened at a point, it stays in the window: moved up or left where it would go past an edge.
  // Placed before it is pushed as a layer, which puts the focus in it.
  useLayoutEffect(() => {
    const el = box.current;
    if (!at || !el) return;
    const { width, height } = el.getBoundingClientRect();
    el.style.left = `${Math.max(8, Math.min(at.x, window.innerWidth - width - 8))}px`;
    el.style.top = `${at.y + height > window.innerHeight - 8 ? Math.max(8, at.y - height) : at.y}px`;
  }, [at?.x, at?.y]);
  const layer = useLayer(box, {
    modal: false,
    close: onClose,
    returnTo,
    initial: (el) => el.querySelector<HTMLElement>('[role="menuitem"]:not(:disabled)'),
  });
  useLayoutEffect(() => {
    const away = (event: MouseEvent) => {
      const target = event.target as Node;
      if (box.current?.contains(target) || within?.current?.contains(target)) return;
      layer.quiet();
      onClose();
    };
    window.addEventListener("mousedown", away, true);
    return () => window.removeEventListener("mousedown", away, true);
  }, [onClose]);

  const keys = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const all = [...(box.current?.querySelectorAll<HTMLElement>(STOPS) ?? [])];
    const at = all.indexOf(document.activeElement as HTMLElement);
    const go = (i: number) => {
      event.preventDefault();
      all[(i + all.length) % all.length]?.focus();
    };
    // In a text field, Home and End are the field's.
    const inField = event.target instanceof HTMLElement && event.target.matches("input, textarea");
    if (event.key === "ArrowDown") go(at + 1);
    else if (event.key === "ArrowUp") go(at < 0 ? -1 : at - 1);
    else if (event.key === "Home" && !inField) go(0);
    else if (event.key === "End" && !inField) go(-1);
    else if (event.key === "Tab") {
      event.preventDefault();
      onClose();
    } else layer.onKeyDown(event);
  };

  return (
    <div
      ref={box}
      role="menu"
      aria-label={label}
      aria-busy={busy || undefined}
      tabIndex={-1}
      className={`hx-menu${at ? " anchored" : ""}${className ? ` ${className}` : ""}`}
      data-testid={testId}
      style={at ? { left: at.x, top: at.y } : undefined}
      onKeyDown={keys}
    >
      {children}
    </div>
  );
}
