/**
 * Interface modules that come with what is installed (concept §5.8, contribution API 3): an engine
 * package's `[[ui]]` modules, which the `plugins` extension serves at `/apps/plugins/ui/<package>/
 * <file>`, and a runtime application's at `/apps/<id>/ui/<file>`. `harnix.ui` lists them, and
 * this follows it: a module that appears is imported and registers; one whose package goes takes
 * everything it registered with it, as one batch.
 *
 * A module is a precompiled ES module the page imports from its own origin, with the page's
 * credential on its address (those routes want one, and an `import()` takes no header). It brings
 * no React of its own: `react`, `react-dom`, `react/jsx-runtime` and `@harnix/react` resolve
 * through the page's import map to the copies the shell itself runs on, so hooks and the host's
 * context are the shell's. It exports `register(registry, host)` like a compiled-in module,
 * through a registry that signs everything with the package's name — a package cannot register as
 * `builtin`, or bring strings outside its own namespace.
 *
 * The backend sends only what will take: entries for another contribution API are listed as not
 * taking in the plugins section, and with `[ext.plugins] ui_modules = false` nothing is sent at all.
 */

import type { UiView } from "@harnix/core";
import { CONTRIB_API_VERSION } from "@harnix/core/protocol";
import { t, type Locale, type HostApi, type RegisterOptions, type Registry } from "@harnix/react";

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
    register: (point: string, contribution: unknown, _owner?: string, options?: RegisterOptions) => registry.register(point, contribution, owner, options),
    definePoint: (name: string, spec: { kind: "register" | "render" | "fold" }) => registry.definePoint(name, spec, owner),
    list: registry.list.bind(registry),
    tokens: (tokens: Record<string, string>) => registry.tokens(tokens, owner),
    strings: (locale: Locale, dictionary: Record<string, string>) => registry.strings(locale, dictionary, owner),
    icons: (icons: Record<string, string>) => registry.icons(icons, owner),
    icon: registry.icon.bind(registry),
    at: registry.at.bind(registry),
  };
  return facade as unknown as Registry;
}

/** The modules this page imported, by address, and whose each is. */
const loaded = new WeakMap<App, Map<string, string>>();

/** The modules `harnix.ui` lists now: imported if new; a package none of whose modules is listed any more, withdrawn. */
export async function followModules(app: App, view: UiView): Promise<void> {
  const entries = (Array.isArray(view.contributions) ? view.contributions : []) as Contribution[];
  let mine = loaded.get(app);
  if (!mine) loaded.set(app, (mine = new Map()));
  const listed = new Map<string, string>();
  for (const entry of entries) {
    const owner = entry.plugin ?? "?";
    const module = entry.module ?? "";
    // A frame's entry, or one refused in the page: the surfaces' to handle.
    if (!module) continue;
    if (entry.api_version !== CONTRIB_API_VERSION) {
      app.registry.note({ point: entry.point ?? "?", owner, id: module, reason: t("contrib.v1Inert") });
      continue;
    }
    // Only this origin's module routes.
    if (!module.startsWith("/apps/")) continue;
    listed.set(module, owner);
  }
  // Gone: its package takes back everything it registered, once none of its modules is left.
  const owners = new Set(listed.values());
  for (const [module, owner] of [...mine]) {
    if (listed.has(module)) continue;
    mine.delete(module);
    if (!owners.has(owner)) app.registry.withdraw(owner);
  }
  for (const [module, owner] of listed) {
    // A package's file is listed once per point it declares; it registers once.
    if (mine.has(module)) continue;
    mine.set(module, owner);
    try {
      const imported = (await import(/* @vite-ignore */ app.credentialed(module))) as { register?: (r: Registry, host: HostApi) => void };
      if (typeof imported.register !== "function") throw new Error(t("contrib.noRegister"));
      const register = imported.register;
      app.registry.activate(owner, () => register(signed(app.registry, owner), hostFor(app, owner)));
    } catch (e) {
      app.registry.note({ point: entries.find((x) => x.module === module)?.point ?? "module", owner, id: module, reason: e instanceof Error ? e.message : String(e) });
    }
  }
}
