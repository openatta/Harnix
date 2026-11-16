/**
 * Keyboard shortcuts as named commands (harnix-ext.md §22.8): a `shortcut` contribution is a
 * command with a title, a default key per platform, where it is in force and what it runs. The
 * shell keeps the person's own keys in this browser, finds the command a key press is for and runs
 * it; this module is the part a UI package needs: the shapes, and how a key is written, read back
 * and shown.
 *
 * A key is written as modifiers and one key joined by `+`: `Mod+K`, `Alt+ArrowUp`, `Mod+Shift+\`.
 * `Mod` is ⌘ on a Mac and Ctrl elsewhere; `Ctrl`, `Alt` (⌥), `Shift` and `Meta` (⌘) are themselves
 * on every platform. Keys are matched by where they are on the keyboard (`KeyboardEvent.code`), so
 * ⌥K is K whatever character the layout makes of it.
 */

import type { SessionRef } from "./host";

export type ShortcutPlatform = "mac" | "other";

/** Where a command is in force: anywhere on the page, or only while the focus is in the
 *  conversation's composer or in the right column. A focused place's command goes before a global
 *  one on the same key. */
export const SHORTCUT_SCOPES = ["global", "composer", "sidebar"] as const;
export type ShortcutScope = (typeof SHORTCUT_SCOPES)[number];

/** A key as written (`Mod+K`); the same on every platform, or one per platform. */
export type ShortcutKeys = string | { mac?: string | null; other?: string | null };

/** What a command is run with: the front space, its open session, and where the key was pressed. */
export interface ShortcutContext {
  space: string;
  session: SessionRef | null;
  scope: ShortcutScope;
}

/** A `shortcut` contribution: a command a key runs. */
export interface ShortcutCommand {
  /** The command's name, unique on the page: `<owner>.<what>`. The person's own key for it is kept
   *  under this name. */
  id: string;
  /** What the settings list it as. Read at every draw. */
  title(): string;
  /** Its key until the person picks another; none: it has no key until they do. */
  keys?: ShortcutKeys | null;
  /** `global` unless said otherwise. */
  scope?: ShortcutScope;
  /** False now: the key is left to the page, as if the command had none. A throw counts as false. */
  enabled?(): boolean;
  /** A failure is the shell's error banner. */
  run(ctx: ShortcutContext): void | Promise<void>;
}

/** A key, read: modifiers and the physical key (`KeyboardEvent.code`). */
export interface Chord {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
  code: string;
}

const PUNCTUATION: Record<string, string> = {
  "/": "Slash",
  "\\": "Backslash",
  ",": "Comma",
  ".": "Period",
  ";": "Semicolon",
  "'": "Quote",
  "[": "BracketLeft",
  "]": "BracketRight",
  "-": "Minus",
  "=": "Equal",
  "`": "Backquote",
};
const PUNCTUATION_BACK = Object.fromEntries(Object.entries(PUNCTUATION).map(([k, v]) => [v, k]));

const NAMED = ["Enter", "Escape", "Space", "Tab", "Backspace", "Delete", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown"];
const ALIASES: Record<string, string> = { esc: "Escape", up: "ArrowUp", down: "ArrowDown", left: "ArrowLeft", right: "ArrowRight", del: "Delete", return: "Enter", " ": "Space" };

/** Whether `code` is a key this module knows. */
function knownCode(code: string): boolean {
  return /^(Key[A-Z]|Digit[0-9]|F([1-9]|1[0-9]|2[0-4]))$/.test(code) || code in PUNCTUATION_BACK || NAMED.includes(code);
}

/** The code of a key as written (`K`, `/`, `ArrowUp`, `KeyK`); null for one this module does not know. */
function codeOfName(name: string): string | null {
  if (/^[a-z]$/i.test(name)) return `Key${name.toUpperCase()}`;
  if (/^[0-9]$/.test(name)) return `Digit${name}`;
  if (name in PUNCTUATION) return PUNCTUATION[name]!;
  const alias = ALIASES[name.toLowerCase()];
  if (alias) return alias;
  const named = NAMED.find((n) => n.toLowerCase() === name.toLowerCase());
  if (named) return named;
  if (/^f([1-9]|1[0-9]|2[0-4])$/i.test(name)) return name.toUpperCase();
  return knownCode(name) ? name : null;
}

/** The platform this page runs on. */
export function shortcutPlatform(): ShortcutPlatform {
  const nav = (globalThis as { navigator?: Navigator & { userAgentData?: { platform?: string } } }).navigator;
  const name = nav?.userAgentData?.platform || nav?.platform || "";
  return /mac|iphone|ipad|ipod/i.test(name) ? "mac" : "other";
}

/** A key as written, read for `platform`; null when it is not one (an unknown key, a modifier twice, no key). */
export function parseKeys(text: string, platform: ShortcutPlatform): Chord | null {
  if (typeof text !== "string" || !text) return null;
  // The last part is the key; `+` is never one (it is Shift+=).
  const parts = text.split("+");
  const key = parts.pop();
  if (!key) return null;
  const chord: Chord = { ctrl: false, alt: false, shift: false, meta: false, code: "" };
  for (const raw of parts) {
    const name = raw.trim().toLowerCase();
    const which =
      name === "mod" ? (platform === "mac" ? "meta" : "ctrl")
      : name === "ctrl" || name === "control" ? "ctrl"
      : name === "alt" || name === "option" || name === "opt" ? "alt"
      : name === "shift" ? "shift"
      : name === "meta" || name === "cmd" || name === "command" ? "meta"
      : null;
    if (!which || chord[which]) return null;
    chord[which] = true;
  }
  const code = codeOfName(key.trim() || key);
  if (!code) return null;
  chord.code = code;
  return chord;
}

/** The key a command has by default on `platform`; null when it has none there (or it is not a key). */
export function defaultKeys(keys: ShortcutKeys | null | undefined, platform: ShortcutPlatform): Chord | null {
  if (!keys) return null;
  const text = typeof keys === "string" ? keys : keys[platform];
  return text ? parseKeys(text, platform) : null;
}

/** A key written out the way `parseKeys` reads it back on any platform: `Ctrl+Alt+Shift+Meta+K`. */
export function writeKeys(chord: Chord): string {
  const key = /^Key[A-Z]$/.test(chord.code) ? chord.code.slice(3) : /^Digit[0-9]$/.test(chord.code) ? chord.code.slice(5) : (PUNCTUATION_BACK[chord.code] ?? chord.code);
  return [chord.ctrl && "Ctrl", chord.alt && "Alt", chord.shift && "Shift", chord.meta && "Meta", key].filter(Boolean).join("+");
}

/** The same key, whichever way it was written: what two keys are compared by. Also what the shell
 *  tells a frame its keys are (`Environment.keys` in `@harnix/ext-sdk`). */
export function chordId(chord: Chord): string {
  return [chord.ctrl && "Ctrl", chord.alt && "Alt", chord.shift && "Shift", chord.meta && "Meta", chord.code].filter(Boolean).join("+");
}

const MAC_KEYS: Record<string, string> = { ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→", Enter: "↩", Backspace: "⌫", Delete: "⌦", Tab: "⇥", Escape: "Esc", Space: "Space" };
const OTHER_KEYS: Record<string, string> = { ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→", Escape: "Esc" };

/** A key as a person reads it: `⌘K`, `⌥↑` on a Mac; `Ctrl+K`, `Alt+↑` elsewhere. */
export function keyLabel(chord: Chord, platform: ShortcutPlatform): string {
  const written = writeKeys(chord).split("+").pop()!;
  if (platform === "mac") {
    const key = MAC_KEYS[chord.code] ?? written;
    return `${chord.ctrl ? "⌃" : ""}${chord.alt ? "⌥" : ""}${chord.shift ? "⇧" : ""}${chord.meta ? "⌘" : ""}${key}`;
  }
  const key = OTHER_KEYS[chord.code] ?? written;
  return [chord.ctrl && "Ctrl", chord.alt && "Alt", chord.shift && "Shift", chord.meta && "Meta", key].filter(Boolean).join("+");
}

/** The key a key press is; null for a modifier on its own or a key this module does not know. The
 *  physical key when the event says it, else the one its character is on a US layout. */
export function chordOf(event: Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey">): Chord | null {
  let code = event.code && knownCode(event.code) ? event.code : null;
  if (!code && event.key && !["Control", "Alt", "Shift", "Meta", "OS"].includes(event.key)) code = codeOfName(event.key);
  if (!code) return null;
  return { ctrl: event.ctrlKey, alt: event.altKey, shift: event.shiftKey, meta: event.metaKey, code };
}

/** Keys the page leaves to the browser or to editing text, whoever asks: copy, paste, cut, select
 *  all, undo, redo, and the browser's own window and tab keys. */
const RESERVED = ["Mod+C", "Mod+V", "Mod+X", "Mod+A", "Mod+Z", "Mod+Shift+Z", "Mod+Y", "Mod+W", "Mod+T", "Mod+N", "Mod+Q", "Mod+R", "Mod+L"];

/** Why `chord` cannot be a command's key, as a text key; null when it can. A key needs Ctrl, ⌘
 *  or ⌥ (or is a function key): a bare letter would be taken from whoever is typing. Escape is
 *  the layers'. */
export function keysProblem(chord: Chord, platform: ShortcutPlatform): "shortcuts.problem.modifier" | "shortcuts.problem.reserved" | null {
  if (chord.code === "Escape" || RESERVED.some((r) => chordId(parseKeys(r, platform)!) === chordId(chord))) return "shortcuts.problem.reserved";
  if (!chord.ctrl && !chord.meta && !chord.alt && !/^F\d+$/.test(chord.code)) return "shortcuts.problem.modifier";
  return null;
}

/** Whether two commands' places can both be where a key is pressed: a global one is everywhere,
 *  the composer and the right column never both. */
export function scopesOverlap(a: ShortcutScope, b: ShortcutScope): boolean {
  return a === b || a === "global" || b === "global";
}

/** A command with the key it has now. */
export interface ShortcutBinding {
  id: string;
  scope: ShortcutScope;
  chord: Chord | null;
}

/** For each command, the others that have its key where it is in force too. Registration order
 *  decides which of them a press runs; every one of them is told about the others. */
export function shortcutConflicts(bindings: readonly ShortcutBinding[]): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const a of bindings) {
    if (!a.chord) continue;
    const id = chordId(a.chord);
    const others = bindings.filter((b) => b !== a && b.chord && chordId(b.chord) === id && scopesOverlap(a.scope, b.scope)).map((b) => b.id);
    if (others.length) out.set(a.id, others);
  }
  return out;
}

/**
 * The command a press runs: among those whose key it is and whose place holds the focus, a focused
 * place's before a global one, then the earliest registered. `scopes` are the places the focus is
 * in (always with `global`).
 */
export function pickShortcut<B extends ShortcutBinding>(bindings: readonly B[], chord: Chord, scopes: readonly ShortcutScope[]): B | null {
  const id = chordId(chord);
  const candidates = bindings.filter((b) => b.chord && chordId(b.chord) === id && scopes.includes(b.scope));
  return candidates.find((b) => b.scope !== "global") ?? candidates[0] ?? null;
}
