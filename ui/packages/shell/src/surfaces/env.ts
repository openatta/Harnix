/**
 * The shell's environment as a frame sees it (`Environment`, `harnix-ui-bridge@1`): the theme in
 * effect, the design tokens' values in it, the language, whether the window is narrow. Read off
 * the page itself — `<html>`'s `data-theme`, its token overrides (`style`), its `lang` — and
 * followed with one observer, so a theme switch, a brand's colours or a language change reaches
 * every frame the moment the page has it, without the shell telling anyone.
 *
 * `visible` and `modal` are each frame's own (`bridge.ts`).
 */

import { currentLocale } from "@harnix/react";
import type { Environment } from "@harnix/app-sdk/protocol";

import { themeTokens } from "../brand.ts";

export type ShellEnvironment = Omit<Environment, "visible" | "modal">;

const NARROW = "(max-width: 960px)";

let names: string[] = [];
let cached: { key: string; env: ShellEnvironment } | null = null;

/** The environment now. Token values are worked out once per theme and set of overrides. */
export function shellEnvironment(): ShellEnvironment {
  const root = document.documentElement;
  const theme: ShellEnvironment["theme"] = root.dataset.theme === "dark" ? "dark" : "light";
  const narrow = typeof matchMedia === "function" && matchMedia(NARROW).matches;
  const locale = currentLocale();
  const key = `${theme}|${root.getAttribute("style") ?? ""}`;
  if (!cached || cached.key !== key) {
    if (!names.length) names = themeTokens();
    const style = getComputedStyle(root);
    const tokens: Record<string, string> = {};
    for (const name of names) {
      const value = style.getPropertyValue(name).trim();
      if (value) tokens[name] = value;
    }
    cached = { key, env: { theme, tokens, locale, narrow } };
  }
  return { ...cached.env, locale, narrow };
}

const listeners = new Set<() => void>();
let watching: (() => void) | null = null;

function watch(): () => void {
  let queued = false;
  const changed = () => {
    if (queued) return;
    queued = true;
    // After the change has settled (a theme switch sets the attribute, then the overrides).
    requestAnimationFrame(() => {
      queued = false;
      for (const fn of [...listeners]) fn();
    });
  };
  const observer = new MutationObserver(changed);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "style", "lang"] });
  const media = typeof matchMedia === "function" ? matchMedia(NARROW) : null;
  media?.addEventListener("change", changed);
  document.addEventListener("visibilitychange", changed);
  return () => {
    observer.disconnect();
    media?.removeEventListener("change", changed);
    document.removeEventListener("visibilitychange", changed);
  };
}

/** Be told when the environment may have changed. */
export function onShellEnvironment(fn: () => void): () => void {
  listeners.add(fn);
  watching ??= watch();
  return () => {
    listeners.delete(fn);
    if (!listeners.size && watching) {
      watching();
      watching = null;
    }
  };
}
