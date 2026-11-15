/**
 * The flow module's interface as a UI package: what the shell activates while the backend's `flow`
 * extension is enabled.
 *
 * The flows are an application's toolchain, so where a console of the process is defined
 * (`system.console.tab`) they are a tab of it, and a flow clicked there is a space shown inside the
 * console's space. Where nobody defines that point, they are a space of their own (`orElse`). Which
 * one is decided by whether the point is defined, not by which extensions run.
 */

import { t, type HostApi, type Registry } from "@harnix/react";

import { registerFlowSpace, registerSystemFlows } from "./space";

/** The backend extension this is the interface of. */
export const extension = "flow";

/** The point a console tab goes into, and the backend space the flows' conversations run in (the
 *  module's default; an application that renames it registers `registerFlowSpace` itself). */
const CONSOLE = "system.console.tab";
const SPACE = "flow";

export function register(r: Registry, host: HostApi): void {
  const tab = { id: "flow.flows", order: 20, title: () => t("flowui.tab.flows"), about: () => t("flowui.about.flows") };
  // The console is there already: the flows go inside the space that defines it.
  const definer = r.definitions().find((d) => d.name === CONSOLE)?.owner;
  if (definer !== undefined) {
    const view = registerSystemFlows(r, { space: SPACE, within: definer, host });
    r.register(CONSOLE, { ...tab, render: view.render }, extension);
    return;
  }
  // Not yet: if it is by the time every package has registered, the tab is drawn empty (its space
  // came after this one); if it never is, the flows are a space of their own.
  r.register(CONSOLE, { ...tab, render: () => null }, extension, { orElse: () => void registerFlowSpace(r, { space: SPACE, host }) });
}
