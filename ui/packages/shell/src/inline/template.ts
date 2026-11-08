/**
 * The words of an application's declared inline points (`[inline]` in its manifest): a text is a
 * string or one per locale, and may hold `{root.path}` placeholders filled from what the point
 * draws — a tool call's `input` and `result`, a card's entry `payload`, a button's `result`.
 *
 * Everything filled in is text: it goes into the page through React, never as markup. A card's
 * body is the one Markdown, rendered by the shell's own renderer (which escapes first).
 */

import { currentLocale } from "@harnix/react";

/** `"…"`, or `{ en: "…", "zh-CN": "…" }`. */
export type Text = string | Record<string, string>;

/** A text in the reader's locale: theirs, else English, else the first given. */
export function localized(text: Text | null | undefined): string {
  if (text == null) return "";
  if (typeof text === "string") return text;
  return text[currentLocale()] ?? text.en ?? Object.values(text)[0] ?? "";
}

/** The value at `a.b.c` in `scope`. */
export function lookup(scope: Record<string, unknown>, path: string): unknown {
  let at: unknown = scope;
  for (const key of path.split(".")) {
    if (at == null || typeof at !== "object") return undefined;
    at = (at as Record<string, unknown>)[key];
  }
  return at;
}

/** A value as words: nothing for null, JSON for an object. */
export function words(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return "";
  }
}

const PLACEHOLDER = /\{([A-Za-z0-9_.-]+)\}/g;

/** Every `{path}` in `template`, filled from `scope`. */
export function fill(template: string, scope: Record<string, unknown>): string {
  return template.replace(PLACEHOLDER, (_, path: string) => words(lookup(scope, path)));
}

/** A localized text, filled. */
export function say(text: Text | null | undefined, scope: Record<string, unknown>): string {
  return fill(localized(text), scope);
}

/**
 * A button's params: every string filled; a string that is a single placeholder becomes the value
 * itself, so `{payload.count}` stays a number.
 */
export function fillValue(value: unknown, scope: Record<string, unknown>): unknown {
  if (typeof value === "string") {
    const whole = /^\{([A-Za-z0-9_.-]+)\}$/.exec(value);
    return whole ? (lookup(scope, whole[1]!) ?? null) : fill(value, scope);
  }
  if (Array.isArray(value)) return value.map((v) => fillValue(v, scope));
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, fillValue(v, scope)]));
  }
  return value;
}

/** A tool call's input as an object: it may arrive as JSON text. */
export function inputOf(input: unknown): unknown {
  if (typeof input !== "string") return input;
  try {
    return JSON.parse(input);
  } catch {
    return input;
  }
}
