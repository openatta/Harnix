/**
 * The UI packages compiled into the shell (`virtual:harnix-modules`), activated by what is enabled.
 *
 * A package that names the backend extension it is the interface of (`export const extension`)
 * registers only while that extension is enabled: at hello, synchronously, before the first draw;
 * later as `harnix.ui` says it came up; and when it goes down, everything its `register` did goes
 * with it, as one batch. A package that names none — the application's own — always registers.
 * The shell knows no package by name: which runs is the backend's to say.
 */

import type { ExtensionEntry } from "@harnix/core";
import type { HostApi, Registry } from "@harnix/react";

import type { App } from "./app.ts";

/** One compiled-in UI package. */
export interface UiPackage {
  /** The backend extension it is the interface of; absent: always registered. */
  extension?: string;
  register?: (registry: Registry, host: HostApi) => void;
}

/** Whether an enabled extension is drawn by this package: its `bundle`, else its own id. */
function wanted(extensions: readonly ExtensionEntry[], extension: string): boolean {
  return extensions.some((e) => e.enabled && (e.bundle ?? e.id) === extension);
}

/** The packages active on this page. */
const active = new WeakMap<App, Set<UiPackage>>();

function run(app: App, host: HostApi, pkg: UiPackage, batch: string, owner: string): void {
  const register = pkg.register;
  if (typeof register !== "function") return;
  try {
    app.registry.activate(batch, () => register(app.registry, host));
  } catch (e) {
    app.registry.note({ point: "module", owner, reason: e instanceof Error ? e.message : String(e) });
  }
}

/**
 * Bring the compiled-in packages in step with `extensions`: register the ones whose extension is
 * enabled and are not active yet (and, the first time, the ones that name none), withdraw the ones
 * whose extension is no longer. Answers whether anything changed; the caller settles the registry.
 */
export function activate(app: App, host: HostApi, packages: readonly UiPackage[], extensions: readonly ExtensionEntry[]): boolean {
  let on = active.get(app);
  const first = !on;
  if (!on) active.set(app, (on = new Set()));
  let changed = false;
  for (const pkg of packages) {
    const ext = pkg.extension;
    if (typeof ext !== "string") {
      if (first) run(app, host, pkg, "application", "application");
      continue;
    }
    const want = wanted(extensions, ext);
    if (want && !on.has(pkg)) {
      on.add(pkg);
      run(app, host, pkg, ext, ext);
      changed = true;
    } else if (!want && on.has(pkg)) {
      on.delete(pkg);
      app.registry.withdraw(ext);
      changed = true;
    }
  }
  return changed || first;
}
