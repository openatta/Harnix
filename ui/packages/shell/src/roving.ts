/**
 * Arrow keys inside a group the Tab key enters once: a tab strip, a tree. One item of the group is
 * in the Tab order — the one with the focus, else the selected one (`aria-selected="true"`), else
 * the first — and the arrows, Home and End move between them. In a tab strip moving to a tab
 * shows it; in a tree Right opens an item and Left closes it, or goes to the item it hangs under.
 *
 * The items' `tabindex` is set here, on the elements, after each draw of the group: the component
 * that draws them gives them none.
 */

import { useCallback, useLayoutEffect, type FocusEvent, type KeyboardEvent, type RefObject } from "react";

export interface Roving {
  /** The group's items, as a selector inside it. */
  items: string;
  /** `horizontal`: Left and Right move (a tab strip). `vertical`: Up and Down (a list, a tree). */
  orientation: "horizontal" | "vertical";
  /** A tab strip whose tabs show as the arrows reach them. */
  activate?: boolean;
  /** A tree: the item `item` hangs under, where Left goes from a closed or leaf item. */
  parentOf?: (item: HTMLElement) => HTMLElement | null;
  /** A tree: open or close `item` (its `aria-expanded` says which it is now). */
  toggle?: (item: HTMLElement) => void;
}

function itemsOf(box: HTMLElement, selector: string): HTMLElement[] {
  return [...box.querySelectorAll<HTMLElement>(selector)].filter((el) => !el.closest("[inert]") && !(el as HTMLButtonElement).disabled);
}

/** Make `item` the group's one stop. */
function settle(items: HTMLElement[], item: HTMLElement | undefined): void {
  for (const el of items) el.tabIndex = el === item ? 0 : -1;
}

export function useRovingTabIndex(ref: RefObject<HTMLElement | null>, spec: Roving): { onKeyDown: (event: KeyboardEvent) => void; onFocus: (event: FocusEvent) => void } {
  const { items, orientation, activate, parentOf, toggle } = spec;
  // After every draw: a new item, or a new selection, moves the stop.
  useLayoutEffect(() => {
    const box = ref.current;
    if (!box) return;
    const all = itemsOf(box, items);
    const active = document.activeElement;
    settle(all, all.find((el) => active instanceof Node && el.contains(active)) ?? all.find((el) => el.getAttribute("aria-selected") === "true") ?? all[0]);
  });
  const onFocus = useCallback(
    (event: FocusEvent) => {
      const box = ref.current;
      if (!box) return;
      const all = itemsOf(box, items);
      const item = all.find((el) => el.contains(event.target as Node));
      if (item) settle(all, item);
    },
    [ref, items],
  );
  const onKeyDown = useCallback(
    (event: KeyboardEvent) => {
      const box = ref.current;
      if (!box || event.altKey || event.ctrlKey || event.metaKey) return;
      const all = itemsOf(box, items);
      const at = all.indexOf(event.target as HTMLElement);
      // A key in something inside an item (a field, an inner button) is that thing's.
      if (at < 0) return;
      const item = all[at]!;
      const next = orientation === "horizontal" ? "ArrowRight" : "ArrowDown";
      const back = orientation === "horizontal" ? "ArrowLeft" : "ArrowUp";
      let to: HTMLElement | null | undefined;
      if (event.key === next) to = all[(at + 1) % all.length];
      else if (event.key === back) to = all[(at - 1 + all.length) % all.length];
      else if (event.key === "Home") to = all[0];
      else if (event.key === "End") to = all[all.length - 1];
      else if (orientation === "vertical" && (event.key === "ArrowRight" || event.key === "ArrowLeft")) {
        const expanded = item.getAttribute("aria-expanded");
        if (event.key === "ArrowRight" && expanded === "false" && toggle) {
          event.preventDefault();
          toggle(item);
          return;
        }
        if (event.key === "ArrowLeft" && expanded === "true" && toggle) {
          event.preventDefault();
          toggle(item);
          return;
        }
        if (event.key === "ArrowLeft") to = parentOf?.(item);
      }
      if (!to || to === item) return;
      event.preventDefault();
      settle(all, to);
      to.focus();
      if (activate) to.click();
    },
    [ref, items, orientation, activate, parentOf, toggle],
  );
  return { onKeyDown, onFocus };
}

/**
 * Keys on a row that is picked by a click (a list's, a table's): Enter or Space picks it, Up and
 * Down go to the row before or after in `group` (`[data-row]` there), Right and Left open and close
 * it when `toggle` is given. Every such row is in the Tab order of its own.
 */
export function rowKeys(pick: () => void, toggle?: { open: boolean; set: (open: boolean) => void }) {
  return (event: KeyboardEvent<HTMLElement>) => {
    if (event.target !== event.currentTarget || event.altKey || event.ctrlKey || event.metaKey) return;
    const row = event.currentTarget;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      pick();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      const group = row.closest("[data-rows]");
      if (!group) return;
      const rows = [...group.querySelectorAll<HTMLElement>("[data-row]")];
      const to = rows[rows.indexOf(row) + (event.key === "ArrowDown" ? 1 : -1)];
      if (!to) return;
      event.preventDefault();
      to.focus();
    } else if (toggle && event.key === "ArrowRight" && !toggle.open) {
      event.preventDefault();
      toggle.set(true);
    } else if (toggle && event.key === "ArrowLeft" && toggle.open) {
      event.preventDefault();
      toggle.set(false);
    }
  };
}
