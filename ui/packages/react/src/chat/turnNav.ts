/**
 * The previous and next turn as `shortcut` commands (`harnix-ext.md` §22.8): ⌥↑ / ⌥↓ go to the turn before or
 * after the one being read, in the conversation that has the focus, else the one opened last.
 *
 * Global, so they work while the person reads — the focus on the page, the transcript or the rail — and not
 * only from the composer. A text field with text in it keeps them: there ⌥↑ / ⌥↓ move the caret by paragraph
 * (macOS), and that is the field's. An empty composer has nothing to move in, so they navigate from it too.
 */

import type { Registry } from "../contrib";
import { t } from "../i18n/index";

/** What a mounted rail offers the commands. */
export interface TurnRailHandle {
  /** The conversation it belongs to: where the focus is looked for. */
  root(): Element | null;
  /** How many turns it lists now. */
  count(): number;
  /** Go to the turn before (-1) or after (+1) the one being read. */
  step(delta: -1 | 1): void;
}

export const TURN_PREVIOUS = "chat.turn.previous";
export const TURN_NEXT = "chat.turn.next";

const rails: TurnRailHandle[] = [];

/** A rail mounts: the commands can reach it until the returned removal. */
export function addTurnRail(rail: TurnRailHandle): () => void {
  rails.push(rail);
  return () => {
    const at = rails.indexOf(rail);
    if (at >= 0) rails.splice(at, 1);
  };
}

/** The rail the commands act on: the one whose conversation has the focus, else the one mounted last. */
export function currentTurnRail(): TurnRailHandle | null {
  const focus = typeof document === "undefined" ? null : document.activeElement;
  const live = rails.filter((r) => r.count() > 0 && r.root()?.isConnected);
  return live.find((r) => focus != null && r.root()?.contains(focus)) ?? live[live.length - 1] ?? null;
}

/** The focus is in a text field that keeps ⌥↑ / ⌥↓: any with text in it, or that is not the composer. */
function leftToTheField(): boolean {
  const el = typeof document === "undefined" ? null : document.activeElement;
  if (!(el instanceof HTMLElement)) return false;
  if (el.isContentEditable) return true;
  if (el instanceof HTMLSelectElement) return true;
  if (!(el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement)) return false;
  return !(el.matches('[data-testid="conv-input"]') && el.value === "");
}

/** Register the two commands under `owner`: once per page, by whoever registers the page's own commands. */
export function registerTurnShortcuts(registry: Registry, owner: string): void {
  const command = (id: string, title: string, keys: string, delta: -1 | 1) =>
    registry.register(
      "shortcut",
      {
        id,
        title: () => t(title),
        keys,
        enabled: () => !leftToTheField() && currentTurnRail() != null,
        run: () => currentTurnRail()?.step(delta),
      },
      owner,
    );
  command(TURN_PREVIOUS, "shortcuts.cmd.turnPrevious", "Alt+ArrowUp", -1);
  command(TURN_NEXT, "shortcuts.cmd.turnNext", "Alt+ArrowDown", 1);
}
