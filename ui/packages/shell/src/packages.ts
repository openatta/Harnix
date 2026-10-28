/**
 * Installed packages' interface modules (design §10.5, contribution API 2).
 *
 * A v2 module is a precompiled ES module the page imports from its own origin
 * (`/plugins/<package>/ui/<file>`). It brings no React of its own: `react`, `react-dom`,
 * `react/jsx-runtime` and `@harnix/react` resolve through the page's import map to the copies the
 * shell itself runs on, so hooks and the host's context are the shell's. It exports
 * `register(registry, host)` like a compiled-in module, through a registry that signs everything
 * with the package's name — a package cannot register as `builtin`, or bring strings outside its
 * own namespace.
 *
 * The backend sends only what will take: entries for another contribution API are listed as not
 * taking in the plugins section, and with `[ui] runtime_modules = false` nothing is sent at all.
 */

import type { Hello } from "@harnix/core";
import { CONTRIB_API_VERSION } from "@harnix/core/protocol";
import { t, type Locale, type NestHost, type Registry } from "@harnix/react";

import type { App } from "./app.ts";
import { hostFor } from "./host.ts";

interface Contribution {
  plugin?: string;
  point?: string;
  module?: string;
  api_version?: number | null;
}

/** What a package module's `register` is handed instead of the registry itself. */
function signed(registry: Registry, owner: string): Registry {
  const facade = {
    register: (point: string, contribution: unknown) => registry.register(point, contribution, owner),
    tokens: (tokens: Record<string, string>) => registry.tokens(tokens, owner),
    strings: (locale: Locale, dictionary: Record<string, string>) => registry.strings(locale, dictionary, owner),
    icons: (icons: Record<string, string>) => registry.icons(icons, owner),
    icon: registry.icon.bind(registry),
    at: registry.at.bind(registry),
  };
  return facade as unknown as Registry;
}

export async function loadPackages(app: App, hello: Hello): Promise<void> {
  const entries = (Array.isArray(hello.contributions) ? hello.contributions : []) as Contribution[];
  const loaded = new Set<string>();
  for (const entry of entries) {
    const owner = entry.plugin ?? "?";
    const module = entry.module ?? "";
    if (entry.api_version !== CONTRIB_API_VERSION) {
      app.registry.note({ point: entry.point ?? "?", owner, id: module, reason: t("contrib.v1Inert") });
      continue;
    }
    // A package's file is listed once per point it declares; it registers once.
    if (!module.startsWith("/plugins/") || loaded.has(module)) continue;
    loaded.add(module);
    try {
      const imported = (await import(/* @vite-ignore */ module)) as { register?: (r: Registry, host: NestHost) => void };
      if (typeof imported.register !== "function") throw new Error(t("contrib.noRegister"));
      imported.register(signed(app.registry, owner), hostFor(app, owner));
    } catch (e) {
      app.registry.note({ point: entry.point ?? "module", owner, id: module, reason: e instanceof Error ? e.message : String(e) });
    }
  }
}
