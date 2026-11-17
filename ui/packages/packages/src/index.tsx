/**
 * @harnix/packages — the `packages` extension's interface: the settings section where extension
 * packages are installed, updated, consented to, switched, uninstalled and cleared; and how each one
 * runs (`PackagesTab`: its state, calls, a process's last lines of stderr), a block of the settings'
 * diagnostics (`settings.section` `within: "diagnostics"`).
 */

import { t, type HostApi, type Registry } from "@harnix/react";

import { PackagesTab } from "./console.tsx";
import { PackagesSettings } from "./settings.tsx";
import "./packages.css";

export { PackagesTab } from "./console.tsx";

/** The backend extension this is the interface of. */
export const extension = "packages";

/** The diagnostics' block of the packages as they run: what `host.settings.open` names to show it. */
export const RUNTIME = "packages.runtime";

export function register(r: Registry, host: HostApi): void {
  r.register("settings.section", { id: "packages", label: () => t("settings.nav.packages"), render: () => <PackagesSettings host={host} /> }, extension);
  r.register("settings.section", { id: RUNTIME, within: "diagnostics", label: () => t("packages.runtime"), render: () => <PackagesTab host={host} /> }, extension);
}
