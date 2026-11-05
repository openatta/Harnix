/**
 * The flow space (xops-02 §7): built in, but only where the backend has the flow module.
 *
 * What the page knows at start tells it: `harnix.reachable` lists `app.flow.defs` when the module
 * is registered (under the id `flow`, as the empty application and the SDK's builder do), and the
 * space whose conversations run the module's `flow-author` scene is the one in
 * `harnix.spaces.list` with that scene — `flow` unless the application renamed it. A profile that
 * leaves `flow` out of `[shell] spaces` does not get it.
 */

import { registerFlowSpace } from "@harnix/flow";

import type { App } from "./app.ts";
import { hostFor } from "./host.ts";

/** The method whose presence says the backend has the flow module. */
export const FLOW_PROBE = "app.flow.defs";
/** The scene the flow module gives its space's conversations. */
export const FLOW_AUTHOR_SCENE = "flow-author";

interface SpacesList {
  spaces?: { id: string; scene?: string | null }[];
}

/**
 * The id of the backend space the flow space shows, or null when there is none: no flow module,
 * or no space running `flow-author`. Asking for the list failing is not knowing: `flow`, the
 * module's default.
 */
export async function flowSpaceOf(reachable: string[], list: () => Promise<unknown>): Promise<string | null> {
  if (!reachable.includes(FLOW_PROBE)) return null;
  if (!reachable.includes("harnix.spaces.list")) return "flow";
  let listed: SpacesList | null;
  try {
    listed = (await list()) as SpacesList | null;
  } catch {
    return "flow";
  }
  return listed?.spaces?.find((s) => s.scene === FLOW_AUTHOR_SCENE)?.id ?? null;
}

/**
 * Register the flow space when the backend has it and the profile keeps it — `flow` in `[shell]
 * spaces` names the built-in, whatever id the application gave its backend space.
 */
export async function registerFlow(app: App, keeps: (space: string) => boolean): Promise<void> {
  if (!keeps("flow")) return;
  const space = await flowSpaceOf(app.snapshot.reachable, () => app.call("harnix.spaces.list", {}));
  if (space === null) return;
  registerFlowSpace(app.registry, { space, host: hostFor(app) });
}
