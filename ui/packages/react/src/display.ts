/**
 * How this browser draws a conversation: the preferences the settings page's display section
 * sets, read by the chat components. In memory, kept in `localStorage` under one key and
 * applied live — the type scale and the families through the tokens (`--font-size-base`,
 * `--font-sans`, `--font-code`), the rest as the components read them at their next draw.
 * Browser-local: none of it goes to the engine.
 */

import { useSyncExternalStore } from "react";

export type UsageDetail = "none" | "compact" | "full";
export type LinkOpening = "same" | "new" | "ask";
export type BusyEnter = "queue" | "steer";

export interface Display {
  /** The type scale's base, in px. */
  fontSize: number;
  /** A font-family list for the text; blank is the product's. */
  fontBody: string;
  /** The same, for code. */
  fontCode: string;
  /** A tool's details are open when it is first drawn. */
  toolsOpen: boolean;
  /** A finished thought is folded when it is first drawn. */
  thinkingFolded: boolean;
  /** What a conversation says about the tokens it used. */
  usage: UsageDetail;
  /** What a link in a message does. */
  links: LinkOpening;
  /** Enter while a turn runs: wait for the turn to end, or go in at the next step. */
  enter: BusyEnter;
}

export const DISPLAY_DEFAULTS: Display = {
  fontSize: 14,
  fontBody: "",
  fontCode: "",
  toolsOpen: false,
  thinkingFolded: true,
  usage: "full",
  links: "new",
  enter: "queue",
};

export const FONT_SIZE_RANGE = { min: 11, max: 20 } as const;

const KEY = "harnix.display";

/** Whether `value` is one a key accepts; what is stored is the browser's, not trusted. */
export function accepts<K extends keyof Display>(key: K, value: unknown): value is Display[K] {
  switch (key) {
    case "fontSize":
      return typeof value === "number" && Number.isInteger(value) && value >= FONT_SIZE_RANGE.min && value <= FONT_SIZE_RANGE.max;
    case "fontBody":
    case "fontCode":
      // A family list goes into a style declaration: nothing that could end it or open another.
      return typeof value === "string" && value.length <= 200 && !/[;{}<>\\]/.test(value);
    case "toolsOpen":
    case "thinkingFolded":
      return typeof value === "boolean";
    case "usage":
      return value === "none" || value === "compact" || value === "full";
    case "links":
      return value === "same" || value === "new" || value === "ask";
    case "enter":
      return value === "queue" || value === "steer";
  }
  return false;
}

function stored(): Partial<Display> {
  try {
    const raw = JSON.parse(globalThis.localStorage?.getItem(KEY) ?? "{}") as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(DISPLAY_DEFAULTS) as (keyof Display)[]) if (accepts(key, raw[key])) out[key] = raw[key];
    return out as Partial<Display>;
  } catch {
    return {};
  }
}

let current: Display = { ...DISPLAY_DEFAULTS, ...stored() };
const listeners = new Set<() => void>();

/** The type tokens on the document: set only where they differ from the product's. */
export function applyTypography(d: Display = current, root: HTMLElement | null = typeof document === "undefined" ? null : document.documentElement): void {
  if (!root) return;
  const put = (name: string, value: string | null) => (value ? root.style.setProperty(name, value) : root.style.removeProperty(name));
  put("--font-size-base", d.fontSize === DISPLAY_DEFAULTS.fontSize ? null : `${d.fontSize}px`);
  put("--font-sans", d.fontBody.trim() || null);
  put("--font-code", d.fontCode.trim() || null);
}
applyTypography();

export function display(): Display {
  return current;
}

/** Change some of them (a value a key does not accept is ignored), keep them, and draw again. */
export function setDisplay(patch: Partial<Display>): void {
  const next: Display = { ...current };
  for (const key of Object.keys(patch) as (keyof Display)[]) {
    const value = patch[key];
    if (accepts(key, value)) (next as unknown as Record<string, unknown>)[key] = value;
  }
  current = next;
  try {
    const differing = Object.fromEntries(Object.entries(next).filter(([k, v]) => v !== DISPLAY_DEFAULTS[k as keyof Display]));
    if (Object.keys(differing).length) globalThis.localStorage?.setItem(KEY, JSON.stringify(differing));
    else globalThis.localStorage?.removeItem(KEY);
  } catch {
    /* private mode: the choice does not survive a reload */
  }
  applyTypography(next);
  for (const fn of [...listeners]) fn();
}

export function subscribeDisplay(fn: () => void): () => void {
  listeners.add(fn);
  return () => void listeners.delete(fn);
}

/** The preferences, drawn again when one changes. */
export function useDisplay(): Display {
  return useSyncExternalStore(subscribeDisplay, display);
}
