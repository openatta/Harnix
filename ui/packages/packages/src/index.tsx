/**
 * @harnix/packages — the `packages` extension's interface: the settings section where extension
 * packages are installed, updated, consented to, switched, uninstalled and cleared, and the
 * system console's tab of how each one runs (where a space defines `system.console.tab`; a console
 * nobody defines drops the tab quietly, and the section is where the packages are then).
 */

import { t, type HostApi, type Registry } from "@harnix/react";

import { PackagesTab } from "./console.tsx";
import { PackagesSettings } from "./settings.tsx";
import "./packages.css";

/** The backend extension this is the interface of. */
export const extension = "packages";

export function register(r: Registry, host: HostApi): void {
  r.register("settings.section", { id: "packages", label: () => t("settings.nav.packages"), render: () => <PackagesSettings host={host} /> }, extension);
  r.register(
    "system.console.tab",
    { id: "packages", order: 30, title: () => t("system.packages"), about: () => t("system.about.packages"), render: () => <PackagesTab host={host} /> },
    extension,
    { orElse: () => undefined },
  );
}
