/**
 * The flows on the system space's page: what `flow.ts` hands over once the backend is known to have
 * the flow module (`registerSystemFlows`), drawn as the console's Flows tab. Nothing until then —
 * the tab is not there.
 */

import type { ReactNode } from "react";

import type { App } from "../app.ts";

export interface SystemFlowsTab {
  title(): string;
  render(): ReactNode;
}

const offered = new WeakMap<App, SystemFlowsTab>();

/** The console's Flows tab, from now on; the page is drawn again to show it. */
export function offerSystemFlows(app: App, tab: SystemFlowsTab): void {
  offered.set(app, tab);
  app.redraw();
}

/** The console's Flows tab, if the flows are in this page's system space. */
export function systemFlows(app: App): SystemFlowsTab | null {
  return offered.get(app) ?? null;
}
