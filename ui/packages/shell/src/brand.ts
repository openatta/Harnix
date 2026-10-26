/**
 * The product's look (design §10.5): `[brand] name` in the page title, and the token overrides
 * modules registered, set on `<html>` over the theme's values. An application changes its look
 * this way without forking `ui/`; the stylesheets themselves stay one set.
 *
 * Overrides are set through the CSSOM (`style.setProperty`), which the page's CSP permits —
 * nothing inline is written into the document.
 */

import { useEffect } from "react";
import { useRegistry, useT } from "@harnix/react";

import type { App } from "./app.ts";
import { useShell } from "./hooks.ts";
import { brandName } from "./Sidebar.tsx";

/** The custom properties the theme declares on `:root`: the names an override may use. */
export function themeTokens(): string[] {
  const names = new Set<string>();
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try {
      rules = sheet.cssRules;
    } catch {
      continue; // not ours to read
    }
    for (const rule of Array.from(rules)) {
      if (!(rule instanceof CSSStyleRule) || !rule.selectorText.split(",").some((s) => s.trim() === ":root")) continue;
      for (const name of Array.from(rule.style)) if (name.startsWith("--")) names.add(name);
    }
  }
  return [...names];
}

export function useBrand(app: App): void {
  const t = useT();
  const s = useShell(app);
  const registry = useRegistry(app.registry);
  const name = brandName(app, t);
  useEffect(() => {
    document.title = name;
  }, [name]);
  useEffect(() => {
    registry.knowTokens(themeTokens());
  }, [registry]);
  const overrides = registry.tokenOverrides();
  const key = JSON.stringify(overrides);
  useEffect(() => {
    const root = document.documentElement;
    for (const [token, value] of Object.entries(overrides)) root.style.setProperty(token, value);
    return () => {
      for (const token of Object.keys(overrides)) root.style.removeProperty(token);
    };
    // `key` stands for the overrides' contents.
  }, [key, s.theme]);
}
