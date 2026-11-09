/**
 * The shell's menus: the avatar's, a session's, a project's. One kind, anchored to what opened it —
 * a button, a row, the pointer of a right click — and not a dialog: it is a layer (`layers.ts`), so
 * Escape closes it first and a dialog opening closes it, but the page under it stays live.
 *
 * `role="menu"`; the focus goes to its first item, ↑/↓ (and Home/End) move between items, Enter
 * or a click runs one and closes the menu, Escape or Tab closes it, and the focus goes back to what
 * opened it. A click outside closes it and leaves the focus where the click put it. An item that
 * has to be confirmed opens its dialog after the menu has closed.
 */

import { useLayoutEffect, useRef, useSyncExternalStore, type KeyboardEvent as ReactKeyboardEvent, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";

import { Glyph } from "./glyph.tsx";
import { useLayer } from "./layers.ts";

export interface MenuEntry {
  /** Its `data-item`: what a test or an application finds it by. */
  id: string;
  label: ReactNode;
  icon?: string;
  /** A second line, smaller. */
  sub?: string;
  /** Drawn after the label, at the end of the row (a current value). */
  value?: ReactNode;
  danger?: boolean;
  disabled?: boolean;
  testId?: string;
  run: () => void;
}

export interface MenuProps {
  /** What a reader hears it as. */
  label: string;
  items: MenuEntry[];
  /** It wants to close: an item was run, Escape, Tab, a click outside. */
  onClose: () => void;
  /** Where it opens, in the window: drawn over the page there. Without it, where its CSS puts it. */
  at?: { x: number; y: number };
  /** A line over the items: what the menu is about. */
  title?: ReactNode;
  className?: string;
  testId?: string;
  /** A click in here is not "outside" (the button that toggles it). */
  within?: RefObject<HTMLElement | null>;
  /** Where the focus goes back to; default what had it when the menu opened. */
  returnTo?: HTMLElement | null;
}

export function Menu({ label, items, onClose, at, title, className, testId, within, returnTo }: MenuProps) {
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
    const all = [...(box.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled)') ?? [])];
    const at = all.indexOf(document.activeElement as HTMLElement);
    const go = (i: number) => {
      event.preventDefault();
      all[(i + all.length) % all.length]?.focus();
    };
    if (event.key === "ArrowDown") go(at + 1);
    else if (event.key === "ArrowUp") go(at < 0 ? -1 : at - 1);
    else if (event.key === "Home") go(0);
    else if (event.key === "End") go(-1);
    else if (event.key === "Tab") {
      event.preventDefault();
      onClose();
    } else layer.onKeyDown(event);
  };

  const menu = (
    <div
      ref={box}
      role="menu"
      aria-label={label}
      tabIndex={-1}
      className={`hx-menu${at ? " anchored" : ""}${className ? ` ${className}` : ""}`}
      data-testid={testId}
      style={at ? { left: at.x, top: at.y } : undefined}
      onKeyDown={keys}
    >
      {title ? <div className="hx-menu-title">{title}</div> : null}
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          role="menuitem"
          tabIndex={-1}
          className={item.danger ? "danger" : ""}
          data-item={item.id}
          data-testid={item.testId}
          disabled={item.disabled}
          onClick={() => {
            onClose();
            item.run();
          }}
        >
          {item.icon ? <Glyph name={item.icon} className="glyph" /> : null}
          <span className="hx-menu-label">
            {item.label}
            {item.sub ? <span className="sub">{item.sub}</span> : null}
          </span>
          {item.value}
        </button>
      ))}
    </div>
  );
  return at ? createPortal(menu, document.getElementById("layers") ?? document.body) : menu;
}

/* ── the menu opened from a row, a button, a right click ─────────────── */

export interface OpenMenu extends Omit<MenuProps, "onClose" | "at"> {
  at: { x: number; y: number };
}

let open: OpenMenu | null = null;
const listeners = new Set<() => void>();
const changed = () => {
  for (const fn of [...listeners]) fn();
};
const subscribe = (fn: () => void) => {
  listeners.add(fn);
  return () => listeners.delete(fn);
};

/** Open a menu (one at a time: another one open closes). */
export function openMenu(menu: OpenMenu): void {
  open = menu;
  changed();
}

export function closeMenu(): void {
  if (!open) return;
  open = null;
  changed();
}

/** The menu that is open, for a test or the console. */
export function openedMenu(): OpenMenu | null {
  return open;
}

/**
 * Where a menu opened from `event` goes: at the pointer of a right click; under the element for a
 * key (Enter on `⋯`, the context-menu key), which has no pointer.
 */
export function menuPoint(event: { clientX?: number; clientY?: number; currentTarget: EventTarget | null }): { x: number; y: number } {
  if (event.clientX || event.clientY) return { x: event.clientX ?? 0, y: event.clientY ?? 0 };
  return menuBelow(event.currentTarget instanceof Element ? event.currentTarget : null);
}

/** Under `el`, at its start: where a menu opened by a button goes. */
export function menuBelow(el: Element | null): { x: number; y: number } {
  const r = el?.getBoundingClientRect();
  return r ? { x: r.left, y: r.bottom + 4 } : { x: 0, y: 0 };
}

/** Draws the open menu; one on the page, in the shell. */
export function MenuHost() {
  const menu = useSyncExternalStore(subscribe, () => open);
  if (!menu) return null;
  return <Menu key={`${menu.testId}:${menu.at.x}:${menu.at.y}`} {...menu} onClose={closeMenu} />;
}
