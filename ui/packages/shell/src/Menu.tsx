/**
 * The shell's menus: the avatar's, a session's, a project's. Their box, keys and layer are
 * `@harnix/react`'s `MenuBox`, which the composer's menus use too; this draws the items, and opens
 * one at a point over the page. Enter or a click runs an item and closes the menu; an item that has
 * to be confirmed opens its dialog after the menu has closed.
 */

import { useSyncExternalStore, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { MenuBox } from "@harnix/react";

import { Glyph } from "./glyph.tsx";

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
  const menu = (
    <MenuBox label={label} onClose={onClose} at={at} className={className} testId={testId} within={within} returnTo={returnTo}>
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
    </MenuBox>
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
