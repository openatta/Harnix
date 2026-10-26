/**
 * 界面文案（设计 §10.1：只用 key，不写死文案）。两本词典的键相同、占位符相同，由测试守着；
 * 缺的键先回落英文，再回落成键本身，只告警一次。
 *
 * 语言的选择：localStorage `harnix.locale`，否则浏览器语言里第一个 `zh*` 就是简体中文，再否则英文。
 * 组件经 `useT()` 订阅，换语言时整页重画。
 */

import { useSyncExternalStore } from "react";
import en from "./en.ts";
import zhCN from "./zh-CN.ts";

export type Locale = "zh-CN" | "en";
export type LocaleChoice = Locale | "system";
export type Translate = (key: string, vars?: Record<string, unknown>) => string;

export const DICTIONARIES: Record<Locale, Record<string, string>> = { "zh-CN": zhCN, en };
const KEY = "harnix.locale";

const warned = new Set<string>();
const listeners = new Set<() => void>();
let locale: Locale = resolveInitial();

function stored(): Locale | null {
  try {
    const value = localStorage.getItem(KEY);
    return value === "zh-CN" || value === "en" ? value : null;
  } catch {
    return null;
  }
}

function fromSystem(): Locale {
  const languages = typeof navigator === "undefined" ? [] : navigator.languages ?? [navigator.language];
  return languages.some((l) => l?.toLowerCase().startsWith("zh")) ? "zh-CN" : "en";
}

function resolveInitial(): Locale {
  return stored() ?? fromSystem();
}

/**
 * A module's language pack (design §10.5): keys it brings for its own text, merged into the
 * dictionaries. Every key carries the module's namespace — `<owner>.` — so a pack can neither
 * rewrite the shell's text nor another module's; keys outside it are returned, not added.
 */
export function addStrings(locale: Locale, strings: Record<string, string>, owner: string): string[] {
  const refused: string[] = [];
  for (const [key, text] of Object.entries(strings)) {
    if (!key.startsWith(`${owner}.`) || typeof text !== "string") refused.push(key);
    else DICTIONARIES[locale][key] = text;
  }
  notify();
  return refused;
}

export function t(key: string, vars?: Record<string, unknown>): string {
  let text = DICTIONARIES[locale][key] ?? DICTIONARIES.en[key];
  if (text === undefined) {
    if (!warned.has(key)) {
      warned.add(key);
      console.warn(`[i18n] missing key: ${key}`);
    }
    return key;
  }
  if (vars) {
    text = text.replace(/\{(\w+)\}/g, (whole, name: string) =>
      vars[name] === undefined ? whole : String(vars[name]),
    );
  }
  return text;
}

export function currentLocale(): Locale {
  return locale;
}

export function localeChoice(): LocaleChoice {
  return stored() ?? "system";
}

export function setLocale(choice: LocaleChoice): void {
  try {
    if (choice === "system") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, choice);
  } catch {
    /* private mode: the choice does not survive a reload */
  }
  const next = choice === "system" ? fromSystem() : choice;
  if (next === locale) return;
  locale = next;
  if (typeof document !== "undefined") document.documentElement.lang = locale;
  notify();
}

/** Bumped whenever what `t` answers may have changed: a language, or a module's pack. */
let version = 0;

function notify(): void {
  version += 1;
  for (const fn of [...listeners]) fn();
}

function subscribe(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** `t`, re-rendering the component when the language changes or a pack arrives. */
export function useT(): Translate {
  useSyncExternalStore(subscribe, () => version);
  return t;
}

export function useLocale(): Locale {
  return useSyncExternalStore(subscribe, currentLocale);
}
