/**
 * @harnix/packages — the `packages` extension's interface: the settings section where extension
 * packages are installed, updated, consented to, switched, uninstalled and cleared. How each one
 * runs (`PackagesTab`: its state, calls, a process's last lines of stderr) is a component for
 * whoever has a page for it.
 */

import { t, type HostApi, type Registry } from "@harnix/react";

import { PackagesSettings } from "./settings.tsx";
import "./packages.css";

export { PackagesTab } from "./console.tsx";

/** The backend extension this is the interface of. */
export const extension = "packages";

export function register(r: Registry, host: HostApi): void {
  r.register("settings.section", { id: "packages", label: () => t("settings.nav.packages"), render: () => <PackagesSettings host={host} /> }, extension);
}
